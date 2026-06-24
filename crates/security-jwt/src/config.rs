//! JWT configuration types — algorithm selection and validation parameters.

/// The signing algorithm (and associated key material) used to verify JWTs.
///
/// Each variant owns the relevant key material so that the authenticator is
/// fully self-contained once constructed.
pub enum JwtAlgorithm {
    /// HMAC-SHA256. The shared secret is a raw byte sequence.
    Hs256 {
        /// The HMAC secret key bytes.
        secret: Vec<u8>,
    },

    /// RSA-PKCS1-SHA256. Only the public key is needed for verification.
    Rs256 {
        /// PEM-encoded RSA public key (begins with `-----BEGIN PUBLIC KEY-----`
        /// or `-----BEGIN RSA PUBLIC KEY-----`).
        public_key_pem: String,
    },
}

/// Full configuration for a [`super::JwtAuthenticator`].
///
/// Pass this to [`super::JwtAuthenticator::new`] together with a
/// [`ego_domain::auth::Clock`] to construct an authenticator.
pub struct JwtConfig {
    /// The algorithm and key material used to verify token signatures.
    pub algorithm: JwtAlgorithm,

    /// If `Some`, the token's `iss` claim MUST equal this value.
    /// If `None`, any issuer (including absent) is accepted.
    pub expected_iss: Option<String>,

    /// If `Some`, the token's `aud` claim MUST contain at least one of these values.
    /// If `None`, the `aud` claim is not validated.
    pub expected_aud: Option<Vec<String>>,
}
