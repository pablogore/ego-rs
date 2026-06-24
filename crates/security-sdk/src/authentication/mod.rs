//! Authentication provider contract — synchronous, returns [`SecurityContext`].

use crate::{context::SecurityContext, credential::Credential};
use ego_domain::auth::AuthenticationError;

/// Resolves a presented [`Credential`] into an authenticated [`SecurityContext`].
///
/// Synchronous per AD-004: authentication is CPU-bound, performs no I/O.
/// Object-safe: stored and invoked as `Arc<dyn AuthenticationProvider>`.
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticates `credential` and returns the resolved [`SecurityContext`].
    ///
    /// # Errors
    /// Returns [`AuthenticationError`] on invalid credentials, expired tokens,
    /// unsupported algorithms, missing required claims, or signature mismatch.
    fn authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::context::SecurityContext;
    use crate::credential::Credential;
    use crate::principal::{Principal, PrincipalKind, SubjectId};

    use super::*;

    struct StubAuthProvider;

    impl AuthenticationProvider for StubAuthProvider {
        fn authenticate(
            &self,
            _credential: &Credential,
        ) -> Result<SecurityContext, AuthenticationError> {
            unimplemented!()
        }
    }

    #[test]
    fn provider_is_object_safe() {
        let _: Arc<dyn AuthenticationProvider> = Arc::new(StubAuthProvider);
    }

    #[test]
    fn provider_dyn_is_send_and_sync() {
        fn assert_send_sync<T: ?Sized + Send + Sync>() {}
        assert_send_sync::<dyn AuthenticationProvider>();
    }

    #[test]
    fn authenticate_returns_security_context() {
        struct ReturnsUser;

        impl AuthenticationProvider for ReturnsUser {
            fn authenticate(
                &self,
                _credential: &Credential,
            ) -> Result<SecurityContext, AuthenticationError> {
                let principal = Principal::new(PrincipalKind::User, SubjectId::new("user:stub").unwrap());
                Ok(SecurityContext::new(principal))
            }
        }

        let cred = Credential::Bearer("tok".into());
        let result = ReturnsUser.authenticate(&cred);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().principal().subject.as_str(), "user:stub");
    }
}
