//! Deny-all authorization provider for lockdown and secure-by-default configurations.

use async_trait::async_trait;

use crate::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    principal::Principal,
};

/// Deny-all authorization provider for lockdown and secure-by-default configurations.
///
/// This provider unconditionally returns
/// `Ok(AuthorizationDecision::Deny { reason: "deny-all" })` for every request,
/// regardless of the `Principal`, `AccessRequest`, or `SecurityContext` inputs.
///
/// # Intended use cases
///
/// - **Secure-by-default**: use as a safe fallback when no authorization policy
///   has been configured yet, ensuring zero accidental permissiveness.
/// - **Lockdown mode**: temporarily deny all access during maintenance or incident
///   response without changing application code.
/// - **Testing**: verify that denied access is handled gracefully throughout the
///   call stack.
pub struct DenyAllAuthorizationProvider;

#[async_trait]
impl AuthorizationProvider for DenyAllAuthorizationProvider {
    async fn authorize(
        &self,
        _principal: &Principal,
        _request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Deny {
            reason: "deny-all".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        authorization::{AccessRequest, Action, AuthorizationDecision, Resource},
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

    // TS-016
    #[tokio::test]
    async fn deny_all_returns_deny_for_any_principal_and_request() {
        let provider = DenyAllAuthorizationProvider;
        let (principal, ctx) = make_principal_and_ctx();
        let req = make_request();
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        assert!(
            matches!(decision, AuthorizationDecision::Deny { .. }),
            "DenyAllAuthorizationProvider must always return Deny"
        );
    }

    // TS-017
    #[tokio::test]
    async fn deny_all_reason_is_deny_all() {
        let provider = DenyAllAuthorizationProvider;
        let (principal, ctx) = make_principal_and_ctx();
        let req = make_request();
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        match decision {
            AuthorizationDecision::Deny { reason } => {
                assert_eq!(reason, "deny-all", "reason must be exactly 'deny-all'");
            }
            AuthorizationDecision::Allow => panic!("expected Deny, got Allow"),
        }
    }

    // TS-018
    #[test]
    fn deny_all_is_send_sync() {
        fn _assert<T: Send + Sync>() {}
        _assert::<DenyAllAuthorizationProvider>();
    }

    // FR-018 arc-safety
    #[test]
    fn deny_all_arc_injectable() {
        let _: Arc<dyn AuthorizationProvider> = Arc::new(DenyAllAuthorizationProvider);
    }
}
