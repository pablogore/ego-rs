// Compile-pass (feedback G2): `#[service(version = "...", impl_of = Trait)]`
// combined on one struct expands to a usable `HasServiceTag` impl — proves
// `version` and `impl_of` compose at real macro-expansion time, not just at
// the `ServiceArgs` parser level (see the unit tests in
// `service-sdk-macros/src/tests.rs` for the parser-only coverage).
use std::sync::Arc;

use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::runtime::HasServiceTag;
#[allow(unused_imports)]
use ego_service_sdk_macros::operation;
use ego_service_sdk_macros::service;

#[service(version = "1.0.0")]
pub trait GreetingService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

#[service(version = "1.0.0", impl_of = GreetingService)]
struct GreetingServiceImpl;

#[ego_service_sdk::async_trait::async_trait]
impl GreetingService for GreetingServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hello, {name}"))
    }
}

fn main() {
    let instance = Arc::new(GreetingServiceImpl);
    let _service: Arc<dyn GreetingService> = HasServiceTag::into_service(instance);
}
