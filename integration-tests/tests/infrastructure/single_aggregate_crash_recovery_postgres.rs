//! **Guarantee (PROD-012 audit Gap 2):** a process that dies *after* an
//! operation's aggregate write and reservation commit both durably succeeded,
//! but before anything downstream observes that success, leaves nothing for a
//! retry to repeat. A retry with the same operation key and fingerprint gets
//! the recorded answer back — the reservation replays it — and neither the
//! aggregate nor its receipt is touched a second time.
//!
//! **Why a new service, not `RegisterUser`.** `RegisterUser`
//! (`dual_aggregate_crash_recovery_postgres.rs`,
//! `register_user_multi_aggregate_recovery.rs`) always writes two aggregates,
//! and the crash scenario already covered there is a *different* one: a death
//! **between** the two aggregate writes, recovered by resuming the unfinished
//! half. This scenario needs the opposite shape — one aggregate, fully
//! committed, nothing left unfinished — so the retry has only one honest
//! answer: replay, not resume. `EnsureOrg` below is that minimal shape: one
//! `#[idempotent]` operation over the *same* `TenantOrganizationEntity` domain
//! type `RegisterUser` already uses, wired through the *same*
//! `PostgreSQLEventStore` / `PostgresOperationReservationStore` composition —
//! no new domain modelling, no production source touched.
//!
//! **What is deliberately not exercised here.** There is no HTTP layer: the
//! call goes through `Runtime::resolve::<EnsureOrgTag>()` directly, the same
//! way `crates/service-sdk/tests/idempotent_dispatch.rs` drives `#[idempotent]`
//! operations. The property under test — the reservation store's own durable
//! replay, under a real crash — lives entirely below the transport, and
//! `RegisterUser`'s own HTTP-level coverage is not being duplicated here.
//!
//! **Layers traversed:** the generated `#[idempotent]` dispatch →
//! `PostgresOperationReservationStore` → `EntityRuntime<OrganizationEnsured>`
//! backed by a real `PostgreSQLEventStore` → real SQL against a real
//! PostgreSQL, across **two operating-system processes**.
//!
//! # Why two processes, and why `abort`
//!
//! Same reasoning as `dual_aggregate_crash_recovery_postgres.rs`: a
//! recoverable `Err` unwinds — destructors run, pools close — which leaves a
//! *tidier* partial state than a real crash leaves. `panic!` has the same
//! problem; `exit` still runs atexit handlers. So the interruption is
//! `std::process::abort()` in a **child process**: SIGABRT, no unwinding, no
//! cleanup. The parent checks for SIGABRT specifically, not merely a non-zero
//! exit — a child that failed an assertion or lost its database also exits
//! non-zero, and neither of those is a crash.
//!
//! Unlike the dual-aggregate scenario, no failpoint is installed anywhere:
//! the child runs the operation to a normal, successful completion — the
//! reservation's `complete()` call happens synchronously inside that same
//! `.await`, exactly as `idempotent_dispatch.rs`'s
//! `a_completed_operation_records_its_response_under_the_permits_fence`
//! demonstrates — then the child verifies the durable commit **by direct SQL
//! query** before aborting. That is the second option this scenario's own
//! brief names explicitly: verify the commit landed, then crash, rather than
//! trying to land the abort inside a window that does not exist as a gap in
//! this process at all.
//!
//! # Unix only, and behind no feature flag
//!
//! `#[cfg(unix)]` at the module boundary, same as its sibling: reading a
//! signal from an exit status is `std::os::unix`, and the signal *is* the
//! evidence. No `crash-test-failpoint`-style gate is needed because nothing
//! here reads an environment variable to decide whether to abort — the child
//! test function always aborts, and it only runs at all when the parent's
//! `CHILD_DB_URL` env var is present.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.

