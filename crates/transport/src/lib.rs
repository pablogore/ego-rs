//! Transport layer for ego-rs.
//!
//! Provides a minimal, generic axum HTTP layer — application state,
//! JWT-based security-context extraction, an error-response mapper, and the
//! carriers that read an inbound operation key and trace context out of a
//! request (AD-2: mechanism only, no reference-app-specific routes).
//! Concrete routes belong to the application that mounts this layer's
//! `Router`.
//!
//! # gRPC-shaped types, and still no gRPC server
//!
//! Two things here are gRPC-shaped. [`GrpcServerConfig`] is a validated
//! bind-address/port subtree an application composes into its own
//! configuration, and — behind the non-default `grpc` feature —
//! `GrpcMetadataCarrier` reads the same operation-key location out of a
//! `tonic` metadata map that [`HeaderCarrier`] reads out of HTTP headers.
//!
//! Neither makes this crate a gRPC transport, and the distinction is
//! load-bearing rather than pedantic. Nothing here binds a socket for gRPC,
//! builds a `tonic` service, or routes a gRPC call: [`serve`] is axum-only,
//! and the two gRPC types are a configuration shape and a read-only view over
//! a metadata map. Reading the presence of a metadata carrier as "gRPC is
//! wired up in this layer" is what would let the second transport arrive
//! sideways — a route table, then a dispatch path, then per-protocol
//! behaviour — under a name that only ever promised somewhere to look.
//!
//! That is also why both carriers stay this thin. Each one contributes a
//! location and hands its raw value to the SDK's `resolve_operation_key`;
//! validation and the missing-key policy live there and nowhere else. A
//! second wire format therefore adds a place to look and no second answer,
//! which is the property that keeps the idempotency guarantee identical
//! whichever protocol carried the request.

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
