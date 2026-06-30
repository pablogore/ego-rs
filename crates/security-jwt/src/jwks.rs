//! JWKS key resolution — `JwksProvider` SPI, `HttpJwksProvider`, and `JwksKeyResolver`.
//!
//! `JwksKeyResolver` wraps a provider with an in-memory `RwLock<HashMap>` cache.
//! Hot path: read lock → cache hit → return clone (INV-7).
//! Cache miss: ONE forced refresh via RESOLVER_POOL, then re-read.
//! Background: `tokio::spawn` + interval task refreshes the cache on TTL.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ego_domain::auth::AuthenticationError;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, JwkSet};
use tracing::warn;

use crate::authenticator::resolver_pool;
use crate::config::JwtAlgorithm;
use crate::key_resolver::{KeyResolver, KeyResolverError, VerificationKey};

// ---------------------------------------------------------------------------
// JwksProvider SPI
// ---------------------------------------------------------------------------

/// Fetches and parses a JWKS from a given URI.
///
/// `pub` SPI — custom implementations (Vault, k8s secrets, files) are first-class.
#[async_trait]
pub trait JwksProvider: Send + Sync {
    /// Fetch the JWKS and return a list of `(kid, VerificationKey)` pairs.
    async fn fetch_jwks(
        &self,
        jwks_uri: &url::Url,
    ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError>;
}

// ---------------------------------------------------------------------------
// HttpJwksProvider
// ---------------------------------------------------------------------------

/// Default `JwksProvider` backed by `reqwest`.
pub struct HttpJwksProvider {
    client: reqwest::Client,
}

impl HttpJwksProvider {
    /// Create with the default reqwest client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .use_rustls_tls()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("failed to build reqwest client"),
        }
    }
}

impl Default for HttpJwksProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JwksProvider for HttpJwksProvider {
    async fn fetch_jwks(
        &self,
        jwks_uri: &url::Url,
    ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
        let resp = self
            .client
            .get(jwks_uri.as_str())
            .send()
            .await
            .map_err(|e| {
                AuthenticationError::ProviderUnavailable(format!("JWKS fetch error: {e}"))
            })?
            .error_for_status()
            .map_err(|e| {
                AuthenticationError::ProviderUnavailable(format!("JWKS endpoint returned error status: {e}"))
            })?;

        let jwk_set: JwkSet = resp.json().await.map_err(|e| {
            AuthenticationError::ProviderUnavailable(format!("JWKS parse error: {e}"))
        })?;

        let mut keys = Vec::new();
        for jwk in &jwk_set.keys {
            match jwk_to_verification_key(jwk) {
                Ok(vk) => keys.push((jwk.common.key_id.clone(), vk)),
                Err(e) => warn!("skipping unrecognised JWK: {e}"),
            }
        }
        Ok(keys)
    }
}

// ---------------------------------------------------------------------------
// JWK → VerificationKey conversion
// ---------------------------------------------------------------------------

fn jwk_to_verification_key(
    jwk: &jsonwebtoken::jwk::Jwk,
) -> Result<VerificationKey, AuthenticationError> {
    match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => {
            let pem = rsa_components_to_pem(&rsa.n, &rsa.e)?;
            Ok(VerificationKey::RsaPem(pem))
        }
        AlgorithmParameters::EllipticCurve(ec) => {
            // Only P-256 is supported; P-384/P-521 need different coordinate padding and OID
            if ec.curve != EllipticCurve::P256 {
                return Err(AuthenticationError::AlgorithmNotSupported(format!(
                    "EC curve {:?} is not supported; only P-256 (ES256)",
                    ec.curve
                )));
            }
            let pem = ec_components_to_pem(&ec.x, &ec.y)?;
            Ok(VerificationKey::EcPem(pem))
        }
        _ => Err(AuthenticationError::AlgorithmNotSupported(
            "unsupported JWK algorithm type".to_string(),
        )),
    }
}

