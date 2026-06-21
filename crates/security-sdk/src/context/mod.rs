//! Security context — explicit propagation of the authenticated principal.

use std::collections::HashMap;

use crate::principal::Principal;

/// Carries the authenticated [`Principal`] plus decision-relevant scope.
///
/// Propagated **explicitly** through `ServiceContext` — no thread-local,
/// no task-local, no global, no implicit ambient state.
///
/// **Invariant**: if a `SecurityContext` exists, a [`Principal`] is guaranteed.
/// `principal` is non-optional by design: `SecurityContext` cannot be constructed
/// without a `Principal`.
#[derive(Debug, Clone)]
pub struct SecurityContext {
    /// The authenticated principal — always present.
    pub principal: Principal,
    /// Decision-relevant scope key/values (e.g. tenant, environment).
    pub scope: HashMap<String, String>,
}

impl SecurityContext {
    /// Creates a context for the given authenticated principal.
    pub fn new(principal: Principal) -> Self {
        Self {
            principal,
            scope: HashMap::new(),
        }
    }

    /// Builder: adds a scope entry.
    pub fn with_scope(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.scope.insert(key.into(), value.into());
        self
    }

    /// Returns the authenticated principal.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
}

#[cfg(test)]
mod tests {
    use super::SecurityContext;
    use crate::principal::{Principal, PrincipalKind, SubjectId};

    fn make_principal(subject: &str) -> Principal {
        Principal::new(
            PrincipalKind::User,
            SubjectId::new(subject).unwrap(),
        )
    }

    #[test]
    fn constructs_from_principal() {
        let p = make_principal("user:42");
        let ctx = SecurityContext::new(p.clone());
        assert_eq!(ctx.principal().subject.as_str(), "user:42");
    }

    #[test]
    fn with_scope_adds_entry() {
        let ctx = SecurityContext::new(make_principal("u:1"))
            .with_scope("tenant", "t1");
        assert_eq!(ctx.scope.get("tenant").map(String::as_str), Some("t1"));
    }

    #[test]
    fn principal_is_non_optional() {
        // principal() returns &Principal directly — no Option unwrap needed.
        let ctx = SecurityContext::new(make_principal("u:1"));
        let subject: &str = ctx.principal().subject.as_str();
        assert_eq!(subject, "u:1");
    }

    #[test]
    fn no_ambient_state_leak() {
        // Two contexts from different principals must not share state.
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
        // Compile-time check: SecurityContext must be Send + Sync + Clone.
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<SecurityContext>();
    }
}
