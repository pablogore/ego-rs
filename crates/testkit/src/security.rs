//! `SecurityContext` helpers (CORE-022 Phase 3, design.md AD-4).

use ego_domain::auth::Claims;
use ego_security_sdk::{context::SecurityContext, principal::Principal};

/// A real `SecurityContext` for `principal`, with empty claims. Indistinguishable
/// to consuming code from what a real `AuthenticationProvider` would produce.
pub fn authenticated(principal: Principal) -> SecurityContext {
    SecurityContext::empty(principal)
}

/// A real `SecurityContext` for `principal` carrying the given `claims`.
pub fn authenticated_with_claims(principal: Principal, claims: Claims) -> SecurityContext {
    SecurityContext::new(principal, claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PrincipalBuilder;
    use ego_domain::auth::Claims;
    use serde_json::json;

    #[test]
    fn authenticated_carries_the_given_principal_with_empty_claims() {
        let p = PrincipalBuilder::new().subject("user:1").build();
        let ctx = authenticated(p);
        assert_eq!(ctx.principal().subject_id.as_str(), "user:1");
        assert!(ctx.claims().custom.is_empty());
    }

    #[test]
    fn authenticated_with_claims_carries_given_claims() {
        let p = PrincipalBuilder::new().subject("user:2").build();
        let mut claims = Claims::empty();
        claims.custom.insert("scopes".into(), json!(["read"]));
        let ctx = authenticated_with_claims(p, claims);
        assert_eq!(ctx.principal().subject_id.as_str(), "user:2");
        assert_eq!(ctx.claims().custom.get("scopes"), Some(&json!(["read"])));
    }
}
