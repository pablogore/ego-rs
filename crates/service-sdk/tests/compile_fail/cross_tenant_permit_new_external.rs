use ego_service_sdk::CrossTenantPermit;

fn main() {
    // Path B: calling CrossTenantPermit::new() from outside crate::runtime.
    // new() is pub(super) — not callable from external code.
    let _p = CrossTenantPermit::new();
}
