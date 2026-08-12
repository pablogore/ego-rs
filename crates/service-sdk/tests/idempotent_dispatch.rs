//! Behavioural coverage of the `#[idempotent]` slot — PROD-012 B6.4.
//!
//! These tests exist because a structural one cannot close this unit. A slot
//! that expands to nothing satisfies every assertion about generated shape or
//! ordering, so everything here is stated in terms of what was *observed*:
//! how many times the store's `reserve` ran, how many times the handler body
//! ran, and what the store was handed. Moving the slot above a guard, or
//! emptying it, has to kill a test by an observed count.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use ego_domain::context::TenantId;
use ego_domain::operation::{
    FencingToken, Lease, OperationFingerprint, OperationId, OperationKey,
    OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
    ReserveRequest, StoredServiceResponse,
};
use ego_domain::time::Clock;
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
    runtime::{
        operation_fingerprint, ReservationRejection, Runtime, RuntimeBuilder, TenantEnforcementMode,
    },
};
#[allow(unused_imports)]
use ego_service_sdk_macros::{authorize, idempotent, operation, service, tenant_scoped};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// The service under test
// ---------------------------------------------------------------------------

/// The operation's semantic input. Two fields, declared in an order that is
/// deliberately *not* alphabetical, so a canonicalisation that leaked struct
/// declaration order would still be stable while one that leaked map iteration
/// order would not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChargeRequest {
    pub reference: String,
    pub amount_cents: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChargeReceipt {
    pub confirmation: String,
}

/// Keeps the two refusal families apart so a test can assert *which* one came
/// back. Collapsing them to a string would let a test pass on the right error
/// for the wrong reason.
#[derive(Debug, PartialEq)]
pub enum BillingError {
    Security(String),
    Refused(ReservationRejection),
}

impl From<SecurityError> for BillingError {
    fn from(e: SecurityError) -> Self {
        BillingError::Security(e.to_string())
    }
}

impl From<ReservationRejection> for BillingError {
    fn from(r: ReservationRejection) -> Self {
        BillingError::Refused(r)
    }
}

impl ServiceErrorTrait for BillingError {
    fn code(&self) -> &str {
        "BILLING_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        ErrorCategory::Business
    }
    fn message(&self) -> String {
        format!("{self:?}")
    }
}

#[service(version = "1.0.0")]
pub trait BillingService {
    /// Fully guarded: authorization, then tenant scoping, then the reservation.
    /// The one operation that can show a guard failure leaves `reserve`
    /// untouched.
    #[operation]
    #[authorize(context = ctx, permission = "billing:charge")]
    #[tenant_scoped]
    #[idempotent]
    async fn charge(
        &self,
        ctx: ServiceContext,
        request: ChargeRequest,
    ) -> Result<ChargeReceipt, BillingError>;

    /// Carries the outcome matrix without dragging an *authorization* fixture
    /// through every case — no `#[authorize]`, so these tests build a runtime
    /// with no authorization provider at all.
    ///
    /// It does carry `#[tenant_scoped]`, and that is no longer optional for
    /// anything that reserves. A reservation is namespaced by the resolved tenant
    /// scope, so an operation with nothing on its path to resolve one has no
    /// namespace to reserve under and is refused
    /// (`ReservationRejection::TenantUnresolved`). This operation was previously
    /// declared without it and the cases below relied on an unresolved context
    /// being silently filed in the shared tenant-less partition — which is exactly
    /// the cross-tenant replay B7.8 found. So a security fixture is now
    /// unavoidable here; only the authorization half is still avoided.
    #[operation]
    #[tenant_scoped]
    #[idempotent]
    async fn settle(
        &self,
        ctx: ServiceContext,
        request: ChargeRequest,
    ) -> Result<ChargeReceipt, BillingError>;
}

/// Counts how many times a handler body actually ran. Every "did not execute"
/// assertion in this file reads this counter.
///
/// It also records the request identity the body *observed on its context* —
/// which is what a real service body would thread into each aggregate's
/// `CommandContext`. Reading it here rather than trusting the field to exist is
/// the difference between proving the bridge works and proving it compiles.
#[derive(Default)]
struct ObservedIdentity {
    key: Option<OperationKey>,
    fingerprint: Option<OperationFingerprint>,
}

