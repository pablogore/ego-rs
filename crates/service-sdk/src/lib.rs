//! Service SDK — contracts, runtime, DI, and interceptors for ego-rs services.

pub mod app;
pub mod context;
pub mod contract;
pub mod di;
pub mod error;
pub mod health;
pub mod idempotency;
pub mod implementation;
pub mod interceptor;
pub mod registry;
pub mod runtime;

// Shared #[cfg(test)] fixtures for internal unit tests (code-review fix:
// consolidates near-identical AllowCrossTenant/DenyCrossTenant/authenticated_ctx
// stubs that had already drifted — context/mod.rs's copy was missing the
// Deny variant).
#[cfg(test)]
mod test_support;

pub use app::*;
pub use context::*;
pub use contract::*;
pub use di::*;
pub use error::*;
pub use health::*;
pub use idempotency::*;
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
