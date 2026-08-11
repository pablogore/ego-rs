//! PROD-012 B6.4a — the multi-aggregate recovery scenario.
//!
//! This is the scenario that justifies the whole receipt layer, and it is the
//! one that cannot run until the service body carries the reservation's
//! identity into every `CommandContext` it creates.
//!
//! The situation it reconstructs is a partial failure that already happened:
//! one operation touched two aggregates, the organization step completed and
//! recorded its receipt, and the user step did not. The caller retries the
//! identical request. What must happen is *recovery*, not repetition — the
//! organization must not run again, and the user must run exactly once.
//!
//! Everything below is stated as something observed: how many times each store
//! was written, what the reservation was handed, and what receipt the second
//! aggregate confirmed. A test that only checked the call returned `Ok` would
//! pass just as happily if both steps re-ran.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use ego_domain::operation::{
    AggregateOutcome, OperationFingerprint, OperationKey, OperationReceipt,
    OperationReservationStore, OwnerId, ReservationError, ReservationOutcome, ReserveRequest,
};
use ego_domain::persistence::{EventStore, EventStoreUnitOfWork, PersistenceError, StoredEvent};
use ego_domain::time::Clock;
use ego_service_sdk::runtime::operation_fingerprint;
use ego_testkit::{PrincipalBuilder, ServiceTestFixture};
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::testing::InMemoryEventStore;
use reference_app::application::{RegisterInput, RegisterUser, RegisterUserImpl, RegisterUserTag};
use reference_app::domain::tenant_org::OrganizationEnsured;
use reference_app::domain::user::UserRegistered;

const KEY: &str = "op-register-recovery-1";
const TENANT: &str = "tenant-a";
const OWNER: &str = "reference-app-under-test";

/// The one request, retried. Both runs of this operation are this exact value —
/// which is what makes the retry a retry rather than a different request
/// reusing a key.
fn input() -> RegisterInput {
    RegisterInput {
        user_id: "user-1".to_string(),
        email: "user-1@example.com".to_string(),
        tenant_id: TENANT.to_string(),
        org_name: "Acme".to_string(),
    }
}

struct FrozenClock;
impl Clock for FrozenClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(1_000, 0).single().expect("valid instant")
    }
}

/// Permits the operation and records what it was asked to reserve.
///
/// The recorded request is the anchor for every identity assertion below: the
/// fingerprint this test seeds a receipt with is only trustworthy because it is
/// compared against the one the production path actually presented here.
struct SpyReservations {
    seen: Mutex<Vec<ReserveRequest>>,
}

impl SpyReservations {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.seen.lock().expect("not poisoned").len()
    }

    fn first(&self) -> ReserveRequest {
        self.seen
            .lock()
            .expect("not poisoned")
            .first()
            .cloned()
            .expect("the operation reserved at least once")
    }
}

#[async_trait::async_trait]
impl OperationReservationStore for SpyReservations {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        let lease = ego_domain::operation::Lease {
            operation_id: ego_domain::operation::OperationId::new(
                req.tenant.clone(),
                req.operation_key.clone(),
            ),
            owner_id: OwnerId::new(OWNER),
            fencing_token: ego_domain::operation::FencingToken::initial(),
            lease_until: Utc.timestamp_opt(1_030, 0).single().expect("valid instant"),
        };
        self.seen.lock().expect("not poisoned").push(req);
        // The reservation permits the retry. It has to: the previous attempt
        // died mid-operation, so there is no stored response to replay and the
        // per-aggregate receipts are the only record of what already happened.
        Ok(ReservationOutcome::TakenOver(lease))
    }
    async fn renew(
        &self,
        _f: &ego_domain::operation::OwnerFence,
        _u: DateTime<Utc>,
    ) -> Result<(), ReservationError> {
        Ok(())
    }
    async fn complete(
        &self,
        _f: &ego_domain::operation::OwnerFence,
        _r: ego_domain::operation::StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        // B6.8 is what calls this. Reaching it here would mean the epilogue
        // landed early, not that this scenario failed.
        Ok(())
    }
    async fn abandon(
        &self,
        _f: &ego_domain::operation::OwnerFence,
    ) -> Result<(), ReservationError> {
        Ok(())
    }
    async fn purge_completed_before(
        &self,
        _c: DateTime<Utc>,
        _b: usize,
    ) -> Result<u64, ReservationError> {
        Ok(0)
    }
    async fn probe(&self) -> Result<(), ReservationError> {
        Ok(())
    }
}