struct CountingBilling {
    charge_calls: Arc<AtomicUsize>,
    settle_calls: Arc<AtomicUsize>,
    observed: Arc<Mutex<ObservedIdentity>>,
    /// When set, `settle` runs and then fails. The body having *run* is the
    /// point: the epilogue must distinguish "the operation produced an answer"
    /// from "the operation was reached", and only the first may be recorded.
    settle_fails: bool,
}

impl CountingBilling {
    fn observe(&self, ctx: &ServiceContext) {
        *self.observed.lock().expect("not poisoned") = ObservedIdentity {
            key: ctx.operation_key().cloned(),
            fingerprint: ctx.operation_fingerprint().cloned(),
        };
    }
}

#[async_trait]
impl BillingService for CountingBilling {
    async fn charge(
        &self,
        ctx: ServiceContext,
        request: ChargeRequest,
    ) -> Result<ChargeReceipt, BillingError> {
        self.observe(&ctx);
        self.charge_calls.fetch_add(1, Ordering::Relaxed);
        Ok(ChargeReceipt {
            confirmation: format!("charged:{}", request.reference),
        })
    }

    async fn settle(
        &self,
        ctx: ServiceContext,
        request: ChargeRequest,
    ) -> Result<ChargeReceipt, BillingError> {
        self.observe(&ctx);
        self.settle_calls.fetch_add(1, Ordering::Relaxed);
        if self.settle_fails {
            return Err(BillingError::Security("settlement declined".to_string()));
        }
        Ok(ChargeReceipt {
            confirmation: format!("settled:{}", request.reference),
        })
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct FrozenClock;
impl Clock for FrozenClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(1_000, 0).single().expect("valid instant")
    }
}

/// Answers `reserve` from a script and records every request it saw.
///
/// The recorded requests are the evidence for "the definitive key and
/// fingerprint reached the store" — asserting on a value the test computed
/// itself would prove nothing about what the generated code sent.
struct SpyStore {
    script: Mutex<Vec<Result<ReservationOutcome, ReservationError>>>,
    seen: Mutex<Vec<ReserveRequest>>,
    /// Every `complete` this store was asked to perform, with the fence it was
    /// asked to perform it under. The epilogue's only observable effect, so
    /// every assertion about it reads this.
    completed: Mutex<Vec<(OwnerFence, StoredServiceResponse)>>,
    /// What `complete` answers. `Ok(())` unless a test is about the failure.
    complete_answer: Mutex<Result<(), ReservationError>>,
}

impl SpyStore {
    fn scripted(answers: Vec<Result<ReservationOutcome, ReservationError>>) -> Arc<Self> {
        Arc::new(Self {
            // Popped from the back, so the script reads in call order.
            script: Mutex::new(answers.into_iter().rev().collect()),
            seen: Mutex::new(Vec::new()),
            completed: Mutex::new(Vec::new()),
            complete_answer: Mutex::new(Ok(())),
        })
    }

    /// Makes `complete` fail. The operation has already succeeded by the time
    /// the epilogue runs, so this is about what the *caller* sees, not about
    /// whether the work happened.
    fn completing_with(self: Arc<Self>, answer: ReservationError) -> Arc<Self> {
        *self.complete_answer.lock().expect("not poisoned") = Err(answer);
        self
    }

    fn complete_calls(&self) -> usize {
        self.completed.lock().expect("not poisoned").len()
    }

    fn first_completion(&self) -> (OwnerFence, StoredServiceResponse) {
        self.completed
            .lock()
            .expect("not poisoned")
            .first()
            .cloned()
            .expect("the operation completed at least once")
    }

    fn reserve_calls(&self) -> usize {
        self.seen.lock().expect("not poisoned").len()
    }

    fn first_request(&self) -> ReserveRequest {
        self.seen
            .lock()
            .expect("not poisoned")
            .first()
            .cloned()
            .expect("reserve was called at least once")
    }
}

#[async_trait]
impl OperationReservationStore for SpyStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.seen.lock().expect("not poisoned").push(req);
        self.script
            .lock()
            .expect("not poisoned")
            .pop()
            .expect("the script covers every reserve this test makes")
    }
    async fn renew(&self, _f: &OwnerFence, _u: DateTime<Utc>) -> Result<(), ReservationError> {
        panic!("nothing renews a lease yet");
    }
    async fn complete(
        &self,
        f: &OwnerFence,
        r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.completed
            .lock()
            .expect("not poisoned")
            .push((f.clone(), r));
        self.complete_answer.lock().expect("not poisoned").clone()
    }
    async fn abandon(&self, _f: &OwnerFence) -> Result<(), ReservationError> {
        panic!("a failed operation leaves its lease to expire rather than abandoning it");
    }
    async fn purge_completed_before(
        &self,
        _c: DateTime<Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        panic!("slot 3 only reserves");
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
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

struct DenyProvider;
#[async_trait]
impl AuthorizationProvider for DenyProvider {
    async fn authorize(
        &self,
        _p: &Principal,
        _r: &AccessRequest,
        _c: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Deny {
            reason: "denied-for-test".to_string(),
        })
    }
}

