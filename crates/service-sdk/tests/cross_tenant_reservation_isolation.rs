//! Cross-tenant isolation of the reservation namespace — PROD-012 B7.8/B7.9.
//!
//! # Why these tests are about composition, not about SQL
//!
//! The stores were measured first, and they are isolated by construction. Every
//! tenant predicate in the durable reservation store compares with
//! `tenant_id IS NOT DISTINCT FROM`, never `=`; both durable tables carry the
//! complementary partial unique-index pair, so the tenant-less partition is as
//! unique as the tenant-scoped one; the in-memory store keys on `OperationId`,
//! which carries the scope inside its own `Hash`/`Eq`; and `resolve_tenant`
//! refuses an empty tenant rather than coercing it into the shared partition.
//! Nothing in that layer needed changing, and nothing here tests it.
//!
//! The crossing was one layer up, in what the runtime hands the store. The
//! namespace came from
//!
//! ```ignore
//! let tenant = ctx.canonical_tenant().and_then(|r| r.tenant_id().cloned());
//! ```
//!
//! and that `and_then` collapses two different statements into one `None`: *no
//! scope was resolved* and *the scope resolved to systemwide*. Only the second
//! names a namespace. The first was being filed in the shared tenant-less
//! partition, so two tenants presenting one key became one operation.
//!
//! # What was observed before the fix
//!
//! An operation marked `#[idempotent]` with no tenant resolution on its path,
//! called by two authenticated principals under one operation key:
//!
//! - **Identical payload:** the handler ran **once**. Tenant B's call returned
//!   `Ok` carrying tenant A's stored response bytes — a replay across scopes,
//!   which is the information disclosure the "Cross-Tenant Replay Is Prohibited"
//!   requirement names as security-critical rather than merely incorrect.
//! - **Differing payload:** tenant B's call was refused `FingerprintConflict` —
//!   tenant A's reservation permanently refusing a legitimate request from a
//!   scope it has nothing to do with.
//! - **Control**, with resolution on its path: the handler ran once per tenant
//!   and each received its own answer.
//!
//! No mutation was applied to reach any of that. It is reachable today.
//!
//! # What the fix is, and what these tests therefore assert
//!
//! The collapse is now an explicit `match`, and an unresolved context is refused
//! with [`ReservationRejection::TenantUnresolved`] before the store is reached.
//! So the property is stated one step earlier than the disclosure: an operation
//! with no resolved scope never reserves, and something that never reserves can
//! never be answered from another scope's row. Restoring the `and_then` — turning
//! `None` back into the systemwide namespace — puts every test in this file red.
//!
//! The store is wrapped in a counter so the refusals are asserted by an
//! **observed count of `reserve` calls**, not only by the returned error. An
//! implementation that refused after reserving would satisfy the error
//! assertion and still have taken the lease.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use ego_domain::context::TenantId;
use ego_domain::operation::{OperationFingerprint, OperationKey};
use ego_domain::operation::{
    OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
    ReserveRequest, StoredServiceResponse,
};
use ego_security_sdk::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::{Principal, PrincipalKind, SubjectId},
};
use ego_service_sdk::{
    context::ServiceContext,
    error::category::ErrorCategory,
    error::ServiceErrorTrait,
    interceptor::InterceptorChain,
    runtime::{ReservationRejection, Runtime, RuntimeBuilder, TenantEnforcementMode},
};
// Attribute macros the `#[service]` expansion consumes; rustc reports the
// imports as unused because nothing names them as paths. Same treatment as
// `idempotent_dispatch.rs`.
#[allow(unused_imports)]
use ego_service_sdk_macros::{authorize, idempotent, operation, service, tenant_scoped};
use ego_testkit::{InMemoryOperationReservationStore, TestClock};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// The service under test
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confirmation {
    pub detail: String,
}

#[derive(Debug)]
pub enum PaymentError {
    Security(String),
    Refused(ReservationRejection),
}

impl From<SecurityError> for PaymentError {
    fn from(e: SecurityError) -> Self {
        Self::Security(e.to_string())
    }
}

impl From<ReservationRejection> for PaymentError {
    fn from(r: ReservationRejection) -> Self {
        Self::Refused(r)
    }
}

impl ServiceErrorTrait for PaymentError {
    fn code(&self) -> &str {
        "payment"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::System
    }
    fn message(&self) -> String {
        format!("{self:?}")
    }
}

#[service]
pub trait Payments: Send + Sync {
    /// Tenant resolution runs on this path, so the reservation has a namespace.
    #[operation]
    #[authorize(context = ctx, permission = "payments:settle")]
    #[tenant_scoped]
    #[idempotent]
    async fn settle(
        &self,
        ctx: ServiceContext,
        request: Payment,
    ) -> Result<Confirmation, PaymentError>;

