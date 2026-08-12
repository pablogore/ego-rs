//! **Guarantee:** two identical `POST /register` requests execute the operation
//! once, record one durable answer in PostgreSQL, and serve the second request
//! from that stored row without running the body again.
//!
//! **Layers traversed:** HTTP (the production router, real authentication and
//! guards) → the generated `#[idempotent]` dispatch → `OperationReservationStore`
//! → `PostgresOperationReservationStore` → real SQL against a real PostgreSQL,
//! including the migrations that create the reservations table.
//!
//! **Why in-process cannot show this.** The in-process sibling of this test
//! (`examples/reference-app/tests/http_replay_and_conflict.rs`) proves the
//! *dispatch* replays when a store says `Succeeded`. It cannot prove the answer
//! survived, because its store hands back a value the test itself scripted. Here
//! nothing is scripted: the response is written by the first request's real
//! commit, read back by a real query, and the row is the only place it exists.
//!
//! # How the replay is tied to the stored bytes
//!
//! `RegisterOutput` copies `user_id` and `tenant_id` from the input, so two
//! identical requests produce byte-identical responses whether the body ran
//! once, twice, or never. **Comparing the two responses cannot fail, which makes
//! it worth nothing.**
//!
//! So between the two requests this test reaches into PostgreSQL and overwrites
//! the stored response with a value the handler could not produce from this
//! input. If that value comes back over HTTP, it came from the row — there is no
//! other source for it. That is the whole point of the SQL write in the middle:
//! it is not setup, it is the instrument that makes provenance observable.
//!
//! It also settles "no second completion" without needing to trust a counter
//! alone: a second `complete()` would overwrite the sentinel with the genuine
//! answer, so finding the sentinel still in place afterwards is independent
//! evidence that nothing wrote again.
//!
//! Run: `cargo test --manifest-path integration-tests/Cargo.toml`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
    build_runtime, AppConfig, BuiltRuntime, DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE,
};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

const KEY: &str = "op-replay-under-test";

/// The value the handler cannot produce. Written into the row by SQL between the
/// two requests, so its arrival over HTTP has exactly one possible explanation.
const SENTINEL_USER_ID: &str = "REPLAYED-FROM-POSTGRES";

/// Counts executions while delegating to the **real** `RegisterUser`.
///
/// A counting stub would also count, but it would replace the domain logic this
/// test claims to traverse. Wrapping keeps the production implementation in the
/// path and still makes "ran once" observable — the count is the only thing
/// added.
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

/// Counts what the durable store was asked to do, and delegates every call to it.
///
/// Not a substitute for the SQL assertions below — it answers a question SQL
/// cannot. The row shows the *final state*; these counters show how many times
/// the runtime *asked*. "No second completion" needs both: a `complete()` that
/// was attempted and refused leaves the same row as one never attempted.
struct CountingStore {
    inner: PostgresOperationReservationStore,
    reserves: AtomicUsize,
    completes: AtomicUsize,
}

impl CountingStore {
    fn wrapping(inner: PostgresOperationReservationStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reserves: AtomicUsize::new(0),
            completes: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl OperationReservationStore for CountingStore {
    async fn reserve(&self, req: ReserveRequest) -> Result<ReservationOutcome, ReservationError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
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
///
/// Duplicated from the reference app's `tests/support` module rather than
/// imported: that module belongs to another crate's test binaries and is not
/// part of its public surface. Both build on the same public constants, so they
/// cannot drift on the signing key or audience.
fn make_token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(DEV_SIGNING_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}

/// The production router, over the durable store. Only two things are wrapped,
/// and both only count.
fn app(store: Arc<CountingStore>, calls: Arc<AtomicUsize>) -> Router {
    let BuiltRuntime {
        authn,
        read_side: read_side_handles,
        ..
    } = build_runtime(&AppConfig::default()).expect("build_runtime succeeds");

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

/// The identical request, both times. Same key and same payload: this is one
/// operation retried, not two operations.
fn post() -> Request<Body> {
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
                "email": "user@example.com",
                "tenant_id": "tenant-a",
                "org_name": "Acme",
            })
            .to_string(),
        ))
        .unwrap()
}

