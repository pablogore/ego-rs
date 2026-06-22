mod common;

use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::{
    authorization::{
        AccessRequest, Action, AuthorizationDecision, AuthorizationProvider, Resource,
    },
    authorize_in_context,
    context::SecurityContext,
    error::SecurityError,
    policy::{InMemoryRoleStore, Permission},
    principal::{Principal, PrincipalKind, Role, SubjectId},
    providers::rbac::RbacProvider,
};

use common::make_ctx_from_subject;

struct StubAllow;

#[async_trait]
impl AuthorizationProvider for StubAllow {
    async fn authorize(
        &self,
        _: &Principal,
        _: &AccessRequest,
        _: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Allow)
    }
}

struct StubDeny {
    reason: String,
}

#[async_trait]
impl AuthorizationProvider for StubDeny {
    async fn authorize(
        &self,
        _: &Principal,
        _: &AccessRequest,
        _: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Ok(AuthorizationDecision::Deny {
            reason: self.reason.clone(),
        })
    }
}

struct StubProviderError;

#[async_trait]
impl AuthorizationProvider for StubProviderError {
    async fn authorize(
        &self,
        _: &Principal,
        _: &AccessRequest,
        _: &SecurityContext,
    ) -> Result<AuthorizationDecision, SecurityError> {
        Err(SecurityError::ProviderError("x".into()))
    }
}

#[tokio::test]
async fn allow_returns_ok_unit() {
    let ctx = make_ctx_from_subject("user:alice");
    let result = authorize_in_context(
        Some(&ctx),
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
        &StubAllow,
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn deny_returns_authorization_denied() {
    let ctx = make_ctx_from_subject("user:alice");
    let result = authorize_in_context(
        Some(&ctx),
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
        &StubDeny {
            reason: "no role".into(),
        },
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
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
        &StubAllow,
    )
    .await;
    assert!(matches!(result, Err(SecurityError::MissingContext)));
}

#[tokio::test]
async fn full_path_allow() {
    let store = InMemoryRoleStore::new().with_role(
        Role("editor".into()),
        vec![Permission {
            resource: "posts".into(),
            action: "write".into(),
        }],
    );
    let provider = RbacProvider::new(Arc::new(store));
    let sub = SubjectId::new("user:ed").unwrap();
    let principal = Principal::new(PrincipalKind::User, sub).with_role(Role("editor".into()));
    let ctx = SecurityContext::new(principal);
    let result = authorize_in_context(
        Some(&ctx),
        Resource {
            kind: "posts".into(),
            id: None,
        },
        Action("write".into()),
        &provider,
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn provider_error_surfaces_as_provider_error() {
    let ctx = make_ctx_from_subject("user:alice");
    let result = authorize_in_context(
        Some(&ctx),
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
        &StubProviderError,
    )
    .await;
    assert!(matches!(result, Err(SecurityError::ProviderError(_))));
}

#[tokio::test]
async fn full_path_deny() {
    let store = InMemoryRoleStore::new().with_role(
        Role("editor".into()),
        vec![Permission {
            resource: "posts".into(),
            action: "write".into(),
        }],
    );
    let provider = RbacProvider::new(Arc::new(store));
    let sub = SubjectId::new("user:viewer").unwrap();
    let principal = Principal::new(PrincipalKind::User, sub).with_role(Role("viewer".into()));
    let ctx = SecurityContext::new(principal);
    let result = authorize_in_context(
        Some(&ctx),
        Resource {
            kind: "posts".into(),
            id: None,
        },
        Action("write".into()),
        &provider,
    )
    .await;
    assert!(matches!(
        result,
        Err(SecurityError::AuthorizationDenied { .. })
    ));
}
