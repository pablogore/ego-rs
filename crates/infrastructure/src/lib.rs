//! Infrastructure layer for ego-rs.
//!
//! Provides concrete implementations of application layer ports
//! (e.g., repositories, external service clients, persistence).

pub mod observability;
pub mod persistence;

pub use observability::NoopObservability;
