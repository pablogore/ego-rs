//! Canonical tenant model and resolution seam (CORE-008A).
//!
//! `TenantResolver::resolve` is the sole Policy Evaluator (AD-013) for tenant
//! outcome, wired into `RuntimeInner::enforce_tenant`, which
//! `#[tenant_scoped]`-generated operations call fallibly (AD-009). Unmarked
//! operations never call `enforce_tenant` at all (AD-007) — the enforcement
//! mechanism itself is fully wired, adoption on individual operations is
//! per-operation opt-in.

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

/// An Established Fact (AD-013): proof that the authenticated principal has
/// already been authorized, upstream of policy evaluation, to operate
/// against `destination`. Constructed only from an already-granted
/// [`crate::context::ServiceContext`] state (via
/// [`crate::context::ServiceContext::cross_tenant_grant`]) — never fetched,
/// checked, or re-derived during `TenantResolver::resolve`'s own evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CrossTenantGrant {
    destination: TenantId,
}

impl CrossTenantGrant {
    /// Wraps an already-established destination tenant as a Established Fact.
    /// The destination itself was validated once, at permit-issuance time
    /// (`CrossTenantPermit`/`RuntimeInner::issue_cross_tenant_permit`) — this
    /// constructor performs no validation of its own.
    pub(crate) fn new(destination: TenantId) -> Self {
        Self { destination }
    }

    pub(crate) fn destination(&self) -> &TenantId {
        &self.destination
    }
}

/// The closed, immutable set of facts `TenantResolver::resolve` evaluates
/// (AD-013). Bundling these into one named value — rather than a growing
/// parameter list — makes the Fact Establishment / Policy Evaluation
/// boundary visible in the type system: this is exactly what AD-013 calls
/// "a closed, immutable set of Established Facts." Scoped to exactly what
/// exists today; do not add speculative fields for not-yet-designed policy
/// dimensions (delegation, impersonation, hierarchy, ...) ahead of need.
pub(crate) struct EstablishedTenantFacts<'a> {
    security: Option<&'a SecurityContext>,
    hint: Option<&'a str>,
    cross_tenant_grant: Option<CrossTenantGrant>,
}

