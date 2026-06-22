use std::sync::Arc;

use async_trait::async_trait;
use ego_security_sdk::{
    authentication::AuthenticationProvider,
    credential::Credential,
    error::SecurityError,
    principal::{Principal, PrincipalKind, SubjectId},
    providers::basic::{BasicAuthenticationProvider, CredentialVerifier},
};

struct StaticVerifier {
    username: String,
    secret: String,
}

#[async_trait]
impl CredentialVerifier for StaticVerifier {
    async fn verify(
        &self,
        username: &str,
        secret: &str,
    ) -> Result<Option<Principal>, SecurityError> {
        if username == self.username && secret == self.secret {
            let subject = SubjectId::new(format!("user:{username}")).unwrap();
            Ok(Some(Principal::new(PrincipalKind::User, subject)))
        } else {
            Ok(None)
        }
    }
}

struct ErrorVerifier;

#[async_trait]
impl CredentialVerifier for ErrorVerifier {
    async fn verify(&self, _: &str, _: &str) -> Result<Option<Principal>, SecurityError> {
        Err(SecurityError::ProviderError("io".into()))
    }
}

#[tokio::test]
async fn injected_verifier_returns_principal() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider
        .authenticate(&Credential::Basic {
            username: "alice".into(),
            secret: "s3cr3t".into(),
        })
        .await;
    assert!(result.is_ok());
    let p = result.unwrap();
    assert_eq!(p.subject.as_str(), "user:alice");
}

#[tokio::test]
async fn injected_verifier_returns_none_gives_auth_failed() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider
        .authenticate(&Credential::Basic {
            username: "alice".into(),
            secret: "wrong".into(),
        })
        .await;
    assert!(matches!(result, Err(SecurityError::AuthenticationFailed(_))));
}

#[tokio::test]
async fn non_basic_credential_returns_invalid_credential() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider.authenticate(&Credential::Bearer("tok".into())).await;
    assert!(matches!(result, Err(SecurityError::InvalidCredential(_))));
}

#[tokio::test]
async fn verifier_backend_error_gives_provider_error() {
    let provider = BasicAuthenticationProvider::new(Arc::new(ErrorVerifier));

    let result = provider
        .authenticate(&Credential::Basic {
            username: "alice".into(),
            secret: "s3cr3t".into(),
        })
        .await;
    assert!(matches!(result, Err(SecurityError::ProviderError(_))));
}
