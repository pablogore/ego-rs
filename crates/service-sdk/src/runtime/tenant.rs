//! Canonical tenant model and resolution seam (CORE-008A).
//!
//! `TenantResolver::resolve` is wired into `RuntimeInner::enforce_tenant`
//! (Phase 2). Still inert end-to-end: no `#[operation]` is marked
//! `#[tenant_scoped]` yet (Phase 3), so the macro's generated unmarked-path
//! call discards `enforce_tenant`'s `Result` and no operation observably
//! changes behavior.

use ego_domain::context::TenantId;
use ego_security_sdk::context::SecurityContext;
use ego_security_sdk::error::SecurityError;

/// The single canonical in-runtime tenant representation (AD-002).
///
/// # Construction is `crate::runtime`-only (AD-003)
///
/// **Deviation from design.md's literal sketch, flagged per apply
/// instructions.** design.md's Interfaces/Contracts section sketches this
/// as a plain `pub enum CanonicalTenant { Scoped(TenantId), Systemwide }`.
/// That literal shape cannot satisfy AD-003: Rust enum variants always
/// share the visibility of their enum (`error[E0449]`), and there is no
/// per-variant visibility narrower than the enum itself for a variant that
/// carries public data — so a plain public `Scoped(TenantId)` tuple variant
/// would be freely constructible by any external crate holding a
/// `TenantId` (which is itself public), defeating "only `TenantResolver`
/// may create a `CanonicalTenant`" the moment it compiled. `#[non_exhaustive]`
/// does not fix this either: applied per-variant it blocks *matching* the
/// variant from other crates too, which would break the very read path
/// AD-011 introduces in Phase 2 (`ServiceContext::canonical_tenant()` is
/// meant to be readable by application code in other crates).
///
/// The smallest fix consistent with AD-002/AD-003/AD-004's intent: wrap a
/// private representation and expose read-only accessors instead of raw
/// variants. This mirrors `CrossTenantPermit`'s `pub(super)`-constructor
/// discipline in `permit.rs` (private field / private constructor, and
/// `pub(super)` visible to `crate::runtime` and its sibling submodules)
/// while keeping the type itself public and freely readable — no
/// pattern-matching restriction leaks to future external consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTenant(Repr);

// `Systemwide` is not constructed by any production caller as of Phase 2 —
// the macro's unmarked-op path never calls it (Implementation Note 2); it
// remains available for explicit/manual resolver use (AD-002).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Repr {
    /// A concrete resolved tenant (authenticated or permitted-internal path).
    Scoped(TenantId),
    /// Valid tenant-less system / single-tenant execution (D1).
    Systemwide,
}

impl CanonicalTenant {
    /// Mints a scoped canonical tenant. Visible only within `crate::runtime`
    /// and its sibling submodules — the only caller is [`TenantResolver::resolve`].
    pub(super) fn scoped(tenant: TenantId) -> Self {
        Self(Repr::Scoped(tenant))
    }

    /// Mints the tenant-less systemwide value (D1). Visible only within
    /// `crate::runtime` and its sibling submodules.
    // Unused until an explicit/manual resolver caller needs it (AD-002) —
    // the macro's unmarked-op path never constructs it (Implementation Note 2).
    #[allow(dead_code)]
    pub(super) fn systemwide() -> Self {
        Self(Repr::Systemwide)
    }

    /// The resolved tenant id, or `None` for the tenant-less systemwide mode (D1).
    pub fn tenant_id(&self) -> Option<&TenantId> {
        match &self.0 {
            Repr::Scoped(t) => Some(t),
            Repr::Systemwide => None,
        }
    }

    /// `true` for the tenant-less systemwide execution mode (D1).
    pub fn is_systemwide(&self) -> bool {
        matches!(self.0, Repr::Systemwide)
    }
}

