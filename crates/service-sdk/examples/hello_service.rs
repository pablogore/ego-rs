//! Hello Service Example — the canonical CORE-025 developer journey.
//!
//! Unlike `order_service.rs` (a pre-existing, older "manual equivalent" that
//! predates this change and never uses the real macros or a registration
//! API), this example uses the actual `#[service]`/`#[operation]` proc-macros
//! and the canonical `RuntimeBuilder::with_service` / `Runtime::resolve` path
//! added by CORE-025 (design.md's "Quick path").
//!
//! ```ignore
//! let rt = RuntimeBuilder::new()
//!     .with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl) as Arc<dyn HelloService>)?
//!     .build();
//!
//! let hello = rt.resolve::<HelloServiceTag>()?;
//! let out = hello.greet(ServiceContext::new(), "world".into()).await?;
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::runtime::RuntimeBuilder;
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
    // 1. Register the implementation under its generated tag — one call,
    //    reusing the existing ServiceRegistry/Resolvable machinery.
    let rt = RuntimeBuilder::new()
        .with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl) as Arc<dyn HelloService>)
        .expect("registration succeeds")
        .build();

    // 2. Resolve the tag to its concrete, macro-generated, fully-guarded proxy.
    let hello = rt
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
        let rt = RuntimeBuilder::new()
            .with_service::<HelloServiceTag>(Arc::new(HelloServiceImpl) as Arc<dyn HelloService>)
            .expect("registration succeeds")
            .build();

        let hello = rt
            .resolve::<HelloServiceTag>()
            .expect("registered tag resolves");
        let out = hello
            .greet(ServiceContext::new(), "world".into())
            .await
            .expect("invocation succeeds");
        assert_eq!(out, "hello, world");
    }
}
