//! Authorization provider contract, access request types, and decision types.

pub mod access_request;
pub mod decision;

pub use access_request::{AccessRequest, Action, Resource};
pub use decision::AuthorizationDecision;

use std::panic::AssertUnwindSafe;

use async_trait::async_trait;
use futures_util::FutureExt as _;

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
/// - [`SecurityError::CapabilityNotEnabled`] if security is not enabled in the runtime.
/// - [`SecurityError::AuthorizationDenied`] if the decision is `Deny`.
/// - [`SecurityError::ProviderError`] if the provider returns an error or panics.
///   A panicking provider maps to `ProviderError` — the system always fails closed.
pub async fn authorize_in_context(
    security: Option<&SecurityContext>,
    resource: Resource,
    action: Action,
    provider: &dyn AuthorizationProvider,
) -> Result<(), SecurityError> {
    let sec = security.ok_or(SecurityError::CapabilityNotEnabled)?;
    let principal = sec.principal();
    let request = AccessRequest::new(resource, action);
    let decision = AssertUnwindSafe(provider.authorize(principal, &request, sec))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(SecurityError::ProviderError(
                "authorization provider panicked".into(),
            ))
        })?;
    match decision {
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

    // ── AuthorizationProvider object-safety tests ─────────────────────────────

    /// Inline test stub — always grants access. NOT a public type; see
    /// `providers::allow_all::AllowAllAuthorizationProvider` for the public variant.
    struct InlineAllow;

    #[async_trait]
    impl AuthorizationProvider for InlineAllow {
        async fn authorize(
            &self,
            _: &Principal,
            _: &AccessRequest,
            _: &SecurityContext,
        ) -> Result<AuthorizationDecision, SecurityError> {
            Ok(AuthorizationDecision::Allow)
        }
    }

    fn make_ctx() -> SecurityContext {
        let subject = SubjectId::new("user:test").unwrap();
        let principal = Principal::new(PrincipalKind::User, subject);
        SecurityContext::empty(principal)
    }

    #[test]
    fn provider_is_object_safe() {
        let _: Arc<dyn AuthorizationProvider> = Arc::new(InlineAllow);
    }

    #[test]
    fn provider_dyn_is_send_and_sync() {
        fn assert_send_sync<T: ?Sized + Send + Sync>() {}
        assert_send_sync::<dyn AuthorizationProvider>();
    }

    #[tokio::test]
    async fn allow_and_deny_are_matchable() {
        let ctx = make_ctx();
        let req = AccessRequest::new(
            Resource {
                kind: "res".into(),
                id: None,
            },
            Action("act".into()),
        );

        let allow_result = InlineAllow
            .authorize(ctx.principal(), &req, &ctx)
            .await
            .unwrap();
        assert!(matches!(allow_result, AuthorizationDecision::Allow));

        struct InlineDeny;

        #[async_trait]
        impl AuthorizationProvider for InlineDeny {
            async fn authorize(
                &self,
                _: &Principal,
                _: &AccessRequest,
                _: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                Ok(AuthorizationDecision::Deny {
                    reason: "denied".into(),
                })
            }
        }

        let deny_result = InlineDeny
            .authorize(ctx.principal(), &req, &ctx)
            .await
            .unwrap();
        assert!(matches!(deny_result, AuthorizationDecision::Deny { .. }));
    }

    // ── authorize_in_context tests ────────────────────────────────────────────

    #[tokio::test]
    async fn allow_returns_ok_unit() {
        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
            &InlineAllow,
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
                Ok(AuthorizationDecision::Deny {
                    reason: "no role".into(),
                })
            }
        }

        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource {
                kind: "orders".into(),
                id: None,
            },
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
    async fn none_security_returns_capability_not_enabled() {
        let result = authorize_in_context(
            None,
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
            &InlineAllow,
        )
        .await;
        assert!(matches!(result, Err(SecurityError::CapabilityNotEnabled)));
    }

    #[tokio::test]
    async fn panicking_provider_returns_provider_error() {
        struct PanicProvider;

        #[async_trait]
        impl AuthorizationProvider for PanicProvider {
            async fn authorize(
                &self,
                _: &Principal,
                _: &AccessRequest,
                _: &SecurityContext,
            ) -> Result<AuthorizationDecision, SecurityError> {
                panic!("provider bug")
            }
        }

        let ctx = make_ctx();
        let result = authorize_in_context(
            Some(&ctx),
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
            &PanicProvider,
        )
        .await;
        assert!(
            matches!(result, Err(SecurityError::ProviderError(_))),
            "expected ProviderError on panic, got: {result:?}"
        );
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
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
            &ErrorProvider,
        )
        .await;
        assert!(matches!(result, Err(SecurityError::ProviderError(_))));
    }
}
