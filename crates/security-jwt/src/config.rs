//! JWT configuration types — algorithm selection and validation parameters.

/// The signing algorithm used to verify JWTs.
///
/// This is a pure marker enum — key material has been moved to
/// [`crate::VerificationKey`] inside a [`crate::KeyResolver`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JwtAlgorithm {
    /// HMAC-SHA256.
    Hs256,

    /// RSA-PKCS1-SHA256. Only the public key is needed for verification.
    Rs256,

    /// ECDSA-P256-SHA256. Only the public key is needed for verification.
    Es256,
}

/// Shared validation configuration for the single-algorithm providers.
///
/// Holds optional issuer and audience constraints. Key material lives in the
/// injected [`crate::KeyResolver`] — not here. The algorithm is encoded at
/// the type level by each provider, so no `algorithm` field is needed.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct JwtProviderConfig {
    /// If `Some`, the token's `iss` claim MUST equal this value.
    pub expected_iss: Option<String>,
    /// If `Some`, the token's `aud` claim MUST contain at least one of these values.
    pub expected_aud: Option<Vec<String>>,
}

impl Default for JwtProviderConfig {
    fn default() -> Self {
        Self { expected_iss: None, expected_aud: None }
    }
}

/// Type alias for clarity at call sites using [`crate::Hs256AuthenticationProvider`].
pub type Hs256Config = JwtProviderConfig;
/// Type alias for clarity at call sites using [`crate::Rs256AuthenticationProvider`].
pub type Rs256Config = JwtProviderConfig;
/// Type alias for clarity at call sites using [`crate::Es256AuthenticationProvider`].
pub type Es256Config = JwtProviderConfig;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es256_variant_equality() {
        assert_eq!(JwtAlgorithm::Es256, JwtAlgorithm::Es256);
    }
}
