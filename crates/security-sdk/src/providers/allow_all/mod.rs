//! Allow-all authorization provider for development and testing use cases.

use async_trait::async_trait;

use crate::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::Principal,
};

/// Development-only allow-all authorization provider. Always grants access.
///
/// # Warning — NOT FOR PRODUCTION
///
/// This provider unconditionally returns [`AuthorizationDecision::Allow`] for
/// every request regardless of the `Principal`, `AccessRequest`, or
/// `SecurityContext` inputs. It is intended **only** for:
///
/// - Local development runtimes
/// - Integration tests that need a no-op authorization layer
/// - Demo and sandbox environments
///
/// Do **not** use this provider in production or any environment where
/// access control has real security implications.
pub struct AllowAllAuthorizationProvider;

#[async_trait]
impl AuthorizationProvider for AllowAllAuthorizationProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        authorization::{AccessRequest, Action, Resource},
        context::SecurityContext,
        principal::{Principal, PrincipalKind, SubjectId},
    };

    fn make_principal_and_ctx() -> (Principal, SecurityContext) {
        let subject = SubjectId::new("user:test").unwrap();
        let principal = Principal::new(PrincipalKind::User, subject);
        let ctx = SecurityContext::empty(principal.clone());
        (principal, ctx)
    }

    fn make_request() -> AccessRequest {
        AccessRequest::new(
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
        )
    }

    // TS-014
    #[tokio::test]
    async fn allow_all_returns_allow_for_any_principal_and_request() {
        let provider = AllowAllAuthorizationProvider;
        let (principal, ctx) = make_principal_and_ctx();
        let req = make_request();
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        assert!(
            matches!(decision, AuthorizationDecision::Allow),
            "AllowAllAuthorizationProvider must always return Allow"
        );
    }

    // TS-015
    #[test]
    fn allow_all_is_send_sync() {
        fn _assert<T: Send + Sync>() {}
        _assert::<AllowAllAuthorizationProvider>();
    }

    // FR-017 arc-safety
    #[test]
    fn allow_all_arc_injectable() {
        let _: Arc<dyn AuthorizationProvider> = Arc::new(AllowAllAuthorizationProvider);
    }

}
