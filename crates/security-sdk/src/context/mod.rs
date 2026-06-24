//! Security context — authenticated principal + raw claims.
//!
//! Propagated **explicitly** through `ServiceContext` — no ambient state.

use ego_domain::auth::Claims;

use crate::principal::Principal;

/// Carries the authenticated [`Principal`] plus raw [`Claims`] from the credential.
///
/// Propagated **explicitly** through `ServiceContext` — no thread-local,
/// no task-local, no global, no implicit ambient state (AD-005).
///
/// **Invariant**: if a `SecurityContext` exists, a [`Principal`] is guaranteed.
/// `principal` is non-optional by design: `SecurityContext` cannot be constructed
/// without a `Principal`.
///
/// Claims are request-scoped (AD-002) and MUST NOT be persisted in aggregates,
/// events, snapshots, projections, or repositories.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// The authenticated principal — always present.
    pub principal: Principal,
    /// Raw authentication claims from the credential (request-scoped only).
    pub claims: Claims,
}

impl SecurityContext {
    /// Creates a context with the given authenticated principal and claims.
    pub fn new(principal: Principal, claims: Claims) -> Self {
        Self { principal, claims }
    }

    /// Creates a context with the given principal and empty claims.
    pub fn empty(principal: Principal) -> Self {
        Self {
            principal,
            claims: Claims::empty(),
        }
    }

    /// Returns the authenticated principal.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Returns the raw authentication claims.
    pub fn claims(&self) -> &Claims {
        &self.claims
    }
}

#[cfg(test)]
mod tests {
    use super::SecurityContext;
    use crate::principal::{Principal, PrincipalKind, SubjectId};
    use ego_domain::auth::Claims;

    fn make_principal(subject: &str) -> Principal {
        Principal::new(PrincipalKind::User, SubjectId::new(subject).unwrap())
    }

    #[test]
    fn constructs_from_principal_and_claims() {
        let p = make_principal("user:42");
        let claims = Claims::empty();
        let ctx = SecurityContext::new(p.clone(), claims.clone());
        assert_eq!(ctx.principal().subject_id.as_str(), "user:42");
        assert!(ctx.claims().custom.is_empty());
    }

    #[test]
    fn empty_creates_context_without_claims() {
        let ctx = SecurityContext::empty(make_principal("u:1"));
        assert!(ctx.claims().custom.is_empty());
    }

    #[test]
    fn no_ambient_state_leak() {
        let ctx_a = SecurityContext::empty(make_principal("user:a"));
        let ctx_b = SecurityContext::empty(make_principal("user:b"));
        assert_eq!(ctx_a.principal().subject_id.as_str(), "user:a");
        assert_eq!(ctx_b.principal().subject_id.as_str(), "user:b");
        assert_ne!(
            ctx_a.principal().subject_id.as_str(),
            ctx_b.principal().subject_id.as_str()
        );
    }

    #[test]
    fn is_clone_and_send_sync() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<SecurityContext>();
    }
}