impl<'a> EstablishedTenantFacts<'a> {
    pub(crate) fn new(
        security: Option<&'a SecurityContext>,
        hint: Option<&'a str>,
        cross_tenant_grant: Option<CrossTenantGrant>,
    ) -> Self {
        Self { security, hint, cross_tenant_grant }
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

    /// The single resolution algorithm mandated by D2 (AD-001), extended per
    /// AD-013 to also evaluate an Established cross-tenant Fact, if one is
    /// present in `facts`. Transport-neutral inputs only; consumes exactly
    /// the closed fact set `facts` already carries — never fetches, checks,
    /// or establishes anything itself (AD-013: Policy Evaluation, not Fact
    /// Establishment).
    ///
    /// Branch order matters: (a) is checked before (b)/(c) — a
    /// present-but-conflicting hint must never be evaluated against an
    /// absent Principal tenant claim (gap fix, see tasks.md TASK-003).
    pub(crate) fn resolve(
        &self,
        facts: EstablishedTenantFacts<'_>,
    ) -> Result<CanonicalTenant, SecurityError> {
        match facts.security {
            Some(security) => match security.principal().tenant_id.as_ref() {
                // (a) Authenticated but no Principal tenant claim — a caller-supplied
                // hint is never trusted as a substitute (D2 gap fix).
                None => Err(SecurityError::MissingContext),
                // (b) Authenticated, hint absent/blank or agreeing — Principal is
                // canonical. A blank hint (code-review fix) is treated the same as
                // an absent one: a transport binding that defaults a missing header
                // to `Some(String::new())` instead of `None` must not spuriously
                // mismatch against a real Principal tenant. The Principal's tenant
                // is already validated at JWT-mapping time (CORE-024) — clone, don't
                // re-validate.
                Some(principal_tenant) => {
                    let expected = principal_tenant.as_str();
                    if let Some(hint) = facts.hint {
                        // (c) Authenticated, hint present, non-blank, and disagrees.
                        if !hint.trim().is_empty() && hint != expected {
                            // (c') AD-013/FR-006: an Established cross-tenant grant
                            // scoped to exactly this hint's destination lets the
                            // disagreement resolve to the granted tenant instead of
                            // a hard error. The grant's TenantId was already
                            // validated at permit-issuance time — clone it, don't
                            // re-parse `hint`.
                            if let Some(grant) = &facts.cross_tenant_grant {
                                if grant.destination().as_str() == hint {
                                    return Ok(CanonicalTenant::scoped(
                                        grant.destination().clone(),
                                    ));
                                }
                            }
                            // Hard error, never a silent pick — no grant covers
                            // this destination.
                            return Err(SecurityError::TenantMismatch {
                                expected: expected.to_string(),
                                actual: hint.to_string(),
                            });
                        }
                    }
                    Ok(CanonicalTenant::scoped(principal_tenant.clone()))
                }
            },
            // (d)/(e) No SecurityContext: system/internal branch.
            None => match (self.mode, facts.hint) {
                // (d) System-internal hint is untrusted raw input — parse it
                // inline here (validated() is deleted, see design AD-2). This is
                // the ONLY remaining raw-string→TenantId parse in resolve().
                (TenantEnforcementMode::AllowSystemInternal, Some(hint)) => TenantId::new(hint)
                    .map(CanonicalTenant::scoped)
                    .map_err(|_| SecurityError::MissingContext),
                _ => Err(SecurityError::MissingContext),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use ego_domain::context::TenantId;
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};

    use super::{
        CanonicalTenant, CrossTenantGrant, EstablishedTenantFacts, TenantEnforcementMode,
        TenantResolver,
    };

    fn principal_with_tenant(tenant: Option<&str>) -> Principal {
        let mut p = Principal::new(PrincipalKind::User, SubjectId::new("alice").unwrap());
        p.tenant_id = tenant.map(|t| TenantId::new(t).unwrap());
        p
    }

    fn security_with_tenant(tenant: Option<&str>) -> SecurityContext {
        SecurityContext::empty(principal_with_tenant(tenant))
    }

    fn grant_for(destination: &str) -> CrossTenantGrant {
        CrossTenantGrant::new(TenantId::new(destination).unwrap())
    }

    fn facts<'a>(
        security: Option<&'a SecurityContext>,
        hint: Option<&'a str>,
        grant: Option<CrossTenantGrant>,
    ) -> EstablishedTenantFacts<'a> {
        EstablishedTenantFacts::new(security, hint, grant)
    }

    // Branch (a) — MUST be checked before (b)/(c): a present-but-conflicting
    // hint must never be evaluated against an absent Principal tenant claim.
    #[test]
    fn resolve_authenticated_no_principal_tenant_fails_closed_even_with_hint() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(None);

        let result = resolver.resolve(facts(Some(&security), Some("tenant-x"), None));

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

        let result = resolver.resolve(facts(Some(&security), None, None));

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

        let result = resolver.resolve(facts(Some(&security), None, None));

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

        let result = resolver.resolve(facts(Some(&security), Some("tenant-a"), None));

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

        let result = resolver.resolve(facts(Some(&security), Some(""), None));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-a\")), not TenantMismatch");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-a")
        );
    }

    // Branch (c) — authenticated, hint disagrees, no grant at all: hard TenantMismatch.
    #[test]
    fn resolve_authenticated_hint_disagrees_is_tenant_mismatch() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(facts(Some(&security), Some("tenant-b"), None));

        match result {
            Err(SecurityError::TenantMismatch { expected, actual }) => {
                assert_eq!(expected, "tenant-a");
                assert_eq!(actual, "tenant-b");
            }
            other => panic!("expected Err(TenantMismatch{{..}}), got: {:?}", other),
        }
    }

    // Branch (c) — authenticated, hint disagrees, grant scoped to a DIFFERENT
    // destination than the hint: still a hard TenantMismatch. Proves the grant
    // is destination-specific, not a blanket "cross-tenant enabled" switch.
    #[test]
    fn resolve_grant_for_different_destination_is_still_tenant_mismatch() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(facts(
            Some(&security),
            Some("tenant-b"),
            Some(grant_for("tenant-c")),
        ));

        match result {
            Err(SecurityError::TenantMismatch { expected, actual }) => {
                assert_eq!(expected, "tenant-a");
                assert_eq!(actual, "tenant-b");
            }
            other => panic!("expected Err(TenantMismatch{{..}}), got: {:?}", other),
        }
    }

    // Branch (c') — AD-013/FR-006: authenticated, hint disagrees, but an
    // Established grant scoped to exactly that hint's destination lets the
    // disagreement resolve to the granted tenant instead of erroring. This is
    // FR-006's positive acceptance scenario.
    #[test]
    fn resolve_authorized_cross_tenant_grant_succeeds() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(facts(
            Some(&security),
            Some("tenant-b"),
            Some(grant_for("tenant-b")),
        ));

        let canonical = result.expect("valid grant for the requested destination must succeed");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-b")
        );
    }

    // Fail-closed — a grant present but unused (hint absent) must not alter an
    // ordinary same-tenant call: resolves to the Principal's own tenant, as if
    // no grant existed.
    #[test]
    fn resolve_unused_grant_does_not_affect_hint_absent_resolution() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-a"));

        let result = resolver.resolve(facts(Some(&security), None, Some(grant_for("tenant-b"))));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-a\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-a")
        );
    }

    // Regression — a grant present that happens to match the Principal's OWN
    // tenant (redundant, since the hint already agrees) must not divert
    // resolution away from the ordinary branch (b) path.
    #[test]
    fn resolve_redundant_grant_matching_own_tenant_resolves_normally() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);
        let security = security_with_tenant(Some("tenant-b"));

        let result = resolver.resolve(facts(
            Some(&security),
            Some("tenant-b"),
            Some(grant_for("tenant-b")),
        ));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-b\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-b")
        );
    }

    // Branch (d) — unauthenticated, AllowSystemInternal, hint present: resolves to the hint.
    #[test]
    fn resolve_unauthenticated_allow_system_internal_with_hint_resolves_to_hint() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AllowSystemInternal);

        let result = resolver.resolve(facts(None, Some("tenant-c"), None));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-c\"))");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-c")
        );
    }

    // Regression — a grant is meaningless without an authenticated Principal
    // it was issued to; branch (d)'s raw-hint validation is unaffected by its
    // presence.
    #[test]
    fn resolve_grant_has_no_effect_without_security_context() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AllowSystemInternal);

        let result = resolver.resolve(facts(None, Some("tenant-c"), Some(grant_for("tenant-c"))));

        let canonical = result.expect("expected Ok(Scoped(\"tenant-c\")) via the raw-hint path");
        assert_eq!(
            canonical.tenant_id().map(TenantId::as_str),
            Some("tenant-c")
        );
    }

    // Branch (e) — unauthenticated, AuthenticatedOnly mode: fails closed regardless of hint.
    #[test]
    fn resolve_unauthenticated_authenticated_only_mode_fails_closed() {
        let resolver = TenantResolver::new(TenantEnforcementMode::AuthenticatedOnly);

        let result = resolver.resolve(facts(None, Some("tenant-c"), None));

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

        let result = resolver.resolve(facts(None, None, None));

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
