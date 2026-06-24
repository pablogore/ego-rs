mod common;

use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::{
    authentication::AuthenticationProvider,
    authorization::{AccessRequest, Action, AuthorizationProvider, Resource},
    credential::Credential,
    error::SecurityError,
    policy::{Permission, RoleStore},
    principal::Role,
    providers::rbac::RbacProvider,
};

use common::{make_ctx, principal_with_role};

struct FailingRoleStore;

#[async_trait]
impl RoleStore for FailingRoleStore {
    async fn permissions_for_role(&self, _: &Role) -> Result<Vec<Permission>, SecurityError> {
        Err(SecurityError::ProviderError("db down".into()))
    }
}

#[tokio::test]
async fn role_store_failure_gives_provider_error() {
    let provider = RbacProvider::new(Arc::new(FailingRoleStore));
    let principal = principal_with_role("admin");
    let ctx = make_ctx(&principal);
    let req = AccessRequest::new(
        Resource {
            kind: "orders".into(),
            id: None,
        },
        Action("read".into()),
    );
    let result = provider.authorize(&principal, &req, &ctx).await;
    assert!(matches!(result, Err(SecurityError::ProviderError(_))));
}

#[test]
fn provider_error_display_contains_no_vendor_name() {
    let err = SecurityError::ProviderError("internal failure".into());
    let display = err.to_string();
    assert!(!display.contains("jsonwebtoken"));
    assert!(!display.contains("ldap"));
    assert!(!display.contains("openfga"));
}

#[test]
fn authentication_provider_error_is_neutral() {
    use ego_security_sdk::context::SecurityContext;
    use ego_domain::auth::AuthenticationError;

    struct ErrorAuthProvider;

    impl AuthenticationProvider for ErrorAuthProvider {
        fn authenticate(
            &self,
            _: &Credential,
        ) -> Result<SecurityContext, AuthenticationError> {
            Err(AuthenticationError::InvalidToken("provider error".into()))
        }
    }

    let result = ErrorAuthProvider.authenticate(&Credential::Bearer("tok".into()));
    assert!(matches!(result, Err(AuthenticationError::InvalidToken(_))));
    if let Err(e) = result {
        assert!(!e.to_string().contains("jsonwebtoken"));
    }
}
