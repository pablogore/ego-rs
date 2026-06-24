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
    /// Creates a context for the given authenticated principal with empty claims.
    pub fn new(principal: Principal) -> Self {
        Self {
            principal,
            claims: Claims::empty(),
        }
    }

    /// Creates a context with principal and claims.
    pub fn with_claims(mut self, claims: Claims) -> Self {
        self.claims = claims;
        self
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

    fn make_principal(subject: &str) -> Principal {
        Principal::new(PrincipalKind::User, SubjectId::new(subject).unwrap())
    }

    #[test]
    fn constructs_from_principal() {
        let p = make_principal("user:42");
        let ctx = SecurityContext::new(p.clone());
        assert_eq!(ctx.principal().subject.as_str(), "user:42");
    }

    #[test]
    fn claims_defaults_to_empty() {
        let ctx = SecurityContext::new(make_principal("u:1"));
        assert!(ctx.claims().custom.is_empty());
    }

    #[test]
    fn principal_is_non_optional() {
        let ctx = SecurityContext::new(make_principal("u:1"));
        let subject: &str = ctx.principal().subject.as_str();
        assert_eq!(subject, "u:1");
    }

    #[test]
    fn no_ambient_state_leak() {
        let ctx_a = SecurityContext::new(make_principal("user:a"));
        let ctx_b = SecurityContext::new(make_principal("user:b"));
        assert_eq!(ctx_a.principal().subject.as_str(), "user:a");
        assert_eq!(ctx_b.principal().subject.as_str(), "user:b");
        assert_ne!(
            ctx_a.principal().subject.as_str(),
            ctx_b.principal().subject.as_str()
        );
    }

    #[test]
    fn is_clone_and_send_sync() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<SecurityContext>();
    }
}
