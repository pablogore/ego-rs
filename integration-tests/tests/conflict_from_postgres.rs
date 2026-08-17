//! **Guarantee:** a second request reusing a completed operation's key with a
//! different payload is refused permanently, never executes, and leaves the
//! first request's durable answer exactly as it was.
//!
//! **Layers traversed:** HTTP (the production router, real authentication and
//! guards) → the generated `#[idempotent]` dispatch → `OperationReservationStore`
//! → `PostgresOperationReservationStore` → real SQL, including the migrations
//! that create the reservations table and its two partial unique indexes.
//!
//! **Why in-process cannot show this.** One correction first, because this
//! test's own README row used to claim the wrong thing. The fingerprint
//! comparison is **not** a database constraint — it is
//! `existing.fingerprint != req.fingerprint` in Rust, on a value read back from
//! the row. Saying otherwise would justify this test with a mechanism that does
//! not exist.
//!
//! What *is* infrastructure-only is the step before it. Reserving an operation is
//! two statements — `INSERT … ON CONFLICT DO NOTHING`, then read the row back and
//! compare — and it only ever *reaches* the comparison because
//! `(tenant_id, operation_key)` is genuinely unique. Without that uniqueness the
//! insert succeeds, a **second row** appears, and the conflict is never detected
//! while every in-process test keeps passing.
//!
//! # Which index this scenario actually loads
//!
//! The protocol needs uniqueness across **both** tenancy modes, and migration 010
//! provides it as two partial indexes over complementary predicates — one
//! `WHERE tenant_id IS NOT NULL`, one `WHERE tenant_id IS NULL` — because
//! `NULLS NOT DISTINCT` postdates the declared PostgreSQL floor.
//!
//! This scenario runs entirely under `tenant-a`, so it loads exactly one of them:
//! the tenant-scoped index. Measured, both directions:
//!
//! - Deleting **only** `ux_operation_reservations_identity_tenant`: the second
//!   request is admitted, a second row appears, the body runs again, and a
//!   valid-looking `201 {"user_id":"user-1","tenant_id":"tenant-a"}` comes back.
//! - Deleting **only** `ux_operation_reservations_identity_systemwide`: this test
//!   stays **green**. It never files a row with a null tenant, so that index is
//!   not on its path.
//!
//! That second result is why the claim here is scoped rather than sweeping. This
//! file guards the tenant-scoped predicate; the systemwide one is covered by the
//! reservation store's own conformance tests, which exercise the null-tenant
//! scope in-process.
//!
//! A null-tenant variant is deliberately **not** added. It would re-exercise the
//! same mechanism through a different predicate, turning this end-to-end test into
//! a matrix over an index's `WHERE` clause and spending container time to learn
//! nothing new about the protocol. That is the fast suite's job.
//!
//! **Two assertions catch the tenant-scoped mutation independently**, and both are
//! kept on purpose. The status assertion fires first and is what a client would
//! notice. The row count (`= 1`) was checked separately under the same mutation
//! and also fails, at `2 != 1` — it is the one that names the *mechanism*, so a
//! future change that produced a 409 for some other reason would still not
//! satisfy it. Neither is redundant: the status describes the consequence, the row
//! count describes why.
//!
//! # The fingerprint's representation, and why shape alone was not enough
//!
//! The comparison that produces a conflict is string equality against the
//! `fingerprint` column, so the digest has to come back in the form it went in.
//!
//! An earlier version asserted only that the stored value was 64 characters and
//! all hex. That is not the property: `to_uppercase()` satisfies both and still
//! breaks the equality the protocol runs on. So the real assertion compares the
//! stored value against the fingerprint **observed at the port** by
//! `CountingStore` — what PostgreSQL was handed, versus what PostgreSQL returned.
//! Nothing is recomputed, so the test commits to no assumption about how the
//! generated dispatch builds a fingerprint.
//!
//! Measured, by uppercasing the digest in the adapter's `INSERT`: this test fails
//! at that equality, `EFBD…` against `efbd…`, while both shape checks pass.
//!
//! Stated precisely, because it is easy to overclaim: all three tests in this
//! suite fail under that mutation, since every one of them depends on the digest
//! round-tripping. What this assertion adds is *localisation* — the others report
//! a wrong status or outcome several layers away, and this one names the boundary
//! and shows both values.
//!
//! One further thing only a real database can show: a refused request does not
//! corrupt the completed reservation it collided with.
//!
//! # What is deliberately not re-proven here
//!
//! The six-way refusal mapping — which reservation outcome becomes which HTTP
//! status — is already covered in-process in
//! `examples/reference-app/tests/http_replay_and_conflict.rs`. This file asserts
//! the conflict status once, as the observable consequence, and does not
//! re-enumerate the table. A scenario that re-tested the mapping against a
//! container would run identically without one, which is the definition of a test
//! that does not belong in this suite.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Utc};
use ego_domain::operation::{
    OperationReservationStore, OwnerFence, OwnerId, ReservationError, ReservationOutcome,
    ReserveRequest, StoredServiceResponse,
};
use ego_domain::time::SystemClock;
use ego_persistence::postgres::migrations;
use ego_persistence::postgres::reservation::PostgresOperationReservationStore;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::{
    encode_stored_response, IdempotencyEnforcementMode, RuntimeBuilder,
};
use ego_testkit::{ScriptedAuthorizationProvider, TestJwtBuilder};
use ego_transport::AppState;
use persistent_entity::builder::EntityRuntimeBuilder;
use reference_app::application::{
    RegisterInput, RegisterOutput, RegisterUser, RegisterUserError, RegisterUserImpl,
    RegisterUserTag,
};
use reference_app::ports::http::build_router;
use reference_app::{
    build_runtime_in_memory, AppConfig, BuiltRuntime, DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE,
};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

