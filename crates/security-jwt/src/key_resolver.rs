//! Key resolution types for JWT verification — trait, error, key, and in-memory resolver.

use async_trait::async_trait;

use crate::config::JwtAlgorithm;

// ---------------------------------------------------------------------------
// KeyResolverError
// ---------------------------------------------------------------------------

/// Errors returned by a [`KeyResolver`] when key retrieval fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyResolverError {
    /// No key found for the given `kid` and algorithm combination.
    #[error("key not found (kid: {kid:?})")]
    KeyNotFound {
        /// The JWT `kid` header value that was requested, if present.
        kid: Option<String>,
    },

    /// The resolver holds a key for a different algorithm than was requested.
    #[error("algorithm mismatch: expected {expected:?}, requested {requested:?}")]
    AlgorithmMismatch {
        /// The algorithm this resolver is configured for.
        expected: JwtAlgorithm,
        /// The algorithm that was requested by the caller.
        requested: JwtAlgorithm,
    },

    /// The key material is present but cannot be used for verification.
    #[error("invalid key material: {0}")]
    InvalidKeyMaterial(String),
}

// ---------------------------------------------------------------------------
// VerificationKey
// ---------------------------------------------------------------------------

/// A resolved verification key ready for JWT signature validation.
///
/// This enum is `#[non_exhaustive]` — future variants (ES256, EdDSA, JWK-backed)
/// may be added without changing [`KeyResolver`] or
/// [`ego_security_sdk::AuthenticationProvider`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum VerificationKey {
    /// HMAC-SHA256 shared secret bytes.
    Hmac(Vec<u8>),
    /// RSA public key in PEM format (RS256). The string begins with
    /// `-----BEGIN PUBLIC KEY-----` or `-----BEGIN RSA PUBLIC KEY-----`.
    RsaPem(String),
    /// EC public key in PEM format (ES256). The string begins with
    /// `-----BEGIN PUBLIC KEY-----` (PKIX/SubjectPublicKeyInfo form).
    EcPem(String),
}

// ---------------------------------------------------------------------------
// KeyResolver trait
// ---------------------------------------------------------------------------

/// Resolves verification keys for JWT signature validation.
///
/// Implementations MUST be cache-first: `resolve` must return from locally
/// available state. Remote key acquisition (JWKS, database) MUST happen
/// outside `authenticate()` — via background refresh, warm-up, or scheduled
/// sync. This is the foundational safety contract (AD-013) that makes the
/// async-to-sync bridge with `futures_executor::block_on` correct.
///
/// `dyn KeyResolver` is object-safe and can be stored behind `Arc`.
#[async_trait]
pub trait KeyResolver: Send + Sync {
    /// Resolve the verification key for the given `kid` and `algorithm`.
    ///
    /// `kid` is the JWT `kid` header claim, if present. Implementations may
    /// use it for key selection or ignore it (see CLAR-009 — for
    /// [`LocalKeyResolver`], `kid` is advisory).
    ///
    /// # Errors
    ///
    /// Returns [`KeyResolverError`] when no key can be provided for the
    /// requested algorithm or kid.
    async fn resolve(
        &self,
        kid: Option<&str>,
        algorithm: JwtAlgorithm,
    ) -> Result<VerificationKey, KeyResolverError>;
}

// ---------------------------------------------------------------------------
// LocalKeyResolver
// ---------------------------------------------------------------------------

/// An in-memory [`KeyResolver`] backed by a single static verification key.
///
/// Satisfies the cache-first contract (AD-013): `resolve` returns immediately
/// from memory with no I/O. This is the reference implementation for local
/// and test use cases.
///
/// Per CLAR-009, `kid` is advisory — `LocalKeyResolver` resolves its configured
/// key regardless of what `kid` value is passed, as long as the algorithm matches.
pub struct LocalKeyResolver {
    algorithm: JwtAlgorithm,
    key: VerificationKey,
}

