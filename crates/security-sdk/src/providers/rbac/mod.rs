//! RBAC authorization provider.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    authorization::{AccessRequest, AuthorizationDecision, AuthorizationProvider},
    context::SecurityContext,
    error::SecurityError,
    policy::RoleStore,
    principal::Principal,
};

/// Role-Based Access Control provider backed by a [`RoleStore`].
///
/// For each role on the principal, fetches its permissions from the store
/// and checks whether any grant the requested resource+action.
/// A wildcard action (`"*"`) matches any requested action.
pub struct RbacProvider {
    store: Arc<dyn RoleStore>,
}

impl RbacProvider {
    /// Creates a provider backed by the given [`RoleStore`].
    pub fn new(store: Arc<dyn RoleStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AuthorizationProvider for RbacProvider {
    async fn authorize(
        &self,
        principal: &Principal,
        request: &AccessRequest,
        _ctx: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        for role in &principal.roles {
            let perms = self.store.permissions_for_role(role).await?;
            for perm in perms {
                if perm.resource == request.resource.kind
                    && (perm.action == request.action.0 || perm.action == "*")
                {
                    return Ok(AuthorizationDecision::Allow);
                }
            }
        }
        Ok(AuthorizationDecision::Deny {
            reason: format!(
                "no permission for {}:{}",
                request.resource.kind, request.action.0
            ),
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
        error::SecurityError,
        policy::{InMemoryRoleStore, MockRoleStore, Permission},
        principal::{Principal, PrincipalKind, Role, SubjectId},
    };

    fn make_ctx(principal: &Principal) -> SecurityContext {
        SecurityContext::new(principal.clone())
    }

    fn user_with_role(role: &str) -> Principal {
        let subject = SubjectId::new(format!("user:{role}")).unwrap();
        Principal::new(PrincipalKind::User, subject).with_role(Role(role.into()))
    }

    fn write_posts_req() -> AccessRequest {
        AccessRequest::new(
            Resource { kind: "posts".into(), id: None },
            Action("write".into()),
        )
    }

    #[tokio::test]
    async fn role_grants_allow() {
        let store = InMemoryRoleStore::new().with_role(
            Role("editor".into()),
            vec![Permission { resource: "posts".into(), action: "write".into() }],
        );
        let principal = user_with_role("editor");
        let ctx = make_ctx(&principal);
        let provider = RbacProvider::new(Arc::new(store));
        let decision = provider.authorize(&principal, &write_posts_req(), &ctx).await.unwrap();
        assert!(matches!(decision, AuthorizationDecision::Allow));
    }

    #[tokio::test]
    async fn missing_role_returns_deny() {
        let store = InMemoryRoleStore::new().with_role(
            Role("editor".into()),
            vec![Permission { resource: "posts".into(), action: "write".into() }],
        );
        let principal = user_with_role("viewer");
        let ctx = make_ctx(&principal);
        let provider = RbacProvider::new(Arc::new(store));
        let decision = provider.authorize(&principal, &write_posts_req(), &ctx).await.unwrap();
        assert!(matches!(decision, AuthorizationDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn wildcard_action_grants_allow() {
        let store = InMemoryRoleStore::new().with_role(
            Role("admin".into()),
            vec![Permission { resource: "data".into(), action: "*".into() }],
        );
        let principal = user_with_role("admin");
        let ctx = make_ctx(&principal);
        let provider = RbacProvider::new(Arc::new(store));
        let req = AccessRequest::new(
            Resource { kind: "data".into(), id: None },
            Action("delete".into()),
        );
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        assert!(matches!(decision, AuthorizationDecision::Allow));
    }

    #[tokio::test]
    async fn unknown_role_contributes_nothing() {
        let store = InMemoryRoleStore::new();
        let principal = user_with_role("ghost");
        let ctx = make_ctx(&principal);
        let provider = RbacProvider::new(Arc::new(store));
        let req = AccessRequest::new(
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
        );
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        assert!(matches!(decision, AuthorizationDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn custom_role_store_compiles() {
        let mut mock_store = MockRoleStore::new();
        mock_store.expect_permissions_for_role().returning(|_| Ok(vec![]));
        let provider = RbacProvider::new(Arc::new(mock_store));
        let subject = SubjectId::new("user:test").unwrap();
        let principal = Principal::new(PrincipalKind::User, subject);
        let ctx = make_ctx(&principal);
        let req = AccessRequest::new(
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
        );
        let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
        assert!(matches!(decision, AuthorizationDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn provider_error_propagates() {
        let mut mock_store = MockRoleStore::new();
        mock_store
            .expect_permissions_for_role()
            .returning(|_| Err(SecurityError::ProviderError("db down".into())));
        let provider = RbacProvider::new(Arc::new(mock_store));
        let principal = user_with_role("admin");
        let ctx = make_ctx(&principal);
        let req = AccessRequest::new(
            Resource { kind: "orders".into(), id: None },
            Action("read".into()),
        );
        let result = provider.authorize(&principal, &req, &ctx).await;
        assert!(matches!(result, Err(SecurityError::ProviderError(_))));
    }
}
