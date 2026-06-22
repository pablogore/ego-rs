//! Authorization provider contract, access request types, and decision types.

pub mod access_request;
pub mod decision;

pub use access_request::{AccessRequest, Action, Resource};
pub use decision::AuthorizationDecision;

use async_trait::async_trait;

use crate::{context::SecurityContext, error::SecurityError, principal::Principal};

/// Decides whether a [`Principal`] may perform the action named by an
/// [`AccessRequest`].
///
/// Object-safe; invoked as `Arc<dyn AuthorizationProvider>`. A clean Deny
/// is returned as `Ok(Deny { .. })`, NOT an error — only backend failures
/// return `Err(SecurityError)`.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AuthorizationProvider: Send + Sync {
    /// Evaluates the request and returns an [`AuthorizationDecision`].
    ///
    /// # Errors
    /// Returns `Err(SecurityError)` only for backend failures — a policy
    /// denial is `Ok(Deny { reason })`.
    async fn authorize(
        &self,
        principal: &Principal,
        request: &AccessRequest,
        ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError>;
}

/// Resolves the security context, builds an [`AccessRequest`], calls the
/// authorization provider, and maps a [`AuthorizationDecision::Deny`] decision to a
/// [`SecurityError::AuthorizationDenied`].
///
/// This is the stable seam a future `#[authorize("resource:action")]` macro
/// targets. The caller passes the already-resolved `Option<&SecurityContext>`
/// (extracted from `ctx.security`) so this function remains ego-dep-free.
///
/// # Errors
/// - [`SecurityError::MissingContext`] if `security` is `None`.
/// - [`SecurityError::AuthorizationDenied`] if the decision is `Deny`.
/// - Propagates any provider error.
pub async fn authorize_in_context(
    security: Option<&SecurityContext>,
    resource: Resource,
    action: Action,
    provider: &dyn AuthorizationProvider,
) -> Result<(), SecurityError> {
    let sec = security.ok_or(SecurityError::MissingContext)?;
    let principal = sec.principal();
    let request = AccessRequest::new(resource, action);
    match provider.authorize(principal, &request, sec).await? {
        AuthorizationDecision::Allow => Ok(()),
        AuthorizationDecision::Deny { reason } => {
            Err(SecurityError::AuthorizationDenied { reason })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        context::SecurityContext,
        error::SecurityError,
        principal::{Principal, PrincipalKind, SubjectId},
    };

    // ── AuthorizationProvider object-safety tests (TASK-017) ──────────────────

    struct AlwaysAllow;

    #[async_trait]
    impl AuthorizationProvider for AlwaysAllow {
        async fn authorize(
            &self,
            _: &Principal,
            _: &AccessRequest,
            _: &SecurityContext,
        ) -> Result<AuthorizationDecision, SecurityError> {
            Ok(AuthorizationDecision::Allow)
        }
    }

    struct AlwaysDeny;

    #[async_trait]
    impl AuthorizationProvider for AlwaysDeny {
        async fn authorize(
            &self,
            _: &Principal,
            _: &AccessRequest,
            _: &SecurityContext,
        ) -> Result<AuthorizationDecision, SecurityError> {
            Ok(AuthorizationDecision::Deny { reason: "denied".into() })
        }
    }

    fn make_ctx() -> SecurityContext {
        let subject = SubjectId::new("user:test").unwrap();
        let principal = Principal::new(PrincipalKind::User, subject);
        SecurityContext::new(principal)
    }

    #[test]
    fn provider_is_object_safe() {
        let _: Arc<dyn AuthorizationProvider> = Arc::new(AlwaysAllow);
    }

    #[tokio::test]
    async fn allow_and_deny_are_matchable() {
        let ctx = make_ctx();
        let req = AccessRequest::new(
            Resource { kind: "res".into(), id: None },
            Action("act".into()),
        );

        let allow_result = AlwaysAllow.authorize(ctx.principal(), &req, &ctx).await.unwrap();
        assert!(matches!(allow_result, AuthorizationDecision::Allow));

        let deny_result = AlwaysDeny.authorize(ctx.principal(), &req, &ctx).await.unwrap();
        assert!(matches!(deny_result, AuthorizationDecision::Deny { .. }));
    }

    #[test]
    fn external_provider_impl_compiles() {
        let _: Arc<dyn AuthorizationProvider> = Arc::new(AlwaysAllow);
    }

    // ── authorize_in_context tests (TASK-025) ─────────────────────────────────

    #[tokio::test]
    async fn allow_returns_ok_unit() {
        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
            &AlwaysAllow,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn deny_maps_to_authorization_denied() {
        struct DenyProvider;

        #[async_trait]
        impl AuthorizationProvider for DenyProvider {
            async fn authorize(
                &self,
                _: &Principal,
                _: &AccessRequest,
                _: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                Ok(AuthorizationDecision::Deny { reason: "no role".into() })
            }
        }

        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
            &DenyProvider,
        )
        .await;
        assert!(matches!(
            result,
            Err(SecurityError::AuthorizationDenied { reason }) if reason == "no role"
        ));
    }

    #[tokio::test]
    async fn none_security_returns_missing_context() {
        let result = authorize_in_context(
            None,
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
            &AlwaysAllow,
        )
        .await;
        assert!(matches!(result, Err(SecurityError::MissingContext)));
    }

    #[tokio::test]
    async fn provider_error_propagates() {
        struct ErrorProvider;

        #[async_trait]
        impl AuthorizationProvider for ErrorProvider {
            async fn authorize(
                &self,
                _: &Principal,
                _: &AccessRequest,
                _: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                Err(SecurityError::ProviderError("x".into()))
            }
        }

        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
            &ErrorProvider,
        )
        .await;
        assert!(matches!(result, Err(SecurityError::ProviderError(_))));
    }
}