/// What one aggregate's store was asked to do, and what it confirmed.
#[derive(Default)]
struct StoreCalls {
    appends: AtomicUsize,
    lookups: AtomicUsize,
    /// Every receipt confirmed through a unit of work. This is the evidence
    /// that the identity reached *this* aggregate — a body that forgot to
    /// transfer it leaves the gate inactive, and an inactive gate confirms
    /// nothing.
    receipts: Mutex<Vec<OperationReceipt>>,
    /// The operation keys this store's receipt lookup was asked about.
    looked_up: Mutex<Vec<String>>,
}

/// An event store that can be seeded with one prior receipt, and records every
/// call the actor makes on it.
struct RecoveryStore<E: ego_domain::DomainEvent> {
    inner: InMemoryEventStore<E>,
    seeded: Option<OperationReceipt>,
    calls: Arc<StoreCalls>,
}

impl<E> RecoveryStore<E>
where
    E: ego_domain::DomainEvent
        + Clone
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    fn new(calls: Arc<StoreCalls>, seeded: Option<OperationReceipt>) -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryEventStore::new(),
            seeded,
            calls,
        })
    }
}

#[async_trait::async_trait]
impl<E> EventStore<E> for RecoveryStore<E>
where
    E: ego_domain::DomainEvent
        + Clone
        + serde::Serialize
        + serde::de::DeserializeOwned
        + Send
        + Sync
        + 'static,
{
    async fn append(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        self.calls.appends.fetch_add(1, Ordering::SeqCst);
        self.inner
            .append(
                aggregate_type,
                aggregate_id,
                tenant_id,
                expected_version,
                events,
            )
            .await
    }

    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError> {
        self.inner
            .load(aggregate_type, aggregate_id, tenant_id)
            .await
    }

    async fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        self.inner.list_aggregate_ids(tenant_id).await
    }

    async fn find_receipt(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
        operation_key: &str,
    ) -> Result<Option<OperationReceipt>, PersistenceError> {
        self.calls.lookups.fetch_add(1, Ordering::SeqCst);
        self.calls
            .looked_up
            .lock()
            .expect("not poisoned")
            .push(operation_key.to_string());
        Ok(self.seeded.clone())
    }

    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError> {
        Ok(Box::new(RecordingUow {
            inner: self.inner.begin().await?,
            calls: self.calls.clone(),
        }))
    }
}

/// Passes everything through and keeps a copy of each confirmed receipt.
struct RecordingUow<E: ego_domain::DomainEvent> {
    inner: Box<dyn EventStoreUnitOfWork<E>>,
    calls: Arc<StoreCalls>,
}

#[async_trait::async_trait]
impl<E> EventStoreUnitOfWork<E> for RecordingUow<E>
where
    E: ego_domain::DomainEvent + Clone + serde::Serialize + Send + Sync + 'static,
{
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError> {
        self.calls.appends.fetch_add(1, Ordering::SeqCst);
        self.inner
            .append(
                aggregate_type,
                aggregate_id,
                tenant_id,
                expected_version,
                events,
            )
            .await
    }

    async fn confirm_receipt(
        &mut self,
        receipt: &OperationReceipt,
    ) -> Result<(), PersistenceError> {
        self.calls
            .receipts
            .lock()
            .expect("not poisoned")
            .push(receipt.clone());
        self.inner.confirm_receipt(receipt).await
    }

    async fn commit(self: Box<Self>) -> Result<(), PersistenceError> {
        self.inner.commit().await
    }
}

/// The receipt the organization step left behind when it completed under this
/// operation, before the user step failed.
fn prior_org_receipt(fingerprint: &OperationFingerprint) -> OperationReceipt {
    OperationReceipt::new(
        "tenant_organization",
        TENANT,
        Some(ego_domain::context::TenantId::new(TENANT).expect("a valid tenant")),
        OperationKey::parse(KEY).expect("a non-empty key parses"),
        fingerprint.clone(),
        AggregateOutcome::NoEvents,
    )
}

