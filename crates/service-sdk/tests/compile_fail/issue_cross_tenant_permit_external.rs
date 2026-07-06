// Compile-fail test: `issue_cross_tenant_permit` is `pub(crate)` and must not
// be callable from external crates.
//
// If this file starts compiling successfully it means the visibility gate was
// accidentally widened — restore it to `pub(crate)`.
fn main() {
    let rt = ego_service_sdk::runtime::RuntimeBuilder::new().build();
    let _permit = rt.inner().issue_cross_tenant_permit();
}
