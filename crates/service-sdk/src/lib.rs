//! Service SDK crate for defining and resolving application services.
//!
//! This crate provides the core functionality for declaring service contracts,
//! resolving services through a registry, and managing service dependencies.
//!
//! # Core Concepts
//!
//! - **Service Contracts**: Declared via `#[service]` attribute on traits
//! - **Service Registry**: Central registry for service implementations
//! - **Service References**: Generated proxy types for service invocation
//! - **Dependency Injection**: Field-declared dependencies resolved at runtime
//! - **Service Context**: Propagated across service calls for tracing and tenant isolation
//!
//! # Examples
//!
//! See the [`ego-service-sdk-macros`](https://docs.rs/ego-service-sdk-macros)
//! crate for usage examples and the `tests/` directory for working integration tests.

pub use kitlogger;

pub mod builder;
pub mod context;
pub mod contract;
pub mod error;
pub mod implementation;
pub mod interceptor;
pub mod lib_tests;
pub mod logging_example;
pub mod reference;
pub mod registry;
pub mod runtime;
pub mod tenant;
pub mod testing;
pub mod service_tests;

pub use builder::*;
pub use context::*;
pub use contract::*;
pub use error::*;
pub use implementation::*;
pub use interceptor::*;
pub use logging_example::*;
pub use reference::*;
pub use registry::*;
pub use runtime::*;
pub use tenant::*;