struct StubAuthn;
impl ego_security_sdk::authentication::AuthenticationProvider for StubAuthn {
    fn authenticate(
        &self,
        _c: &ego_security_sdk::credential::Credential,
    ) -> Result<SecurityContext, ego_security_sdk::AuthenticationError> {
        unimplemented!("not used here");
    }
}

const OWNER: &str = "owner-under-test";
const LEASE: Duration = Duration::from_secs(30);

fn lease_until() -> DateTime<Utc> {
    Utc.timestamp_opt(1_030, 0).single().expect("valid instant")
}

/// A runtime that reserves, under the fail-closed default enforcement mode —
/// the mode a real deployment runs. The store's presence is what makes that
/// mode buildable.
fn runtime_with(store: Arc<SpyStore>, authz: Option<Arc<dyn AuthorizationProvider>>) -> Runtime {
    let mut builder = RuntimeBuilder::new()
        .with_operation_reservation_store(store)
        .with_reservation_clock(Arc::new(FrozenClock))
        .with_reservation_owner_id(OwnerId::new(OWNER))
        .with_reservation_lease_duration(LEASE)
        .with_tenant_enforcement_mode(TenantEnforcementMode::AuthenticatedOnly);
    if let Some(authz) = authz {
        builder = builder.with_security(Arc::new(StubAuthn), authz);
    }
    builder.build()
}

/// The proxy, the two counters its inner service increments, and the request
/// identity the last handler body read off its own context.
fn proxy(
    rt: &Runtime,
) -> (
    BillingServiceRef,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<ObservedIdentity>>,
) {
    proxy_inner(rt, false)
}

/// Same fixture, with a `settle` that runs and then fails.
fn failing_proxy(rt: &Runtime) -> (BillingServiceRef, Arc<AtomicUsize>) {
    let (proxy, _, settle_calls, _) = proxy_inner(rt, true);
    (proxy, settle_calls)
}

fn proxy_inner(
    rt: &Runtime,
    settle_fails: bool,
) -> (
    BillingServiceRef,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<ObservedIdentity>>,
) {
    let charge_calls = Arc::new(AtomicUsize::new(0));
    let settle_calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(ObservedIdentity::default()));
    let inner: Arc<dyn BillingService> = Arc::new(CountingBilling {
        charge_calls: charge_calls.clone(),
        settle_calls: settle_calls.clone(),
        observed: observed.clone(),
        settle_fails,
    });
    let proxy = BillingServiceRef::new(
        inner,
        Arc::new(InterceptorChain::new()),
        Arc::downgrade(rt.inner()),
    );
    (proxy, charge_calls, settle_calls, observed)
}

fn key() -> OperationKey {
    OperationKey::parse("op-under-test").expect("a non-empty key parses")
}

fn request() -> ChargeRequest {
    ChargeRequest {
        reference: "ref-1".to_string(),
        amount_cents: 4_200,
    }
}

/// An authenticated context whose principal carries a tenant, so
/// `#[tenant_scoped]` resolves rather than failing closed.
fn authenticated_ctx() -> ServiceContext {
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:test").expect("valid subject"),
    )
    .with_tenant_id(TenantId::new("acme").expect("valid tenant"));
    ServiceContext::new()
        .with_security(Arc::new(SecurityContext::empty(principal)))
        .with_operation_key(key())
}

/// Authenticated and tenant-carrying, but with **no operation key**.
///
/// For the cases whose subject is "this dispatch legitimately did not reserve".
/// They need to reach the body, so they still have to satisfy the tenant guard
/// that now runs ahead of the slot — the absence under test is the key's, not the
/// scope's, and conflating the two would make those tests pass for the wrong
/// reason.
fn authenticated_keyless_ctx() -> ServiceContext {
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:test").expect("valid subject"),
    )
    .with_tenant_id(TenantId::new("acme").expect("valid tenant"));
    ServiceContext::new().with_security(Arc::new(SecurityContext::empty(principal)))
}

