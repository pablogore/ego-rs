//! Security error types.

use thiserror::Error;

/// Unified, provider-neutral security error.
///
/// No third-party error type (e.g. `jsonwebtoken::Error`) appears in
/// this public surface. Provider failures are mapped to opaque strings.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// Authentication ran but the credential was rejected.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The presented credential was malformed or of an unsupported scheme.
    #[error("invalid credential: {0}")]
    InvalidCredential(String),

    /// Subject identifier is invalid (must be non-empty).
    #[error("invalid subject id: {0}")]
    InvalidSubjectId(String),

    /// Authorization denied access.
    #[error("authorization denied: {reason}")]
    AuthorizationDenied {
        /// Why access was denied.
        reason: String,
    },

    /// No security context was present where one was required.
    #[error("missing security context")]
    MissingContext,

    /// A provider or backing store failed. Underlying cause is flattened to a
    /// string so no vendor type leaks through the public surface.
    #[error("provider error: {0}")]
    ProviderError(String),

    /// An access request descriptor was malformed (e.g. bad `"resource:action"` format).
    #[error("invalid access request: {0}")]
    InvalidAccessRequest(String),
}

#[cfg(test)]
mod tests {
    use super::SecurityError;

    #[test]
    fn display_authentication_failed() {
        let err = SecurityError::AuthenticationFailed("bad".into());
        assert!(
            err.to_string().contains("authentication failed"),
            "expected 'authentication failed', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_credential() {
        let err = SecurityError::InvalidCredential("wrong".into());
        assert!(
            err.to_string().contains("invalid credential"),
            "expected 'invalid credential', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_subject_id() {
        let err = SecurityError::InvalidSubjectId("".into());
        assert!(
            err.to_string().contains("invalid subject id"),
            "expected 'invalid subject id', got: {}",
            err
        );
    }

    #[test]
    fn display_authorization_denied() {
        let err = SecurityError::AuthorizationDenied {
            reason: "nope".into(),
        };
        assert!(
            err.to_string().contains("authorization denied"),
            "expected 'authorization denied', got: {}",
            err
        );
    }

    #[test]
    fn display_missing_context() {
        let err = SecurityError::MissingContext;
        assert!(
            err.to_string().contains("missing security context"),
            "expected 'missing security context', got: {}",
            err
        );
    }

    #[test]
    fn display_provider_error() {
        let err = SecurityError::ProviderError("io".into());
        assert!(
            err.to_string().contains("provider error"),
            "expected 'provider error', got: {}",
            err
        );
    }

    #[test]
    fn display_invalid_access_request() {
        let err = SecurityError::InvalidAccessRequest("bad".into());
        assert!(
            err.to_string().contains("invalid access request"),
            "expected 'invalid access request', got: {}",
            err
        );
    }

    #[test]
    fn no_external_type_in_variants() {
        // Compile-time check: SecurityError must be Send + Sync + 'static + std::error::Error.
        fn assert_bounds<T: std::error::Error + Send + Sync + 'static>() {}
        assert_bounds::<SecurityError>();
    }
}