const KEY: &str = "op-conflict-under-test";

/// The two payloads differ in `email` only.
///
/// Chosen so the *output* would be identical either way — `RegisterOutput` carries
/// only `user_id` and `tenant_id`. That isolates the fingerprint as the sole
/// discriminator: if the second request were admitted, it would answer with the
/// same bytes as the first and a response comparison would notice nothing.
const EMAIL_FIRST: &str = "user@example.com";
const EMAIL_SECOND: &str = "someone-else@example.com";

/// Counts executions while delegating to the real `RegisterUser`.
struct CountingRegister {
    inner: Arc<dyn RegisterUser>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl RegisterUser for CountingRegister {
    async fn register(
        &self,
        ctx: ServiceContext,
        input: RegisterInput,
    ) -> Result<RegisterOutput, RegisterUserError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.register(ctx, input).await
    }
}

/// Counts what the durable store was asked to do, and delegates every call.
///
/// It also records the fingerprint of every `ReserveRequest` that passes through,
/// in order. That is the boundary where the runtime hands a fingerprint to
/// storage, so it is the one place a test can learn what PostgreSQL was *given*
/// without recomputing it — and therefore without knowing how the generated
/// dispatch produced it.
struct CountingStore {
    inner: PostgresOperationReservationStore,
    reserves: AtomicUsize,
    completes: AtomicUsize,
    fingerprints: Mutex<Vec<String>>,
}

impl CountingStore {
    fn wrapping(inner: PostgresOperationReservationStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reserves: AtomicUsize::new(0),
            completes: AtomicUsize::new(0),
            fingerprints: Mutex::new(Vec::new()),
        })
    }

    /// The fingerprints handed to storage, in the order the requests arrived.
    fn fingerprints(&self) -> Vec<String> {
        self.fingerprints.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl OperationReservationStore for CountingStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
        self.fingerprints
            .lock()
            .expect("not poisoned")
            .push(req.fingerprint.as_str().to_string());
        self.inner.reserve(req).await
    }
    async fn renew(&self, f: &OwnerFence, until: DateTime<Utc>) -> Result<(), ReservationError> {
        self.inner.renew(f, until).await
    }
    async fn complete(
        &self,
        f: &OwnerFence,
        r: StoredServiceResponse,
    ) -> Result<(), ReservationError> {
        self.completes.fetch_add(1, Ordering::SeqCst);
        self.inner.complete(f, r).await
    }
    async fn abandon(&self, f: &OwnerFence) -> Result<(), ReservationError> {
        self.inner.abandon(f).await
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

/// Mints a token the reference app's own authentication provider accepts.
fn make_token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(DEV_SIGNING_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}

/// The production router over the durable store. Only the counting is added.
fn app(store: Arc<CountingStore>, calls: Arc<AtomicUsize>) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime_in_memory(&AppConfig::default()).expect("build_runtime succeeds");

    let real: Arc<dyn RegisterUser> = Arc::new(RegisterUserImpl::new(
        Arc::new(EntityRuntimeBuilder::new().build()),
        Arc::new(EntityRuntimeBuilder::new().build()),
        None,
    ));
    let counted: Arc<dyn RegisterUser> = Arc::new(CountingRegister { inner: real, calls });

    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::MandatoryKey)
        .with_operation_reservation_store(store)
        .with_reservation_owner_id(OwnerId::new("replica-under-test"))
        .with_reservation_lease_duration(Duration::from_secs(30))
        .with_service::<RegisterUserTag>(counted)
        .expect("registration succeeds")
        .with_security(
            authn.clone(),
            Arc::new(ScriptedAuthorizationProvider::allow_all()),
        )
        .build();

    build_router(
        AppState::new(runtime.resolver(), authn),
        read_side_handles.query.clone(),
    )
}

