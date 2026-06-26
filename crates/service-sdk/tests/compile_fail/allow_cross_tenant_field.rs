use ego_service_sdk::context::ServiceContext;

fn main() {
    let mut ctx = ServiceContext::new();
    ctx.allow_cross_tenant = true;
}
