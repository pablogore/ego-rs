//! Hello Service Example — the canonical CORE-025 developer journey.
//!
//! Unlike `order_service.rs` (a pre-existing, older "manual equivalent" that
//! predates this change and never uses the real macros or a registration
//! API), this example uses the actual `#[service]`/`#[operation]` proc-macros
//! and the canonical `App::builder().service_instance` / `App::resolve` path
//! (CORE-028), which builds on the lower-level `RuntimeBuilder` primitive.
//!
//! ```ignore
//! let instance: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
//! let app = App::builder()
//!     .service_instance::<HelloServiceTag>(instance)
//!     .build()?;
//!
//! let hello = app.resolve::<HelloServiceTag>()?;
//! let out = hello.greet(ServiceContext::new(), "world".into()).await?;
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::app::App;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::ServiceError;
#[allow(unused_imports)]
use ego_service_sdk_macros::operation;
use ego_service_sdk_macros::service;

/// The service contract — defined with the real `#[service]`/`#[operation]`
/// macros, not a manual equivalent.
#[service(version = "1.0.0")]
pub trait HelloService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

/// The concrete implementation.
pub struct HelloServiceImpl;

#[async_trait]
impl HelloService for HelloServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hello, {name}"))
    }
}

#[tokio::main]
async fn main() {
    // 1. Register the pre-built implementation under its generated tag and
    //    compose the app — the canonical `App::builder()` entrypoint.
    let instance: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let app = App::builder()
        .idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .service_instance::<HelloServiceTag>(instance)
        .build()
        .expect("composition succeeds");

    // 2. Resolve the tag to its concrete, macro-generated, fully-guarded proxy.
    let hello = app
        .resolve::<HelloServiceTag>()
        .expect("registered tag resolves");

    // 3. Invoke it with an explicit ServiceContext — no hidden state.
    let out = hello
        .greet(ServiceContext::new(), "world".into())
        .await
        .expect("invocation succeeds");
    println!("{out}");
    assert_eq!(out, "hello, world");

    println!("\n✓ Example completed successfully");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hello_service_registers_resolves_and_invokes() {
        let instance: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
        let app = App::builder()
            .idempotency_enforcement_mode(
                ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
            )
            .service_instance::<HelloServiceTag>(instance)
            .build()
            .expect("composition succeeds");

        let hello = app
            .resolve::<HelloServiceTag>()
            .expect("registered tag resolves");
        let out = hello
            .greet(ServiceContext::new(), "world".into())
            .await
            .expect("invocation succeeds");
        assert_eq!(out, "hello, world");
    }
}
