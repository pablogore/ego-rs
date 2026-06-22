//! Basic authentication provider.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    authentication::AuthenticationProvider,
    credential::Credential,
    error::SecurityError,
    principal::Principal,
};

/// Verifies a username/secret pair against a backing store.
///
/// Returns `Ok(Some(principal))` on success, `Ok(None)` when credentials
/// don't match, and `Err(SecurityError::ProviderError)` for backend failure.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CredentialVerifier: Send + Sync {
    /// Verifies `secret` for `username`.
    ///
    /// Returns `Ok(Some(principal))` on match, `Ok(None)` on mismatch,
    /// and `Err(SecurityError::ProviderError)` for backend failure.
    async fn verify(
        &self,
        username: &str,
        secret: &str,
    ) -> Result<Option<Principal>, SecurityError>;
}

/// Authentication provider for the HTTP Basic scheme.
///
/// Delegates credential verification to a [`CredentialVerifier`], keeping
/// the provider independent from any specific storage backend.
pub struct BasicAuthenticationProvider {
    verifier: Arc<dyn CredentialVerifier>,
}

impl BasicAuthenticationProvider {
    /// Creates a provider backed by `verifier`.
    pub fn new(verifier: Arc<dyn CredentialVerifier>) -> Self {
        Self { verifier }
    }
}

#[async_trait]
impl AuthenticationProvider for BasicAuthenticationProvider {
    async fn authenticate(&self, credential: &Credential) -> Result<Principal, SecurityError> {
        match credential {
            Credential::Basic { username, secret } => {
                match self.verifier.verify(username, secret).await? {
                    Some(p) => Ok(p),
                    None => Err(SecurityError::AuthenticationFailed(
                        "invalid username or password".into(),
                    )),
                }
            }
            _ => Err(SecurityError::InvalidCredential(
                "BasicAuthenticationProvider requires a Basic credential".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        error::SecurityError,
        principal::{Principal, PrincipalKind, SubjectId},
    };

    struct InMemoryVerifier {
        username: String,
        secret: String,
    }

    #[async_trait]
    impl CredentialVerifier for InMemoryVerifier {
        async fn verify(
            &self,
            username: &str,
            secret: &str,
        ) -> Result<Option<Principal>, SecurityError> {
            if username == self.username && secret == self.secret {
                let subject = SubjectId::new(format!("user:{}", username)).unwrap();
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
    async fn valid_credential_authenticates() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
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
    async fn invalid_secret_fails() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
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
    async fn non_basic_credential_rejected() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
            username: "alice".into(),
            secret: "s3cr3t".into(),
        }));
        let result = provider
            .authenticate(&Credential::Bearer("tok".into()))
            .await;
        assert!(matches!(result, Err(SecurityError::InvalidCredential(_))));
    }

    #[tokio::test]
    async fn verifier_backend_error_surfaces_provider_error() {
        let provider = BasicAuthenticationProvider::new(Arc::new(ErrorVerifier));
        let result = provider
            .authenticate(&Credential::Basic {
                username: "alice".into(),
                secret: "s3cr3t".into(),
            })
            .await;
        assert!(matches!(result, Err(SecurityError::ProviderError(_))));
    }
}
