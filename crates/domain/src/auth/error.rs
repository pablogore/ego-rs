//! Authentication error types for the domain layer.
//!
//! Defines [`AuthenticationError`] — the canonical error type for all
//! authentication failures. Each variant carries enough context for logging
//! and structured diagnostics.

/// Error returned by an authentication provider's `authenticate` method.
///
/// Each variant carries enough context to produce a structured log entry
/// without leaking sensitive token material.
///
/// # Adding new variants
///
/// This enum is `#[non_exhaustive]` and derives `PartialEq`. Adding a field
/// whose type does not implement `PartialEq` (e.g. `std::io::Error`) **breaks
/// the derive and fails to compile** — hand-roll `PartialEq` for the whole enum
/// before introducing such a field.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthenticationError {
    /// The token could not be parsed or its claims are structurally invalid.
    #[error("invalid token: {0}")]
    InvalidToken(String),

    /// The token's `exp` claim is in the past relative to the current time.
    #[error("token has expired")]
    ExpiredToken,

    /// The token header specifies an algorithm the provider does not support.
    #[error("algorithm not supported: {0}")]
    AlgorithmNotSupported(String),

    /// A required claim is absent from the token payload.
    #[error("missing required claim: {0}")]
    MissingClaim(String),

    /// The token signature does not match the configured key.
    #[error("invalid token signature")]
    InvalidSignature,

    /// The authentication provider backend is unavailable or returned an unexpected error.
    #[error("authentication provider unavailable: {0}")]
    ProviderUnavailable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_token_displays_message() {
        let err = AuthenticationError::InvalidToken("bad base64".into());
        assert_eq!(err.to_string(), "invalid token: bad base64");
    }

    #[test]
    fn expired_token_displays_correctly() {
        let err = AuthenticationError::ExpiredToken;
        assert_eq!(err.to_string(), "token has expired");
    }

    #[test]
    fn algorithm_not_supported_carries_name() {
        let err = AuthenticationError::AlgorithmNotSupported("ES256".into());
        assert_eq!(err.to_string(), "algorithm not supported: ES256");
    }

    #[test]
    fn missing_claim_carries_name() {
        let err = AuthenticationError::MissingClaim("sub".into());
        assert_eq!(err.to_string(), "missing required claim: sub");
    }

    #[test]
    fn invalid_signature_displays_correctly() {
        let err = AuthenticationError::InvalidSignature;
        assert_eq!(err.to_string(), "invalid token signature");
    }

    #[test]
    fn error_variants_are_clone_and_eq() {
        let a = AuthenticationError::ExpiredToken;
        let b = a.clone();
        assert_eq!(a, b);
    }
}
