use ego_service_sdk::context::ServiceContext;

fn main() {
    let ctx = ServiceContext::new();
    // Method is now pub but requires &CrossTenantPermit — missing arg triggers E0061.
    let _ = ctx.with_cross_tenant_access();
}