async fn send(router: Router) -> (StatusCode, RegisterOutput) {
    let response = router.oneshot(post()).await.expect("the router answers");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body")
        .to_vec();
    let parsed = serde_json::from_slice(&body).unwrap_or_else(|e| {
        panic!(
            "expected a RegisterOutput, got {e} for: {}",
            String::from_utf8_lossy(&body)
        )
    });
    (status, parsed)
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

/// The durable row for this operation: state, when it completed, and the bytes.
async fn stored_row(pool: &PgPool) -> (String, Option<DateTime<Utc>>, Option<Vec<u8>>) {
    sqlx::query_as(
        "SELECT state, completed_at, response \
         FROM operation_reservations WHERE operation_key = $1",
    )
    .bind(KEY)
    .fetch_one(pool)
    .await
    .expect("exactly one reservation row for this key")
}

#[tokio::test]
async fn two_identical_posts_execute_once_and_the_second_is_served_from_postgres() {
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

    // --- First POST ---------------------------------------------------------
    let (status, first) = send(app(store.clone(), calls.clone())).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        first.user_id, "user-1",
        "the first answer is built from the request — which is what makes the \
         sentinel below meaningful, since it proves this path returns \
         request-derived values when it really executes"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the body ran exactly once");
    assert_eq!(
        store.completes.load(Ordering::SeqCst),
        1,
        "one execution records one answer"
    );

    // --- What the first request made durable --------------------------------
    let (state, first_completed_at, first_bytes) = stored_row(&pool).await;
    assert_eq!(state, "completed");
    assert!(
        first_completed_at.is_some(),
        "a completed reservation carries when it completed"
    );
    let genuine = first_bytes.expect("the response was stored, not left null");
    assert_eq!(
        genuine,
        encode_stored_response(&RegisterOutput {
            user_id: "user-1".to_string(),
            tenant_id: "tenant-a".to_string(),
        })
        .expect("encodes")
        .as_bytes()
        .to_vec(),
        "the stored bytes are the encoded answer the handler actually produced"
    );

    // --- The instrument: replace the stored answer with an impossible one ----
    //
    // Not setup. Two identical requests over a deterministic handler produce
    // identical bytes, so nothing about the second response could distinguish a
    // replay from a re-execution. After this write, one value means "read from
    // the row" and any other means "recomputed".
    let sentinel = encode_stored_response(&RegisterOutput {
        user_id: SENTINEL_USER_ID.to_string(),
        tenant_id: "tenant-a".to_string(),
    })
    .expect("encodes");
    let updated =
        sqlx::query("UPDATE operation_reservations SET response = $1 WHERE operation_key = $2")
            .bind(sentinel.as_bytes().to_vec())
            .bind(KEY)
            .execute(&pool)
            .await
            .expect("the sentinel is written");
    assert_eq!(
        updated.rows_affected(),
        1,
        "the sentinel must actually land, or the assertion below would pass \
         against the genuine answer and prove nothing"
    );

    // --- Second, identical POST ---------------------------------------------
    let (status, second) = send(app(store.clone(), calls.clone())).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        second.user_id, SENTINEL_USER_ID,
        "the second response carries the bytes that were in PostgreSQL. The \
         handler cannot produce this value from this input, so a re-execution \
         would have answered 'user-1' instead"
    );

    // --- Nothing ran, and nothing was recorded again -------------------------
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "still one execution: a replay that re-runs the body produces a second \
         set of effects, which is the duplicate this whole mechanism prevents"
    );
    assert_eq!(
        store.reserves.load(Ordering::SeqCst),
        2,
        "both requests consulted the store — the second was answered by it, not \
         short-circuited before reaching it"
    );
    assert_eq!(
        store.completes.load(Ordering::SeqCst),
        1,
        "no second completion was even attempted. A replay carries a stored \
         response and no permit, so there is no fence to complete under"
    );

    // --- and the row is untouched by the replay ------------------------------
    //
    // Read back independently of the counter above. A `complete()` that was
    // attempted and refused leaves the same row as one never attempted, so the
    // counter answers "was it asked" and this answers "did anything change".
    let (state_after, completed_at_after, bytes_after) = stored_row(&pool).await;
    assert_eq!(state_after, "completed");
    assert_eq!(
        completed_at_after, first_completed_at,
        "the completion timestamp is the first request's, unchanged"
    );
    assert_eq!(
        bytes_after.as_deref(),
        Some(sentinel.as_bytes()),
        "the sentinel survived. A second completion would have overwritten it \
         with the genuine answer, so this is independent evidence that the \
         replay wrote nothing"
    );
}
