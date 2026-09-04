//! **Guarantee:** `build_runtime_with`'s `Profile::Production` tenancy gate
//! (PROD-P0.3) is genuinely fail-closed: `single_tenant_mode = false`
//! refuses to build rather than silently accepting a deployment whose
//! authenticated tenants would all persist into the same fixed durable
//! scope (`EntityRuntime::entity_ref` never receives a per-request tenant —
//! see the gate's own comment in `reference_app::build_runtime_with`). The
//! accept path is exercised too, so this file cannot pass by making the
//! gate reject everything.
//!
//! **Layers traversed:** the same composition root `main.rs` calls —
//! `build_runtime_with` — over a real, migrated PostgreSQL pool via
//! `EntityEventStores::open`, the only way `Profile::Production` is
//! reachable (see `durable_entity_progress_postgres.rs`'s own note on this).
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

use ego_integration_tests::{isolated_database, IsolatedDatabase, TEST_PRODUCTION_JWT_KEY};
use persistent_entity::profile::Profile;
use persistent_entity::runtime::RuntimeConfig;
use reference_app::{AppConfig, EntityEventStores, ExternalEffectsWiring, IdempotencyWiring};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("the isolated database accepts connections")
}

async fn production_stores() -> (IsolatedDatabase, EntityEventStores) {
    let db = isolated_database().await;
    let pool = connect(db.url()).await;
    let stores = EntityEventStores::open(pool)
        .await
        .expect("the stores open against a migrated database");
    assert_eq!(stores.profile(), Profile::Production);
    (db, stores)
}

/// Required Test 1: `single_tenant_mode = false` under `Profile::Production`
/// is refused — an `Err`, never a panic, never a started runtime. Persistence
/// tenant identity is fixed once per `EntityRuntime` (see
/// `RuntimeConfig.tenant_id`/`entity_ref`), so this mode would silently
/// collapse every authenticated tenant's durable data into one shared scope.
#[tokio::test]
async fn production_profile_with_shared_multi_tenant_mode_is_refused() {
    let (db, stores) = production_stores().await;

    let result = reference_app::build_runtime_with(
        &AppConfig {
            runtime: RuntimeConfig {
                single_tenant_mode: false,
                tenant_id: "tenant-a".to_string(),
                ..RuntimeConfig::default()
            },
            jwt_verification_key: Some(TEST_PRODUCTION_JWT_KEY.to_vec()),
            ..AppConfig::default()
        },
        stores,
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        None,
        None,
    );

    let Err(err) = result else {
        panic!("Profile::Production with single_tenant_mode = false must be refused");
    };
    let message = err.to_string();
    assert!(
        message.contains("single_tenant_mode"),
        "the refusal must name the unsupported tenancy setting, got: {message}"
    );

    db.close().await;
}

/// Required Test 2: the supported single-tenant-per-deployment mode still
/// builds under `Profile::Production` — the accept path, so Test 1 above
/// cannot pass by making the gate reject everything unconditionally.
#[tokio::test]
async fn production_profile_with_single_tenant_mode_builds() {
    let (db, stores) = production_stores().await;
    assert!(
        AppConfig::default().runtime.single_tenant_mode,
        "sanity: the supported mode is also the default"
    );

    reference_app::build_runtime_with(
        &AppConfig {
            jwt_verification_key: Some(TEST_PRODUCTION_JWT_KEY.to_vec()),
            ..AppConfig::default()
        },
        stores,
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        None,
        None,
    )
    .expect("single-tenant-per-deployment must be accepted under Profile::Production");

    db.close().await;
}
