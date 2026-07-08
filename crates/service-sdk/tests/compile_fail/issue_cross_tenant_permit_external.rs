// Compile-fail test: `issue_cross_tenant_permit` is `pub(crate)` and must not
// be callable from external crates.
//
// If this file starts compiling successfully it means the visibility gate was
// accidentally widened — restore it to `pub(crate)`.
//
// The call below is deliberately malformed for a *visible* method too (no
// `.await`, no real args) — that is fine: `E0624` (private method) fires
// before argument/type checking even runs, so this still proves the
// visibility gate, not the new async/fallible/destination-scoped signature.
fn main() {
    let rt = ego_service_sdk::runtime::RuntimeBuilder::new().build();
    let ctx = ego_service_sdk::context::ServiceContext::new();
    let destination = ego_domain::context::TenantId::new("tenant-b").unwrap();
    let _permit = rt.inner().issue_cross_tenant_permit(&ctx, destination);
}