/// Runtime-configured tenant enforcement policy (AD-012).
///
/// Deliberately distinct from persistence-side `single_tenant_mode` /
/// `tenant_id` on `persistent_entity::EntityRuntimeBuilder` (CORE-016) — this
/// type governs enforcement/resolution, not persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantEnforcementMode {
    /// Default. Only authenticated principals resolve a tenant (FR-002).
    /// Unauthenticated tenant-scoped calls fail closed with `MissingContext` (FR-004).
    AuthenticatedOnly,
    /// Additionally permit an explicit system/internal caller-supplied tenant (FR-003).
    AllowSystemInternal,
}

/// The single resolution seam mandated by D2 (AD-001).
///
/// Transport-neutral inputs only: an already-produced [`SecurityContext`]
/// and an optional caller-supplied tenant hint. Not `dyn`-dispatched — the
/// resolution policy is a fixed invariant, not a per-deployment plugin.
pub struct TenantResolver {
    mode: TenantEnforcementMode,
}

impl TenantResolver {
    /// Builds a resolver enforcing the given [`TenantEnforcementMode`].
    pub(crate) fn new(mode: TenantEnforcementMode) -> Self {
        Self { mode }
    }

    /// The single resolution algorithm mandated by D2. Transport-neutral inputs only.
    ///
    /// Branch order matters: (a) is checked before (b)/(c) — a
    /// present-but-conflicting hint must never be evaluated against an
    /// absent Principal tenant claim (gap fix, see tasks.md TASK-003).
    pub(crate) fn resolve(
        &self,
        security: Option<&SecurityContext>,
        supplied_tenant: Option<&str>,
    ) -> Result<CanonicalTenant, SecurityError> {
        match security {
            Some(security) => match security.principal().tenant_id.as_deref() {
                // (a) Authenticated but no Principal tenant claim — a caller-supplied
                // hint is never trusted as a substitute (D2 gap fix).
                None => Err(SecurityError::MissingContext),
                // (b) Authenticated, hint absent/blank or agreeing — Principal is
                // canonical. A blank hint (code-review fix) is treated the same as
                // an absent one: a transport binding that defaults a missing header
                // to `Some(String::new())` instead of `None` must not spuriously
                // mismatch against a real Principal tenant.
                Some(principal_tenant) => match supplied_tenant {
                    None => Ok(CanonicalTenant::scoped(Self::validated(principal_tenant)?)),
                    Some(hint) if hint.trim().is_empty() || hint == principal_tenant => {
                        Ok(CanonicalTenant::scoped(Self::validated(principal_tenant)?))
                    }
                    // (c) Authenticated, hint disagrees — hard error, never a silent pick.
                    Some(hint) => Err(SecurityError::TenantMismatch {
                        expected: principal_tenant.to_string(),
                        actual: hint.to_string(),
                    }),
                },
            },
            // (d)/(e) No SecurityContext: system/internal branch.
            None => match (self.mode, supplied_tenant) {
                (TenantEnforcementMode::AllowSystemInternal, Some(hint)) => {
                    Ok(CanonicalTenant::scoped(Self::validated(hint)?))
                }
                _ => Err(SecurityError::MissingContext),
            },
        }
    }

    /// Validates a raw tenant string into the domain `TenantId` newtype.
    /// An empty tenant string (violating `TenantId`'s non-empty invariant)
    /// is treated as an unresolvable context, not a panic or silent default.
    fn validated(raw: &str) -> Result<TenantId, SecurityError> {
        TenantId::new(raw).map_err(|_| SecurityError::MissingContext)
    }
}

#[cfg(test)]
mod tests {
    use ego_domain::context::TenantId;
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    use super::{CanonicalTenant, TenantEnforcementMode, TenantResolver};

    fn principal_with_tenant(tenant: Option<&str>) -> Principal {
        let mut p = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
        p.tenant_id = tenant.map(|t| t.to_string());
        p
    }

    fn security_with_tenant(tenant: Option<&str>) -> SecurityContext {
        SecurityContext::empty(principal_with_tenant(tenant))
    }

    // Branch (a) — MUST be checked before (b)/(c): a present-but-conflicting
    // hint must never be evaluated against an absent Principal tenant claim.
    #[test]
    fn resolve_authenticated_no_principal_tenant_fails_closed_even_with_hint() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(None);

