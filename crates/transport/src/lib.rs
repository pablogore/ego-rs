//! Transport layer for ego-rs.
//!
//! Provides HTTP/gRPC handlers and routing that delegate to application services.

pub mod config;

pub use config::GrpcServerConfig;