impl LocalKeyResolver {
    /// Create a resolver holding a single key for a single algorithm.
    ///
    /// `algorithm` — the [`JwtAlgorithm`] this resolver is configured for.
    /// `key` — the [`VerificationKey`] to return on a successful resolve.
    pub fn new(algorithm: JwtAlgorithm, key: VerificationKey) -> Self {
        Self { algorithm, key }
    }
}

#[async_trait]
impl KeyResolver for LocalKeyResolver {
    async fn resolve(
        &self,
        _kid: Option<&str>,
        algorithm: JwtAlgorithm,
    ) -> Result<VerificationKey, KeyResolverError> {
        if algorithm != self.algorithm {
            return Err(KeyResolverError::AlgorithmMismatch {
                expected: self.algorithm,
                requested: algorithm,
            });
        }
        Ok(self.key.clone())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Fixture pinning (compile-only check that EC fixture files are present)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod fixture_pin_tests {
    // These include_str! calls fail to compile if any fixture is missing from disk.
    const _EC_PRIVATE: &str = include_str!("../tests/fixtures/test_ec_private.pem");
    const _EC_PUBLIC: &str = include_str!("../tests/fixtures/test_ec_public.pem");
    const _EC_OTHER_PRIVATE: &str = include_str!("../tests/fixtures/test_ec_other_private.pem");
    const _EC_OTHER_PUBLIC: &str = include_str!("../tests/fixtures/test_ec_other_public.pem");

    // fixtures are include_str! consts; the assert documents intent even though
    // clippy can prove non-emptiness at compile time.
    #[allow(clippy::const_is_empty)]
    #[test]
    fn ec_fixtures_are_non_empty() {
        assert!(!_EC_PRIVATE.is_empty());
        assert!(!_EC_PUBLIC.is_empty());
        assert!(!_EC_OTHER_PRIVATE.is_empty());
        assert!(!_EC_OTHER_PUBLIC.is_empty());
    }
}

#[cfg(test)]
mod key_resolver_error_tests {
    use super::*;

    #[test]
    fn key_not_found_carries_kid() {
        let err = KeyResolverError::KeyNotFound { kid: Some("k1".into()) };
        let repr = format!("{err:?}");
        assert!(repr.contains("k1"));
        let display = err.to_string();
        assert!(display.contains("k1"));
    }

    #[test]
    fn algorithm_mismatch_carries_both_sides() {
        let err = KeyResolverError::AlgorithmMismatch {
            expected: JwtAlgorithm::Hs256,
            requested: JwtAlgorithm::Rs256,
        };
        let display = err.to_string();
        assert!(display.contains("Hs256"));
        assert!(display.contains("Rs256"));
    }
}

#[cfg(test)]
mod verification_key_tests {
    use super::*;

    #[test]
    fn ec_pem_variant_stores_string() {
        let pem = "-----BEGIN PUBLIC KEY-----\ntest\n-----END PUBLIC KEY-----".to_string();
        let key = VerificationKey::EcPem(pem.clone());
        match key {
            VerificationKey::EcPem(stored) => assert_eq!(stored, pem),
            _ => panic!("expected EcPem"),
        }
    }

    #[test]
    fn hmac_variant_stores_bytes() {
        let bytes = vec![1u8, 2, 3];
        let key = VerificationKey::Hmac(bytes.clone());
        match key {
            VerificationKey::Hmac(b) => assert_eq!(b, bytes),
            _ => panic!("expected Hmac variant"),
        }
    }

    #[test]
    fn rsa_pem_variant_stores_string() {
        let pem = "-----BEGIN PUBLIC KEY-----\nfake\n-----END PUBLIC KEY-----".to_string();
        let key = VerificationKey::RsaPem(pem.clone());
        match key {
            VerificationKey::RsaPem(s) => assert_eq!(s, pem),
            _ => panic!("expected RsaPem variant"),
        }
    }
}

#[cfg(test)]
mod key_resolver_trait_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn key_resolver_is_object_safe() {
        struct AlwaysHmac;

        #[async_trait]
        impl KeyResolver for AlwaysHmac {
            async fn resolve(
                &self,
                _kid: Option<&str>,
                _algorithm: JwtAlgorithm,
            ) -> Result<VerificationKey, KeyResolverError> {
                Ok(VerificationKey::Hmac(vec![]))
            }
        }

        let _resolver: Arc<dyn KeyResolver> = Arc::new(AlwaysHmac);
    }

    #[test]
    fn local_key_resolver_is_runtime_free() {
        let resolver = LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(b"test-key".to_vec()),
        );
        let result = futures_executor::block_on(resolver.resolve(None, JwtAlgorithm::Hs256));
        match result {
            Ok(VerificationKey::Hmac(bytes)) => assert_eq!(bytes, b"test-key"),
            other => panic!("expected Ok(Hmac), got {other:?}"),
        }
    }
}