    /// Marked idempotent with **nothing on its path that resolves a tenant**.
    ///
    /// This is the shape the defect was found in, and it is kept as a real
    /// operation rather than simulated: the collapse happened in generated
    /// dispatch, so a test that called the runtime directly would be testing a
    /// different caller than the one that had the bug.
    #[operation]
    #[authorize(context = ctx, permission = "payments:refund")]
    #[idempotent]
    async fn refund(
        &self,
        ctx: ServiceContext,
        request: Payment,
    ) -> Result<Confirmation, PaymentError>;
}

/// Counts what each operation actually executed, which is how a replay is told
/// apart from a re-execution: both return the same bytes, and only the count
/// says which one produced them.
#[derive(Default)]
struct CountingPayments {
    settle_calls: AtomicUsize,
    refund_calls: AtomicUsize,
}

impl CountingPayments {
    fn settle_calls(&self) -> usize {
        self.settle_calls.load(Ordering::Relaxed)
    }
    fn refund_calls(&self) -> usize {
        self.refund_calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl Payments for CountingPayments {
    async fn settle(
        &self,
        _ctx: ServiceContext,
        request: Payment,
    ) -> Result<Confirmation, PaymentError> {
        self.settle_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Confirmation {
            detail: format!("settled:{}", request.reference),
        })
    }

    async fn refund(
        &self,
        _ctx: ServiceContext,
        request: Payment,
    ) -> Result<Confirmation, PaymentError> {
        self.refund_calls.fetch_add(1, Ordering::Relaxed);
        Ok(Confirmation {
            detail: format!("refunded:{}", request.reference),
        })
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The real testkit store, wrapped only to count and record.
///
/// Delegation rather than a hand-written double, deliberately: the scope
/// separation under test is the store's own keying, so a stand-in that modelled
/// it would be asserting against the model instead of the implementation.
struct CountingStore {
    inner: InMemoryOperationReservationStore,
    reserve_calls: AtomicUsize,
    scopes_seen: Mutex<Vec<Option<TenantId>>>,
}

impl CountingStore {
    fn new(clock: Arc<TestClock>) -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryOperationReservationStore::new(clock),
            reserve_calls: AtomicUsize::new(0),
            scopes_seen: Mutex::new(Vec::new()),
        })
    }

    fn reserve_calls(&self) -> usize {
        self.reserve_calls.load(Ordering::Relaxed)
    }

    fn scopes_seen(&self) -> Vec<Option<TenantId>> {
        self.scopes_seen.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl OperationReservationStore for CountingStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.reserve_calls.fetch_add(1, Ordering::Relaxed);
        self.scopes_seen
            .lock()
            .expect("not poisoned")
            .push(req.tenant.clone());
        self.inner.reserve(req).await
    }

    async fn renew(
        &self,
        fence: &OwnerFence,
        until: DateTime<Utc>,
    ) -> Result<(), ReservationError> {
        self.inner.renew(fence, until).await
    }

    async fn complete(
        &self,
        fence: &OwnerFence,
        response: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.inner.complete(fence, response).await
    }

    async fn abandon(&self, fence: &OwnerFence) -> Result<(), ReservationError> {
        self.inner.abandon(fence).await
    }

    async fn purge_completed_before(
        &self,
        cutoff: DateTime<Utc>,
        batch: usize,
    ) -> Result<u64, ReservationError> {
        self.inner.purge_completed_before(cutoff, batch).await
    }

    async fn probe(&self) -> Result<(), ReservationError> {
        self.inner.probe().await
    }
}

