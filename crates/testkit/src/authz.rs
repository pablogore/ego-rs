//! Scripted `AuthorizationProvider` for tests (CORE-022 Phase 5, design.md AD-3).
//!
//! `ScriptedAuthorizationProvider` implements the REAL async
//! `ego_security_sdk::authorization::AuthorizationProvider` trait, so it is
//! invoked via the same dispatch a production policy engine would use, and a
//! scripted `Deny` maps through the real `authorize_in_context` seam to
//! `SecurityError::AuthorizationDenied` exactly like production does.

use std::collections::HashMap;

use async_trait::async_trait;
use ego_security_sdk::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::Principal,
};

#[cfg(feature = "dev-providers")]
pub use ego_security_sdk::AllowAllAuthorizationProvider;
pub use ego_security_sdk::DenyAllAuthorizationProvider;

/// Deterministic per-`(resource kind, action)` authorizer implementing the
/// real [`AuthorizationProvider`] trait (design.md AD-3).
///
/// Denials flow through the production `authorize_in_context` seam to
/// [`SecurityError::AuthorizationDenied`] exactly like a real policy engine's
/// denial would — there is no shortcut or parallel denial path.
pub struct ScriptedAuthorizationProvider {
    default: AuthorizationDecision,
    rules: HashMap<(String, String), AuthorizationDecision>,
}

impl ScriptedAuthorizationProvider {
    /// Default decision `Allow` for any `(kind, action)` not overridden by [`Self::deny`].
    pub fn allow_all() -> Self {
        Self {
            default: AuthorizationDecision::Allow,
            rules: HashMap::new(),
        }
    }

    /// Default decision `Deny` for any `(kind, action)` not overridden by [`Self::allow`].
    pub fn deny_all() -> Self {
        Self {
            default: AuthorizationDecision::Deny {
                reason: "deny-all".to_string(),
            },
            rules: HashMap::new(),
        }
    }

    /// Allows exactly this `(resource_kind, action)` pair, regardless of the default.
    pub fn allow(mut self, resource_kind: impl Into<String>, action: impl Into<String>) -> Self {
        self.rules.insert(
            (resource_kind.into(), action.into()),
            AuthorizationDecision::Allow,
        );
        self
    }

    /// Denies exactly this `(resource_kind, action)` pair with `reason`, regardless of the default.
    pub fn deny(
        mut self,
        resource_kind: impl Into<String>,
        action: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        self.rules.insert(
            (resource_kind.into(), action.into()),
            AuthorizationDecision::Deny {
                reason: reason.into(),
            },
        );
        self
    }
}

#[async_trait]
impl AuthorizationProvider for ScriptedAuthorizationProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        let key = (
            request.resource.kind.to_string(),
            request.action.0.to_string(),
        );
        Ok(self
            .rules
            .get(&key)
            .cloned()
            .unwrap_or_else(|| self.default.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_security_sdk::{
        authorization::AuthorizationProvider, authorize_in_context, context::SecurityContext,
        Action, Resource, SecurityError,
    };

    use super::ScriptedAuthorizationProvider;
    use crate::{identity::principal, security::authenticated};

    fn resource(kind: &str) -> Resource {
        Resource {
            kind: kind.to_string().into(),
            id: None,
        }
    }

    /// Drives `provider` through the real `authorize_in_context` seam for one `(kind, action)` pair.
    async fn check(
        ctx: Option<&SecurityContext>,
        provider: &dyn AuthorizationProvider,
        kind: &str,
        action: &str,
    ) -> Result<(), SecurityError> {
        authorize_in_context(
            ctx,
            resource(kind),
            Action(action.to_string().into()),
            provider,
        )
        .await
    }

    #[tokio::test]
    async fn allow_all_allows_any_kind_and_action() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::allow_all();

        let a = check(Some(&ctx), &provider, "orders", "read").await;
        let b = check(Some(&ctx), &provider, "invoices", "delete").await;

        assert!(a.is_ok());
        assert!(b.is_ok());
    }

    #[tokio::test]
    async fn deny_all_with_allow_override_allows_only_that_pair() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::deny_all().allow("orders", "read");

        let allowed = check(Some(&ctx), &provider, "orders", "read").await;
        let still_denied = check(Some(&ctx), &provider, "orders", "write").await;
        let other_kind_denied = check(Some(&ctx), &provider, "invoices", "read").await;

        assert!(allowed.is_ok());
        assert!(matches!(
            still_denied,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "deny-all"
        ));
        assert!(matches!(
            other_kind_denied,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "deny-all"
        ));
    }

    #[tokio::test]
    async fn deny_surfaces_the_given_reason_through_authorize_in_context() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::allow_all().deny("orders", "read", "no role");

        let result = check(Some(&ctx), &provider, "orders", "read").await;

        assert!(matches!(
            result,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "no role"
        ));
    }

    #[test]
    fn scripted_provider_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ScriptedAuthorizationProvider>();
    }

    #[test]
    fn scripted_provider_is_object_safe_as_arc_dyn() {
        let _: Arc<dyn AuthorizationProvider> =
            Arc::new(ScriptedAuthorizationProvider::allow_all());
    }

    #[tokio::test]
    async fn deny_all_with_no_overrides_denies_every_action() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::deny_all();

        let a = check(Some(&ctx), &provider, "orders", "read").await;
        let b = check(Some(&ctx), &provider, "invoices", "delete").await;

        assert!(matches!(
            a,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "deny-all"
        ));
        assert!(matches!(
            b,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "deny-all"
        ));
    }

    #[tokio::test]
    async fn authorize_in_context_with_no_security_context_surfaces_capability_not_enabled() {
        let provider = ScriptedAuthorizationProvider::allow_all();

        let result = check(None, &provider, "orders", "read").await;

        assert!(matches!(result, Err(SecurityError::CapabilityNotEnabled)));
    }

    #[tokio::test]
    async fn deny_all_authorization_provider_denies_through_authorize_in_context() {
        use ego_security_sdk::DenyAllAuthorizationProvider;

        let ctx = authenticated(principal());
        let provider = DenyAllAuthorizationProvider;

        let result = check(Some(&ctx), &provider, "orders", "read").await;

        assert!(matches!(
            result,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "deny-all"
        ));
    }

    #[cfg(feature = "dev-providers")]
    #[tokio::test]
    async fn allow_all_authorization_provider_allows_through_authorize_in_context() {
        use ego_security_sdk::AllowAllAuthorizationProvider;

        let ctx = authenticated(principal());
        let provider = AllowAllAuthorizationProvider;

        let result = check(Some(&ctx), &provider, "orders", "read").await;

        assert!(result.is_ok());
    }
}
