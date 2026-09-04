//! **Guarantee:** PROD-P1.1 Required Test 2 — a real Production-style
//! composition, built over a real, migrated PostgreSQL, serves `/health`
//! and `/ready` over a real TCP socket and real HTTP framing, through the
//! exact `ego_transport::serve`/`build_router` chain `main.rs` runs — not a
//! bare `HealthAggregator::new()` built just to answer the probe.
//!
//! **Layers traversed:** a real `reqwest::Client` → `ego_transport::serve`
//! on a `TcpListener` bound to `127.0.0.1:0` →
//! `reference_app::ports::http::build_router` → `AppState.runtime`
//! (`RuntimeResolver::readiness()`/`liveness()`) → the real
//! `HealthAggregator` the composition built.
//!
//! **Known limitation, reported rather than simulated (P1.1 brief):** this
//! composition — identical to `wire_register_postgres.rs`'s, `main.rs`'s own
//! `IdempotencyWiring::Compatibility` with no reservation store or data
//! provider — registers zero `HealthContributor`s today, so PostgreSQL
//! connectivity does not yet participate in this readiness signal. `/ready`
//! is proven 200 for a real, healthy Production-style composition; it does
//! not yet flip to 503 when Postgres goes away. That gap is not invented
//! away here — see the PROD-P1.1 final report.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_integration_tests::{isolated_database, TEST_PRODUCTION_JWT_KEY};
use ego_transport::AppState;
use reference_app::ports::http::build_router;
use reference_app::{AppConfig, EntityEventStores, ExternalEffectsWiring};
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

/// Builds the reference app over a real, migrated Postgres pool — the same
/// composition root `main.rs` calls — and serves its real router on a real,
/// ephemeral TCP socket. Mirrors `wire_register_postgres.rs::spawn_real_server`.
async fn spawn_real_server(url: &str) -> (std::net::SocketAddr, JoinHandle<()>) {
    let pool = connect(url).await;
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");
    let built = reference_app::build_runtime_with(
        &AppConfig {
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

#[tokio::test(flavor = "multi_thread")]
async fn wire_health_and_ready_return_200_for_a_real_postgres_backed_composition() {
    let db = isolated_database().await;
    let url = db.url().to_string();
    let (addr, server) = spawn_real_server(&url).await;

    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    let ready = client
        .get(format!("http://{addr}/ready"))
        .send()
        .await
        .expect("a real HTTP client reaches the bound socket");
    assert_eq!(
        ready.status(),
        reqwest::StatusCode::OK,
        "a real, healthy Production-style composition must report ready"
    );

    server.abort();
    db.close().await;
}