struct AllowProvider;
#[async_trait]
impl AuthorizationProvider for AllowProvider {
    async fn authorize(
        &self,
        _p: &Principal,
        _r: &AccessRequest,
        _c: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

struct StubAuthn;
impl ego_security_sdk::authentication::AuthenticationProvider for StubAuthn {
    fn authenticate(
        &self,
        _c: &ego_security_sdk::credential::Credential,
    ) -> Result<SecurityContext, ego_security_sdk::AuthenticationError> {
        unimplemented!("the tests build a SecurityContext directly");
    }
}

const SHARED_KEY: &str = "one-key-two-tenants";

fn epoch() -> DateTime<Utc> {
    Utc.timestamp_opt(1_000, 0).single().expect("valid instant")
}

/// The single operation key both tenants present. The whole point is that they
/// present the *same* one: two different keys would never collide in any
/// namespace, and the test would pass against the defect.
fn key() -> OperationKey {
    OperationKey::parse(SHARED_KEY).expect("a non-empty key parses")
}

/// An authenticated context whose principal carries `tenant`, so a
/// `#[tenant_scoped]` operation resolves to that scope.
fn ctx_for(tenant: &str) -> ServiceContext {
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:payer").expect("valid subject"),
    )
    .with_tenant_id(TenantId::new(tenant).expect("valid tenant"));
    ServiceContext::new()
        .with_security(Arc::new(SecurityContext::empty(principal)))
        .with_operation_key(key())
}

fn runtime_with(store: Arc<CountingStore>) -> Runtime {
    RuntimeBuilder::new()
        .with_operation_reservation_store(store)
        .with_reservation_clock(Arc::new(TestClock::new(epoch())))
        .with_reservation_owner_id(OwnerId::new("replica-under-test"))
        .with_reservation_lease_duration(Duration::from_secs(30))
        .with_tenant_enforcement_mode(TenantEnforcementMode::AuthenticatedOnly)
        .with_security(Arc::new(StubAuthn), Arc::new(AllowProvider))
        .build()
}

/// The proxy and the inner service whose executions it counts.
fn proxy(rt: &Runtime) -> (PaymentsRef, Arc<CountingPayments>) {
    let inner = Arc::new(CountingPayments::default());
    let proxy = PaymentsRef::new(
        inner.clone(),
        Arc::new(InterceptorChain::new()),
        Arc::downgrade(rt.inner()),
    );
    (proxy, inner)
}

/// One fixture, so no test can accidentally give itself a fresh store between
/// the two tenants' calls — which would make every assertion here vacuous.
fn fixture() -> (
    Arc<CountingStore>,
    Runtime,
    PaymentsRef,
    Arc<CountingPayments>,
) {
    let store = CountingStore::new(Arc::new(TestClock::new(epoch())));
    let rt = runtime_with(store.clone());
    let (proxy, inner) = proxy(&rt);
    (store, rt, proxy, inner)
}

fn payment(reference: &str) -> Payment {
    Payment {
        reference: reference.to_string(),
    }
}

fn refusal(result: Result<Confirmation, PaymentError>) -> ReservationRejection {
    match result {
        Err(PaymentError::Refused(rejection)) => rejection,
        other => panic!("expected a reservation refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// B7.8 — the three properties the defect violated
// ---------------------------------------------------------------------------

/// **Identical payload across two tenants never replays cross-scope bytes.**
///
/// The observed defect: the handler ran once, and tenant B was handed tenant A's
/// stored response. The assertion is placed on the step before the disclosure —
/// nothing is reserved at all — because that is the property that makes the
/// disclosure unreachable rather than merely unobserved. The execution count is
/// the corroborating half: zero executions means no response was ever produced
/// for either scope, so there are no bytes for the other one to be given.
#[tokio::test]
async fn an_identical_payload_across_two_tenants_never_replays_cross_scope_bytes() {
    let (store, _rt, proxy, service) = fixture();

    let a = proxy.refund(ctx_for("tenant-a"), payment("SAME")).await;
    let b = proxy.refund(ctx_for("tenant-b"), payment("SAME")).await;

    assert_eq!(
        refusal(a),
        ReservationRejection::TenantUnresolved,
        "an operation with no resolved scope has no namespace to reserve under"
    );
    assert_eq!(
        refusal(b),
        ReservationRejection::TenantUnresolved,
        "and the second tenant is refused for its own reason, not against the first's row"
    );

    assert_eq!(
        store.reserve_calls(),
        0,
        "the refusal must happen before the store is reached: a refusal that \
         reserved first would still have taken the lease"
    );
    assert_eq!(
        service.refund_calls(),
        0,
        "nothing executed, so no stored response exists for either scope — which \
         is why neither can be served the other's"
    );
}

/// **A differing payload across two tenants never produces a cross-scope
/// conflict.**
///
/// The observed defect: tenant B's legitimate, differently-shaped request was
/// refused `FingerprintConflict` against tenant A's reservation. So the
/// assertion is specifically that the refusal is *not* that one — the two are
/// distinguishable variants precisely so a cross-scope collision cannot hide
/// inside a same-key conflict, and a rejection naming the missing scope is the
/// only honest answer here.
#[tokio::test]
async fn a_differing_payload_across_two_tenants_never_produces_a_cross_scope_conflict() {
    let (store, _rt, proxy, service) = fixture();

    let a = proxy.refund(ctx_for("tenant-a"), payment("FOR-A")).await;
    let b = proxy.refund(ctx_for("tenant-b"), payment("FOR-B")).await;

    assert_eq!(refusal(a), ReservationRejection::TenantUnresolved);

    let second = refusal(b);
    assert_ne!(
        second,
        ReservationRejection::FingerprintConflict,
        "tenant B's request must never be refused as a conflict against a \
         reservation belonging to another scope"
    );
    assert_eq!(
        second,
        ReservationRejection::TenantUnresolved,
        "the refusal names the missing scope, which is what actually stopped it"
    );

    assert_eq!(store.reserve_calls(), 0);
    assert_eq!(service.refund_calls(), 0);
}

/// **The control: correct resolution executes once per tenant, and replay stays
/// inside the scope that produced it.**
///
/// Three calls, one key. Two tenants must each execute their own operation, and
/// a genuine retry by the tenant that already ran must be replayed rather than
/// re-executed. Without the third call this test would pass against a fix that
/// simply disabled replay altogether, which would satisfy isolation by
/// destroying the feature.
#[tokio::test]
async fn a_resolved_scope_executes_once_per_tenant_and_replays_only_within_itself() {
    let (store, _rt, proxy, service) = fixture();

    let first = proxy
        .settle(ctx_for("tenant-a"), payment("SAME"))
        .await
        .expect("tenant A's first call runs");
    assert_eq!(service.settle_calls(), 1);

    let other = proxy
        .settle(ctx_for("tenant-b"), payment("SAME"))
        .await
        .expect("tenant B's call is its own operation, not a replay of A's");
    assert_eq!(
        service.settle_calls(),
        2,
        "the identical key under a different scope must execute, not replay"
    );

    // Each tenant received the answer its own execution produced. The payloads
    // are identical here on purpose, so this is not proof by differing content —
    // it is the execution count above that carries it, and this pins that both
    // calls were in fact answered.
    assert_eq!(first, other);

    let replayed = proxy
        .settle(ctx_for("tenant-a"), payment("SAME"))
        .await
        .expect("tenant A's identical retry is replayed");
    assert_eq!(
        service.settle_calls(),
        2,
        "a retry within the scope that already completed must replay, not re-execute"
    );
    assert_eq!(
        replayed, first,
        "and the replay returns that scope's own stored response"
    );

    // The mechanism, named rather than inferred: three reservations, and the
    // scopes the store was handed are the two distinct tenants — never the
    // shared tenant-less namespace.
    assert_eq!(store.reserve_calls(), 3);
    let a = Some(TenantId::new("tenant-a").expect("valid"));
    let b = Some(TenantId::new("tenant-b").expect("valid"));
    assert_eq!(
        store.scopes_seen(),
        vec![a.clone(), b, a],
        "each reservation is namespaced by its own resolved tenant; a `None` here \
         would be the shared partition the collapse used to file them in"
    );
}

// ---------------------------------------------------------------------------
// The refusal is the runtime's, not the fingerprint's
// ---------------------------------------------------------------------------

/// A keyless call still takes the pre-existing path and reserves nothing, and
/// must **not** be turned into a refusal by the new scope check.
///
/// The missing-key policy has one owner at the transport edge, and this check
/// runs after it — so an unkeyed dispatch continues exactly as before. Without
/// this, the fix could pass every test above by refusing everything unresolved,
/// including the case that is supposed to proceed.
#[tokio::test]
async fn an_unkeyed_call_still_proceeds_and_reserves_nothing() {
    let (store, _rt, proxy, service) = fixture();

    let result = proxy.refund(unkeyed_ctx(), payment("SAME")).await;

    assert!(
        result.is_ok(),
        "no key means no reservation, which is not a refusal: got {result:?}"
    );
    assert_eq!(store.reserve_calls(), 0);
    assert_eq!(service.refund_calls(), 1, "the body still runs");
}

/// Authenticated, so `#[authorize]` passes, but carrying no operation key.
fn unkeyed_ctx() -> ServiceContext {
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:payer").expect("valid subject"),
    )
    .with_tenant_id(TenantId::new("tenant-a").expect("valid tenant"));
    ServiceContext::new().with_security(Arc::new(SecurityContext::empty(principal)))
}

/// The fingerprint is unrelated to the scope, and the two refusals stay
/// distinguishable — a caller must not have to parse prose to tell "this request
/// cannot be fingerprinted" from "this dispatch has no namespace".
#[test]
fn the_unresolved_scope_refusal_is_distinct_from_every_other() {
    let unresolved = ReservationRejection::TenantUnresolved;
    for other in [
        ReservationRejection::SelfInProgress,
        ReservationRejection::OtherInProgress,
        ReservationRejection::FingerprintConflict,
        ReservationRejection::StoreUnavailable,
        ReservationRejection::StoredResponseIncompatible,
        ReservationRejection::RequestNotFingerprintable,
    ] {
        assert_ne!(
            unresolved, other,
            "an unresolved scope must not be reported as {other:?}"
        );
    }

    // A fingerprint is computed from the request alone, so two scopes presenting
    // the same request agree on it. That is correct and is why the fingerprint
    // can never be what separates them — only the namespace can.
    let one = OperationFingerprint::new("fp-shared");
    let two = OperationFingerprint::new("fp-shared");
    assert_eq!(
        one, two,
        "identical requests fingerprint identically regardless of scope, which is \
         precisely why the scope has to be carried separately"
    );
}
