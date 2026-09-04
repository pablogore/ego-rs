//! Shared test-support helpers (Findings 8 & 10 of the CORE-018 review):
//! `RegisterUserImpl` construction boilerplate and JWT minting were each
//! hand-rolled, byte-for-byte identical, across several test files.
//!
//! A submodule of the single `tests/reference_app.rs` integration-test
//! binary (issue #305): every other file under `tests/reference_app/` reaches
//! it via `use crate::support;`.
//!
//! Each consuming module only uses a subset of these helpers, so
//! `dead_code` is expected per-module (not a real problem) — hence the
//! blanket allow below, the standard idiom for shared test-support modules.
#![allow(dead_code)]

use std::sync::Arc;

use ego_domain::Observability;
use ego_testkit::TestJwtBuilder;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::runtime::EntityRuntime;
use reference_app::application::{RegisterUser, RegisterUserImpl};
use reference_app::domain::tenant_org::OrganizationEnsured;
use reference_app::read_side::ReadSideSink;
use reference_app::{DEV_SIGNING_KEY, REFERENCE_APP_AUDIENCE};
use serde_json::Value;

/// Builds a fresh `RegisterUserImpl` (two independent in-memory
/// `EntityRuntime`s, AD-4) plus the org-side runtime, for callers that need
/// to inspect persisted org state afterward (e.g. the partial-failure
/// orphan proof) or wire a read-side sink.
pub fn make_register_user_full(
    observability: Option<Arc<dyn Observability>>,
    read_side_sink: Option<ReadSideSink>,
) -> (
    Arc<dyn RegisterUser>,
    Arc<EntityRuntime<OrganizationEnsured>>,
) {
    let org_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let user_runtime = Arc::new(EntityRuntimeBuilder::new().build());
    let mut service = RegisterUserImpl::new(org_runtime.clone(), user_runtime, observability);
    if let Some(sink) = read_side_sink {
        service = service.with_read_side_sink(sink);
    }
    (Arc::new(service) as Arc<dyn RegisterUser>, org_runtime)
}

/// Builds a fresh `RegisterUserImpl` when the caller doesn't need the
/// underlying entity runtimes or a read-side sink afterward — the common
/// case across the guard-chain/observability test files.
pub fn make_register_user(observability: Option<Arc<dyn Observability>>) -> Arc<dyn RegisterUser> {
    make_register_user_full(observability, None).0
}

/// Mints an Hs256 JWT that authenticates against `build_runtime`'s
/// `Hs256AuthenticationProvider` (see `reference_app::DEV_SIGNING_KEY`).
///
/// The token carries an `aud` claim equal to
/// [`reference_app::REFERENCE_APP_AUDIENCE`] — the audience `AppConfig`'s
/// `jwt.expected_aud` now requires. Without it, validation would fail with an
/// `aud` mismatch instead of authenticating.
pub fn make_token(sub: &str, tenant_id: &str) -> String {
    TestJwtBuilder::new(DEV_SIGNING_KEY.to_vec())
        .subject(sub)
        .tenant_id(tenant_id)
        .claim("aud", Value::from(REFERENCE_APP_AUDIENCE))
        .build()
}
