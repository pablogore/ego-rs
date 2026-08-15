#![deny(missing_docs)]
//! TestKit — reusable test building blocks for ego.rs services.
//!
//! **Same-contract principle**: everything TestKit hands to a test is the
//! real production type or a real implementation of a real production
//! trait (`ServiceContext`, `SecurityContext`, `Principal`,
//! `Arc<dyn AuthorizationProvider>`, `Arc<KITLogger>`, `ConfigValue<C>`), or a
//! thin ergonomic wrapper over the real `RuntimeBuilder`/`Runtime`. TestKit
//! never introduces a parallel or divergent implementation of a production
//! contract, so a test exercises real dispatch and real validation logic,
//! not a look-alike stand-in that can silently drift from production.

mod assertions;
mod authz;
mod config;
mod context;
mod effects;
mod event_store;
mod fixtures;
mod health;
mod idempotency;
mod identity;
mod jwt;
mod logger;
mod observability_conformance;
mod providers;
mod reservation;
mod reservation_conformance;
mod security;

pub use assertions::{assert_authorized, assert_denied};
#[cfg(feature = "dev-providers")]
pub use authz::AllowAllAuthorizationProvider;
pub use authz::{DenyAllAuthorizationProvider, ScriptedAuthorizationProvider};
pub use config::TestConfig;
pub use context::{test_context, TestContextBuilder};
pub use effects::{RecordedAttempt, RecordingExecutor};
pub use event_store::assert_event_store_conformance;
pub use fixtures::{FixtureBuilder, ServiceTestFixture};
pub use health::StaticHealthContributor;
pub use idempotency::assert_carrier_conformance;
pub use identity::{principal, PrincipalBuilder};
pub use jwt::TestJwtBuilder;
pub use logger::{CapturedRecord, CapturingLogger};
pub use observability_conformance::{
    assert_metric_attributes_are_preserved, RecordedMetric, PROBE_METRIC,
};
pub use providers::{RecordingDataProvider, StaticDataProvider};
pub use reservation::{InMemoryOperationReservationStore, TestClock};
pub use reservation_conformance::{
    assert_lease_mutation_conformance, assert_purge_conformance,
    assert_reservation_store_conformance, assert_reserve_conformance,
};
pub use security::{authenticated, authenticated_with_claims};