fn fresh_lease() -> Lease {
    Lease {
        operation_id: OperationId::new(None, key()),
        owner_id: OwnerId::new(OWNER),
        fencing_token: FencingToken::initial(),
        lease_until: lease_until(),
    }
}

// ---------------------------------------------------------------------------
// The guards run first, and a failing one leaves the store untouched
// ---------------------------------------------------------------------------

/// A reservation taken for a call that is about to be refused is a reservation
/// nobody releases: the key stays leased until it expires, and the caller's
/// legitimate retry is refused as self-contention. This is the assertion a
/// mutation moving slot 3 above `#[authorize]` has to fail.
#[tokio::test]
async fn a_denied_authorization_never_reaches_the_store() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), Some(Arc::new(DenyProvider)));
    let (proxy, charge_calls, _, _observed) = proxy(&rt);

    let result = proxy.charge(authenticated_ctx(), request()).await;

    assert!(matches!(result, Err(BillingError::Security(_))));
    assert_eq!(store.reserve_calls(), 0, "authorization runs before slot 3");
    assert_eq!(charge_calls.load(Ordering::Relaxed), 0);
}

/// Same property for slot 2. The context here is unauthenticated, so
/// `enforce_tenant` fails closed under `AuthenticatedOnly` — and it must fail
/// before anything is reserved, because a reservation namespaced by a tenant
/// that could not be resolved has no namespace at all.
#[tokio::test]
async fn a_rejected_tenant_never_reaches_the_store() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), Some(Arc::new(AllowProvider)));
    let (proxy, charge_calls, _, _observed) = proxy(&rt);

    // Authenticated (so `#[authorize]` passes) but with no tenant claim, which
    // is exactly what the resolver refuses to substitute a hint for.
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:test").expect("valid subject"),
    );
    let ctx = ServiceContext::new()
        .with_security(Arc::new(SecurityContext::empty(principal)))
        .with_operation_key(key());

    let result = proxy.charge(ctx, request()).await;

    assert!(matches!(result, Err(BillingError::Security(_))));
    assert_eq!(
        store.reserve_calls(),
        0,
        "tenant scoping runs before slot 3"
    );
    assert_eq!(charge_calls.load(Ordering::Relaxed), 0);
}

/// Both guards passing reserves exactly once — not zero (an empty slot) and not
/// twice (a reservation duplicated per guard).
#[tokio::test]
async fn both_guards_passing_reserves_exactly_once() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))]);
    let rt = runtime_with(store.clone(), Some(Arc::new(AllowProvider)));
    let (proxy, charge_calls, _, _observed) = proxy(&rt);

    let result = proxy.charge(authenticated_ctx(), request()).await;

    assert!(result.is_ok(), "got {result:?}");
    assert_eq!(store.reserve_calls(), 1);
    assert_eq!(charge_calls.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// What the store is handed
// ---------------------------------------------------------------------------

/// The key is the one carried from ingress, never regenerated; the tenant is
/// the resolved canonical one, never the raw hint; the lease comes from the
/// configured clock. A test computing the fingerprint the same way the
/// production code does would be circular, so the fingerprint's own properties
/// are asserted separately below — here it is only pinned to be non-empty and
/// stable across an identical retry.
#[tokio::test]
async fn the_definitive_key_tenant_and_fingerprint_reach_the_store() {
    let store = SpyStore::scripted(vec![
        Ok(ReservationOutcome::Fresh(fresh_lease())),
        Ok(ReservationOutcome::Fresh(fresh_lease())),
    ]);
    let rt = runtime_with(store.clone(), Some(Arc::new(AllowProvider)));
    let (proxy, _, _, _observed) = proxy(&rt);

    // A caller-supplied tenant hint that disagrees with nothing, present only
    // to show the reservation is not namespaced by it.
    let ctx = authenticated_ctx().with_tenant_id("acme");
    proxy
        .charge(ctx.clone(), request())
        .await
        .expect("the reservation is fresh");

    let seen = store.first_request();
    assert_eq!(seen.operation_key, key(), "the ingress key, unmodified");
    assert_eq!(
        seen.tenant,
        Some(TenantId::new("acme").expect("valid tenant")),
        "the namespace is the resolved canonical tenant"
    );
    assert_eq!(seen.owner_id, OwnerId::new(OWNER));
    assert_eq!(
        seen.lease_until,
        lease_until(),
        "lease_until is the configured clock plus the configured lease"
    );

    // An identical retry must present the identical fingerprint, or a legitimate
    // retry would be refused as a permanent conflict.
    proxy
        .charge(ctx, request())
        .await
        .expect("the reservation is fresh");
    let all = store.seen.lock().expect("not poisoned").clone();
    assert_eq!(all[0].fingerprint, all[1].fingerprint);
}

/// The other half of the bridge. Handing the store a fingerprint is only half
/// the job: the body has to be able to read *that same* fingerprint back off its
/// context, because that is the value it threads into each aggregate's
/// `CommandContext` for the per-aggregate receipt gate. Comparing it against
/// what the store actually saw — rather than against a value this test computed
/// — is what makes the assertion about the bridge and not about the algorithm.
#[tokio::test]
async fn the_body_reads_back_the_identity_the_store_was_handed() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))]);
    let rt = runtime_with(store.clone(), Some(Arc::new(AllowProvider)));
    let (proxy, _, _, observed) = proxy(&rt);

    proxy
        .charge(authenticated_ctx(), request())
        .await
        .expect("the reservation is fresh");

    let seen = store.first_request();
    let observed = observed.lock().expect("not poisoned");
    assert_eq!(
        observed.key.as_ref(),
        Some(&seen.operation_key),
        "the body must see the same key the reservation was taken under"
    );
    assert_eq!(
        observed.fingerprint.as_ref(),
        Some(&seen.fingerprint),
        "a body that cannot read the reservation's fingerprint cannot hand it \
         to an aggregate, and the receipt gate downstream has nothing to \
         compare against"
    );
}