/// The same operation key every time; only `email` varies.
fn post(email: &str) -> Request<Body> {
    let token = make_token("user-1", "tenant-a");
    Request::builder()
        .method("POST")
        .uri("/register")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", KEY)
        .body(Body::from(
            serde_json::json!({
                "user_id": "user-1",
                "email": email,
                "tenant_id": "tenant-a",
                "org_name": "Acme",
            })
            .to_string(),
        ))
        .unwrap()
}

async fn send(router: Router, email: &str) -> (StatusCode, Vec<u8>) {
    let response = router
        .oneshot(post(email))
        .await
        .expect("the router answers");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body")
        .to_vec();
    (status, body)
}

async fn migrated_pool(url: &str) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections");
    migrations::run(&pool)
        .await
        .expect("the real migrations apply — including the reservations table");
    pool
}

/// How many rows exist for this key. The assertion the unique indexes carry.
async fn row_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM operation_reservations WHERE operation_key = $1")
        .bind(KEY)
        .fetch_one(pool)
        .await
        .expect("the count comes back")
}

/// The durable row: which fingerprint it was filed under, and what it answers.
async fn stored_row(pool: &PgPool) -> (String, String, Option<DateTime<Utc>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "SELECT fingerprint, state, completed_at, response \
         FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(KEY)
    .fetch_one(pool)
    .await
    .expect("exactly one reservation row for this key")
}