// Convert base64url-encoded RSA n/e components to a PEM public key.
fn rsa_components_to_pem(n_b64: &str, e_b64: &str) -> Result<String, AuthenticationError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let n = URL_SAFE_NO_PAD
        .decode(n_b64)
        .map_err(|e| AuthenticationError::ProviderUnavailable(format!("bad RSA n: {e}")))?;
    let e = URL_SAFE_NO_PAD
        .decode(e_b64)
        .map_err(|e| AuthenticationError::ProviderUnavailable(format!("bad RSA e: {e}")))?;

    // M-2: X.690 §8.3.1 — INTEGER must have at least one content octet.
    if n.is_empty() || e.is_empty() {
        return Err(AuthenticationError::InvalidToken(
            "RSA key has zero-length modulus or exponent".into(),
        ));
    }

    // Build DER-encoded SubjectPublicKeyInfo for an RSA public key.
    // Structure per RFC 5280 §4.1 and RFC 3447 Appendix C:
    //   SubjectPublicKeyInfo ::= SEQUENCE {
    //     algorithm AlgorithmIdentifier,  -- SEQUENCE { OID rsaEncryption (1.2.840.113549.1.1.1), NULL }
    //     subjectPublicKey BIT STRING      -- DER of RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }
    //   }
    let mut inner_content = Vec::new();
    inner_content.extend(der_integer(&n));
    inner_content.extend(der_integer(&e));
    let inner_seq = der_sequence(&inner_content);

    let mut alg_content = Vec::new();
    // OID 1.2.840.113549.1.1.1 (rsaEncryption) — RFC 3447 Appendix C
    alg_content.extend_from_slice(&[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]);
    // NULL parameters — required by RFC 3447 §3.1
    alg_content.extend_from_slice(&[0x05, 0x00]);
    let alg_seq = der_sequence(&alg_content);

    let mut spki_content = Vec::new();
    spki_content.extend(alg_seq);
    spki_content.extend(der_bitstring(&inner_seq));
    let spki = der_sequence(&spki_content);

    Ok(der_to_pem(&spki))
}

// Convert base64url EC x/y coordinates to a PEM public key (P-256).
fn ec_components_to_pem(x_b64: &str, y_b64: &str) -> Result<String, AuthenticationError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let x = URL_SAFE_NO_PAD
        .decode(x_b64)
        .map_err(|e| AuthenticationError::ProviderUnavailable(format!("bad EC x: {e}")))?;
    let y = URL_SAFE_NO_PAD
        .decode(y_b64)
        .map_err(|e| AuthenticationError::ProviderUnavailable(format!("bad EC y: {e}")))?;

    if x.is_empty() || y.is_empty() || x.len() > 32 || y.len() > 32 {
        return Err(AuthenticationError::ProviderUnavailable(
            "EC P-256 coordinate must be 1–32 bytes — malformed JWK".to_string(),
        ));
    }

    // Uncompressed EC point: 0x04 || x || y
    let mut point = vec![0x04u8];
    // Pad to 32 bytes each for P-256
    let pad_x = 32usize.saturating_sub(x.len());
    let mut xp = vec![0u8; pad_x];
    xp.extend_from_slice(&x);
    let pad_y = 32usize.saturating_sub(y.len());
    let mut yp = vec![0u8; pad_y];
    yp.extend_from_slice(&y);
    point.extend_from_slice(&xp);
    point.extend_from_slice(&yp);

    // Build DER-encoded SubjectPublicKeyInfo for an EC P-256 public key.
    // Structure per RFC 5480 §2:
    //   SubjectPublicKeyInfo ::= SEQUENCE {
    //     algorithm AlgorithmIdentifier,  -- SEQUENCE { OID id-ecPublicKey, OID prime256v1 }
    //     subjectPublicKey BIT STRING      -- uncompressed point: 0x04 || x || y (RFC 5480 §2.2)
    //   }
    let mut ec_alg_content = Vec::new();
    // OID 1.2.840.10045.2.1 (id-ecPublicKey) — RFC 5480 §2.1.1
    ec_alg_content.extend_from_slice(&[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    // OID 1.2.840.10045.3.1.7 (prime256v1 / P-256) — RFC 5480 §2.1.1.1
    ec_alg_content.extend_from_slice(&[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]);
    let algorithm = der_sequence(&ec_alg_content);

    let mut spki_content = Vec::new();
    spki_content.extend(algorithm);
    spki_content.extend(der_bitstring(&point));
    let spki = der_sequence(&spki_content);

    Ok(der_to_pem(&spki))
}

// Wrap raw DER bytes as a PEM public key block (PKIX SubjectPublicKeyInfo).
fn der_to_pem(der: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = STANDARD.encode(der);
    let lines: Vec<&str> = b64
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).expect("base64 output is always ASCII"))
        .collect();
    format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        lines.join("\n")
    )
}

// Minimal DER helpers — X.690 §8 (BER/DER encoding rules)
fn der_length(n: usize) -> Vec<u8> {
    if n < 128 {
        vec![n as u8]
    } else if n < 256 {
        vec![0x81, n as u8]
    } else {
        vec![0x82, (n >> 8) as u8, (n & 0xff) as u8]
    }
}

