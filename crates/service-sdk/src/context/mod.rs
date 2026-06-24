use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use tokio_util::sync::CancellationToken;

/// A service context that propagates across service calls for tracing, tenant isolation,
/// and other cross-cutting concerns.
///
/// `ServiceContext` is an explicit-propagation value type: it is constructed at the entry
/// point of a request and passed forward by value (ownership, clone, or parameter) to every
/// component that needs it. There is no ambient or thread-local fallback.
///
/// ## Fields
///
/// - `tenant_id` / `correlation_id` / `trace_id`: `Option<String>` — cloned by value (heap copy).
/// - `deadline` / `timeout`: `Option<SystemTime>` / `Option<Duration>` — stack-size copies.
/// - `additional_context`: `HashMap<String, String>` — cloned by value; cost is proportional
///   to the number of entries. Keep this map small.
/// - `cancellation_token`: `Option<CancellationToken>` — cheap clone (internally reference-counted).
/// - `security`: `Option<Arc<SecurityContext>>` — cheap clone (Arc reference-count increment only;
///   the underlying `SecurityContext` is NOT copied).
///
/// ## Clone cost
///
/// `ServiceContext::clone()` performs a shallow clone of `security` (Arc increment) and a
/// deep clone of string fields and the additional-context map. For typical request contexts
/// (3-5 string fields, empty or small map), this is a few heap allocations.
///
/// For hot paths that clone context frequently, prefer keeping `additional_context` empty
/// and relying on the typed fields. Avoid storing large payloads in `additional_context`.
///
/// ## Ownership model
///
/// Each component that requires a `ServiceContext` MUST declare that dependency in its
/// public signature. The context is passed forward — not looked up. Use `.clone()` when
/// passing to both an interceptor chain and an inner handler in the same call.
#[derive(Debug, Clone)]
pub struct ServiceContext {
    /// The tenant ID.
    pub tenant_id: Option<String>,
    /// The correlation ID.
    pub correlation_id: Option<String>,
    /// The trace ID.
    pub trace_id: Option<String>,
    /// The deadline.
    pub deadline: Option<SystemTime>,
    /// The timeout.
    pub timeout: Option<Duration>,
    /// The additional context.
    pub additional_context: HashMap<String, String>,
    /// Whether cross-tenant access is allowed.
    pub allow_cross_tenant: bool,
    /// Optional push-style cancellation token.
    pub cancellation_token: Option<CancellationToken>,
    /// Attached security context carrying the authenticated principal, if any.
    pub security: Option<Arc<SecurityContext>>,
}

impl ServiceContext {
    /// Creates a new service context.
    ///
    /// # Returns
    /// A new `ServiceContext` with default values
    pub fn new() -> Self {
        Self {
            tenant_id: None,
            correlation_id: None,
            trace_id: None,
            deadline: None,
            timeout: None,
            additional_context: HashMap::new(),
            allow_cross_tenant: false,
            cancellation_token: None,
            security: None,
        }
    }

    /// Sets the tenant ID.
    ///
    /// # Arguments
    /// * `tenant_id` - The tenant identifier to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the tenant ID set
    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Sets the correlation ID.
    ///
    /// # Arguments
    /// * `correlation_id` - The correlation identifier to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the correlation ID set
    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    /// Sets the trace ID.
    ///
    /// # Arguments
    /// * `trace_id` - The trace identifier to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the trace ID set
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Sets the deadline.
    ///
    /// # Arguments
    /// * `deadline` - The deadline to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the deadline set
    pub fn with_deadline(mut self, deadline: SystemTime) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Sets the timeout.
    ///
    /// # Arguments
    /// * `timeout` - The timeout to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the timeout set
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the additional context.
    ///
    /// # Arguments
    /// * `additional_context` - The additional context to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the additional context set
    pub fn with_additional_context(mut self, additional_context: HashMap<String, String>) -> Self {
        self.additional_context = additional_context;
        self
    }

    /// Allows cross-tenant access.
    ///
    /// # Returns
    /// A new `ServiceContext` with cross-tenant access enabled
    pub fn allow_cross_tenant(mut self) -> Self {
        self.allow_cross_tenant = true;
        self
    }

