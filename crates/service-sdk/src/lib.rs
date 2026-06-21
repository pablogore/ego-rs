//! Service SDK crate for defining and resolving application services.
//!
//! This crate provides the core functionality for declaring service contracts,
//! resolving services through a registry, and managing service dependencies.
//!
//! # Core Concepts
//!
//! - **Service Contracts**: Declared via `#[service]` attribute on traits
//! - **Service Registry**: Central registry for service implementations
//! - **Dependency Injection**: Field-declared dependencies resolved at runtime
//! - **Service Context**: Propagated across service calls for tracing and tenant isolation

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