#[tokio::test]
async fn a_different_payload_under_a_completed_key_is_refused_and_changes_nothing() {
    let container = Postgres::default()
        .start()
        .await
        .expect("a PostgreSQL container starts");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        container.get_host().await.expect("a host"),
        container
            .get_host_port_ipv4(5432)
            .await
            .expect("the mapped port"),
    );
    let pool = migrated_pool(&url).await;

    let store = CountingStore::wrapping(PostgresOperationReservationStore::new(
        pool.clone(),
        Arc::new(SystemClock),
    ));
    let calls = Arc::new(AtomicUsize::new(0));

    // --- First POST: the operation runs and completes ------------------------
    let (status, body) = send(app(store.clone(), calls.clone()), EMAIL_FIRST).await;
    assert_eq!(status, StatusCode::CREATED);
    let first: RegisterOutput = serde_json::from_slice(&body).expect("a RegisterOutput");
    assert_eq!(first.user_id, "user-1");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the body ran once");
    assert_eq!(store.completes.load(Ordering::SeqCst), 1);

    let (stored_fp, state, completed_at, stored_response) = stored_row(&pool).await;
    assert_eq!(state, "completed");
    // The digest round-tripped as the same hex string it was written as.
    //
    // Not a truncation guard: PostgreSQL *rejects* an over-length value for
    // `VARCHAR(n)` rather than silently shortening it — verified directly, a
    // 16-character value into `VARCHAR(8)` raises `value too long for type
    // character varying(8)`. An earlier version of this comment justified the
    // check by silent truncation, which does not happen.
    //
    // What is at stake is representation. The comparison that produces a conflict
    // is string equality against this column, so anything that changed the
    // digest's form on the way through — case folding, padding, hex decoded to
    // bytes and re-encoded some other way — would compare unequal to a freshly
    // computed digest and turn every replay into a conflict, or the reverse.
    //
    // Shape alone does not establish that, and an earlier version of this test
    // stopped there. `to_uppercase()` is still 64 hex characters and would sail
    // past both assertions below while breaking exactly the equality the protocol
    // depends on. So the shape checks stay as a sanity check on what a failure
    // message will show, and the real assertion is the one after them.
    assert_eq!(
        stored_fp.len(),
        64,
        "a SHA-256 hex digest is 64 characters; got {stored_fp:?}"
    );
    assert!(
        stored_fp.chars().all(|c| c.is_ascii_hexdigit()),
        "the stored fingerprint is hex, not a mangled encoding: {stored_fp:?}"
    );

    // The digest PostgreSQL was handed is the digest PostgreSQL returned.
    //
    // The left side is observed at the port boundary by `CountingStore`, not
    // recomputed. That matters twice over: it is the actual value the runtime
    // produced for this request, and it commits this test to nothing about *how*
    // the generated dispatch builds a fingerprint. Comparing against a locally
    // computed `operation_fingerprint(&(&input,))` would pin the macro's argument
    // encoding and fail this test on an unrelated change to it.
    let observed = store.fingerprints();
    assert_eq!(
        observed.len(),
        1,
        "one request has reached the store so far, got {observed:?}"
    );
    assert_eq!(
        stored_fp, observed[0],
        "the fingerprint read back from PostgreSQL must be byte-identical to the \
         one it was given. A store that re-encoded it would still hold 64 hex \
         characters while comparing unequal to every future request — silently \
         turning replays into conflicts"
    );

    // --- Second POST: same key, different payload ----------------------------
    let (status, body) = send(app(store.clone(), calls.clone()), EMAIL_SECOND).await;

    // --- The premise, from what the runtime actually produced -----------------
    //
    // The whole scenario rests on these two payloads fingerprinting differently.
    // If they collided — a canonicalisation that dropped `email`, say — the second
    // request would be a *replay*, and the assertions below would be describing
    // something else entirely.
    //
    // Asserted from the two fingerprints observed at the port rather than from
    // locally recomputed ones, for the same reason as above: no assumption about
    // the dispatch's encoding, and it is the real values that decided this
    // request's fate. Checked before the status, so a collision reports itself
    // rather than surfacing as a confusing 201.
    let observed = store.fingerprints();
    assert_eq!(observed.len(), 2, "two requests have reached the store");
    assert_ne!(
        observed[0], observed[1],
        "the two payloads must fingerprint differently, or this test is silently \
         exercising replay instead of conflict"
    );

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "reusing a completed key with different arguments is a permanent \
         refusal, not a replay: got body {}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "still one execution — a conflicting request must never reach the body"
    );
    assert_eq!(
        store.completes.load(Ordering::SeqCst),
        1,
        "nothing ran, so there was no second answer to record"
    );

    // --- What the unique indexes are actually doing ---------------------------
    //
    // Reserving is `INSERT … ON CONFLICT DO NOTHING` followed by a read-back:
    // without genuine uniqueness on (tenant_id, operation_key) the insert would
    // succeed, a second row would exist, and the comparison that produces the
    // conflict would never have been reached.
    //
    // The status assertion above already catches that, so this is not the only
    // line standing between the mutation and a green run. It is kept because it
    // names the mechanism rather than the symptom: verified independently under
    // the dropped-index mutation, where it fails at 2 != 1.
    //
    // The index this loads is the tenant-scoped one — this scenario never files a
    // row with a null tenant. See the module docs for the measurement, including
    // the converse: deleting the systemwide index leaves this test green.
    assert_eq!(
        row_count(&pool).await,
        1,
        "exactly one reservation row for this key. A second row means the insert \
         was not refused, so the conflict was never detected — the 409 above \
         would then be reporting something other than what happened"
    );

    // --- Permanent, not transient -------------------------------------------
    //
    // "Permanent" is a claim about repetition, so it is tested by repeating.
    // A conflict that resolved itself on retry would be a very different
    // contract, and a client told 409 would be right to retry against it.
    let (status_again, _) = send(app(store.clone(), calls.clone()), EMAIL_SECOND).await;
    assert_eq!(
        status_again,
        StatusCode::CONFLICT,
        "the refusal repeats — nothing about the first refusal cleared the way"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "and still nothing ran");
    assert_eq!(
        store.reserves.load(Ordering::SeqCst),
        3,
        "all three requests reached the store; the refusals came from it"
    );

    // --- The collided-with reservation is untouched --------------------------
    //
    // A refusal that corrupted the row it collided with would serve the wrong
    // answer to every later replay of the *original* request — a failure only
    // visible by reading the row back after the fact.
    let (fp_after, state_after, completed_at_after, response_after) = stored_row(&pool).await;
    assert_eq!(state_after, "completed");
    assert_eq!(
        fp_after, stored_fp,
        "still filed under the first payload's fingerprint; the refused request \
         did not overwrite the identity of the operation that won"
    );
    assert_eq!(
        completed_at_after, completed_at,
        "the completion timestamp is the first request's, unchanged"
    );
    assert_eq!(
        response_after, stored_response,
        "and the stored answer is byte-identical to what the first request \
         recorded"
    );
    assert_eq!(
        response_after.as_deref(),
        Some(
            encode_stored_response(&RegisterOutput {
                user_id: "user-1".to_string(),
                tenant_id: "tenant-a".to_string(),
            })
            .expect("encodes")
            .as_bytes()
        ),
        "spelled out rather than only compared to the earlier read, so this holds \
         even if the first read had itself returned something unexpected"
    );
}