/// The negative control for the stamp. An operation that legitimately did not
/// reserve must leave the fingerprint unset rather than carrying one that no
/// reservation stands behind — a receipt gate downstream would otherwise gate on
/// a request identity that nothing authorised.
#[tokio::test]
async fn a_dispatch_that_did_not_reserve_stamps_no_fingerprint() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, observed) = proxy(&rt);

    proxy
        .settle(authenticated_keyless_ctx(), request())
        .await
        .expect("no key, no reservation");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.reserve_calls(), 0);
    assert_eq!(
        observed.lock().expect("not poisoned").fingerprint,
        None,
        "nothing reserved, so nothing may claim it did"
    );
}

/// AD-3f, stated as the property that matters: the fingerprint follows the
/// typed values, not the syntax that produced them.
#[test]
fn the_fingerprint_follows_the_typed_values_not_their_syntax() {
    let one: ChargeRequest =
        serde_json::from_str(r#"{"reference":"ref-1","amount_cents":4200}"#).expect("valid");
    // The same values, in a different key order, with whitespace.
    let two: ChargeRequest =
        serde_json::from_str("{\n  \"amount_cents\" : 4200 ,\n  \"reference\" : \"ref-1\"\n}")
            .expect("valid");

    assert_eq!(
        operation_fingerprint(&(&one,)).expect("serialisable"),
        operation_fingerprint(&(&two,)).expect("serialisable"),
        "two syntactically different requests that deserialise to the same \
         typed values must reserve under the same fingerprint — otherwise a \
         legitimate retry is refused as a permanent conflict"
    );

    let different = ChargeRequest {
        amount_cents: 4_201,
        ..one.clone()
    };
    assert_ne!(
        operation_fingerprint(&(&one,)).expect("serialisable"),
        operation_fingerprint(&(&different,)).expect("serialisable"),
        "two different typed values must not share a fingerprint, or a \
         different request would silently replay another's answer"
    );
}

/// Length prefixing, checked where naive concatenation fails: `["ab"]` and
/// `["a","b"]` flatten to the same bytes without it, and two genuinely
/// different requests would deduplicate against each other.
#[test]
fn adjacent_arguments_cannot_be_reassociated_into_the_same_fingerprint() {
    let split = operation_fingerprint(&(&"a", &"b")).expect("serialisable");
    let joined = operation_fingerprint(&(&"ab", &"")).expect("serialisable");
    assert_ne!(split, joined);
}

// ---------------------------------------------------------------------------
// The three dispatch outcomes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn proceed_executes_the_operation_exactly_once() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    let out = proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("a fresh reservation proceeds");

    assert_eq!(out.confirmation, "settled:ref-1");
    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.reserve_calls(), 1);
}

/// A takeover proceeds too. It is a separate case from `Fresh` because it is
/// the one where somebody else's lease expired — if it stopped dispatch, an
/// operation whose owner died would be unrecoverable.
#[tokio::test]
async fn a_takeover_also_executes_the_operation() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::TakenOver(fresh_lease()))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("a takeover proceeds");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
}

