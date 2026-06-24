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
}

/// Full configuration for a [`super::JwtAuthenticator`].
///
/// Pass this to [`super::JwtAuthenticator::new`] together with a
/// [`crate::KeyResolver`] and an [`ego_domain::auth::Clock`] to construct
/// an authenticator. This struct holds only functional validation parameters —
/// key material lives in the resolver.
pub struct JwtConfig {
    /// Algorithm discriminant — selects HS256 or RS256. Key material is
    /// provided separately via [`crate::KeyResolver`].
    pub algorithm: JwtAlgorithm,

    /// If `Some`, the token's `iss` claim MUST equal this value.
    /// If `None`, any issuer (including absent) is accepted.
    pub expected_iss: Option<String>,

    /// If `Some`, the token's `aud` claim MUST contain at least one of these values.
    /// If `None`, the `aud` claim is not validated.
    pub expected_aud: Option<Vec<String>>,
}
