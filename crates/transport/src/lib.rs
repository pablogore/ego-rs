//! Transport layer for ego-rs.
//!
//! Provides a minimal, generic axum HTTP layer — application state,
//! JWT-based security-context extraction, and an error-response mapper
//! (AD-2: mechanism only, no gRPC transport, no reference-app-specific
//! routes). Concrete routes belong to the application that mounts this
//! layer's `Router`.

pub mod config;
pub mod error;
pub mod idempotency;
pub mod operation_key;
pub mod propagation;
pub mod security;
pub mod server;
pub mod state;

pub use config::GrpcServerConfig;
pub use error::TransportError;
#[cfg(feature = "grpc")]
pub use idempotency::GrpcMetadataCarrier;
pub use idempotency::HeaderCarrier;
pub use operation_key::OperationKeyExtractor;
pub use propagation::TraceContextExtractor;
pub use security::AuthenticatedContext;
pub use server::serve;
pub use state::AppState;
