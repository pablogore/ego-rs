// Compile-fail test: `CanonicalTenant` cannot be constructed from outside
// `crate::runtime` (AD-003). `scoped`/`systemwide` are `pub(super)` —
// visible only within `ego_service_sdk::runtime` and its sibling submodules.
//
// If this file starts compiling successfully it means the visibility gate
// was accidentally widened — restore the constructors to `pub(super)`.
fn main() {
    let tenant_id = ego_domain::context::TenantId::new("tenant-a").unwrap();
    let _tenant = ego_service_sdk::runtime::CanonicalTenant::scoped(tenant_id);
}