/// The point of the whole change: the second arrival of an operation that
/// already completed answers with what the first produced and runs nothing.
#[tokio::test]
async fn replay_returns_the_stored_output_without_executing() {
    let stored_output = ChargeReceipt {
        confirmation: "settled-on-the-first-attempt".to_string(),
    };
    let stored = ego_service_sdk::runtime::encode_stored_response(&stored_output).expect("encodes");
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Succeeded(stored))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    let out = proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("a completed reservation replays");

    assert_eq!(
        out, stored_output,
        "the replay must be the recorded answer, not a fresh execution's"
    );
    assert_eq!(
        settle_calls.load(Ordering::Relaxed),
        0,
        "a replay that re-runs the handler produces a second set of effects, \
         which is the exact bug this change exists to close"
    );
}

/// A stored response this build cannot read is refused, not guessed at, and
/// still does not execute — re-running would be the one thing worse than
/// failing, because the operation already happened.
#[tokio::test]
async fn an_undecodable_stored_response_refuses_without_executing() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Succeeded(
        StoredServiceResponse::new(b"not an envelope this build writes".to_vec()),
    ))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    let result = proxy.settle(authenticated_ctx(), request()).await;

    assert_eq!(
        result.unwrap_err(),
        BillingError::Refused(ReservationRejection::StoredResponseIncompatible)
    );
    assert_eq!(settle_calls.load(Ordering::Relaxed), 0);
}

// ---------------------------------------------------------------------------
// Every refusal stops dispatch, and arrives as itself
// ---------------------------------------------------------------------------

/// The four store-answerable refusals, each asserted by identity rather than by
/// "some error": they call for different caller and operator action, and a test
/// that only checked `is_err()` would pass with all four collapsed into one.
#[tokio::test]
async fn every_refusal_stops_dispatch_and_arrives_as_itself() {
    let cases: Vec<(
        Result<ReservationOutcome, ReservationError>,
        ReservationRejection,
    )> = vec![
        (
            Ok(ReservationOutcome::OwnedInProgress(fresh_lease())),
            ReservationRejection::SelfInProgress,
        ),
        (
            Ok(ReservationOutcome::OtherInProgress),
            ReservationRejection::OtherInProgress,
        ),
        (
            Ok(ReservationOutcome::Conflict),
            ReservationRejection::FingerprintConflict,
        ),
        (
            Err(ReservationError::Backend("down".to_string())),
            ReservationRejection::StoreUnavailable,
        ),
    ];

    for (answer, expected) in cases {
        let store = SpyStore::scripted(vec![answer]);
        let rt = runtime_with(store.clone(), None);
        let (proxy, _, settle_calls, _observed) = proxy(&rt);

        let result = proxy.settle(authenticated_ctx(), request()).await;

        assert_eq!(
            result.unwrap_err(),
            BillingError::Refused(expected.clone()),
            "a refused reservation must arrive as the case it is"
        );
        assert_eq!(
            settle_calls.load(Ordering::Relaxed),
            0,
            "{expected:?} must not execute the operation"
        );
    }
}

/// A dropped runtime must refuse rather than fall through to an unreserved
/// execution — the fail-open branch is the one that silently disables the
/// guarantee.
///
/// # Why this no longer names a variant
///
/// It used to assert `StoreUnavailable`, which held while `settle` reached the
/// reservation slot as the first step needing the runtime. It now carries
/// `#[tenant_scoped]`, and that guard needs the runtime too, so with the runtime
/// gone the tenant guard is what notices — and reports a security failure. Both
/// are refusals and neither executes anything.
///
/// Pinning the variant here would be pinning *guard ordering* under a name about
/// fail-open behaviour, and guard ordering already has its own tests
/// (`a_denied_authorization_never_reaches_the_store`,
/// `a_rejected_tenant_never_reaches_the_store`). What this test owns is that a
/// runtime that has gone away produces a refusal and **not** an execution, so
/// that is what it asserts — with both counters, which are the assertions a
/// fail-open regression actually has to get past.
#[tokio::test]
async fn a_dropped_runtime_refuses_rather_than_running_unreserved() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);
    drop(rt);

    let result = proxy.settle(authenticated_ctx(), request()).await;

    assert!(
        result.is_err(),
        "a vanished runtime must never be answered by running the operation \
         unreserved: got {result:?}"
    );
    assert_eq!(
        settle_calls.load(Ordering::Relaxed),
        0,
        "the body must not run when nothing could reserve it"
    );
    assert_eq!(store.reserve_calls(), 0);
}

