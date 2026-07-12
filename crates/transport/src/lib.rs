//! Transport layer for ego-rs.
//!
//! Provides a minimal, generic axum HTTP layer — application state,
//! JWT-based security-context extraction, and an error-response mapper
//! (AD-2: mechanism only, no gRPC transport, no reference-app-specific
//! routes). Concrete routes belong to the application that mounts this
//! layer's `Router`.

pub mod config;
pub mod error;
pub mod security;
pub mod server;
pub mod state;

pub use config::GrpcServerConfig;
pub use error::TransportError;
pub use security::AuthenticatedContext;
pub use server::serve;
pub use state::AppState;
