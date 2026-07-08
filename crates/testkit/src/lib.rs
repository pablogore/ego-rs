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
mod fixtures;
mod identity;
mod logger;
mod security;

pub use assertions::{assert_authorized, assert_denied};
#[cfg(feature = "dev-providers")]
pub use authz::AllowAllAuthorizationProvider;
pub use authz::{DenyAllAuthorizationProvider, ScriptedAuthorizationProvider};
pub use config::TestConfig;
pub use context::{test_context, TestContextBuilder};
pub use fixtures::{FixtureBuilder, ServiceTestFixture};
pub use identity::{principal, PrincipalBuilder};
pub use logger::{CapturedRecord, CapturingLogger};
pub use security::{authenticated, authenticated_with_claims};
