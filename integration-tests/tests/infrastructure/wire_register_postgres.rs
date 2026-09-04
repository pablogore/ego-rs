//! **Guarantee:** the reference-app's real HTTP adapter — bound to a real TCP
//! socket, serving the exact `Router` production builds — can carry a
//! request across the transport-neutral service boundary all the way to a
//! durable PostgreSQL write, through the real JWT auth path.
//!
//! **Layers traversed:** a real `reqwest::Client` — real TCP, real HTTP
//! framing — → `ego_transport::serve` on a `TcpListener` bound to
//! `127.0.0.1:0` → `reference_app::ports::http::build_router` (the same
//! router `main.rs` serves) → `AuthenticatedContext`'s real
//! `Hs256AuthenticationProvider` JWT verification → `RegisterUser` →
//! `PostgreSQLEventStore` against a real, migrated PostgreSQL.
//!
//! **Why in-process cannot show this:** every existing HTTP-route test
//! (`examples/reference-app/tests/reference_app/http_route.rs`) drives the
//! router via `tower::ServiceExt::oneshot`, which never opens a socket and
//! never runs a real HTTP client or server loop — framing, connection
//! handling and `axum::serve`'s own request plumbing are all skipped. None of
//! those tests use a durable event store either. This is the only place
//! either gap is closed, together.
//!
//! **Not duplicated elsewhere:** `durable_entity_progress_postgres.rs`
//! proves the same durable-store wiring but drives `EntityRef` directly,
//! never through HTTP. `http_route.rs` proves the HTTP route table and auth
//! guard chain, but only in-process and only over in-memory stores. This
//! file is the first to combine "real socket" with "real Postgres."
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_integration_tests::{isolated_database, IsolatedDatabase, TEST_PRODUCTION_JWT_KEY};
use ego_testkit::TestJwtBuilder;
use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{AppConfig, EntityEventStores, ExternalEffectsWiring, REFERENCE_APP_AUDIENCE};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the container accepts connections")
}

async fn postgres() -> (IsolatedDatabase, String) {
    let db = isolated_database().await;
    let url = db.url().to_string();
    (db, url)
}

/// Builds the reference app over a real, migrated Postgres pool — the same
/// composition root `main.rs` calls — and serves its real router on a real,
/// ephemeral TCP socket. Returns the bound address and the server task's
/// handle; the caller aborts the handle once done (this test never exercises
/// graceful shutdown, only the wire path).
async fn spawn_real_server(url: &str) -> (std::net::SocketAddr, JoinHandle<()>) {
    let pool = connect(url).await;
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");
    let built = reference_app::build_runtime_with(
        &AppConfig {
            // PROD-P0.2: this is the CORE-018 real-wire acceptance path —
            // proving it works through the new external key source, not
            // just the old committed dev constant, is the point.
            jwt_verification_key: Some(TEST_PRODUCTION_JWT_KEY.to_vec()),
            ..AppConfig::default()
        },
        stores,
        reference_app::IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        None,
        None,
    )
    .expect("the reference app builds");

    let state = AppState::new(built.app.resolver(), built.authn);
    let router = build_router(state, built.read_side.query.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a real, ephemeral TCP socket");
    let addr = listener
        .local_addr()
        .expect("a bound listener reports a local address");

    let handle = tokio::spawn(async move {
        let _ = ego_transport::serve(listener, router, std::future::pending()).await;
    });

    (addr, handle)
}

fn token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(TEST_PRODUCTION_JWT_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}

/// `tenant_id` is the persistence-layer scope the row was actually written
/// under, not the JWT/domain-level `tenant_id` claim — `AppConfig::default()`
/// (used here and by `main.rs`) leaves `RuntimeConfig::single_tenant_mode` at
/// its default `true`, which `EntityRuntime::entity_ref` resolves to the
/// literal tenant `"default"` (`crates/persistent-entity/src/runtime.rs:259`),
/// not `NULL` — every entity is written under that scope regardless of the
/// tenant named in the request. Callers pass `Some("default")` to match.
async fn user_event_count(pool: &PgPool, user_id: &str, tenant_id: Option<&str>) -> i64 {
    // `IS NOT DISTINCT FROM`, not `=`: `tenant_id` is nullable and `NULL = NULL`
    // is never true in SQL — same idiom as `crates/persistence/src/postgres/
    // event_store.rs`'s tenant-scoped reads.
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM events \
         WHERE aggregate_type = 'user' AND aggregate_id = $1 \
           AND tenant_id IS NOT DISTINCT FROM $2",
    )
    .bind(user_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("the count comes back")
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_register_with_valid_auth_persists_and_returns_created() {
    let (db, url) = postgres().await;
    let (addr, server) = spawn_real_server(&url).await;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/register"))
        .bearer_auth(token("user-wire-1", "tenant-wire"))
        .json(&json!({
            "user_id": "user-wire-1",
            "email": "wire@example.com",
            "tenant_id": "tenant-wire",
            "org_name": "Wire Co",
        }))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);

    let assertion_pool = connect(&url).await;
    assert_eq!(
        user_event_count(&assertion_pool, "user-wire-1", Some("default")).await,
        1,
        "the registration must be durably persisted in PostgreSQL, not just acknowledged over HTTP"
    );

    server.abort();
    db.close().await;
}

/// Aggregate-state dedup (`UserEntity::handle_command`'s documented
/// defence-in-depth against a repeated `Register` on already-`Registered`
/// state — see `examples/reference-app/src/domain/user.rs`), not
/// key/receipt-based idempotency: this runtime is wired with
/// `IdempotencyWiring::Compatibility`, so no `Idempotency-Key` header or
/// reservation store is involved. The guarantee under test is narrower and
/// still real: the same registration replayed over the real wire does not
/// duplicate the durable event.
#[tokio::test(flavor = "multi_thread")]
async fn wire_register_replay_dedupes_via_aggregate_state_not_key_idempotency() {
    let (db, url) = postgres().await;
    let (addr, server) = spawn_real_server(&url).await;

    let client = reqwest::Client::new();
    let body = json!({
        "user_id": "user-wire-2",
        "email": "wire2@example.com",
        "tenant_id": "tenant-wire",
        "org_name": "Wire Co",
    });
    let bearer = token("user-wire-2", "tenant-wire");

    for _ in 0..2 {
        let response = client
            .post(format!("http://{addr}/register"))
            .bearer_auth(&bearer)
            .json(&body)
            .send()
            .await
            .expect("a real HTTP client reaches the bound socket");
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    let assertion_pool = connect(&url).await;
    assert_eq!(
        user_event_count(&assertion_pool, "user-wire-2", Some("default")).await,
        1,
        "replaying the identical registration over the real wire must not duplicate the durable event"
    );

    server.abort();
    db.close().await;
}