use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ego_domain::context::TenantId;
use ego_domain::operation::{OperationKey, OwnerId};
use ego_domain::time::SystemClock;
use ego_domain::{Observability, SemanticEvent};
use ego_integration_tests::isolated_database;
use ego_persistence::postgres::event_store::PostgreSQLEventStore;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::category::ErrorCategory;
use ego_service_sdk::error::ServiceErrorTrait;
use ego_service_sdk::runtime::{
    IdempotencyEnforcementMode, ReservationRejection, Runtime, RuntimeBuilder,
};
#[allow(unused_imports)]
use ego_service_sdk_macros::{idempotent, operation, service, tenant_scoped};
use persistent_entity::command_context::CommandContext;
use persistent_entity::entity_ref::EntityRef;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::CommandResult;
use persistent_entity::runtime::EntityRuntime;
use reference_app::domain::tenant_org::{
    OrganizationEnsured, TenantOrgCommand, TenantOrgState, TenantOrganizationEntity,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// Set by the parent to put the child into child mode, and to tell it which
/// database to use. Absent in an ordinary run, which is what makes the child
/// test a no-op when the suite runs normally.
const CHILD_DB_URL: &str = "EGO_IT_CHILD_DB_URL";

const KEY: &str = "op-e2-single-aggregate-crash";
const TENANT: &str = "tenant-e2";
const ORG_ID: &str = "acme-e2";
const ORG_NAME: &str = "Acme E2";

/// Owner A completes the operation and dies; owner B retries and observes the
/// replay. Distinct owners so the retry is unambiguously a different node, not
/// a renewal.
const OWNER_A: &str = "single-agg-owner-a";
const OWNER_B: &str = "single-agg-owner-b";

const LEASE: Duration = Duration::from_secs(30);

/// `idempotency.reservation.outcome`'s value when a completed operation is
/// answered from the reservation's stored response rather than re-executed.
const RESERVATION_ON_REPLAY: &str = "succeeded";

// ---------------------------------------------------------------------------
// The service under test: one aggregate, one operation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnsureOrgInput {
    pub org_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnsureOrgOutput {
    pub org_id: String,
}

#[derive(Debug)]
pub enum EnsureOrgError {
    Security(SecurityError),
    EntityWrite(String),
    Refused(ReservationRejection),
}

impl From<SecurityError> for EnsureOrgError {
    fn from(e: SecurityError) -> Self {
        EnsureOrgError::Security(e)
    }
}

impl From<ReservationRejection> for EnsureOrgError {
    fn from(r: ReservationRejection) -> Self {
        EnsureOrgError::Refused(r)
    }
}

impl From<EntityError> for EnsureOrgError {
    fn from(e: EntityError) -> Self {
        EnsureOrgError::EntityWrite(e.to_string())
    }
}

impl std::fmt::Display for EnsureOrgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnsureOrgError::Security(e) => write!(f, "security error: {e}"),
            EnsureOrgError::EntityWrite(m) => write!(f, "entity write error: {m}"),
            EnsureOrgError::Refused(r) => write!(f, "operation refused: {r}"),
        }
    }
}

impl ServiceErrorTrait for EnsureOrgError {
    fn code(&self) -> &str {
        "ENSURE_ORG_ERROR"
    }
    fn category(&self) -> ErrorCategory {
        match self {
            EnsureOrgError::Security(_) => ErrorCategory::Authorization,
            EnsureOrgError::EntityWrite(_) => ErrorCategory::System,
            EnsureOrgError::Refused(_) => ErrorCategory::Business,
        }
    }
    fn message(&self) -> String {
        self.to_string()
    }
}

/// One aggregate, one `#[idempotent]` operation — the minimal shape a
/// single-aggregate crash-and-replay scenario needs, deliberately smaller than
/// `RegisterUser`.
#[service(version = "1.0.0")]
pub trait EnsureOrg {
    #[operation]
    #[tenant_scoped]
    #[idempotent]
    async fn ensure(
        &self,
        ctx: ServiceContext,
        input: EnsureOrgInput,
    ) -> Result<EnsureOrgOutput, EnsureOrgError>;
}

