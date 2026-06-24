//! The [`AuthenticationProvider`] trait — the domain port for authentication.
//!
//! Infrastructure crates (e.g. `security-jwt`) implement this trait. The
//! domain layer owns the contract; it never owns the implementation.

use super::{AuthenticationError, Credential, SecurityContext};

/// Synchronous authentication port.
///
/// Implementors validate a [`Credential`], extract claims, and return a
/// populated [`SecurityContext`] on success.
///
/// # Bounds
///
/// - `Send + Sync`: implementations may be stored inside `Arc<dyn AuthenticationProvider>`
///   and shared across threads.
///
/// # Example
///
/// ```rust,ignore
/// let ctx = provider.authenticate(Credential::BearerToken(raw_token))?;
/// println!("Hello, {}", ctx.identity.subject);
/// ```
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticates the given credential.
    ///
    /// Returns a [`SecurityContext`] on success, or an [`AuthenticationError`]
    /// if the credential is invalid, expired, or otherwise unacceptable.
    ///
    /// The credential is consumed by value so that sensitive material is
    /// dropped as soon as validation completes.
    fn authenticate(
        &self,
        credential: Credential,
    ) -> Result<SecurityContext, AuthenticationError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Claims, Identity};
    use std::collections::{BTreeMap, BTreeSet};

    /// A trivial always-accept provider for testing the trait contract.
    struct AlwaysOk;

    impl AuthenticationProvider for AlwaysOk {
        fn authenticate(
            &self,
            credential: Credential,
        ) -> Result<SecurityContext, AuthenticationError> {
            let subject = match &credential {
                Credential::BearerToken(s) => s.clone(),
                #[allow(unreachable_patterns)]
                _ => "unknown".into(),
            };
            let identity = Identity {
                subject,
                tenant_id: None,
                roles: BTreeSet::new(),
                attributes: BTreeMap::new(),
            };
            Ok(SecurityContext::new(identity, Claims::empty()))
        }
    }

    /// A trivial always-reject provider for testing the error path.
    struct AlwaysErr;

    impl AuthenticationProvider for AlwaysErr {
        fn authenticate(
            &self,
            _credential: Credential,
        ) -> Result<SecurityContext, AuthenticationError> {
            Err(AuthenticationError::InvalidSignature)
        }
    }

    #[test]
    fn always_ok_returns_context_with_subject_from_token() {
        let p = AlwaysOk;
        let ctx = p
            .authenticate(Credential::BearerToken("my-token".into()))
            .unwrap();
        assert_eq!(ctx.identity.subject, "my-token");
    }

    #[test]
    fn always_err_returns_authentication_error() {
        let p = AlwaysErr;
        let result = p.authenticate(Credential::BearerToken("x".into()));
        assert_eq!(result.unwrap_err(), AuthenticationError::InvalidSignature);
    }

    #[test]
    fn provider_usable_behind_arc() {
        use std::sync::Arc;
        let p: Arc<dyn AuthenticationProvider> = Arc::new(AlwaysOk);
        let ctx = p
            .authenticate(Credential::BearerToken("arc-token".into()))
            .unwrap();
        assert_eq!(ctx.identity.subject, "arc-token");
    }
}
