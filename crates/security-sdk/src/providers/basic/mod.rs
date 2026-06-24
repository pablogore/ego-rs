//! Basic authentication provider — synchronous.

use std::sync::Arc;

use crate::{
    authentication::AuthenticationProvider, context::SecurityContext, credential::Credential,
    error::SecurityError, principal::Principal,
};
use ego_domain::auth::AuthenticationError;

/// Verifies a username/secret pair against a backing store.
///
/// Synchronous per AD-004: authentication performs no I/O.
/// Backend data MUST be loaded before authentication execution.
pub trait CredentialVerifier: Send + Sync {
    /// Verifies `secret` for `username`.
    ///
    /// Returns `Ok(Some(principal))` on match, `Ok(None)` on mismatch,
    /// and `Err(SecurityError::ProviderError)` for backend failure.
    fn verify(&self, username: &str, secret: &str) -> Result<Option<Principal>, SecurityError>;
}

/// Authentication provider for the HTTP Basic scheme.
///
/// Delegates credential verification to a [`CredentialVerifier`].
pub struct BasicAuthenticationProvider {
    verifier: Arc<dyn CredentialVerifier>,
}

impl BasicAuthenticationProvider {
    /// Creates a provider backed by `verifier`.
    pub fn new(verifier: Arc<dyn CredentialVerifier>) -> Self {
        Self { verifier }
    }
}

impl AuthenticationProvider for BasicAuthenticationProvider {
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        match credential {
            Credential::Basic { username, secret } => {
                match self.verifier.verify(username, secret) {
                    Ok(Some(principal)) => Ok(SecurityContext::new(principal)),
                    Ok(None) => Err(AuthenticationError::InvalidToken(
                        "invalid username or password".into(),
                    )),
                    Err(e) => Err(AuthenticationError::InvalidToken(format!(
                        "provider error: {e}"
                    ))),
                }
            }
            _ => Err(AuthenticationError::InvalidToken(
                "BasicAuthenticationProvider requires a Basic credential".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        credential::Credential,
        error::SecurityError,
        principal::{Principal, PrincipalKind, SubjectId},
    };

    use super::*;

    #[test]
    fn verifier_is_object_safe() {
        struct StubVerifier;

        impl CredentialVerifier for StubVerifier {
            fn verify(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Option<Principal>, SecurityError> {
                unimplemented!()
            }
        }

        let _: Arc<dyn CredentialVerifier> = Arc::new(StubVerifier);
    }

    #[test]
    fn verifier_dyn_is_send_and_sync() {
        fn assert_send_sync<T: ?Sized + Send + Sync>() {}
        assert_send_sync::<dyn CredentialVerifier>();
    }

    struct InMemoryVerifier {
        username: String,
        secret: String,
    }

    impl CredentialVerifier for InMemoryVerifier {
        fn verify(
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

    impl CredentialVerifier for ErrorVerifier {
        fn verify(&self, _: &str, _: &str) -> Result<Option<Principal>, SecurityError> {
            Err(SecurityError::ProviderError("io".into()))
        }
    }

    #[test]
    fn valid_credential_authenticates() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
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
    fn invalid_secret_fails() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
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
    fn non_basic_credential_rejected() {
        let provider = BasicAuthenticationProvider::new(Arc::new(InMemoryVerifier {
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
    fn verifier_backend_error_surfaces_invalid_token() {
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
}
