//! PROD-013 AD-8: `EntityEventStores` carries the deployment [`Profile`] it
//! declares — not a separate flag a host could pass inconsistently with the
//! stores it actually built.
//!
//! Only [`EntityEventStores::in_memory`] is exercised here: no pool, no
//! Docker, so this runs under plain `cargo test -p reference-app`. The
//! `Profile::Production` half (`EntityEventStores::open`) needs a real
//! PostgreSQL and lives in
//! `integration-tests/tests/infrastructure/entity_event_stores_wiring_postgres.rs`.

use ego_service_sdk::runtime::Profile;
use reference_app::EntityEventStores;

#[test]
fn in_memory_stores_declare_profile_dev() {
    assert_eq!(
        EntityEventStores::in_memory().profile(),
        Profile::Dev,
        "in_memory() must never look like a production declaration"
    );
}
