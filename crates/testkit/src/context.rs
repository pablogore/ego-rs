//! `ServiceContext` builder (CORE-022 Phase 4, design.md AD-2, AD-4).

use std::sync::Arc;

use ego_security_sdk::context::SecurityContext;
use ego_service_sdk::ServiceContext;
use kitlogger::KITLogger;

use crate::{identity::principal, security::authenticated};

/// Builds a real [`ServiceContext`]; each `build()` produces an independent
/// value — two builders share no state.
pub struct TestContextBuilder {
    security: Option<SecurityContext>,
    logger: Option<Arc<KITLogger>>,
    tenant: Option<String>,
    correlation: Option<String>,
}

impl TestContextBuilder {
    /// Starts a builder with no security, logger, tenant, or correlation id set.
    pub fn new() -> Self {
        Self {
            security: None,
            logger: None,
            tenant: None,
            correlation: None,
        }
    }

    /// Attaches the given `SecurityContext`.
    pub fn security(mut self, sec: SecurityContext) -> Self {
        self.security = Some(sec);
        self
    }

    /// Represents "no authenticated principal" the same way production code
    /// does: `ServiceContext.security == None` (design.md AD-4).
    pub fn unauthenticated(mut self) -> Self {
        self.security = None;
        self
    }

    /// Attaches the given logger.
    pub fn logger(mut self, logger: Arc<KITLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Sets the tenant id.
    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Sets the correlation id.
    pub fn correlation(mut self, id: impl Into<String>) -> Self {
        self.correlation = Some(id.into());
        self
    }

    /// Builds the real [`ServiceContext`].
    pub fn build(self) -> ServiceContext {
        let mut ctx = ServiceContext::new();
        if let Some(sec) = self.security {
            ctx = ctx.with_security(Arc::new(sec));
        }
        if let Some(logger) = self.logger {
            ctx = ctx.with_logger(logger);
        }
        if let Some(tenant) = self.tenant {
            ctx = ctx.with_tenant_id(tenant);
        }
        if let Some(correlation) = self.correlation {
            ctx = ctx.with_correlation_id(correlation);
        }
        ctx
    }
}

impl Default for TestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: an authenticated `ServiceContext` for `principal()`, no logger.
pub fn test_context() -> ServiceContext {
    TestContextBuilder::new()
        .security(authenticated(principal()))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{identity::principal, security::authenticated};
    use ego_security_sdk::error::SecurityError;

    #[test]
    fn security_override_attaches_security_context() {
        let sec = authenticated(principal());
        let ctx = TestContextBuilder::new().security(sec).build();
        assert!(ctx.security().is_some());
        assert_eq!(
            ctx.security().unwrap().principal().subject_id.as_str(),
            "test:subject"
        );
    }

    #[test]
    fn unauthenticated_leaves_security_none() {
        let ctx = TestContextBuilder::new().unauthenticated().build();
        assert!(ctx.security().is_none());
        assert!(matches!(
            ctx.require_security(),
            Err(SecurityError::CapabilityNotEnabled)
        ));
    }

    #[test]
    fn independently_built_contexts_do_not_leak_state() {
        let a = TestContextBuilder::new().tenant("acme").build();
        let b = TestContextBuilder::new().tenant("contoso").build();
        assert_eq!(a.tenant_id.as_deref(), Some("acme"));
        assert_eq!(b.tenant_id.as_deref(), Some("contoso"));
    }

    #[test]
    fn test_context_is_authenticated_for_default_principal() {
        let ctx = test_context();
        assert_eq!(
            ctx.security().unwrap().principal().subject_id.as_str(),
            "test:subject"
        );
    }
}