/// Counts how many times the handler body actually ran. A replay must leave
/// this at zero — the whole point of the scenario.
pub struct EnsureOrgImpl {
    org_runtime: Arc<EntityRuntime<OrganizationEnsured>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl EnsureOrg for EnsureOrgImpl {
    async fn ensure(
        &self,
        ctx: ServiceContext,
        input: EnsureOrgInput,
    ) -> Result<EnsureOrgOutput, EnsureOrgError> {
        self.calls.fetch_add(1, Ordering::SeqCst);

        let org_ref = self
            .org_runtime
            .entity_ref::<TenantOrgCommand, TenantOrgState>(
                "tenant_organization",
                input.org_id.clone(),
                Arc::new(TenantOrganizationEntity::new()),
            )?;
        let _result: CommandResult<OrganizationEnsured, TenantOrgState> = org_ref
            .send_command(
                TenantOrgCommand::Ensure {
                    org_id: input.org_id.clone(),
                    name: input.name.clone(),
                },
                // The identity the reservation admitted, carried down unchanged —
                // exactly the transfer `RegisterUserImpl::register` performs, and
                // what makes the per-aggregate receipt gate below able to answer a
                // retry without re-running this body.
                CommandContext::new("tenant_organization".to_string())
                    .carrying(ctx.operation_identity()),
            )
            .await?;

        Ok(EnsureOrgOutput {
            org_id: input.org_id,
        })
    }
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

/// Builds the service through the same real-Postgres composition
/// `dual_aggregate_crash_recovery_postgres.rs` uses for `RegisterUser`'s two
/// aggregates, narrowed to the one this scenario needs.
///
/// Returns the `Runtime` itself, not a resolved proxy. `#[tenant_scoped]`'s
/// generated guard upgrades a `Weak<RuntimeInner>` on every call
/// (`self.runtime.upgrade()`) and fails closed with `SecurityError::MissingContext`
/// the instant that upgrade finds no strong reference left — "a dropped runtime
/// is itself an unresolvable context" per its own doc comment. A proxy resolved
/// here and returned alone would be the only strong reference dropped at this
/// function's end, so every call through it would already be talking to a dead
/// runtime. Keeping `Runtime` alive in the caller's scope is what
/// `crates/service-sdk/tests/idempotent_dispatch.rs`'s fixtures do too — they
/// never let `rt` go out of scope before the proxy calls it backs.
async fn productive_app(
    url: &str,
    owner: &str,
    observability: Option<Arc<dyn Observability>>,
    calls: Arc<AtomicUsize>,
) -> Runtime {
    let pool = connect(url).await;
    let org_store = PostgreSQLEventStore::open(
        pool.clone(),
        |_aggregate_type: &str, value: serde_json::Value, occurred_at: DateTime<Utc>| {
            OrganizationEnsured::from_stored(value, occurred_at)
        },
    )
    .await
    .expect("the event store opens against the migrated database");
    let org_runtime = Arc::new(
        persistent_entity::builder::EntityRuntimeBuilder::new()
            .with_event_store(Arc::new(org_store))
            .tenant_id(TENANT)
            .build(),
    );
    let reservations = Arc::new(PostgresOperationReservationStore::new(
        pool,
        Arc::new(SystemClock),
    ));

    let mut builder = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
        .with_operation_reservation_store(reservations)
        .with_reservation_owner_id(OwnerId::new(owner))
        .with_reservation_lease_duration(LEASE);
    if let Some(obs) = observability {
        builder = builder.with_observability(obs);
    }
    builder
        .with_service::<EnsureOrgTag>(Arc::new(EnsureOrgImpl { org_runtime, calls }))
        .expect("registration succeeds")
        .build()
}

async fn connect(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the database accepts connections")
}

fn ctx() -> ServiceContext {
    let principal = Principal::new(
        PrincipalKind::User,
        SubjectId::new("user:e2").expect("valid subject"),
    )
    .with_tenant_id(TenantId::new(TENANT).expect("valid tenant"));
    ServiceContext::new()
        .with_security(Arc::new(SecurityContext::empty(principal)))
        .with_operation_key(OperationKey::parse(KEY).expect("a non-empty key parses"))
}

fn input() -> EnsureOrgInput {
    EnsureOrgInput {
        org_id: ORG_ID.to_string(),
        name: ORG_NAME.to_string(),
    }
}

// --- durable observations ---------------------------------------------------

async fn event_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE aggregate_type = $1 AND tenant_id = $2",
    )
    .bind(aggregate_type)
    .bind(TENANT)
    .fetch_one(pool)
    .await
    .expect("the count comes back")
}

