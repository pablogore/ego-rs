mod common;

use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::{
    authorization::{
        AccessRequest, Action, AuthorizationDecision, AuthorizationProvider, Resource,
    },
    error::SecurityError,
    policy::{InMemoryRoleStore, Permission, RoleStore},
    principal::Role,
    providers::rbac::RbacProvider,
};

use common::{make_ctx, principal_with_role};

struct GrantAllStore {
    permission: Permission,
}

#[async_trait]
impl RoleStore for GrantAllStore {
    async fn permissions_for_role(&self, _: &Role) -> Result<Vec<Permission>, SecurityError> {
        Ok(vec![self.permission.clone()])
    }
}

#[tokio::test]
async fn principal_role_grants_allow() {
    let store = InMemoryRoleStore::new().with_role(
        Role("admin".into()),
        vec![Permission {
            resource: "orders".into(),
            action: "read".into(),
        }],
    );
    let principal = principal_with_role("admin");
    let ctx = make_ctx(&principal);
    let provider = RbacProvider::new(Arc::new(store));
    let req = AccessRequest::new(
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
    );
    let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
    assert!(matches!(decision, AuthorizationDecision::Allow));
}

#[tokio::test]
async fn missing_role_returns_deny() {
    let store = InMemoryRoleStore::new().with_role(
        Role("admin".into()),
        vec![Permission {
            resource: "orders".into(),
            action: "read".into(),
        }],
    );
    let principal = principal_with_role("viewer");
    let ctx = make_ctx(&principal);
    let provider = RbacProvider::new(Arc::new(store));
    let req = AccessRequest::new(
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
    );
    let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
    assert!(matches!(decision, AuthorizationDecision::Deny { .. }));
}

#[tokio::test]
async fn wildcard_action_grants_any_action() {
    let store = InMemoryRoleStore::new().with_role(
        Role("superuser".into()),
        vec![Permission {
            resource: "data".into(),
            action: "*".into(),
        }],
    );
    let principal = principal_with_role("superuser");
    let ctx = make_ctx(&principal);
    let provider = RbacProvider::new(Arc::new(store));
    let req = AccessRequest::new(
        Resource {
            kind: "data".into(),
            id: None,
        },
        Action("delete".into()),
    );
    let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
    assert!(matches!(decision, AuthorizationDecision::Allow));
}

#[tokio::test]
async fn unknown_role_empty_perms_deny() {
    let store = InMemoryRoleStore::new();
    let principal = principal_with_role("ghost");
    let ctx = make_ctx(&principal);
    let provider = RbacProvider::new(Arc::new(store));
    let req = AccessRequest::new(
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
    );
    let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
    assert!(matches!(decision, AuthorizationDecision::Deny { .. }));
}

#[tokio::test]
async fn mock_role_store_injectable() {
    // Custom RoleStore with no dependency on InMemoryRoleStore — proves
    // RbacProvider depends only on Arc<dyn RoleStore>.
    let provider = RbacProvider::new(Arc::new(GrantAllStore {
        permission: Permission {
            resource: "orders".into(),
            action: "read".into(),
        },
    }));
    let principal = principal_with_role("any");
    let ctx = make_ctx(&principal);
    let req = AccessRequest::new(
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
    );
    let decision = provider.authorize(&principal, &req, &ctx).await.unwrap();
    assert!(matches!(decision, AuthorizationDecision::Allow));
}
