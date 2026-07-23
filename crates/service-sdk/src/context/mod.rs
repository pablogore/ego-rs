use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;
use kitlogger::KITLogger;
use tokio_util::sync::CancellationToken;

use ego_domain::context::TenantId;
use ego_domain::TraceContext;

use crate::runtime::{CanonicalTenant, CrossTenantGrant, CrossTenantPermit};

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
/// The cross-tenant grant is preserved on clone — a cloned context retains the same
/// destination-scoped cross-tenant permission as the original. This is intentional: the
/// permit authorizes the context value, not a single use.
///
/// For hot paths that clone context frequently, prefer keeping `additional_context` empty
/// and relying on the typed fields. Avoid storing large payloads in `additional_context`.
///
/// ## Ownership model
///
/// Each component that requires a `ServiceContext` MUST declare that dependency in its
/// public signature. The context is passed forward — not looked up. Use `.clone()` when
/// passing to both an interceptor chain and an inner handler in the same call.
#[derive(Clone)]
pub struct ServiceContext {
    /// A caller-supplied tenant hint — a non-authoritative ingress value only
    /// (CORE-008A AD-011). It is a resolver *input*, never the enforced tenant;
    /// read it via [`ServiceContext::tenant_hint`], write it via
    /// [`ServiceContext::with_tenant_id`]. The authoritative value, once
    /// resolved, is [`ServiceContext::canonical_tenant`].
    ///
    /// Private (CORE-008A closure fix): a workspace-wide audit found no
    /// external caller constructing a `ServiceContext` via struct-literal
    /// `tenant_id` assignment (all callers use `with_tenant_id`), and only
    /// two external direct-field reads, both trivially served by
    /// `tenant_hint()`. Keeping this field accessible only through the
    /// accessor/builder pair closes the exact trap `tenant_hint`'s own doc
    /// comment warns about — reaching for the raw field and mistaking the
    /// hint for the enforced tenant.
    tenant_id: Option<String>,
    /// The correlation ID.
    pub correlation_id: Option<String>,
    /// The trace ID (legacy flat mirror).
    ///
    /// PROD-003 (ADR-4): `TraceContext` is authoritative over this legacy
    /// field **by construction**. The field is PRIVATE so it can only be
    /// mutated through the two builders that maintain the invariant:
    /// [`ServiceContext::with_trace_context`] sets it to
    /// `trace_context().trace_id()` (rendered as W3C hex), and
    /// [`ServiceContext::with_trace_id`] writes the legacy value ONLY when no
    /// `TraceContext` is present (otherwise the `TraceContext` wins and the
    /// legacy setter is ignored). Because no external code can assign the
    /// field directly, [`ServiceContext::trace_id`] is authoritative by
    /// construction — it can never desync from `trace_context().trace_id()`.
    /// Kept for source compatibility with call sites that predate
    /// `TraceContext`.
    trace_id: Option<String>,
    /// The deadline.
    pub deadline: Option<SystemTime>,
    /// The timeout.
    pub timeout: Option<Duration>,
    /// The additional context.
    pub additional_context: HashMap<String, String>,
    /// The destination tenant this context is authorized to cross into, if
    /// any (CORE-008A AD-008/TASK-019), held as the AD-014 Established Fact
    /// [`TenantResolver::resolve`] consumes. Set only via
    /// [`ServiceContext::with_cross_tenant_access`], scoped to the
    /// [`CrossTenantPermit`]'s own destination — a permit issued for
    /// `tenant-b` can never make this context allowed for `tenant-c`. Stored
    /// as `CrossTenantGrant` (not a raw `TenantId`) so [`ServiceContext::cross_tenant_grant`]
    /// can hand `TenantResolver::resolve` a borrowed reference instead of
    /// cloning on every `enforce_tenant` call, most of which never consume it.
    allow_cross_tenant: Option<CrossTenantGrant>,
    /// Optional push-style cancellation token.
    pub cancellation_token: Option<CancellationToken>,
    /// Attached security context carrying the authenticated principal, if any.
    pub security: Option<Arc<SecurityContext>>,
    /// Attached logger, propagated from `Runtime` via `Runtime::logger()`, if any.
    pub logger: Option<Arc<KITLogger>>,
    /// The authoritative, resolver-produced canonical tenant (CORE-008A AD-011).
    /// Set ONLY via [`ServiceContext::set_resolved_tenant`] (`pub(crate)`,
    /// called by `RuntimeInner::enforce_tenant`) — there is no public setter.
    resolved_tenant: Option<CanonicalTenant>,
    /// The explicit distributed-tracing value (PROD-003 ADR-1/ADR-4), carried
    /// by value — never via ambient/task-local state. Set via
    /// [`ServiceContext::with_trace_context`], read via
    /// [`ServiceContext::trace_context`].
    trace_context: Option<TraceContext>,
    /// The operation (service method) name for this invocation, carried as
    /// invocation metadata (PROD-003 follow-up, #212). Populated by the
    /// generated service proxy so the `TracingInterceptor` can name the
    /// request-boundary span after the dispatched operation instead of a fixed
    /// generic name. Private (mirroring the other metadata fields): set via
    /// [`ServiceContext::with_operation_name`], read via
    /// [`ServiceContext::operation_name`]. `None` for contexts built manually
    /// or in tests (no proxy), which fall back to the generic span name.
    operation_name: Option<Arc<str>>,
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
            allow_cross_tenant: None,
            cancellation_token: None,
            security: None,
            logger: None,
            resolved_tenant: None,
            trace_context: None,
            operation_name: None,
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