// ---------------------------------------------------------------------------
// The two ways a marked operation legitimately does not reserve
// ---------------------------------------------------------------------------

/// A runtime that registered no reservation store dispatches normally. That is
/// what `Compatibility` means, and the assertion that matters is that it
/// reaches the handler rather than failing on a capability it never claimed.
#[tokio::test]
async fn a_runtime_without_reservations_dispatches_normally() {
    let rt = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .build();
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("no reservation capability means no reservation");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
}

/// A context with no key does not reserve. The missing-key policy has exactly
/// one owner — `resolve_operation_key`, at the transport edge — and deciding it
/// a second time here would give two places the power to disagree about it.
#[tokio::test]
async fn a_context_without_a_key_does_not_reserve() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_keyless_ctx(), request())
        .await
        .expect("no key, no reservation");

    assert_eq!(store.reserve_calls(), 0);
    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
}

/// An unmarked operation is untouched by any of this. Without it, a slot that
/// fired for every operation would still pass every test above.
#[tokio::test]
async fn an_unmarked_operation_never_reserves() {
    #[service(version = "1.0.0")]
    pub trait PlainService {
        #[operation]
        async fn look(&self, ctx: ServiceContext, id: String) -> Result<String, BillingError>;
    }

    struct Plain;
    #[async_trait]
    impl PlainService for Plain {
        async fn look(&self, _ctx: ServiceContext, id: String) -> Result<String, BillingError> {
            Ok(id)
        }
    }

    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), None);
    let inner: Arc<dyn PlainService> = Arc::new(Plain);
    let proxy = PlainServiceRef::new(
        inner,
        Arc::new(InterceptorChain::new()),
        Arc::downgrade(rt.inner()),
    );

    let out = proxy
        .look(
            ServiceContext::new().with_operation_key(key()),
            "x".to_string(),
        )
        .await
        .expect("an unmarked operation dispatches");

    assert_eq!(out, "x");
    assert_eq!(
        store.reserve_calls(),
        0,
        "only #[idempotent] operations reserve"
    );
}

/// The fingerprint covers the arguments and nothing else. Two attempts that
/// differ only in context metadata — a different correlation id — must produce
/// the same fingerprint, or every retry would look like a different request.
#[tokio::test]
async fn context_metadata_does_not_enter_the_fingerprint() {
    let store = SpyStore::scripted(vec![
        Ok(ReservationOutcome::Fresh(fresh_lease())),
        Ok(ReservationOutcome::Fresh(fresh_lease())),
    ]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, _, _observed) = proxy(&rt);

    for correlation in ["attempt-1", "attempt-2"] {
        proxy
            .settle(
                authenticated_ctx().with_correlation_id(correlation),
                request(),
            )
            .await
            .expect("fresh");
    }

    let all = store.seen.lock().expect("not poisoned").clone();
    assert_eq!(
        all[0].fingerprint, all[1].fingerprint,
        "correlation id describes the attempt, not the request"
    );
}

/// The fingerprint is bounded. `operation_reservations.fingerprint` is
/// `VARCHAR(255)`, so a fingerprint that grew with the payload would insert
/// fine in tests and fail in production on the first large request.
#[test]
fn the_fingerprint_is_bounded_regardless_of_payload_size() {
    let big = ChargeRequest {
        reference: "x".repeat(100_000),
        amount_cents: 1,
    };
    let fingerprint: OperationFingerprint = operation_fingerprint(&(&big,)).expect("serialisable");
    assert_eq!(
        fingerprint.to_string().len(),
        64,
        "a SHA-256 digest rendered as hex, independent of the input's size"
    );
}

// ---------------------------------------------------------------------------
// The epilogue — what a completed operation records, and what it does not
// ---------------------------------------------------------------------------