#[tokio::test]
async fn a_retry_recovers_the_unfinished_aggregate_without_repeating_the_finished_one() {
    // The fingerprint the reservation will present. Computed here the same way
    // slot 3 does — over the typed arguments and nothing else — because seeding
    // a prior receipt requires knowing it in advance. It is *not* taken on
    // trust: the assertion below compares it against what the reservation store
    // was actually handed, so a divergence between this and the production path
    // fails the test rather than quietly making the receipt unmatchable.
    let fingerprint: OperationFingerprint =
        operation_fingerprint(&(&input(),)).expect("the input serialises");

    let org_calls = Arc::new(StoreCalls::default());
    let user_calls = Arc::new(StoreCalls::default());

    let org_store = RecoveryStore::<OrganizationEnsured>::new(
        org_calls.clone(),
        Some(prior_org_receipt(&fingerprint)),
    );
    let user_store = RecoveryStore::<UserRegistered>::new(user_calls.clone(), None);

    let org_runtime = Arc::new(
        EntityRuntimeBuilder::new()
            .with_event_store(org_store)
            .tenant_id(TENANT)
            .build(),
    );
    let user_runtime = Arc::new(
        EntityRuntimeBuilder::new()
            .with_event_store(user_store)
            .tenant_id(TENANT)
            .build(),
    );

    let service = Arc::new(RegisterUserImpl::new(org_runtime, user_runtime, None));
    let reservations = SpyReservations::new();

    let fixture = ServiceTestFixture::builder()
        .with_service::<RegisterUserTag>(service)
        .expect("registration succeeds")
        .principal(PrincipalBuilder::new().tenant(TENANT).build())
        .with_operation_reservation_store(
            reservations.clone(),
            Arc::new(FrozenClock),
            OwnerId::new(OWNER),
            Duration::from_secs(30),
        )
        .build();

    let proxy = fixture
        .resolve::<RegisterUserTag>()
        .expect("registered tag resolves");

    let ctx = fixture
        .context()
        .with_tenant_id(TENANT)
        .with_operation_key(OperationKey::parse(KEY).expect("a non-empty key parses"));

    let out = proxy
        .register(ctx, input())
        .await
        .expect("the retry must complete, recovering the step that never finished");

    // --- The reservation, and the identity everything below is anchored to ---

    assert_eq!(
        reservations.calls(),
        1,
        "one business operation reserves once, not once per aggregate"
    );
    let reserved = reservations.first();
    assert_eq!(
        reserved.operation_key,
        OperationKey::parse(KEY).expect("a non-empty key parses"),
        "the key carried from ingress, unmodified"
    );
    assert_eq!(
        reserved.fingerprint, fingerprint,
        "the fingerprint this test seeded the prior receipt with must be the one \
         the production path actually presented — otherwise the scenario below \
         proves nothing about recovery, only that a receipt never matched"
    );

    // --- Aggregate 1: already done, so it must not run again ---

    assert_eq!(
        org_calls.lookups.load(Ordering::SeqCst),
        1,
        "the organization's gate must consult its receipt, which only happens \
         when the body transferred the identity into its CommandContext"
    );
    assert_eq!(
        org_calls.looked_up.lock().expect("not poisoned").as_slice(),
        [KEY.to_string()],
        "and it must look up the operation the caller named, not another"
    );
    assert_eq!(
        org_calls.appends.load(Ordering::SeqCst),
        0,
        "the organization already completed under this operation: a second set \
         of events is the exact duplicate the receipt exists to prevent"
    );

    // --- Aggregate 2: never finished, so it must run exactly once ---

    assert_eq!(
        user_calls.lookups.load(Ordering::SeqCst),
        1,
        "the user's gate must consult its receipt too — this is the assertion a \
         body that transferred the identity to the first aggregate and forgot \
         the second one fails"
    );
    assert_eq!(
        user_calls.appends.load(Ordering::SeqCst),
        1,
        "the user step never completed, so the retry must run it — exactly once"
    );

    let user_receipts = user_calls.receipts.lock().expect("not poisoned").clone();
    assert_eq!(
        user_receipts.len(),
        1,
        "having run, the user step must record that it did, or the next retry \
         would run it again"
    );
    assert_eq!(
        user_receipts[0].fingerprint(),
        &reserved.fingerprint,
        "the second aggregate's receipt must be written under the same request \
         identity the reservation used. This is what makes the transfer \
         systematic rather than one wired-up call site: dropping `.carrying(..)` \
         from the user step leaves this receipt unwritten"
    );

    // --- The answer, composed from the request rather than from a replay ---

    assert_eq!(out.user_id, "user-1");
    assert_eq!(out.tenant_id, TENANT);
}