fn der_sequence(content: &[u8]) -> Vec<u8> {
    let mut v = vec![0x30];
    v.extend(der_length(content.len()));
    v.extend_from_slice(content);
    v
}

fn der_bitstring(content: &[u8]) -> Vec<u8> {
    let mut v = vec![0x03];
    let inner_len = content.len() + 1; // +1 for unused-bits byte
    v.extend(der_length(inner_len));
    v.push(0x00); // 0 unused bits
    v.extend_from_slice(content);
    v
}

fn der_integer(bytes: &[u8]) -> Vec<u8> {
    // X.690 §8.3: INTEGER — prepend 0x00 if MSB is set to signal a positive value.
    // X.690 §8.3.1: INTEGER must have at least one content octet — callers must
    // validate non-empty before calling (rsa_components_to_pem and ec guard above).
    debug_assert!(!bytes.is_empty(), "der_integer: empty input produces invalid DER 02 00");
    let needs_pad = bytes.first().map_or(false, |b| b & 0x80 != 0);
    let mut content: Vec<u8> = if needs_pad { vec![0x00] } else { vec![] };
    content.extend_from_slice(bytes);
    let mut v = vec![0x02];
    v.extend(der_length(content.len()));
    v.extend(content);
    v
}

/// Debounce window for forced JWKS refreshes on cache miss (H-4).
/// A second miss within this window is rejected immediately to prevent
/// attacker-triggered JWKS fetch floods with novel kid values.
const FORCE_REFRESH_DEBOUNCE_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// JwksKeyResolver helpers
// ---------------------------------------------------------------------------

/// Maps a `VerificationKey` + requested `JwtAlgorithm` to a result, returning
/// `AlgorithmMismatch` when the key type does not match the algorithm.
fn vk_to_result(vk: VerificationKey, algorithm: JwtAlgorithm) -> Result<VerificationKey, KeyResolverError> {
    match (&vk, algorithm) {
        (VerificationKey::Hmac(_), JwtAlgorithm::Hs256)
        | (VerificationKey::RsaPem(_), JwtAlgorithm::Rs256)
        | (VerificationKey::EcPem(_), JwtAlgorithm::Es256) => Ok(vk),
        (VerificationKey::Hmac(_), alg) => Err(KeyResolverError::AlgorithmMismatch {
            expected: JwtAlgorithm::Hs256,
            requested: alg,
        }),
        (VerificationKey::RsaPem(_), alg) => Err(KeyResolverError::AlgorithmMismatch {
            expected: JwtAlgorithm::Rs256,
            requested: alg,
        }),
        (VerificationKey::EcPem(_), alg) => Err(KeyResolverError::AlgorithmMismatch {
            expected: JwtAlgorithm::Es256,
            requested: alg,
        }),
    }
}

// ---------------------------------------------------------------------------
// JwksKeyResolver
// ---------------------------------------------------------------------------

// ponytail: std::sync::RwLock — no async contention on the cache, INV-7 holds.
// The write lock is held only during cache replacement (warm-up, forced refresh,
// background refresh). The read lock is held only during lookup — never on write.
type Cache = Arc<RwLock<HashMap<Option<String>, VerificationKey>>>;

/// Resolves verification keys from a JWKS endpoint with in-memory `RwLock` cache.
///
/// Implements `KeyResolver` — plugs into the existing `authenticate_inner` path.
pub struct JwksKeyResolver {
    cache: Cache,
    jwks_uri: url::Url,
    provider: Arc<dyn JwksProvider>,
    // ponytail: std::sync::Mutex — debounce guard, never held across await, no async needed.
    last_force_refresh: Mutex<Instant>,
}

impl JwksKeyResolver {
    /// Creates a resolver with the default `HttpJwksProvider`.
    ///
    /// Performs a synchronous warm-up fetch via `RESOLVER_POOL` at construction
    /// (cache-first contract — cache is populated before first `authenticate`).
    pub fn new(jwks_uri: url::Url, cache_ttl: Duration) -> Self {
        Self::with_provider(jwks_uri, cache_ttl, Arc::new(HttpJwksProvider::new()))
    }