#[cfg(test)]
mod local_key_resolver_tests {
    use super::*;
    use futures_executor::block_on;
    use std::sync::Arc;

    fn hmac_secret() -> Vec<u8> {
        b"test-secret".to_vec()
    }

    fn rsa_pem() -> String {
        "-----BEGIN PUBLIC KEY-----\nfake\n-----END PUBLIC KEY-----".to_string()
    }

    #[test]
    fn test_resolves_hs256_key() {
        let resolver = LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hmac_secret()),
        );
        let result = block_on(resolver.resolve(None, JwtAlgorithm::Hs256));
        match result {
            Ok(VerificationKey::Hmac(bytes)) => assert_eq!(bytes, hmac_secret()),
            other => panic!("expected Ok(Hmac), got {other:?}"),
        }
    }

    #[test]
    fn test_resolves_rs256_key() {
        let resolver = LocalKeyResolver::new(
            JwtAlgorithm::Rs256,
            VerificationKey::RsaPem(rsa_pem()),
        );
        let result = block_on(resolver.resolve(None, JwtAlgorithm::Rs256));
        match result {
            Ok(VerificationKey::RsaPem(pem)) => assert_eq!(pem, rsa_pem()),
            other => panic!("expected Ok(RsaPem), got {other:?}"),
        }
    }

    #[test]
    fn test_algorithm_mismatch() {
        let resolver = LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hmac_secret()),
        );
        let result = block_on(resolver.resolve(None, JwtAlgorithm::Rs256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Hs256);
                assert_eq!(requested, JwtAlgorithm::Rs256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_ignores_kid() {
        let resolver = LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hmac_secret()),
        );
        let with_kid = block_on(resolver.resolve(Some("key-id-1"), JwtAlgorithm::Hs256));
        let without_kid = block_on(resolver.resolve(None, JwtAlgorithm::Hs256));
        match (with_kid, without_kid) {
            (Ok(VerificationKey::Hmac(a)), Ok(VerificationKey::Hmac(b))) => {
                assert_eq!(a, b);
            }
            other => panic!("expected both Ok(Hmac), got {other:?}"),
        }
    }

    #[test]
    fn shared_resolver_works_across_multiple_clones() {
        let resolver: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
            JwtAlgorithm::Hs256,
            VerificationKey::Hmac(hmac_secret()),
        ));
        let r2 = Arc::clone(&resolver);
        let res1 = block_on(resolver.resolve(None, JwtAlgorithm::Hs256));
        let res2 = block_on(r2.resolve(None, JwtAlgorithm::Hs256));
        match (res1, res2) {
            (Ok(VerificationKey::Hmac(a)), Ok(VerificationKey::Hmac(b))) => {
                assert_eq!(a, hmac_secret());
                assert_eq!(b, hmac_secret());
            }
            other => panic!("expected both Ok(Hmac), got {other:?}"),
        }
    }
}
