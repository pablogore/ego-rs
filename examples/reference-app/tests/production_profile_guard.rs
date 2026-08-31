//! PROD-013 AD-10: regression guard for the `Profile::Dev` half of IS-12.
//!
//! Two behaviors, not one: `EntityEventStores::in_memory()` must keep
//! declaring [`Profile::Dev`], AND a composition built over it must keep
//! building. A profile-only assertion would not catch a future change that
//! flips the declared profile to `Production` while leaving the stores
//! themselves in memory — the exact drift AD-8 exists to make unreachable.
//! No pool, no Docker: this runs under plain `cargo test -p reference-app`.
//! The `Profile::Production` half lives in the existing
//! `integration-tests/tests/infrastructure/durable_entity_progress_postgres.rs`,
//! which already opens a real pool `EntityEventStores::open` needs.

use ego_service_sdk::runtime::Profile;
use reference_app::{build_runtime_with, AppConfig, EntityEventStores, ExternalEffectsWiring};

#[test]
fn in_memory_stores_declare_profile_dev() {
    assert_eq!(
        EntityEventStores::in_memory().profile(),
        Profile::Dev,
        "in_memory() must never look like a production declaration"
    );
}

#[test]
fn dev_profile_still_builds_at_the_composition_root() {
    build_runtime_with(
        &AppConfig::default(),
        EntityEventStores::in_memory(),
        reference_app::IdempotencyWiring::Compatibility,
        None,
        ExternalEffectsWiring::None,
    )
    .expect("Profile::Dev over in-memory stores must still build (SC-5)");
}
