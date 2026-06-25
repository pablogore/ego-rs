use ego_service_sdk::context::ServiceContext;

fn main() {
    let ctx = ServiceContext::new();
    let _ = ctx.allow_cross_tenant();
}