    /// Creates a resolver with a custom `JwksProvider`.
    ///
    /// Use `FakeJwks` in tests.
    pub fn with_provider(
        jwks_uri: url::Url,
        cache_ttl: Duration,
        provider: Arc<dyn JwksProvider>,
    ) -> Self {
        let cache: Cache = Arc::new(RwLock::new(HashMap::new()));

        // Warm-up: synchronously populate the cache at construction time.
        let provider_ref = Arc::clone(&provider);
        let uri_ref = jwks_uri.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        resolver_pool().spawn_ok(async move {
            let result = provider_ref.fetch_jwks(&uri_ref).await;
            let _ = tx.send(result);
        });
        match rx.recv() {
            Ok(Ok(keys)) if !keys.is_empty() => {
                let mut guard = cache.write().expect("jwks cache poisoned");
                *guard = keys.into_iter().collect();
            }
            Ok(Ok(_)) => warn!("JWKS warm-up returned 0 keys — cache starts empty"),
            Ok(Err(e)) => warn!("JWKS warm-up failed: {e} — cache starts empty, will retry on first auth"),
            Err(_) => warn!("JWKS warm-up: pool dropped sender — cache starts empty, will retry on first auth"),
        }

        // Background refresh task — only when a Tokio runtime is available.
        // In unit tests backed by futures_executor, no runtime exists so we skip.
        let cache_bg = Arc::clone(&cache);
        let provider_bg = Arc::clone(&provider);
        let uri_bg = jwks_uri.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut interval = tokio::time::interval(cache_ttl);
                interval.tick().await; // skip immediate tick (already warm)
                loop {
                    interval.tick().await;
                    match provider_bg.fetch_jwks(&uri_bg).await {
                        Ok(keys) => {
                            // Write lock only during cache replacement (INV-7)
                            if keys.is_empty() {
                                warn!("JWKS fetch returned 0 keys — retaining stale cache to avoid service disruption");
                            } else if let Ok(mut guard) = cache_bg.write() {
                                *guard = keys.into_iter().collect();
                            }
                        }
                        Err(e) => {
                            warn!("JWKS background refresh failed (stale cache retained): {e}");
                        }
                    }
                }
            });
        }

        // Subtract DEBOUNCE+1s so the first cache miss always triggers a refresh.
        let last_force_refresh =
            Mutex::new(Instant::now() - Duration::from_secs(FORCE_REFRESH_DEBOUNCE_SECS + 1));

        Self { cache, jwks_uri, provider, last_force_refresh }
    }

    /// Force a synchronous cache refresh via `RESOLVER_POOL`.
    ///
    /// Updates `last_force_refresh` ONLY on a successful fetch that produces keys,
    /// so a failed refresh does not block retries for 30 s.
    fn force_refresh(&self) {
        let provider_ref = Arc::clone(&self.provider);
        let uri_ref = self.jwks_uri.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        resolver_pool().spawn_ok(async move {
            let result = provider_ref.fetch_jwks(&uri_ref).await;
            let _ = tx.send(result);
        });
        match rx.recv() {
            Ok(Ok(keys)) if !keys.is_empty() => {
                // Write lock only during cache replacement (INV-7)
                if let Ok(mut guard) = self.cache.write() {
                    *guard = keys.into_iter().collect();
                }
                // Update debounce timer only on success — failed fetches must not
                // block retries for the full 30 s window.
                *self.last_force_refresh.lock().expect("debounce mutex poisoned") = Instant::now();
            }
            Ok(Ok(_)) => warn!("JWKS force_refresh returned 0 keys — retaining stale cache"),
            Ok(Err(e)) => warn!("JWKS forced refresh failed: {e} — retaining stale cache"),
            Err(_) => warn!("JWKS forced refresh: pool dropped sender — retaining stale cache"),
        }
    }
}

