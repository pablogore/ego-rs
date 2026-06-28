//! Service SDK — contracts, runtime, DI, and interceptors for ego-rs services.

pub mod context;
pub mod contract;
pub mod di;
pub mod error;
pub mod implementation;
pub mod interceptor;
pub mod registry;
pub mod runtime;

pub use context::*;
pub use contract::*;
pub use di::*;
pub use error::*;
pub use implementation::*;
pub use interceptor::*;
pub use registry::*;
pub use runtime::*;

// Re-export async_trait so generated code can reference it via ego_service_sdk::async_trait.
#[doc(hidden)]
pub use async_trait;

// Re-export for #[service] codegen — avoids requiring service crates to list ego-security-sdk directly.
#[doc(hidden)]
pub use ego_security_sdk as security;
