use std::sync::Arc;

use ego_security_sdk::{
    authentication::AuthenticationProvider,
    credential::Credential,
    error::SecurityError,
    principal::{Principal, PrincipalKind, SubjectId},
    providers::basic::{BasicAuthenticationProvider, CredentialVerifier},
};
use ego_domain::auth::AuthenticationError;

struct StaticVerifier {
    username: String,
    secret: String,
}

impl CredentialVerifier for StaticVerifier {
    fn verify(
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

impl CredentialVerifier for ErrorVerifier {
    fn verify(&self, _: &str, _: &str) -> Result<Option<Principal>, SecurityError> {
        Err(SecurityError::ProviderError("io".into()))
    }
}

#[test]
fn injected_verifier_returns_security_context() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider.authenticate(&Credential::Basic {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    });
    assert!(result.is_ok());
    let ctx = result.unwrap();
    assert_eq!(ctx.principal().subject.as_str(), "user:alice");
}

#[test]
fn injected_verifier_returns_none_gives_invalid_token() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider.authenticate(&Credential::Basic {
        username: "alice".into(),
        secret: "wrong".into(),
    });
    assert!(matches!(
        result,
        Err(AuthenticationError::InvalidToken(_))
    ));
}

#[test]
fn non_basic_credential_returns_invalid_token() {
    let provider = BasicAuthenticationProvider::new(Arc::new(StaticVerifier {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    }));

    let result = provider.authenticate(&Credential::Bearer("tok".into()));
    assert!(matches!(
        result,
        Err(AuthenticationError::InvalidToken(_))
    ));
}

#[test]
fn verifier_backend_error_gives_invalid_token() {
    let provider = BasicAuthenticationProvider::new(Arc::new(ErrorVerifier));

    let result = provider.authenticate(&Credential::Basic {
        username: "alice".into(),
        secret: "s3cr3t".into(),
    });
    assert!(matches!(
        result,
        Err(AuthenticationError::InvalidToken(_))
    ));
}
