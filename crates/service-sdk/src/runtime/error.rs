//! Failure semantics for CORE-017's infrastructure bootstrap and teardown.
//!
//! `RuntimeInfraError` is cross-cutting to CORE-017 — produced by
//! [`crate::runtime::ConfigurationProvider::logging`], `build_logger`, and
//! `Runtime::shutdown()` alike — so it lives in its own module rather than
//! inside whichever of those three files happened to need it first. It is
//! unrelated to [`crate::error::ServiceError`], which models business-level
//! errors returned to service callers; this type models infrastructure
//! construction/lifecycle failures instead.

use thiserror::Error;

/// Only the variants the real APIs can actually produce: `ConfigInvalid`
/// (serde, in [`crate::runtime::ConfigurationProvider::logging`]) and
/// `LoggerInit` (`AdapterError`, in `build_logger`'s `logger.init()` call)
/// cover host bootstrap; `Teardown` covers `Runtime::shutdown()`.
///
/// Host-bootstrap-only today — never propagate this error's `Display`/`Debug`
/// text directly into a client-facing (HTTP/gRPC) response. `reason` wraps an
/// external dependency's internal error text verbatim; treat it as an
/// operator/log-facing diagnostic, not a value safe to hand to callers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeInfraError {
    #[error("invalid configuration: {reason}")]
    ConfigInvalid { reason: String },
    #[error("logger initialization failed: {reason}")]
    LoggerInit { reason: String },
    #[error("infrastructure teardown failed: {reason}")]
    Teardown { reason: String },
}
