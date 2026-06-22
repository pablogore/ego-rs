//! Authentication provider contract.

use async_trait::async_trait;

use crate::{credential::Credential, error::SecurityError, principal::Principal};

/// Resolves a presented [`Credential`] into an authenticated [`Principal`].
///
/// Object-safe and async: stored and invoked as `Arc<dyn AuthenticationProvider>`.
/// No transport types appear in this contract. Providers needing tenant or
/// environment context receive it at construction time via dependency injection.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticates `credential` and returns the resolved [`Principal`].
    ///
    /// # Errors
    /// - [`SecurityError::AuthenticationFailed`] — credential rejected.
    /// - [`SecurityError::InvalidCredential`] — wrong scheme or malformed.
    /// - [`SecurityError::ProviderError`] — backend failure.
    async fn authenticate(&self, credential: &Credential) -> Result<Principal, SecurityError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    struct StubAuthProvider;

    #[async_trait]
    impl AuthenticationProvider for StubAuthProvider {
        async fn authenticate(&self, _credential: &Credential) -> Result<Principal, SecurityError> {
            unimplemented!()
        }
    }

    #[test]
    fn provider_is_object_safe() {
        let _: Arc<dyn AuthenticationProvider> = Arc::new(StubAuthProvider);
    }

    #[tokio::test]
    async fn authenticate_accepts_credential_by_ref() {
        use crate::principal::{Principal, PrincipalKind, SubjectId};

        struct ReturnsUser;

        #[async_trait]
        impl AuthenticationProvider for ReturnsUser {
            async fn authenticate(
                &self,
                _credential: &Credential,
            ) -> Result<Principal, SecurityError> {
                let subject = SubjectId::new("user:stub").unwrap();
                Ok(Principal::new(PrincipalKind::User, subject))
            }
        }

        let cred = Credential::Bearer("tok".into());
        let result = ReturnsUser.authenticate(&cred).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().subject.as_str(), "user:stub");
    }
}
