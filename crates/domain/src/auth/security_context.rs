//! [`SecurityContext`] — the resolved, authenticated execution context.
//!
//! Combines an [`super::Identity`] (who the caller is) with the full
//! [`super::Claims`] extracted from their credential. Produced by a
//! successful call to [`super::AuthenticationProvider::authenticate`].

use super::{Claims, Identity};

/// The resolved security context for an authenticated request.
///
/// Carries both the canonical [`Identity`] (subject, tenant, roles,
/// attributes) and the raw [`Claims`] (standard + custom) so that callers
/// can inspect any claim without re-parsing the token.
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityContext {
    /// The authenticated principal.
    pub identity: Identity,

    /// All claims extracted from the credential.
    pub claims: Claims,
}

impl SecurityContext {
    /// Constructs a [`SecurityContext`] from its constituent parts.
    pub fn new(identity: Identity, claims: Claims) -> Self {
        Self { identity, claims }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::claims::StandardClaims;
    use std::collections::{BTreeMap, BTreeSet};

    fn make_ctx(subject: &str) -> SecurityContext {
        let identity = Identity {
            subject: subject.into(),
            tenant_id: None,
            roles: BTreeSet::new(),
            attributes: BTreeMap::new(),
        };
        let claims = Claims {
            standard: StandardClaims::default(),
            custom: BTreeMap::new(),
        };
        SecurityContext::new(identity, claims)
    }

    #[test]
    fn security_context_stores_identity_and_claims() {
        let ctx = make_ctx("alice");
        assert_eq!(ctx.identity.subject, "alice");
        assert!(ctx.claims.custom.is_empty());
    }

    #[test]
    fn security_context_is_clone_and_eq() {
        let ctx = make_ctx("bob");
        let ctx2 = ctx.clone();
        assert_eq!(ctx, ctx2);
    }

    #[test]
    fn security_context_debug_contains_identity() {
        let ctx = make_ctx("carol");
        let s = format!("{ctx:?}");
        assert!(s.contains("carol"));
    }
}
