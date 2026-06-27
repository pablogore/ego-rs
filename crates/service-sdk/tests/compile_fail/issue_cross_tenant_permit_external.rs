// Compile-fail test: `issue_cross_tenant_permit` is `pub(crate)` and must not
// be callable from external crates.
//
// If this file starts compiling successfully it means the visibility gate was
// accidentally widened — restore it to `pub(crate)`.
fn main() {
    let inner = ego_service_sdk::runtime::RuntimeInner::default();
    let _permit = inner.issue_cross_tenant_permit();
}