#[async_trait]
impl KeyResolver for JwksKeyResolver {
    /// Resolve a verification key by `kid`.
    ///
    /// Hot path: read lock (INV-7). Cache miss: one forced refresh, then re-read.
    async fn resolve(
        &self,
        kid: Option<&str>,
        algorithm: JwtAlgorithm,
    ) -> Result<VerificationKey, KeyResolverError> {
        let key = kid.map(str::to_string);

        // Hot-path: read lock (INV-7 — no write on the hot path)
        {
            let guard = self.cache.read().expect("jwks cache poisoned");
            if let Some(vk) = guard.get(&key) {
                return vk_to_result(vk.clone(), algorithm);
            }
        }

        // Cache miss — debounce: skip force_refresh if one ran within the last 30 s.
        // Prevents N concurrent JWKS fetches when an attacker floods with novel kid values.
        // NOTE: last_force_refresh is updated ONLY on successful refresh (inside force_refresh).
        let should_refresh = {
            let last = self.last_force_refresh.lock().expect("debounce mutex poisoned");
            last.elapsed() >= Duration::from_secs(FORCE_REFRESH_DEBOUNCE_SECS)
        };
        if !should_refresh {
            return Err(KeyResolverError::KeyNotFound { kid: key });
        }
        self.force_refresh();

        {
            let guard = self.cache.read().expect("jwks cache poisoned");
            match guard.get(&key).cloned() {
                Some(vk) => vk_to_result(vk, algorithm),
                None => Err(KeyResolverError::KeyNotFound { kid: key }),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Minimal in-process JwksProvider backed by a static key list.
    struct FakeJwks {
        keys: Vec<(Option<String>, VerificationKey)>,
        call_count: Arc<AtomicUsize>,
    }

    impl FakeJwks {
        fn new(keys: Vec<(Option<String>, VerificationKey)>) -> Self {
            Self { keys, call_count: Arc::new(AtomicUsize::new(0)) }
        }
    }

    #[async_trait]
    impl JwksProvider for FakeJwks {
        async fn fetch_jwks(
            &self,
            _: &url::Url,
        ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(self.keys.clone())
        }
    }



    #[test]
    fn force_refresh_debounce_skips_second_miss_within_window() {
        // FakeJwks returns one key (not the ones being resolved) so force_refresh
        // succeeds and updates last_force_refresh, which is the condition that arms
        // the debounce for the second miss.
        let provider = Arc::new(FakeJwks::new(vec![fake_key("kid-sentinel")]));
        let call_count = Arc::clone(&provider.call_count);

        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );

        // Snapshot after warm-up (1 call).
        let after_warmup = call_count.load(std::sync::atomic::Ordering::SeqCst);

        // First miss: debounce window expired (initialized 31 s ago) → refresh fires.
        let _ = futures_executor::block_on(resolver.resolve(Some("unknown-1"), JwtAlgorithm::Hs256));
        let after_first_miss = call_count.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_first_miss, after_warmup + 1, "first miss should trigger exactly one refresh");

        // Second miss immediately after: still within 30 s window → no refresh.
        let _ = futures_executor::block_on(resolver.resolve(Some("unknown-2"), JwtAlgorithm::Hs256));
        let after_second_miss = call_count.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(after_second_miss, after_first_miss, "second miss within debounce window must not trigger refresh");
    }

    fn fake_key(kid: &str) -> (Option<String>, VerificationKey) {
        (Some(kid.to_string()), VerificationKey::Hmac(vec![kid.as_bytes()[0]]))
    }

    fn make_resolver(keys: Vec<(Option<String>, VerificationKey)>) -> JwksKeyResolver {
        let provider = Arc::new(FakeJwks::new(keys));
        JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        )
    }

