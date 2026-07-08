//! Assertion helpers (CORE-022 Phase 9, design.md AD-8).
//!
//! `assert_authorized`/`assert_denied` call the REAL
//! [`authorize_in_context`] seam — the same one production authorization
//! flows through — rather than reimplementing the decision logic.
//! `assert_service_error!` is `matches!`-based against the real
//! [`ServiceError`] enum, so it ignores message text and only checks the
//! variant.

use ego_security_sdk::{
    authorization::{authorize_in_context, Action, AuthorizationProvider, Resource},
    context::SecurityContext,
    SecurityError,
};

/// Passes iff `authorize_in_context(Some(ctx), resource, action, provider)`
/// returns `Ok(())`. Panics with the actual [`SecurityError`] otherwise.
pub async fn assert_authorized(
    provider: &dyn AuthorizationProvider,
    ctx: &SecurityContext,
    resource: Resource,
    action: Action,
) {
    if let Err(err) = authorize_in_context(Some(ctx), resource, action, provider).await {
        panic!("expected authorization to succeed, but it failed with: {err:?}");
    }
}

/// Passes iff the outcome is specifically
/// `Err(SecurityError::AuthorizationDenied { .. })`. Panics otherwise,
/// including when the call succeeded or failed with a different variant.
pub async fn assert_denied(
    provider: &dyn AuthorizationProvider,
    ctx: &SecurityContext,
    resource: Resource,
    action: Action,
) {
    match authorize_in_context(Some(ctx), resource, action, provider).await {
        Err(SecurityError::AuthorizationDenied { .. }) => {}
        Ok(()) => panic!("expected authorization to be denied, but it succeeded"),
        Err(other) => panic!(
            "expected SecurityError::AuthorizationDenied, but got a different error: {other:?}"
        ),
    }
}

/// Asserts a `Result<_, ServiceError>` matches a specific variant, ignoring
/// message text. Usage: `assert_service_error!(result, ServiceError::NotFound { .. });`
#[macro_export]
macro_rules! assert_service_error {
    ($result:expr, $variant:pat) => {
        match &$result {
            Err($variant) => {}
            other => panic!(
                "expected Err matching {}, but got: {:?}",
                stringify!($variant),
                other
            ),
        }
    };
}

#[cfg(test)]
mod tests {
    use ego_security_sdk::authorization::{Action, Resource};
    use ego_service_sdk::ServiceError;

    use crate::authz::ScriptedAuthorizationProvider;
    use crate::identity::principal;
    use crate::security::authenticated;

    use super::{assert_authorized, assert_denied};

    fn resource(kind: &str) -> Resource {
        Resource {
            kind: kind.to_string().into(),
            id: None,
        }
    }

    #[tokio::test]
    async fn assert_authorized_passes_when_authorize_in_context_returns_ok() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::allow_all();

        assert_authorized(&provider, &ctx, resource("orders"), Action("read".into())).await;
    }

    #[tokio::test]
    #[should_panic(expected = "expected authorization to succeed")]
    async fn assert_authorized_panics_with_clear_message_when_denied() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::deny_all();

        assert_authorized(&provider, &ctx, resource("orders"), Action("read".into())).await;
    }

    #[tokio::test]
    async fn assert_denied_passes_only_on_authorization_denied() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::deny_all();

        assert_denied(&provider, &ctx, resource("orders"), Action("read".into())).await;
    }

    #[tokio::test]
    #[should_panic(expected = "expected authorization to be denied, but it succeeded")]
    async fn assert_denied_panics_on_ok() {
        let ctx = authenticated(principal());
        let provider = ScriptedAuthorizationProvider::allow_all();

        assert_denied(&provider, &ctx, resource("orders"), Action("read".into())).await;
    }

    /// Provider whose `authorize` itself errors (not a `Deny` decision), so
    /// `authorize_in_context` surfaces a variant other than
    /// `AuthorizationDenied` — proves `assert_denied` isn't "any Err passes".
    struct CapabilityNotEnabledProvider;

    #[async_trait::async_trait]
    impl ego_security_sdk::authorization::AuthorizationProvider for CapabilityNotEnabledProvider {
        async fn authorize(
            &self,
            _principal: &ego_security_sdk::Principal,
            _request: &ego_security_sdk::authorization::AccessRequest,
            _ctx: &ego_security_sdk::SecurityContext,
        ) -> Result<
            ego_security_sdk::authorization::AuthorizationDecision,
            ego_security_sdk::SecurityError,
        > {
            Err(ego_security_sdk::SecurityError::CapabilityNotEnabled)
        }
    }

    #[tokio::test]
    #[should_panic(expected = "expected SecurityError::AuthorizationDenied")]
    async fn assert_denied_panics_on_a_different_error_variant() {
        let ctx = authenticated(principal());
        let provider = CapabilityNotEnabledProvider;

        assert_denied(&provider, &ctx, resource("orders"), Action("read".into())).await;
    }

    #[test]
    fn assert_service_error_passes_on_matching_variant_ignoring_message_text() {
        let result: Result<(), ServiceError> = Err(ServiceError::validation("first message"));
        assert_service_error!(result, ServiceError::Validation { .. });

        let result: Result<(), ServiceError> =
            Err(ServiceError::validation("a completely different message"));
        assert_service_error!(result, ServiceError::Validation { .. });
    }

    #[test]
    #[should_panic(expected = "expected Err matching")]
    fn assert_service_error_panics_on_non_matching_variant() {
        let result: Result<(), ServiceError> = Err(ServiceError::validation("oops"));
        assert_service_error!(result, ServiceError::NotFound { .. });
    }
}
