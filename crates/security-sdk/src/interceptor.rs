//! Common interceptor contract for authentication pipelines.
//!
//! Transport adapters (HTTP, gRPC, broker) implement their framework's interceptor
//! interface and delegate to any `dyn Interceptor` for the auth step.

use ego_domain::auth::AuthenticationError;

use crate::context::SecurityContext;
use crate::credential_extractor::RequestContext;

/// Transport-agnostic authentication interceptor.
///
/// Returns `Ok(Some(ctx))` when authentication succeeds,
/// `Ok(None)` when no credential is present (pass-through),
/// `Err` when a credential is present but invalid.
///
/// Object-safe: can be stored as `Arc<dyn Interceptor>` and composed into pipelines.
pub trait Interceptor: Send + Sync {
    /// Inspect `ctx` and produce a [`SecurityContext`] if a credential is present.
    fn intercept(
        &self,
        ctx: &dyn RequestContext,
    ) -> Result<Option<SecurityContext>, AuthenticationError>;
}