    #[test]
    fn cache_hit_returns_key_without_extra_fetch() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&counter);
        struct CountingJwks {
            count: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl JwksProvider for CountingJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Ok(vec![(Some("kid-A".into()), VerificationKey::Hmac(vec![1]))])
            }
        }
        let provider = Arc::new(CountingJwks { count: counter_ref });
        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );
        // Warm-up counted as 1
        let calls_after_warmup = counter.load(Ordering::SeqCst);
        // Cache hit — no additional fetch
        let _ = futures_executor::block_on(resolver.resolve(Some("kid-A"), JwtAlgorithm::Hs256));
        assert_eq!(counter.load(Ordering::SeqCst), calls_after_warmup, "cache hit should not trigger fetch");
    }

    #[test]
    fn cache_miss_triggers_exactly_one_refresh() {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&counter);
        struct CountingJwks {
            count: Arc<AtomicUsize>,
        }
        #[async_trait]
        impl JwksProvider for CountingJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Ok(vec![(Some("kid-B".into()), VerificationKey::Hmac(vec![2]))])
            }
        }
        let provider = Arc::new(CountingJwks { count: counter_ref });
        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );
        let calls_after_warmup = counter.load(Ordering::SeqCst);
        // Cache miss for kid-X — triggers one refresh
        let _ = futures_executor::block_on(resolver.resolve(Some("kid-X"), JwtAlgorithm::Hs256));
        assert_eq!(counter.load(Ordering::SeqCst), calls_after_warmup + 1);
    }

    #[test]
    fn stale_cache_retained_when_refresh_fails() {
        use std::sync::atomic::AtomicBool;

        // A provider that succeeds on warm-up, then fails on every subsequent fetch.
        struct FailAfterFirstJwks {
            fetched: AtomicBool,
            key: (Option<String>, VerificationKey),
        }
        #[async_trait]
        impl JwksProvider for FailAfterFirstJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                if self.fetched.swap(true, Ordering::SeqCst) {
                    Err(AuthenticationError::ProviderUnavailable(
                        "simulated JWKS fetch failure".into(),
                    ))
                } else {
                    Ok(vec![self.key.clone()])
                }
            }
        }

        let provider = Arc::new(FailAfterFirstJwks {
            fetched: AtomicBool::new(false),
            key: fake_key("kid-A"),
        });
        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );

        // Warm-up succeeds: kid-A is now in cache
        let vk = futures_executor::block_on(resolver.resolve(Some("kid-A"), JwtAlgorithm::Hs256));
        assert!(vk.is_ok(), "warm-up should populate the cache");

        // force_refresh calls the provider again — it now returns Err; cache must survive
        resolver.force_refresh();

        let vk2 = futures_executor::block_on(resolver.resolve(Some("kid-A"), JwtAlgorithm::Hs256));
        assert!(vk2.is_ok(), "stale cache must be retained when force_refresh fails");
    }

    #[test]
    fn force_refresh_evicts_removed_keys() {
        // First load: [A, B]. Then force_refresh with provider returning [B, C].
        // After refresh, cache must be exactly {B, C} — A must be gone.
        use std::sync::Mutex;

        struct RotatingJwks {
            rounds: Mutex<Vec<Vec<(Option<String>, VerificationKey)>>>,
        }
        #[async_trait]
        impl JwksProvider for RotatingJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                let mut rounds = self.rounds.lock().unwrap();
                if rounds.is_empty() {
                    Ok(vec![])
                } else {
                    Ok(rounds.remove(0))
                }
            }
        }

        let provider = Arc::new(RotatingJwks {
            rounds: Mutex::new(vec![
                // warm-up: A + B
                vec![fake_key("kid-A"), fake_key("kid-B")],
                // first force_refresh: B + C (A removed)
                vec![fake_key("kid-B"), fake_key("kid-C")],
            ]),
        });

        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );

        // Confirm warm-up: A present
        assert!(
            futures_executor::block_on(resolver.resolve(Some("kid-A"), JwtAlgorithm::Hs256)).is_ok()
        );

        // Force refresh: now provider returns [B, C]
        resolver.force_refresh();

        // B must be present
        assert!(
            futures_executor::block_on(resolver.resolve(Some("kid-B"), JwtAlgorithm::Hs256)).is_ok()
        );
        // C must be present
        assert!(
            futures_executor::block_on(resolver.resolve(Some("kid-C"), JwtAlgorithm::Hs256)).is_ok()
        );
        // A must be evicted — resolve triggers another refresh (3rd round = empty), still no A
        let result = futures_executor::block_on(resolver.resolve(Some("kid-A"), JwtAlgorithm::Hs256));
        assert!(result.is_err(), "kid-A must be evicted after key rotation");
    }

    #[test]
    fn concurrent_reads_succeed() {
        let resolver = Arc::new(make_resolver(vec![fake_key("kid-A")]));
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let r = Arc::clone(&resolver);
                std::thread::spawn(move || {
                    futures_executor::block_on(r.resolve(Some("kid-A"), JwtAlgorithm::Hs256))
                })
            })
            .collect();
        for h in handles {
            assert!(h.join().unwrap().is_ok());
        }
    }

    // HIGH-3: concurrent cache-miss under RESOLVER_POOL must not starve or deadlock.
    // Spawns more than the pool size (4) concurrent misses and verifies all complete.
    #[test]
    fn concurrent_cache_miss_with_more_than_pool_size_threads_completes() {
        // FakeJwks returns kid-sentinel, so every miss triggers force_refresh which
        // posts a task to the pool. With 8 threads (> pool size of 4) all must complete.
        let resolver = Arc::new(make_resolver(vec![fake_key("kid-sentinel")]));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let r = Arc::clone(&resolver);
                std::thread::spawn(move || {
                    // Each thread resolves a unique kid — guaranteed miss after debounce resets.
                    // We ignore the result; the test asserts no deadlock (join completes).
                    let _ = futures_executor::block_on(
                        r.resolve(Some(&format!("miss-{i}")), JwtAlgorithm::Hs256),
                    );
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked — possible deadlock or pool starvation");
        }
    }

    #[test]
    fn algorithm_mismatch_hmac_key_with_rs256_cache_hit() {
        // Cache warm-up loads an HMAC key; resolve with Rs256 must return AlgorithmMismatch.
        let resolver = make_resolver(vec![fake_key("kid-hmac")]);
        let result = futures_executor::block_on(resolver.resolve(Some("kid-hmac"), JwtAlgorithm::Rs256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Hs256);
                assert_eq!(requested, JwtAlgorithm::Rs256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }

    #[test]
    fn algorithm_mismatch_hmac_key_with_rs256_post_refresh() {
        // Warm-up returns empty → cache miss → force_refresh loads HMAC key → AlgorithmMismatch.
        use std::sync::Mutex;

        struct TwoRoundJwks {
            rounds: Mutex<Vec<Vec<(Option<String>, VerificationKey)>>>,
        }
        #[async_trait]
        impl JwksProvider for TwoRoundJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                let mut rounds = self.rounds.lock().unwrap();
                if rounds.is_empty() { Ok(vec![]) } else { Ok(rounds.remove(0)) }
            }
        }

        let provider = Arc::new(TwoRoundJwks {
            rounds: Mutex::new(vec![
                vec![],                    // warm-up → empty, cache stays empty
                vec![fake_key("kid-hmac")], // force_refresh → HMAC key lands in cache
            ]),
        });

        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );

        // kid-hmac not in cache yet (warm-up was empty) → triggers force_refresh
        let result = futures_executor::block_on(resolver.resolve(Some("kid-hmac"), JwtAlgorithm::Rs256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Hs256);
                assert_eq!(requested, JwtAlgorithm::Rs256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }

    // CRITICAL-1: vk_to_result — all missing mismatch directions

    #[test]
    fn algorithm_mismatch_rsa_key_with_es256() {
        let resolver = make_resolver(vec![(
            Some("kid-rsa".into()),
            VerificationKey::RsaPem("fake-pem".into()),
        )]);
        let result = futures_executor::block_on(resolver.resolve(Some("kid-rsa"), JwtAlgorithm::Es256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Rs256);
                assert_eq!(requested, JwtAlgorithm::Es256);
            }
            other => panic!("expected AlgorithmMismatch(Rs256→Es256), got {other:?}"),
        }
    }

    #[test]
    fn algorithm_mismatch_ec_key_with_rs256() {
        let resolver = make_resolver(vec![(
            Some("kid-ec".into()),
            VerificationKey::EcPem("fake-ec-pem".into()),
        )]);
        let result = futures_executor::block_on(resolver.resolve(Some("kid-ec"), JwtAlgorithm::Rs256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Es256);
                assert_eq!(requested, JwtAlgorithm::Rs256);
            }
            other => panic!("expected AlgorithmMismatch(Es256→Rs256), got {other:?}"),
        }
    }

    #[test]
    fn algorithm_mismatch_hmac_key_with_es256() {
        let resolver = make_resolver(vec![fake_key("kid-hmac")]);
        let result = futures_executor::block_on(resolver.resolve(Some("kid-hmac"), JwtAlgorithm::Es256));
        match result {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Hs256);
                assert_eq!(requested, JwtAlgorithm::Es256);
            }
            other => panic!("expected AlgorithmMismatch(Hs256→Es256), got {other:?}"),
        }
    }

    // CRITICAL-4: zero-length EC coordinates must be rejected as ProviderUnavailable

    #[test]
    fn zero_length_ec_x_coordinate_is_rejected() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, CommonParameters, EllipticCurve, EllipticCurveKeyParameters,
            EllipticCurveKeyType, Jwk,
        };
        // x="" decodes to 0 bytes — not a valid P-256 x coordinate
        let jwk = Jwk {
            common: CommonParameters::default(),
            algorithm: AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
                key_type: EllipticCurveKeyType::EC,
                curve: EllipticCurve::P256,
                x: "".to_string(), // empty base64url → 0 bytes
                y: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            }),
        };
        let result = jwk_to_verification_key(&jwk);
        assert!(
            matches!(result, Err(AuthenticationError::ProviderUnavailable(_))),
            "zero-length x must be ProviderUnavailable, got {result:?}"
        );
    }

    #[test]
    fn zero_length_ec_y_coordinate_is_rejected() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, CommonParameters, EllipticCurve, EllipticCurveKeyParameters,
            EllipticCurveKeyType, Jwk,
        };
        let jwk = Jwk {
            common: CommonParameters::default(),
            algorithm: AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
                key_type: EllipticCurveKeyType::EC,
                curve: EllipticCurve::P256,
                x: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                y: "".to_string(), // empty base64url → 0 bytes
            }),
        };
        let result = jwk_to_verification_key(&jwk);
        assert!(
            matches!(result, Err(AuthenticationError::ProviderUnavailable(_))),
            "zero-length y must be ProviderUnavailable, got {result:?}"
        );
    }

    // WARNING-1: force_refresh_evicts_removed_keys — verify A is absent from cache directly
    // (not via a third resolve that may or may not trigger a refresh depending on debounce)

    #[test]
    fn cache_does_not_contain_evicted_key_after_force_refresh() {
        // After force_refresh replaces [A,B] with [B,C], A must not be in the cache.
        use std::sync::Mutex;
        struct TwoRoundJwks {
            rounds: Mutex<Vec<Vec<(Option<String>, VerificationKey)>>>,
        }
        #[async_trait]
        impl JwksProvider for TwoRoundJwks {
            async fn fetch_jwks(
                &self,
                _: &url::Url,
            ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
                let mut rounds = self.rounds.lock().unwrap();
                if rounds.is_empty() { Ok(vec![]) } else { Ok(rounds.remove(0)) }
            }
        }
        let provider = Arc::new(TwoRoundJwks {
            rounds: Mutex::new(vec![
                vec![fake_key("kid-A"), fake_key("kid-B")],
                vec![fake_key("kid-B"), fake_key("kid-C")],
            ]),
        });
        let resolver = JwksKeyResolver::with_provider(
            url::Url::parse("https://fake.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            provider,
        );
        resolver.force_refresh(); // load [B, C]; A is gone from cache
        // Read the cache directly — no resolve() to avoid triggering a third refresh
        let guard = resolver.cache.read().unwrap();
        let has_a = guard.contains_key(&Some("kid-A".to_string()));
        let has_b = guard.contains_key(&Some("kid-B".to_string()));
        let has_c = guard.contains_key(&Some("kid-C".to_string()));
        drop(guard);
        assert!(!has_a, "kid-A must be absent after key rotation");
        assert!(has_b, "kid-B must be present");
        assert!(has_c, "kid-C must be present");
    }

    #[test]
    fn non_p256_curve_returns_algorithm_not_supported() {
        use jsonwebtoken::jwk::{
            AlgorithmParameters, CommonParameters, EllipticCurve, EllipticCurveKeyParameters,
            EllipticCurveKeyType, Jwk,
        };

        // Craft a P-384 EC JWK — coordinates are valid base64url but curve is unsupported.
        let jwk = Jwk {
            common: CommonParameters::default(),
            algorithm: AlgorithmParameters::EllipticCurve(EllipticCurveKeyParameters {
                key_type: EllipticCurveKeyType::EC,
                curve: EllipticCurve::P384,
                x: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
                y: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            }),
        };

        let result = jwk_to_verification_key(&jwk);
        assert!(result.is_err());
        match result.unwrap_err() {
            AuthenticationError::AlgorithmNotSupported(msg) => {
                assert!(
                    msg.contains("P384") || msg.contains("P-384"),
                    "expected P-384 in error: {msg}"
                );
            }
            other => panic!("expected AlgorithmNotSupported, got {other:?}"),
        }
    }

    // C-3: direct vk_to_result unit tests — all mismatch combinations.

    #[test]
    fn vk_to_result_rsa_key_with_es256_returns_mismatch() {
        let vk = VerificationKey::RsaPem("fake-rsa-pem".into());
        match vk_to_result(vk, JwtAlgorithm::Es256) {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Rs256);
                assert_eq!(requested, JwtAlgorithm::Es256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }

    #[test]
    fn vk_to_result_ec_key_with_rs256_returns_mismatch() {
        let vk = VerificationKey::EcPem("fake-ec-pem".into());
        match vk_to_result(vk, JwtAlgorithm::Rs256) {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Es256);
                assert_eq!(requested, JwtAlgorithm::Rs256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }

    #[test]
    fn vk_to_result_hmac_key_with_es256_returns_mismatch() {
        let vk = VerificationKey::Hmac(vec![0u8; 32]);
        match vk_to_result(vk, JwtAlgorithm::Es256) {
            Err(KeyResolverError::AlgorithmMismatch { expected, requested }) => {
                assert_eq!(expected, JwtAlgorithm::Hs256);
                assert_eq!(requested, JwtAlgorithm::Es256);
            }
            other => panic!("expected AlgorithmMismatch, got {other:?}"),
        }
    }
    // M-2: zero-length RSA modulus or exponent must be rejected with InvalidToken.
    // Guards X.690 §8.3.1: INTEGER must have at least one content octet.
    #[test]
    fn rsa_empty_modulus_returns_invalid_token() {
        // base64url("") → empty string
        let result = super::rsa_components_to_pem("", "AQAB");
        assert!(
            matches!(result, Err(AuthenticationError::InvalidToken(_))),
            "empty RSA modulus must return InvalidToken, got: {result:?}"
        );
    }

    #[test]
    fn rsa_empty_exponent_returns_invalid_token() {
        // Valid 2048-bit n (first few bytes), empty e
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";
        let result = super::rsa_components_to_pem(n, "");
        assert!(
            matches!(result, Err(AuthenticationError::InvalidToken(_))),
            "empty RSA exponent must return InvalidToken, got: {result:?}"
        );
    }

}