/// The point of the epilogue: an operation that produced an answer records it,
/// so the next identical arrival replays instead of running. Asserted through
/// the store rather than through the return value, because a `complete` that
/// never happened is invisible from the caller's side — by design.
#[tokio::test]
async fn a_completed_operation_records_its_response_under_the_permits_fence() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    let out = proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("a fresh reservation proceeds");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        store.complete_calls(),
        1,
        "recorded once, not per aggregate"
    );

    let (fence, stored) = store.first_completion();
    // The fence is what makes the write conditional: a lease taken over in the
    // meantime must not have its result overwritten by the owner it replaced.
    let lease = fresh_lease();
    assert_eq!(fence.owner_id, lease.owner_id);
    assert_eq!(fence.fencing_token, lease.fencing_token);
    assert_eq!(fence.operation_id, lease.operation_id);

    // Round-tripped through the same codec the replay path reads with. Asserting
    // the bytes would pin a format; asserting the round-trip pins the contract
    // that actually matters — that a later replay reconstructs this answer.
    let decoded: ChargeReceipt =
        ego_service_sdk::runtime::decode_stored_response(&stored).expect("decodes");
    assert_eq!(decoded, out);
}

/// A failed operation has no answer to record, and recording one would tell the
/// next identical arrival that the work is done. The lease is left to expire
/// instead, so a retry can take it over — which is why `abandon` panics in this
/// file's store rather than being expected.
#[tokio::test]
async fn a_failed_operation_records_nothing() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, settle_calls) = failing_proxy(&rt);

    let result = proxy.settle(authenticated_ctx(), request()).await;

    assert!(result.is_err(), "the handler failed");
    assert_eq!(
        settle_calls.load(Ordering::Relaxed),
        1,
        "the body ran — which is exactly why 'reached' must not be confused with \
         'produced an answer'"
    );
    assert_eq!(store.complete_calls(), 0);
}

/// A replay produced no new response; it returned one that was already stored.
/// Recording again would overwrite a durable answer with a copy of itself, under
/// a fence this dispatch never held.
#[tokio::test]
async fn a_replay_records_nothing() {
    let stored = ego_service_sdk::runtime::encode_stored_response(&ChargeReceipt {
        confirmation: "settled-on-the-first-attempt".to_string(),
    })
    .expect("encodes");
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Succeeded(stored))]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, _, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("a completed reservation replays");

    assert_eq!(store.complete_calls(), 0);
}

/// Every refusal stops dispatch, so there is no answer to record either.
#[tokio::test]
async fn a_refused_operation_records_nothing() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::OtherInProgress)]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, _, _observed) = proxy(&rt);

    let result = proxy.settle(authenticated_ctx(), request()).await;

    assert!(result.is_err());
    assert_eq!(store.complete_calls(), 0);
}

/// A dispatch that never reserved has no fence to present, so there is nothing
/// to complete under — and inventing one would record an answer for an operation
/// nothing authorised.
#[tokio::test]
async fn a_dispatch_that_did_not_reserve_records_nothing() {
    let store = SpyStore::scripted(vec![]);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_keyless_ctx(), request())
        .await
        .expect("no key, no reservation");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.reserve_calls(), 0);
    assert_eq!(store.complete_calls(), 0);
}

/// **A failed completion must not fail an operation that succeeded.** By the
/// time the epilogue runs, the handler returned `Ok` and every aggregate
/// committed. Reporting an error now would describe successful work as a
/// failure and invite a retry of something that must not run twice.
///
/// What is lost is the replay shortcut, not the guarantee: a later identical
/// request re-reserves and re-enters the body, where each aggregate's receipt
/// answers for the step it already did.
#[tokio::test]
async fn a_stale_completion_does_not_fail_the_operation() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))])
        .completing_with(ReservationError::StaleOwner);
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, _, _observed) = proxy(&rt);

    let out = proxy.settle(authenticated_ctx(), request()).await.expect(
        "another owner completed this operation first; ours is discarded, but \
             the work this call did still happened and its answer is still true",
    );

    assert_eq!(out.confirmation, "settled:ref-1");
    assert_eq!(store.complete_calls(), 1, "it was attempted");
}

/// Same rule for the store simply being unreachable. Distinguished from the
/// stale case only in what an operator should do about it, never in what the
/// caller sees.
#[tokio::test]
async fn an_unreachable_store_does_not_fail_a_completed_operation() {
    let store = SpyStore::scripted(vec![Ok(ReservationOutcome::Fresh(fresh_lease()))])
        .completing_with(ReservationError::Backend("down".to_string()));
    let rt = runtime_with(store.clone(), None);
    let (proxy, _, settle_calls, _observed) = proxy(&rt);

    proxy
        .settle(authenticated_ctx(), request())
        .await
        .expect("the operation succeeded; only its bookkeeping did not");

    assert_eq!(settle_calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.complete_calls(), 1);
}