async fn receipt_count(pool: &PgPool, aggregate_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_receipts \
         WHERE aggregate_type = $1 AND operation_key = $2 AND tenant_id = $3",
    )
    .bind(aggregate_type)
    .bind(KEY)
    .bind(TENANT)
    .fetch_one(pool)
    .await
    .expect("the count comes back")
}

/// `(state, response)` for the one reservation row this scenario writes, or
/// `None` if it does not exist yet.
async fn reservation_state(pool: &PgPool) -> Option<(String, Option<Vec<u8>>)> {
    sqlx::query_as(
        "SELECT state, response FROM operation_reservations \
         WHERE operation_key = $1 AND tenant_id = $2",
    )
    .bind(KEY)
    .bind(TENANT)
    .fetch_optional(pool)
    .await
    .expect("the reservation reads back")
}

/// Everything this process's `idempotency.reservation.outcome` counted.
#[derive(Default)]
struct ReservationOutcomes(std::sync::Mutex<Vec<String>>);

impl ReservationOutcomes {
    fn recorded(&self) -> Vec<String> {
        let mut seen = self.0.lock().expect("not poisoned").clone();
        seen.sort();
        seen
    }
}

impl Observability for ReservationOutcomes {
    fn trace(&self, _event: SemanticEvent) {}
    fn log(&self, _level: ego_domain::observability::Level, _message: &str) {}
    fn record_metric(&self, observation: ego_domain::observability::MetricObservation<'_>) {
        if observation.name != "idempotency.reservation.outcome" {
            return;
        }
        if let Some(outcome) = observation
            .attributes
            .iter()
            .find(|a| a.key == "outcome")
            .map(|a| a.value.to_string())
        {
            self.0.lock().expect("not poisoned").push(outcome);
        }
    }
}

// --- the child --------------------------------------------------------------

/// The crashing half, run only as a child process.
///
/// A no-op in an ordinary suite run: without the parent's environment there is
/// no database to use, and aborting here would take the whole suite down with
/// it.
#[tokio::test]
async fn child_completes_ensure_org_then_aborts() {
    let Ok(url) = std::env::var(CHILD_DB_URL) else {
        // Not a child. Nothing to do.
        return;
    };

    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = productive_app(&url, OWNER_A, None, calls.clone()).await;
    let proxy = runtime
        .resolve::<EnsureOrgTag>()
        .expect("registered tag resolves");

    let out = proxy
        .ensure(ctx(), input())
        .await
        .expect("the first attempt completes");
    assert_eq!(out.org_id, ORG_ID, "sanity: the real output");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the handler ran once");

    // Verify the commit landed **by direct SQL query**, before anything else —
    // this is the "verify, then crash" option this scenario's own brief names
    // explicitly, and it is what makes the abort below meaningful: without
    // this check, an abort right after a silently-failed commit would prove
    // nothing about replay.
    let observer = connect(&url).await;
    let (state, response) = reservation_state(&observer)
        .await
        .expect("the reservation this operation just completed exists");
    assert_eq!(
        state, "completed",
        "the reservation must already be durably completed before the crash"
    );
    assert!(
        response.is_some(),
        "and it must carry the stored response the retry will replay"
    );
    assert_eq!(
        event_count(&observer, "tenant_organization").await,
        1,
        "the aggregate's event is durable before the crash"
    );
    assert_eq!(
        receipt_count(&observer, "tenant_organization").await,
        1,
        "and so is its confirmed receipt"
    );
    observer.close().await;

    // Real crash: no unwinding, no destructor, nothing released cleanly.
    std::process::abort();
}