        let result = resolver.resolve(Some(&security), Some("tenant-x"));

        assert!(
            matches!(result, Err(SecurityError::MissingContext)),
            "expected Err(MissingContext), got: {:?}",
            result
        );
    }

    #[test]
    fn resolve_authenticated_no_principal_tenant_fails_closed_without_hint() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(None);

        let result = resolver.resolve(Some(&security), None);

        assert!(
            matches!(result, Err(SecurityError::MissingContext)),
            "expected Err(MissingContext), got: {:?}",
            result
        );
    }

    // Branch (b) — authenticated, hint absent: resolves to the Principal's tenant.
    #[test]
    fn resolve_authenticated_hint_absent_resolves_to_principal_tenant() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(Some(&security), None);

        let canonical = result.expect("expected Ok(Scoped(\"tenant-a\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-a")
        );
        assert!(!canonical.is_systemwide());
    }

    // Branch (b) — authenticated, hint agrees: resolves to the Principal's tenant.
    #[test]
    fn resolve_authenticated_hint_agrees_resolves_to_principal_tenant() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(Some(&security), Some("tenant-a"));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-a\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-a")
        );
    }

    // Branch (b), code-review fix — authenticated, hint is a blank string (as a
    // transport binding might produce for a missing header): treated as absent,
    // never as a mismatch against the Principal's real tenant.
    #[test]
    fn resolve_authenticated_blank_hint_resolves_to_principal_tenant() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(Some(&security), Some(""));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-a\")), not TenantMismatch");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-a")
        );
    }

    // Branch (c) — authenticated, hint disagrees: hard TenantMismatch.
    #[test]
    fn resolve_authenticated_hint_disagrees_is_tenant_mismatch() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(Some(&security), Some("tenant-b"));

        match result {
            Err(SecurityError::TenantMismatch { expected, actual }) => {
                assert_eq!(expected, "tenant-a");
                assert_eq!(actual, "tenant-b");
            }
            other => panic!("expected Err(TenantMismatch{{..}}), got: {:?}", other),
        }
    }

    // Branch (d) — unauthenticated, AllowSystemInternal, hint present: resolves to the hint.
    #[test]
    fn resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AllowSystemInternal);

        let result = resolver.resolve(None, Some("tenant-c"));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-c\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-c")
        );
    }

    // Branch (e) — unauthenticated, AuthenticatedOnly mode: fails closed regardless of hint.
    #[test]
    fn resolve_unauthenticated_authenticated_only_mode_fails_closed() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);

        let result = resolver.resolve(None, Some("tenant-c"));

        assert!(
            matches!(result, Err(SecurityError::MissingContext)),
            "expected Err(MissingContext), got: {:?}",
            result
        );
    }

    // Branch (e) — unauthenticated, AllowSystemInternal but no hint: fails closed.
    #[test]
    fn resolve_unauthenticated_allow_system_internal_without_hint_fails_closed() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AllowSystemInternal);

        let result = resolver.resolve(None, None);

        assert!(
            matches!(result, Err(SecurityError::MissingContext)),
            "expected Err(MissingContext), got: {:?}",
            result
        );
    }

    #[test]
    fn canonical_tenant_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<CanonicalTenant>();
    }

    #[test]
    fn canonical_tenant_systemwide_is_constructible_within_runtime() {
        // `pub(super)` — reachable from within `crate::runtime`; this test lives there.
        let systemwide = CanonicalTenant::systemwide();
        assert!(systemwide.is_systemwide());
        assert_eq!(systemwide.tenant_id(), None);
    }

    #[test]
    fn canonical_tenant_scoped_is_constructible_within_runtime() {
        let tenant_id = TenantId::new("tenant-a").unwrap();
        let scoped = CanonicalTenant::scoped(tenant_id.clone());
        assert_eq!(scoped.tenant_id(), Some(&tenant_id));
        assert!(!scoped.is_systemwide());
    }
}
