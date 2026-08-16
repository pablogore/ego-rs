//! Infrastructure layer for ego-rs.
//!
//! Provides concrete implementations of application layer ports
//! (e.g., repositories, external service clients, persistence).

pub mod metrics_otlp;
pub mod observability;
pub mod persistence;
pub mod tracing_otlp;

pub use observability::NoopObservability;
pub use tracing_otlp::{OtlpConfig, OtlpProtocol, OtlpTracer};
