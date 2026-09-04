//! **Guarantee:** `build_runtime_with`'s `Profile::Production` JWT
//! verification-key gate (PROD-P0.2) is genuinely fail-closed: a missing
//! key, or the repository's own committed `DEV_SIGNING_KEY`, refuses to
//! build rather than silently authenticating production traffic with a
//! public-by-definition secret. The accept path is exercised too, so this
//! file cannot pass by making the gate reject everything.
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
use reference_app::{
    AppConfig, EntityEventStores, ExternalEffectsWiring, IdempotencyWiring, DEV_SIGNING_KEY,
};
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

/// Required Test 1: no configured key → `Err`, never a panic.
#[tokio::test]
async fn production_profile_without_a_configured_key_is_refused() {
    let (db, stores) = production_stores().await;

    let result = reference_app::build_runtime_with(
        &AppConfig::default(),
        stores,
        IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
        None,
        None,
    );

    let Err(err) = result else {
        panic!("Profile::Production with no configured JWT verification key must be refused");
    };
    let message = err.to_string();
    assert!(
        message.contains("jwt_verification_key"),
        "the refusal must name the missing JWT verification key, got: {message}"
    );

    db.close().await;
}

/// Required Test 2: the repository's own committed `DEV_SIGNING_KEY` is
/// refused, not silently accepted — the exact vulnerability PROD-P0.2 closes.
#[tokio::test]
async fn production_profile_with_the_committed_dev_key_is_refused() {
    let (db, stores) = production_stores().await;

    let result = reference_app::build_runtime_with(
        &AppConfig {
            jwt_verification_key: Some(DEV_SIGNING_KEY.to_vec()),
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
        panic!("Profile::Production must refuse the committed development signing key");
    };
    let message = err.to_string();
    let dev_key_utf8 = std::str::from_utf8(DEV_SIGNING_KEY).expect("the dev key is ASCII");
    assert!(
        !message.contains(dev_key_utf8),
        "the refusal must never echo the dev key's own bytes back, got: {message}"
    );
    assert!(
        message.contains("development JWT key"),
        "the refusal must name the reason as the dev key, got: {message}"
    );

    db.close().await;
}

/// Required Test 3: an external, non-dev key through the same config path
/// builds successfully — the accept path, so Tests 1/2 cannot pass by making
/// this gate reject everything unconditionally.
#[tokio::test]
async fn production_profile_with_an_external_key_builds() {
    let (db, stores) = production_stores().await;

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
    .expect("an external, non-dev, >=32-byte key must be accepted under Profile::Production");

    db.close().await;
}
