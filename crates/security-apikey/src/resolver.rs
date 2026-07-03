//! [`ApiKeyResolver`] trait, [`ApiKeyRecord`], and [`InMemoryApiKeyResolver`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use ego_security_sdk::Principal;

use crate::key_hash::ApiKeyHash;
use crate::key_id::ApiKeyId;

/// Error returned by a resolver backend (distinct from authentication errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiKeyResolverError {
    /// The resolver backend encountered an unexpected error.
    #[error("resolver backend error: {0}")]
    Backend(String),
}

/// Stored record for a registered API key.
#[derive(Clone)]
pub struct ApiKeyRecord {
    /// The principal this key authenticates.
    pub principal: Principal,
    /// Scopes granted to this key (opaque strings; not validated by the provider).
    pub scopes: Vec<String>,
    /// Optional expiry; `None` means the key never expires.
    pub expires_at: Option<SystemTime>,
    /// Arbitrary provider metadata (ignored by the provider). `Arc`-wrapped
    /// so resolvers sharing metadata across records (e.g. tenant/plan/owner
    /// fields common to many keys) pay one allocation, not one per record.
    pub metadata: Arc<HashMap<String, String>>,
    /// Stored hash used for constant-time verification.
    pub key_hash: ApiKeyHash,
}

/// Synchronous cache-first resolver.
///
/// # Contract (MUST, not just convention)
///
/// `lookup` MUST return from already-resident local state and MUST NOT
/// perform network or file I/O, database queries, or artificial delay of
/// any kind, on ANY path — including the not-found path. This is a hard
/// requirement for the type, not just cache friendliness: the caller,
/// [`crate::authenticator::ApiKeyAuthenticationProvider`], deliberately does
/// equal work (a hash-verify with a dummy digest) whether `lookup` returns
/// `Some` or `None`, specifically to prevent an attacker from distinguishing
/// "unknown key_id" from "known key_id, wrong secret" via response timing.
/// If `lookup` itself takes meaningfully different time for a hit vs. a
/// miss (e.g. a database round-trip, an HTTP call, a lock with contention),
/// that timing difference reopens the exact side-channel the provider was
/// built to close, regardless of how `key_hash.verify` behaves. The Rust
/// type system cannot enforce this — implementors MUST enforce it by
/// construction (in-memory maps, warmed local caches only; never a
/// pass-through to a remote store).
///
/// Object-safe and storable as `Arc<dyn ApiKeyResolver>`.
#[cfg_attr(test, mockall::automock)]
pub trait ApiKeyResolver: Send + Sync {
    /// Looks up the record for `key_id`.
    ///
    /// Returns `Ok(Some(record))` when found, `Ok(None)` when not found,
    /// and `Err(ApiKeyResolverError::Backend(_))` for backend failures.
    ///
    /// See the trait-level contract above — this MUST NOT perform I/O.
    fn lookup(&self, key_id: &ApiKeyId) -> Result<Option<Arc<ApiKeyRecord>>, ApiKeyResolverError>;
}

/// In-memory `HashMap`-backed resolver. No I/O, no persistence.
///
/// Reference implementation of [`ApiKeyResolver`].
pub struct InMemoryApiKeyResolver {
    store: HashMap<ApiKeyId, Arc<ApiKeyRecord>>,
}

impl InMemoryApiKeyResolver {
    /// Creates an empty resolver.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Registers `record` under `key_id`, replacing any existing entry.
    pub fn insert(&mut self, key_id: ApiKeyId, record: ApiKeyRecord) {
        self.store.insert(key_id, Arc::new(record));
    }
}

impl Default for InMemoryApiKeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyResolver for InMemoryApiKeyResolver {
    fn lookup(&self, key_id: &ApiKeyId) -> Result<Option<Arc<ApiKeyRecord>>, ApiKeyResolverError> {
        Ok(self.store.get(key_id).cloned())
    }
}

/// Type alias for `InMemoryApiKeyResolver` — the local (non-I/O) variant.
pub type LocalApiKeyResolver = InMemoryApiKeyResolver;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ego_security_sdk::principal::{PrincipalKind, SubjectId};

    use super::*;

    fn make_principal(subject: &str) -> Principal {
        Principal::new(PrincipalKind::User, SubjectId::new(subject).unwrap())
    }

    fn make_record(subject: &str, secret: &[u8]) -> ApiKeyRecord {
        ApiKeyRecord {
            principal: make_principal(subject),
            scopes: vec![],
            expires_at: None,
            metadata: Arc::new(HashMap::new()),
            key_hash: crate::key_hash::ApiKeyHash::of(secret),
        }
    }

    #[test]
    fn known_key_returns_some_record() {
        let mut resolver = InMemoryApiKeyResolver::new();
        let key_id = ApiKeyId::new("key-001").unwrap();
        let record = make_record("user:alice", b"secret");
        resolver.insert(key_id.clone(), record);

        let result = resolver.lookup(&key_id).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().principal.subject_id.as_str(), "user:alice");
    }

    #[test]
    fn unknown_key_returns_none() {
        let resolver = InMemoryApiKeyResolver::new();
        let key_id = ApiKeyId::new("missing-key").unwrap();
        let result = resolver.lookup(&key_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn dual_key_coexistence_for_same_principal() {
        let mut resolver = InMemoryApiKeyResolver::new();
        let key_a = ApiKeyId::new("key-a").unwrap();
        let key_b = ApiKeyId::new("key-b").unwrap();
        let record_a = make_record("user:bob", b"secret-a");
        let record_b = make_record("user:bob", b"secret-b");

        resolver.insert(key_a.clone(), record_a);
        resolver.insert(key_b.clone(), record_b);

        let a = resolver.lookup(&key_a).unwrap().unwrap();
        let b = resolver.lookup(&key_b).unwrap().unwrap();

        assert_eq!(a.principal.subject_id.as_str(), "user:bob");
        assert_eq!(b.principal.subject_id.as_str(), "user:bob");
        assert!(a.key_hash.verify(b"secret-a"));
        assert!(b.key_hash.verify(b"secret-b"));
        assert!(!a.key_hash.verify(b"secret-b"));
    }

    #[test]
    fn resolver_is_object_safe_behind_arc() {
        let resolver: Arc<dyn ApiKeyResolver> = Arc::new(InMemoryApiKeyResolver::new());
        let key_id = ApiKeyId::new("any").unwrap();
        let result = resolver.lookup(&key_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn mockall_automock_compiles() {
        // compile-time check: mockall can generate a mock for ApiKeyResolver
        let mut mock = MockApiKeyResolver::new();
        let key_id = ApiKeyId::new("mock-key").unwrap();
        mock.expect_lookup().returning(|_| Ok(None));
        let result = mock.lookup(&key_id).unwrap();
        assert!(result.is_none());
    }
}
