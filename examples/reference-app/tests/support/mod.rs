//! Shared test-support helpers (Findings 8 & 10 of the CORE-018 review):
//! `RegisterUserImpl` construction boilerplate and JWT minting were each
//! hand-rolled, byte-for-byte identical, across several test files.
//!
//! `tests/support/mod.rs` (not `tests/support.rs`) is the standard Rust
//! convention for a module shared across multiple integration-test
//! binaries without becoming its own separate test binary — each file that
//! needs it declares `mod support;`.
//!
//! Each consuming test binary only uses a subset of these helpers, so
//! `dead_code` is expected per-binary (not a real problem) — hence the
//! blanket allow below, the standard idiom for shared test-support modules.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ego_domain::Observability;
use persistent_entity::builder::EntityRuntimeBuilder;
use persistent_entity::runtime::EntityRuntime;
use reference_app::application::{RegisterUser, RegisterUserImpl};
use reference_app::domain::tenant_org::OrganizationEnsured;
use reference_app::read_side::ReadSideSink;
use reference_app::DEV_SIGNING_KEY;

/// Builds a fresh `RegisterUserImpl` (two independent in-memory
/// `EntityRuntime`s, AD-4) plus the org-side runtime, for callers that need
/// to inspect persisted org state afterward (e.g. the partial-failure
/// orphan proof) or wire a read-side sink.
pub fn make_register_user_full(
    observability: Option<Arc<dyn Observability>>,
    read_side_sink: Option<ReadSideSink>,
) -> (Arc<dyn RegisterUser>, Arc<EntityRuntime<OrganizationEnsured>>) {
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
pub fn make_token(sub: &str, tenant_id: &str) -> String {
    let exp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600;
    let claims = serde_json::json!({ "sub": sub, "exp": exp, "tenant_id": tenant_id });
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(DEV_SIGNING_KEY),
    )
    .unwrap()
}
