//! [`ApiKeyHash`] — opaque SHA-256 hash of an API key secret for constant-time verification.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Opaque hash of an API key secret.
///
/// Construction is crate-internal. Verification is public and constant-time.
#[derive(Clone)]
pub struct ApiKeyHash {
    digest: [u8; 32],
}

impl ApiKeyHash {
    /// Constructs an `ApiKeyHash` from a pre-computed SHA-256 digest.
    ///
    /// `digest` MUST be the raw SHA-256 output of the secret bytes. Public so
    /// external `ApiKeyResolver` implementations (e.g. a database-backed
    /// resolver storing digests) can populate [`crate::resolver::ApiKeyRecord`].
    /// Use [`ApiKeyHash::of`] to hash bytes directly.
    pub fn sha256(digest: [u8; 32]) -> Self {
        Self { digest }
    }

    /// Hashes `secret` with SHA-256 and returns the resulting `ApiKeyHash`.
    pub fn of(secret: &[u8]) -> Self {
        Self::sha256(Sha256::digest(secret).into())
    }

    /// Verifies `secret` against this hash in constant time.
    ///
    /// Returns `true` only when the SHA-256 hash of `secret` equals the stored
    /// digest. The comparison never exits early regardless of the first differing
    /// byte (guaranteed by [`subtle::ConstantTimeEq`]).
    pub fn verify(&self, secret: &[u8]) -> bool {
        Sha256::digest(secret).ct_eq(&self.digest).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_matching_secret_returns_true() {
        let secret = b"my-api-secret";
        let hash = ApiKeyHash::of(secret);
        assert!(hash.verify(secret));
    }

    #[test]
    fn verify_non_matching_secret_returns_false() {
        let secret = b"correct-secret";
        let hash = ApiKeyHash::of(secret);
        assert!(!hash.verify(b"wrong-secret"));
    }

    #[test]
    fn verify_runs_to_completion_on_mismatch() {
        // Constant-time: both branches run to completion without panic.
        let hash = ApiKeyHash::of(b"secret");
        let result = hash.verify(b"not-the-secret");
        assert!(!result);
    }

    #[test]
    fn sha256_constructor_accepts_precomputed_digest() {
        let secret = b"test";
        let digest: [u8; 32] = sha2::Sha256::digest(secret).into();
        let hash = ApiKeyHash::sha256(digest);
        assert!(hash.verify(secret));
    }
}