    /// Sets the legacy flat trace ID.
    ///
    /// PROD-003 (ADR-4): `TraceContext` is authoritative. This legacy setter
    /// writes `trace_id` ONLY when no `TraceContext` has been attached; once
    /// [`ServiceContext::with_trace_context`] is present the `TraceContext`
    /// wins and this call is ignored, so `trace_id()` can never desync from
    /// `trace_context().trace_id()`.
    ///
    /// # Arguments
    /// * `trace_id` - The trace identifier to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the legacy trace ID set when no
    /// `TraceContext` is present; otherwise unchanged.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        if self.trace_context.is_none() {
            self.trace_id = Some(trace_id.into());
        }
        self
    }

    /// Sets the explicit distributed-tracing value (PROD-003 ADR-1/ADR-4).
    ///
    /// This is the sole way trace identity travels between the point it is
    /// originated (HTTP ingress: `TraceContext::root()`/`from_inbound`) and
    /// any point it is later needed — no ambient/thread-local/task-local
    /// lookup exists. The flat [`ServiceContext::trace_id`] field/accessor is
    /// kept as a read-through mirror of `trace_context.trace_id()` (rendered
    /// as W3C hex) for source compatibility; it carries no independent value
    /// once this is set. `correlation_id` is a distinct business-causal
    /// concept and is unaffected.
    ///
    /// # Arguments
    /// * `trace_context` - The `TraceContext` to attach
    ///
    /// # Returns
    /// A new `ServiceContext` with the trace-context set (and `trace_id`
    /// mirrored)
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_id = Some(trace_context.trace_id().to_hex());
        self.trace_context = Some(trace_context);
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

    /// Marks the context as permitted for cross-tenant access into the
    /// permit's own destination (CORE-008A AD-008/TASK-019).
    ///
    /// Requires a [`CrossTenantPermit`] issued through the runtime's authorized
    /// cross-tenant permit issuance path (`RuntimeInner::issue_cross_tenant_permit`,
    /// crate-internal). Callers without a valid `&CrossTenantPermit` receive a compile error — no runtime
    /// fallback exists. The permit is borrowed (not consumed) so one issued permit can
    /// authorize multiple context grants, but the grant recorded here is scoped to
    /// exactly the destination the permit was authorized for — see
    /// [`ServiceContext::is_cross_tenant_allowed_for`].
    pub fn with_cross_tenant_access(mut self, permit: &CrossTenantPermit) -> Self {
        self.allow_cross_tenant = Some(CrossTenantGrant::new(permit.destination().clone()));
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

    /// Attaches a logger, propagated from `Runtime` via `Runtime::logger()`.
    ///
    /// # Arguments
    /// * `logger` - The `KITLogger` to associate with this service context
    ///
    /// # Returns
    /// A new `ServiceContext` with the logger set
    pub fn with_logger(mut self, logger: Arc<KITLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    /// Returns the attached logger, if any.
    pub fn logger(&self) -> Option<&KITLogger> {
        self.logger.as_deref()
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

    /// Checks if cross-tenant access is allowed for *some* destination.
    ///
    /// # Returns
    /// `true` if a cross-tenant grant is present, `false` otherwise
    #[deprecated(
        note = "checks 'is any permit attached', not 'is access allowed to the tenant \
                actually being accessed' — gating a real access decision on this method \
                instead of is_cross_tenant_allowed_for(destination) would let a permit \
                for one destination authorize access to a different one. Use \
                is_cross_tenant_allowed_for(destination) instead (CORE-008A AD-008)."
    )]
    pub fn is_cross_tenant_allowed(&self) -> bool {
        self.allow_cross_tenant.is_some()
    }

    /// Checks if cross-tenant access is allowed specifically for `destination`
    /// (CORE-008A AD-008, closes the permit-reuse hole: a permit authorizing
    /// `tenant-b` cannot be reused to reach `tenant-c`).
    ///
    /// # Returns
    /// `true` only if a cross-tenant grant is present AND it was scoped to
    /// this exact `destination`.
    pub fn is_cross_tenant_allowed_for(&self, destination: &TenantId) -> bool {
        self.allow_cross_tenant
            .as_ref()
            .is_some_and(|grant| grant.destination() == destination)
    }

    /// Retrieves the already-established cross-tenant grant, if any, as an
    /// AD-014 Established Fact ready for `TenantResolver::resolve` to
    /// consume. `RuntimeInner::enforce_tenant` calls this alongside
    /// [`ServiceContext::security`]/[`ServiceContext::tenant_hint`] to
    /// gather the closed fact set before evaluation — this accessor does
    /// not itself decide anything, it only reports what was already set via
    /// [`ServiceContext::with_cross_tenant_access`]. Borrowed, not cloned —
    /// `resolve` only ever reads the destination in the narrow branch where
    /// an authenticated hint disagrees with the principal's own tenant, so
    /// paying a clone on every call regardless of outcome is wasted work
    /// (code-review fix).
    pub(crate) fn cross_tenant_grant(&self) -> Option<&CrossTenantGrant> {
        self.allow_cross_tenant.as_ref()
    }

    /// Returns the non-authoritative, caller-supplied tenant hint (CORE-008A
    /// AD-011) — the honest name for the ingress value carried by `tenant_id`.
    /// This is a resolver *input*; it is never the enforced tenant. Use
    /// [`ServiceContext::canonical_tenant`] for the authoritative value.
    pub fn tenant_hint(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }

    /// `true` if a caller-supplied tenant hint is present. See
    /// [`ServiceContext::tenant_hint`].
    pub fn has_tenant_hint(&self) -> bool {
        self.tenant_id.is_some()
    }

    /// Returns the authoritative, resolver-produced canonical tenant
    /// (CORE-008A AD-011), or `None` if `RuntimeInner::enforce_tenant` has not
    /// run for this context yet. This is the ONLY value enforcement and
    /// cross-tenant checks read.
    pub fn canonical_tenant(&self) -> Option<&CanonicalTenant> {
        self.resolved_tenant.as_ref()
    }

    /// Sets the resolved canonical tenant. The sole writer is
    /// `RuntimeInner::enforce_tenant` — there is no public mutator, so a
    /// resolved tenant is immutable for the duration of an operation
    /// (CORE-008A AD-004/AD-011, FR-014).
    pub(crate) fn set_resolved_tenant(&mut self, t: CanonicalTenant) {
        self.resolved_tenant = Some(t);
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

    /// Gets the explicit distributed-tracing value, if set (PROD-003).
    ///
    /// # Returns
    /// The `TraceContext` if set via [`ServiceContext::with_trace_context`],
    /// or `None` if not set
    pub fn trace_context(&self) -> Option<&TraceContext> {
        self.trace_context.as_ref()
    }

    /// Sets the operation (service method) name for this invocation
    /// (PROD-003 follow-up, #212).
    ///
    /// This is invocation metadata populated by the generated service proxy —
    /// the `TracingInterceptor` reads it via [`ServiceContext::operation_name`]
    /// to name the request-boundary span after the dispatched operation. It
    /// carries no ambient state and does not affect tenant, security, or trace
    /// identity.
    ///
    /// # Arguments
    /// * `name` - The operation name to set
    ///
    /// # Returns
    /// A new `ServiceContext` with the operation name set
    pub fn with_operation_name(mut self, name: impl Into<Arc<str>>) -> Self {
        self.operation_name = Some(name.into());
        self
    }

    /// Gets the operation (service method) name for this invocation, if set
    /// (PROD-003 follow-up, #212).
    ///
    /// # Returns
    /// The operation name if populated by the generated proxy, or `None` for
    /// contexts built manually / in tests without a proxy
    pub fn operation_name(&self) -> Option<&str> {
        self.operation_name.as_deref()
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

// `KITLogger` does not implement `Debug`, so `ServiceContext` cannot derive it.
// This mirrors `RuntimeInner`'s hand-rolled `Debug` impl (`runtime/runtime_builder.rs`),
// which faces the same constraint.
impl std::fmt::Debug for ServiceContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceContext")
            .field("tenant_id", &self.tenant_id)
            .field("correlation_id", &self.correlation_id)
            .field("trace_id", &self.trace_id)
            .field("deadline", &self.deadline)
            .field("timeout", &self.timeout)
            .field("additional_context", &self.additional_context)
            .field("allow_cross_tenant", &self.allow_cross_tenant)
            .field("cancellation_token", &self.cancellation_token)
            .field("security", &self.security)
            .field("logger", &self.logger.is_some())
            .field("resolved_tenant", &self.resolved_tenant)
            .field("trace_context", &self.trace_context)
            .field("operation_name", &self.operation_name)
            .finish()
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

    // -- CORE-008A Phase 4 (TASK-018/019): destination-scoped cross-tenant --
    // AllowCrossTenant/authenticated_ctx moved to crate::test_support
    // (code-review fix: this copy had already drifted, missing the
    // DenyCrossTenant variant runtime_builder.rs's copy has).

    use crate::runtime::RuntimeInner;
    use crate::test_support::{authenticated_ctx, AllowCrossTenant};
    use ego_domain::context::TenantId;

    #[tokio::test]
    #[allow(deprecated)]
    async fn with_cross_tenant_access_sets_flag() {
        let rt = RuntimeInner::for_test_with_authz(Arc::new(AllowCrossTenant));
        let destination = TenantId::new("tenant-b").unwrap();
        let permit = rt
            .issue_cross_tenant_permit(&authenticated_ctx(), destination)
            .await
            .expect("Allow decision must yield a permit");
        let ctx = ServiceContext::new().with_cross_tenant_access(&permit);
        assert!(ctx.is_cross_tenant_allowed());
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn clone_preserves_cross_tenant_flag() {
        let rt = RuntimeInner::for_test_with_authz(Arc::new(AllowCrossTenant));
        let destination = TenantId::new("tenant-b").unwrap();
        let permit = rt
            .issue_cross_tenant_permit(&authenticated_ctx(), destination)
            .await
            .expect("Allow decision must yield a permit");
        let ctx = ServiceContext::new().with_cross_tenant_access(&permit);
        let cloned = ctx.clone();
        assert!(cloned.is_cross_tenant_allowed());
    }

    // CORE-008A Phase 5 (TASK-023, Mandatory Seed 3): this test already
    // proves a `CrossTenantPermit` issued for `tenant-b` cannot be reused
    // to reach `tenant-c` — the exact scenario TASK-023 specifies. No new
    // test was added for TASK-023; this one (added in Phase 4 for TASK-019)
    // already satisfies it verbatim.
    #[tokio::test]
    async fn is_cross_tenant_allowed_for_matches_only_the_issued_destination() {
        let rt = RuntimeInner::for_test_with_authz(Arc::new(AllowCrossTenant));
        let tenant_b = TenantId::new("tenant-b").unwrap();
        let permit = rt
            .issue_cross_tenant_permit(&authenticated_ctx(), tenant_b.clone())
            .await
            .expect("Allow decision must yield a permit");
        let ctx = ServiceContext::new().with_cross_tenant_access(&permit);

        let tenant_c = TenantId::new("tenant-c").unwrap();
        assert!(ctx.is_cross_tenant_allowed_for(&tenant_b));
        assert!(!ctx.is_cross_tenant_allowed_for(&tenant_c));
    }

    #[test]
    fn is_cross_tenant_allowed_for_is_false_with_no_grant() {
        let ctx = ServiceContext::new();
        let tenant_b = TenantId::new("tenant-b").unwrap();
        assert!(!ctx.is_cross_tenant_allowed_for(&tenant_b));
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
        assert_eq!(result.unwrap().principal().subject_id.as_str(), "user:test");
    }

    // -- CORE-017: logger access (TASK-018/TASK-019) ------------------------

    #[test]
    fn logger_is_none_by_default() {
        let ctx = ServiceContext::new();
        assert!(ctx.logger().is_none());
    }

    #[test]
    fn with_logger_sets_logger() {
        use kitlogger::KITLogger;
        let ctx = ServiceContext::new().with_logger(Arc::new(KITLogger::default()));
        assert!(ctx.logger().is_some());
    }

    // -- CORE-008A Phase 2 (TASK-006): canonical tenant on ServiceContext --

    #[test]
    fn canonical_tenant_is_none_by_default() {
        let ctx = ServiceContext::new();
        assert!(ctx.canonical_tenant().is_none());
    }

    #[test]
    fn tenant_hint_is_none_by_default() {
        let ctx = ServiceContext::new();
        assert_eq!(ctx.tenant_hint(), None);
        assert!(!ctx.has_tenant_hint());
    }

    #[test]
    fn has_tenant_hint_reflects_hint_presence() {
        assert!(!ServiceContext::new().has_tenant_hint());
        assert!(ServiceContext::new()
            .with_tenant_id("tenant-x")
            .has_tenant_hint());
    }

    #[test]
    fn set_resolved_tenant_makes_canonical_tenant_available() {
        use crate::runtime::{RuntimeInner, TenantEnforcementMode};

        // Routed through the same public seam production code uses
        // (`RuntimeInner::enforce_tenant`) rather than constructing a
        // crate-internal `EstablishedTenantFacts` by hand — this test only
        // needs to prove `set_resolved_tenant`/`canonical_tenant` work, not
        // exercise `TenantResolver::resolve`'s own branches (code-review fix).
        let rt = RuntimeInner::for_test_with_mode(TenantEnforcementMode::AllowSystemInternal);
        let mut ctx = ServiceContext::new().with_tenant_id("tenant-a");

        rt.enforce_tenant(&mut ctx)
            .expect("AllowSystemInternal + hint resolves");

        assert!(ctx.canonical_tenant().is_some());
    }

    // -- PROD-003 Phase 2 (TASK-008): explicit trace-context threading --

    #[test]
    fn with_trace_context_round_trips() {
        use ego_domain::TraceContext;

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);

        assert_eq!(ctx.trace_context(), Some(&tc));
    }

    #[test]
    fn flat_trace_id_mirrors_trace_context_trace_id() {
        use ego_domain::TraceContext;

        let tc = TraceContext::root();
        let ctx = ServiceContext::new().with_trace_context(tc);

        assert_eq!(ctx.trace_id(), Some(tc.trace_id().to_hex().as_str()));
    }

    // PR2 review (P1): TraceContext is authoritative over the legacy flat
    // `trace_id` BY CONSTRUCTION — the private field is maintained as a mirror
    // by both builders, so the two can never desync regardless of call order.

    #[test]
    fn trace_context_wins_when_set_before_with_trace_id() {
        use ego_domain::TraceContext;

        let tc = TraceContext::root();
        let ctx = ServiceContext::new()
            .with_trace_context(tc)
            .with_trace_id("x");

        // with_trace_id is ignored once a TraceContext is present.
        assert_eq!(ctx.trace_id(), Some(tc.trace_id().to_hex().as_str()));
    }

    #[test]
    fn trace_context_wins_when_set_after_with_trace_id() {
        use ego_domain::TraceContext;

        let tc = TraceContext::root();
        let ctx = ServiceContext::new()
            .with_trace_id("x")
            .with_trace_context(tc);

        // with_trace_context overwrites the legacy mirror it does not own.
        assert_eq!(ctx.trace_id(), Some(tc.trace_id().to_hex().as_str()));
    }

    #[test]
    fn legacy_trace_id_is_returned_when_no_trace_context() {
        let ctx = ServiceContext::new().with_trace_id("x");

        assert_eq!(ctx.trace_id(), Some("x"));
    }

    #[test]
    fn correlation_id_is_unaffected_by_trace_context() {
        use ego_domain::TraceContext;

        let ctx = ServiceContext::new()
            .with_correlation_id("corr-1")
            .with_trace_context(TraceContext::root());

        assert_eq!(ctx.correlation_id(), Some("corr-1"));
    }

    #[test]
    fn trace_context_is_none_by_default() {
        let ctx = ServiceContext::new();
        assert_eq!(ctx.trace_context(), None);
    }

    // -- PROD-003 follow-up (#212): operation-level span naming carried as
    // invocation metadata on the context (populated by the generated proxy).

    #[test]
    fn operation_name_is_none_by_default() {
        let ctx = ServiceContext::new();
        assert_eq!(ctx.operation_name(), None);
    }

    #[test]
    fn with_operation_name_round_trips() {
        let ctx = ServiceContext::new().with_operation_name("register");
        assert_eq!(ctx.operation_name(), Some("register"));
    }

    #[test]
    fn operation_name_does_not_affect_correlation_or_trace_context() {
        use ego_domain::TraceContext;

        let tc = TraceContext::root();
        let ctx = ServiceContext::new()
            .with_correlation_id("corr-1")
            .with_trace_context(tc)
            .with_operation_name("register");

        assert_eq!(ctx.operation_name(), Some("register"));
        assert_eq!(ctx.correlation_id(), Some("corr-1"));
        assert_eq!(ctx.trace_context(), Some(&tc));
    }

    // FR-010: with_tenant_id() only ever writes tenant_id, never
    // resolved_tenant — replacing the ingress hint on an owned,
    // already-resolved context cannot override the canonical tenant.
    #[test]
    fn replacing_the_ingress_hint_after_resolution_does_not_change_canonical_tenant() {
        use crate::runtime::{RuntimeInner, TenantEnforcementMode};

        let rt = RuntimeInner::for_test_with_mode(TenantEnforcementMode::AllowSystemInternal);
        let mut ctx = ServiceContext::new().with_tenant_id("tenant-a");
        rt.enforce_tenant(&mut ctx).expect("resolves to tenant-a");
        let resolved_before = ctx.canonical_tenant().cloned();

        let ctx = ctx.with_tenant_id("tenant-b");

        assert_eq!(ctx.tenant_hint(), Some("tenant-b"));
        assert_eq!(ctx.canonical_tenant().cloned(), resolved_before);
    }
}