// --- the parent -------------------------------------------------------------

/// A crash after a single-aggregate operation's full commit is recovered by
/// replay, not repetition.
#[tokio::test]
async fn a_crash_after_full_commit_is_recovered_by_replay_not_repetition() {
    let db = isolated_database().await;
    let pool = db.pool().await;

    // --- phase 1: a real crash, in a real other process, after a real commit ---
    let child =
        std::process::Command::new(std::env::current_exe().expect("this test binary has a path"))
            .args([
                "--exact",
                "infrastructure::single_aggregate_crash_recovery_postgres::\
                 child_completes_ensure_org_then_aborts",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DB_URL, db.url())
            .output()
            .expect("the child process starts");

    assert_eq!(
        child.status.signal(),
        Some(6),
        "the child must die by SIGABRT specifically, not merely exit non-zero — \
         a failed assertion inside the child (e.g. the commit not actually \
         landing) also exits non-zero, and that must be told apart from a \
         crash. Got status {:?}\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr),
    );

    // --- the durable state the crash left, re-checked from the parent -------
    let (state, response) = reservation_state(&pool)
        .await
        .expect("the crash left its completed reservation behind");
    assert_eq!(state, "completed");
    assert!(response.is_some());
    let events_before = event_count(&pool, "tenant_organization").await;
    let receipts_before = receipt_count(&pool, "tenant_organization").await;
    assert_eq!(events_before, 1);
    assert_eq!(receipts_before, 1);

    // --- phase 2: a fresh process's worth of state retries ------------------
    //
    // Not a third OS process — the same convention
    // `dual_aggregate_crash_recovery_postgres.rs` uses: the *parent* never
    // crashed, so a fresh pool, a fresh runtime and a different owner id here
    // are what a genuinely different replica retrying after its peer died
    // actually has.
    let outcomes = Arc::new(ReservationOutcomes::default());
    let calls_b = Arc::new(AtomicUsize::new(0));
    let runtime_b = productive_app(
        db.url(),
        OWNER_B,
        Some(outcomes.clone() as Arc<dyn Observability>),
        calls_b.clone(),
    )
    .await;
    let proxy_b = runtime_b
        .resolve::<EnsureOrgTag>()
        .expect("registered tag resolves");

    let out = proxy_b
        .ensure(ctx(), input())
        .await
        .expect("the retry must complete — by replay, not by re-execution");

    // --- WHAT THIS SCENARIO PROMISES -----------------------------------
    assert_eq!(
        out.org_id, ORG_ID,
        "the replayed answer must be the one the crashed process actually \
         produced"
    );
    assert_eq!(
        calls_b.load(Ordering::SeqCst),
        0,
        "the handler must not run a second time: a completed operation is \
         answered from the reservation's own stored response, so neither the \
         aggregate nor its receipt gate is ever reached"
    );
    assert_eq!(
        outcomes.recorded(),
        vec![RESERVATION_ON_REPLAY.to_string()],
        "the reservation guard itself decided this attempt by replay — not by \
         takeover, which is the outcome a still-in-progress or expired lease \
         would have produced instead"
    );

    // --- no duplicate rows exist afterward -----------------------------
    assert_eq!(
        event_count(&pool, "tenant_organization").await,
        events_before,
        "no second event was written"
    );
    assert_eq!(
        receipt_count(&pool, "tenant_organization").await,
        receipts_before,
        "no second receipt was written"
    );
    let (final_state, final_response) = reservation_state(&pool)
        .await
        .expect("the reservation still exists");
    assert_eq!(final_state, "completed");
    assert!(final_response.is_some());

    pool.close().await;
    db.close().await;
}