    /// Attaches a `CancellationToken` to this context for push-style cancellation.
    ///
    /// # Arguments
    /// * `token` - The `CancellationToken` to associate with this context
    ///
    /// # Returns
    /// A new `ServiceContext` with the cancellation token set
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Attaches a security context (authenticated identity + scope).
    ///
    /// # Arguments
    /// * `security` - The [`SecurityContext`] to associate with this service context
    ///
    /// # Returns
    /// A new `ServiceContext` with the security context set
    pub fn with_security(mut self, security: Arc<SecurityContext>) -> Self {
        self.security = Some(security);
        self
    }

    /// Returns the attached security context, if any.
    pub fn security(&self) -> Option<&SecurityContext> {
        self.security.as_deref()
    }

    /// Returns `true` if the associated `CancellationToken` has been cancelled.
    ///
    /// Returns `false` if no token is attached.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token
            .as_ref()
            .map(|t| t.is_cancelled())
            .unwrap_or(false)
    }

    /// Checks if the deadline has expired.
    ///
    /// # Returns
    /// `true` if the deadline has passed, `false` otherwise
    pub fn is_deadline_expired(&self) -> bool {
        match self.deadline {
            Some(deadline) => SystemTime::now() > deadline,
            None => false,
        }
    }

    /// Checks if cross-tenant access is allowed.
    ///
    /// # Returns
    /// `true` if cross-tenant access is enabled, `false` otherwise
    pub fn is_cross_tenant_allowed(&self) -> bool {
        self.allow_cross_tenant
    }

    /// Checks if the current context has a tenant ID.
    ///
    /// # Returns
    /// `true` if a tenant ID is set, `false` otherwise
    pub fn has_tenant(&self) -> bool {
        self.tenant_id.is_some()
    }

    /// Gets the current tenant ID.
    ///
    /// # Returns
    /// The tenant ID if set, or `None` if not set
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    /// Gets the correlation ID.
    ///
    /// # Returns
    /// The correlation ID if set, or `None` if not set
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    /// Gets the trace ID.
    ///
    /// # Returns
    /// The trace ID if set, or `None` if not set
    pub fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    /// Requires that security be enabled in the runtime.
    ///
    /// This method should be called by service handlers that need to ensure
    /// security is enabled in the runtime. If security is not enabled, it
    /// returns a `SecurityError::CapabilityNotEnabled`.
    ///
    /// # Returns
    /// * `Ok(&SecurityContext)` - If security is enabled and a security context is present
    /// * `Err(SecurityError)` - If security is not enabled in the runtime
    pub fn require_security(&self) -> Result<&SecurityContext, SecurityError> {
        self.security
            .as_deref()
            .ok_or(SecurityError::CapabilityNotEnabled)
    }
}

impl Default for ServiceContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A context key for storing and retrieving typed values from a service context.
///
/// This trait allows for strongly-typed access to context values, providing
/// compile-time type safety when working with service context data.
pub trait ContextKey: Send + Sync {
    /// The type of the value stored in this context key.
    type Value: Send + Sync;

    /// Gets the value from the context.
    ///
    /// # Arguments
    /// * `context` - The service context to retrieve the value from
    ///
    /// # Returns
    /// The value if found, or `None` if not present
    fn get(&self, context: &ServiceContext) -> Option<&Self::Value>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    use super::ServiceContext;

    fn make_security_context() -> SecurityContext {
        let subject = SubjectId::new("user:test").unwrap();
        let principal = Principal::new(PrincipalKind::User, subject);
        SecurityContext::empty(principal)
    }

    #[test]
    fn require_security_returns_err_when_none() {
        let ctx = ServiceContext::new();
        let result = ctx.require_security();
        assert!(matches!(result, Err(SecurityError::CapabilityNotEnabled)));
    }

    #[test]
    fn require_security_returns_ok_when_some() {
        let sec_ctx = make_security_context();
        let ctx = ServiceContext::new().with_security(Arc::new(sec_ctx));
        let result = ctx.require_security();
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().principal().subject_id.as_str(),
            "user:test"
        );
    }
}
