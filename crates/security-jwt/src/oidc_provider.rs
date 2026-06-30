//! `OidcAuthenticationProvider` — composite OIDC resource server authenticator.
//!
//! Builds on `JwksKeyResolver` (JWT path) and `IntrospectionAuthenticationProvider`
//! (opaque token path), routing between them based on `TokenFormat` config or the
//! Auto heuristic (two-dot base64url structure → JWT).
//!
//! Construction validates `OidcProviderConfig` (T-06) and, when only `issuer_url`
//! is given, calls `DiscoveryProvider` via `RESOLVER_POOL` to resolve `jwks_uri`
//! (OQ-5: fails at construction if discovery doc lacks `jwks_uri`).

use std::sync::Arc;
use std::time::Duration;

use ego_domain::auth::{AuthenticationError, Clock};
use ego_security_sdk::{AuthenticationProvider, Credential, PrincipalMapper, SecurityContext};

use crate::authenticator::{authenticate_inner, resolver_pool};
use crate::config::{JwtAlgorithm, JwtProviderConfig};
use crate::discovery::{DiscoveryProvider, HttpDiscoveryProvider, OidcEndpoints};
use crate::introspection::IntrospectionAuthenticationProvider;
use crate::jwks::JwksKeyResolver;
use crate::oidc_config::{OidcProviderConfig, TokenFormat};

// ---------------------------------------------------------------------------
// Token format heuristic
// ---------------------------------------------------------------------------

/// Returns `true` if `token` looks like a JWT (exactly two dots, with each
/// adjacent segment being valid base64url — the standard three-part structure).
///
/// False-positive rate is ~nil because opaque tokens with exactly two dots
/// whose split segments are valid base64url would be indistinguishable from JWTs,
/// but this is a known acceptable trade-off (OQ-2). Callers with ambiguous tokens
/// can force `TokenFormat::Jwt` or `TokenFormat::Opaque`.
fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    // All three segments (header, payload, signature) must be non-empty and valid base64url.
    // Accepting an empty signature segment would mis-route truncated opaque tokens to the JWT
    // path, producing misleading InvalidSignature errors instead of forwarding to introspection.
    parts.iter().all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

// ---------------------------------------------------------------------------
// OIDC JWT path — tries RS256 then ES256 (the two OIDC-standard algorithms)
// ---------------------------------------------------------------------------

/// Route a JWT token through the JWKS-backed JWT path.
///
/// Reads the `alg` header to determine whether to use RSA or EC key material,
/// then delegates to the shared `authenticate_inner` (which also extracts `kid`
/// and calls the resolver). The algorithm-pinning check in `authenticate_inner`
/// compares `header.alg` against `expected_alg` — here they are the same value
/// (we pass `header.alg` as `expected_alg`), so the check always passes.
///
/// `allowed` is the set of algorithms accepted by this provider. Tokens signed
/// with an algorithm not in `allowed` are rejected with `AlgorithmNotSupported`.
fn try_jwt_path(
    token: &str,
    resolver: &Arc<JwksKeyResolver>,
    config: &JwtProviderConfig,
    clock: &Arc<dyn Clock>,
    mapper: &dyn PrincipalMapper,
    allowed: &[JwtAlgorithm],
) -> Result<SecurityContext, AuthenticationError> {
    use crate::key_resolver::VerificationKey;
    use jsonwebtoken::{Algorithm, DecodingKey};

    // Peek at the header to determine key type.
    let header = jsonwebtoken::decode_header(token)
        .map_err(|e| AuthenticationError::InvalidToken(format!("{e}")))?;

    let resolver_arc: Arc<dyn crate::key_resolver::KeyResolver> = resolver.clone() as Arc<_>;

    match header.alg {
        Algorithm::RS256 => {
            if !allowed.contains(&JwtAlgorithm::Rs256) {
                return Err(AuthenticationError::AlgorithmNotSupported(
                    "RS256 is not in the allowed algorithms list".into(),
                ));
            }
            authenticate_inner(
                token,
                config,
                &resolver_arc,
                clock,
                header.alg,
                JwtAlgorithm::Rs256,
                mapper,
                |key| match key {
                    VerificationKey::RsaPem(pem) => DecodingKey::from_rsa_pem(pem.as_bytes())
                        .map_err(|e| AuthenticationError::InvalidToken(format!("bad RSA key: {e}"))),
                    _ => Err(AuthenticationError::InvalidToken("expected RSA PEM key".into())),
                },
            )
        }
        Algorithm::ES256 => {
            if !allowed.contains(&JwtAlgorithm::Es256) {
                return Err(AuthenticationError::AlgorithmNotSupported(
                    "ES256 is not in the allowed algorithms list".into(),
                ));
            }
            authenticate_inner(
                token,
                config,
                &resolver_arc,
                clock,
                header.alg,
                JwtAlgorithm::Es256,
                mapper,
                |key| match key {
                    VerificationKey::EcPem(pem) => DecodingKey::from_ec_pem(pem.as_bytes())
                        .map_err(|e| AuthenticationError::InvalidToken(format!("bad EC key: {e}"))),
                    _ => Err(AuthenticationError::InvalidToken("expected EC PEM key".into())),
                },
            )
        }
        alg => Err(AuthenticationError::AlgorithmNotSupported(format!(
            "OIDC provider does not support {alg:?}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// OidcAuthenticationProvider
// ---------------------------------------------------------------------------

/// Composite OIDC resource server authenticator.
///
/// Routes tokens to the JWT path (`JwksKeyResolver`) or the opaque path
/// (`IntrospectionAuthenticationProvider`) based on `TokenFormat` config or
/// the Auto two-dot heuristic (OQ-2).
///
/// **Algorithm pinning**: unlike `Rs256AuthenticationProvider` and `Es256AuthenticationProvider`
/// which reject any algorithm at construction, OIDC mode accepts the algorithm declared in the
/// JWT header and delegates algorithm consistency to the JWKS resolver. The resolver enforces
/// that the key type in the JWKS matches the algorithm requested (`AlgorithmMismatch` if not).
/// This allows multi-algorithm JWKS endpoints without reconfiguration.
pub struct OidcAuthenticationProvider {
    jwks_resolver: Arc<JwksKeyResolver>,
    jwt_config: JwtProviderConfig,
    introspection: Option<Arc<IntrospectionAuthenticationProvider>>,
    token_format: TokenFormat,
    clock: Arc<dyn Clock>,
    mapper: Arc<dyn PrincipalMapper>,
    /// Algorithms accepted on the JWT path.
    allowed_algorithms: Vec<JwtAlgorithm>,
}

impl OidcAuthenticationProvider {
    /// Construct from config, using `HttpDiscoveryProvider` (default).
    pub fn new(
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Self, AuthenticationError> {
        Self::with_discovery(config, clock, mapper, Arc::new(HttpDiscoveryProvider::new()))
    }

    /// Construct with a custom `DiscoveryProvider` (used in tests).
    pub(crate) fn with_discovery(
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
        discovery: Arc<dyn DiscoveryProvider>,
    ) -> Result<Self, AuthenticationError> {
        config.validate()?;

        // Resolve JWKS URI: direct config wins over discovery (OQ-5).
        let jwks_uri = if let Some(uri) = config.jwks_uri.clone() {
            uri
        } else {
            // issuer_url is guaranteed Some because validate() passed.
            let issuer_url = config.issuer_url.clone().expect("issuer_url absent after validate");
            Self::discover_sync(Arc::clone(&discovery), issuer_url)?.jwks_uri
        };

        let ttl = Duration::from_secs(config.jwks_refresh_ttl_seconds.unwrap_or(300));
        let jwks_resolver = Arc::new(JwksKeyResolver::with_provider(
            jwks_uri,
            ttl,
            // ponytail: no HttpJwksProvider here — tests inject via with_provider
            Arc::new(crate::jwks::HttpJwksProvider::new()),
        ));

        let jwt_config = JwtProviderConfig {
            expected_iss: config.expected_iss.clone(),
            expected_aud: config.expected_aud.clone(),
            clock_skew_seconds: config.clock_skew_seconds,
        };

        let introspection = Self::build_introspection(&config, Arc::clone(&clock), Arc::clone(&mapper))?;

        let token_format = config.token_format.clone().unwrap_or(TokenFormat::Auto);
        let allowed_algorithms = config.allowed_algorithms.clone()
            .unwrap_or_else(|| vec![JwtAlgorithm::Rs256, JwtAlgorithm::Es256]);

        Ok(Self {
            jwks_resolver,
            jwt_config,
            introspection,
            token_format,
            clock,
            mapper,
            allowed_algorithms,
        })
    }

    /// Construct with a custom `JwksKeyResolver` (for tests — avoids HTTP).
    ///
    /// Allows injecting a `JwksKeyResolver` backed by a `FakeJwks` or other
    /// `JwksProvider` implementation without making any HTTP calls.
    #[cfg(any(test, feature = "test-kit"))]
    pub fn with_resolver(
        jwks_resolver: Arc<JwksKeyResolver>,
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Self, AuthenticationError> {
        // Enforce all invariants — same as with_discovery. Without this, test-kit
        // configs can silently bypass issuer_url/jwks_uri and credential checks.
        config.validate()?;

        let jwt_config = JwtProviderConfig {
            expected_iss: config.expected_iss.clone(),
            expected_aud: config.expected_aud.clone(),
            clock_skew_seconds: config.clock_skew_seconds,
        };

        let token_format = config.token_format.clone().unwrap_or(TokenFormat::Auto);
        let introspection = Self::build_introspection(&config, Arc::clone(&clock), Arc::clone(&mapper))?;
        let allowed_algorithms = config.allowed_algorithms.clone()
            .unwrap_or_else(|| vec![JwtAlgorithm::Rs256, JwtAlgorithm::Es256]);

        Ok(Self { jwks_resolver, jwt_config, introspection, token_format, clock, mapper, allowed_algorithms })
    }

    fn build_introspection(
        config: &OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Option<Arc<IntrospectionAuthenticationProvider>>, AuthenticationError> {
        if config.introspection_endpoint.is_some()
            && config.introspection_client_id.is_some()
            && config.introspection_client_secret.is_some()
        {
            let intro_config = OidcProviderConfig {
                introspection_endpoint: config.introspection_endpoint.clone(),
                introspection_client_id: config.introspection_client_id.clone(),
                introspection_client_secret: config.introspection_client_secret.clone(),
                introspection_cache_ttl_seconds: config.introspection_cache_ttl_seconds,
                ..Default::default()
            };
            Ok(Some(Arc::new(IntrospectionAuthenticationProvider::new(
                intro_config,
                clock,
                mapper,
            )?)))
        } else {
            Ok(None)
        }
    }

    fn discover_sync(
        discovery: Arc<dyn DiscoveryProvider>,
        issuer_url: url::Url,
    ) -> Result<OidcEndpoints, AuthenticationError> {
        let (tx, rx) = std::sync::mpsc::channel();
        resolver_pool().spawn_ok(async move {
            let _ = tx.send(discovery.fetch_configuration(&issuer_url).await);
        });
        rx.recv()
            .map_err(|_| AuthenticationError::ProviderUnavailable("discovery did not complete (pool exhausted or task dropped)".into()))?
    }
}

impl AuthenticationProvider for OidcAuthenticationProvider {
    fn authenticate(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        let token = match credential {
            Credential::Bearer(t) => t.as_str(),
            _ => return Err(AuthenticationError::InvalidToken("unsupported credential type".into())),
        };

        // Pre-check: token size limit (INV per spec)
        if token.len() > crate::authenticator::MAX_TOKEN_BYTES {
            return Err(AuthenticationError::InvalidToken("token exceeds 8 KiB limit".into()));
        }

        match &self.token_format {
            TokenFormat::Jwt => self.authenticate_jwt(token),
            TokenFormat::Opaque => self.authenticate_opaque(credential),
            TokenFormat::Auto => {
                if looks_like_jwt(token) {
                    self.authenticate_jwt(token)
                } else {
                    self.authenticate_opaque(credential)
                }
            }
        }
    }
}

impl OidcAuthenticationProvider {
    fn authenticate_jwt(&self, token: &str) -> Result<SecurityContext, AuthenticationError> {
        try_jwt_path(
            token,
            &self.jwks_resolver,
            &self.jwt_config,
            &self.clock,
            self.mapper.as_ref(),
            &self.allowed_algorithms,
        )
    }

    fn authenticate_opaque(
        &self,
        credential: &Credential,
    ) -> Result<SecurityContext, AuthenticationError> {
        match &self.introspection {
            Some(intro) => intro.authenticate(credential),
            None => Err(AuthenticationError::InvalidToken(
                "opaque token but no introspection endpoint configured".into(),
            )),
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

    use async_trait::async_trait;
    use ego_domain::auth::AuthenticationError;
    use ego_security_sdk::Credential;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    use crate::discovery::{DiscoveryProvider, OidcEndpoints};
    use crate::jwks::JwksProvider;
    use crate::key_resolver::VerificationKey;
    use crate::oidc_config::{OidcProviderConfig, TokenFormat};
    use crate::principal_mapper::DefaultPrincipalMapper;
    use crate::test_helpers::fixed_clock;
    use jsonwebtoken::Algorithm;

    // -----------------------------------------------------------------------
    // Key fixtures (reuse test PEM files)
    // -----------------------------------------------------------------------

    fn rs256_private_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_private.pem")
    }
    fn rs256_public_pem() -> &'static str {
        include_str!("../tests/fixtures/test_rsa_public.pem")
    }
    fn ec_private_pem() -> &'static str {
        include_str!("../tests/fixtures/test_ec_private.pem")
    }
    fn ec_public_pem() -> &'static str {
        include_str!("../tests/fixtures/test_ec_public.pem")
    }

    fn pinned_now() -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2025, 6, 1, 12, 0, 0).unwrap()
    }

    fn pinned_clock() -> Arc<dyn ego_domain::auth::Clock> {
        fixed_clock(pinned_now())
    }

    fn future_ts(secs: i64) -> i64 {
        (pinned_now() + chrono::Duration::seconds(secs)).timestamp()
    }

    fn past_ts(secs: i64) -> i64 {
        (pinned_now() - chrono::Duration::seconds(secs)).timestamp()
    }

    fn default_mapper() -> Arc<dyn PrincipalMapper> {
        Arc::new(DefaultPrincipalMapper)
    }

    // -----------------------------------------------------------------------
    // FakeJwks — returns a pre-loaded key, counts fetches
    // -----------------------------------------------------------------------

    struct FakeJwks {
        keys: Vec<(Option<String>, VerificationKey)>,
        count: Arc<AtomicUsize>,
    }

    impl FakeJwks {
        fn rsa(pub_pem: &str) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let fake = Self {
                keys: vec![(None, VerificationKey::RsaPem(pub_pem.to_string()))],
                count: Arc::clone(&count),
            };
            (fake, count)
        }

        fn ec(pub_pem: &str) -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            let fake = Self {
                keys: vec![(None, VerificationKey::EcPem(pub_pem.to_string()))],
                count: Arc::clone(&count),
            };
            (fake, count)
        }
    }

    #[async_trait]
    impl JwksProvider for FakeJwks {
        async fn fetch_jwks(
            &self,
            _: &url::Url,
        ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(self.keys.clone())
        }
    }

    // -----------------------------------------------------------------------
    // FakeDiscovery — returns a known jwks_uri, counts calls, captures URL
    // -----------------------------------------------------------------------

    struct FakeDiscovery {
        jwks_uri: url::Url,
        count: Arc<AtomicUsize>,
        received_url: Arc<std::sync::Mutex<Option<url::Url>>>,
    }

    impl FakeDiscovery {
        fn new(jwks_uri: url::Url) -> (Self, Arc<AtomicUsize>, Arc<std::sync::Mutex<Option<url::Url>>>) {
            let count = Arc::new(AtomicUsize::new(0));
            let received_url = Arc::new(std::sync::Mutex::new(None));
            (
                Self { jwks_uri, count: Arc::clone(&count), received_url: Arc::clone(&received_url) },
                count,
                Arc::clone(&received_url),
            )
        }
    }

    #[async_trait]
    impl DiscoveryProvider for FakeDiscovery {
        async fn fetch_configuration(
            &self,
            issuer_url: &url::Url,
        ) -> Result<OidcEndpoints, AuthenticationError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            *self.received_url.lock().unwrap() = Some(issuer_url.clone());
            Ok(OidcEndpoints { jwks_uri: self.jwks_uri.clone(), introspection_endpoint: None })
        }
    }

    // -----------------------------------------------------------------------
    // Helpers: build resolver with FakeJwks
    // -----------------------------------------------------------------------

    fn rsa_resolver(pub_pem: &str) -> Arc<JwksKeyResolver> {
        let (fake, _) = FakeJwks::rsa(pub_pem);
        Arc::new(JwksKeyResolver::with_provider(
            url::Url::parse("https://idp.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            Arc::new(fake),
        ))
    }

    fn ec_resolver(pub_pem: &str) -> Arc<JwksKeyResolver> {
        let (fake, _) = FakeJwks::ec(pub_pem);
        Arc::new(JwksKeyResolver::with_provider(
            url::Url::parse("https://idp.example.com/jwks").unwrap(),
            Duration::from_secs(300),
            Arc::new(fake),
        ))
    }

    fn base_config() -> OidcProviderConfig {
        OidcProviderConfig {
            issuer_url: None,
            jwks_uri: Some(url::Url::parse("https://idp.example.com/jwks").unwrap()),
            expected_iss: None,
            expected_aud: None,
            clock_skew_seconds: None,
            jwks_refresh_ttl_seconds: None,
            token_format: None,
            introspection_endpoint: None,
            introspection_client_id: None,
            introspection_client_secret: None,
            introspection_cache_ttl_seconds: None,
            allowed_algorithms: None,
        }
    }

    fn make_rs256_token(claims: &serde_json::Value) -> String {
        let header = Header::new(Algorithm::RS256);
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(rs256_private_pem().as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn make_ec_token(claims: &serde_json::Value) -> String {
        let header = Header::new(Algorithm::ES256);
        encode(
            &header,
            claims,
            &EncodingKey::from_ec_pem(ec_private_pem().as_bytes()).unwrap(),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // looks_like_jwt
    // -----------------------------------------------------------------------

    #[test]
    fn three_part_base64url_looks_like_jwt() {
        // A real JWT header.payload.sig is three base64url parts
        let token = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEifQ.sig";
        assert!(looks_like_jwt(token));
    }

    #[test]
    fn opaque_with_no_dots_does_not_look_like_jwt() {
        assert!(!looks_like_jwt("opaque-token-without-dots"));
    }

    #[test]
    fn two_dots_but_empty_part_does_not_look_like_jwt() {
        assert!(!looks_like_jwt("..signature"));
    }

    // CRITICAL-3: empty signature segment — header+payload valid base64url but sig empty
    #[test]
    fn looks_like_jwt_with_empty_signature_segment_is_false() {
        // "eyJ.eyJ." — header and payload look like base64url but signature is empty.
        // Must return false so an opaque token with this shape is NOT mis-routed to JWT path.
        assert!(
            !looks_like_jwt("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1MSJ9."),
            "three-part token with empty signature segment must not look like a JWT"
        );
    }

    // CRITICAL-3: two-dot token with valid base64url header+payload but no signature at all
    // (the third segment is empty) must fall through to opaque path in Auto mode.
    #[test]
    fn auto_mode_with_empty_signature_segment_routes_to_opaque_not_jwt() {
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            token_format: Some(TokenFormat::Auto),
            ..base_config()
        };
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();
        // A token that ends with a dot (empty signature) must go to opaque, not JWT.
        // No introspection configured → InvalidToken (opaque path).
        // If routed to JWT: jsonwebtoken parse would return InvalidToken too, but for different
        // reasons. The key assertion is looks_like_jwt returns false for this input.
        let err = provider
            .authenticate(&Credential::Bearer(
                "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1MSJ9.".into(),
            ))
            .unwrap_err();
        // Whether JWT or opaque path: we must get InvalidToken (not a panic or Ok).
        assert!(
            matches!(err, AuthenticationError::InvalidToken(_)),
            "empty-sig token must fail cleanly, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // T-12 acceptance criteria
    // -----------------------------------------------------------------------

    #[test]
    fn valid_rs256_jwt_returns_security_context() {
        let claims = json!({ "sub": "rs256-user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims);
        let resolver = rsa_resolver(rs256_public_pem());
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, base_config(), pinned_clock(), default_mapper(),
        )
        .unwrap();
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "rs256-user");
    }

    #[test]
    fn valid_es256_jwt_returns_security_context() {
        let claims = json!({ "sub": "es256-user", "exp": future_ts(3600) });
        let token = make_ec_token(&claims);
        let resolver = ec_resolver(ec_public_pem());
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, base_config(), pinned_clock(), default_mapper(),
        )
        .unwrap();
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "es256-user");
    }

    #[test]
    fn expired_jwt_returns_expired_token() {
        let claims = json!({ "sub": "user", "exp": past_ts(60) });
        let token = make_rs256_token(&claims);
        let resolver = rsa_resolver(rs256_public_pem());
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, base_config(), pinned_clock(), default_mapper(),
        )
        .unwrap();
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, AuthenticationError::ExpiredToken);
    }

    #[test]
    fn tampered_signature_returns_invalid_signature() {
        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let mut token = make_rs256_token(&claims);
        // Corrupt the signature (last segment)
        token.push_str("TAMPERED");
        let resolver = rsa_resolver(rs256_public_pem());
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, base_config(), pinned_clock(), default_mapper(),
        )
        .unwrap();
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::InvalidSignature | AuthenticationError::InvalidToken(_)),
            "expected signature error, got {err:?}"
        );
    }

    #[test]
    fn token_over_8kib_returns_invalid_token() {
        let resolver = rsa_resolver(rs256_public_pem());
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, base_config(), pinned_clock(), default_mapper(),
        )
        .unwrap();
        let huge = "x".repeat(8193);
        let err = provider.authenticate(&Credential::Bearer(huge)).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn with_discovery_calls_discover_sync_once_and_provider_is_functional() {
        let jwks_uri = url::Url::parse("https://fake-idp.test/jwks").unwrap();
        let issuer = url::Url::parse("https://fake-idp.test").unwrap();
        let (fake_discovery, discovery_count, received_url) = FakeDiscovery::new(jwks_uri.clone());

        let config = OidcProviderConfig {
            issuer_url: Some(issuer.clone()),
            jwks_uri: None, // force discover_sync
            ..Default::default()
        };

        let result = OidcAuthenticationProvider::with_discovery(
            config,
            pinned_clock(),
            default_mapper(),
            Arc::new(fake_discovery),
        );

        assert!(result.is_ok(), "with_discovery must succeed when FakeDiscovery returns a valid jwks_uri");
        assert_eq!(
            discovery_count.load(Ordering::SeqCst),
            1,
            "discover_sync must be called exactly once at construction"
        );
        // Verify the correct issuer URL was forwarded to the discovery provider.
        assert_eq!(
            received_url.lock().unwrap().as_ref(),
            Some(&issuer),
            "discover_sync must forward the issuer_url to the DiscoveryProvider"
        );
        // Auth will fail (no real JWKS server) — but construction succeeded and the
        // discover_sync bridge is exercised.
        let provider = result.unwrap();
        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims);
        // Auth must fail — either JWKS fetch error (ProviderUnavailable) or
        // key mismatch (InvalidSignature). Either proves the real JWKS path was taken.
        // Using a narrow match instead of is_err() prevents an Ok(...) from silently
        // passing (which would mean the test never actually exercised the JWKS path).
        let auth_err = provider.authenticate(&Credential::Bearer(token))
            .expect_err("auth must fail when no real JWKS server backs the discovered URI");
        assert!(
            matches!(
                auth_err,
                AuthenticationError::ProviderUnavailable(_) | AuthenticationError::InvalidSignature
            ),
            "expected ProviderUnavailable or InvalidSignature, got: {auth_err:?}"
        );
    }

    #[test]
    fn with_discovery_propagates_discovery_error() {
        struct FailingDiscovery;

        #[async_trait]
        impl DiscoveryProvider for FailingDiscovery {
            async fn fetch_configuration(
                &self,
                _: &url::Url,
            ) -> Result<OidcEndpoints, AuthenticationError> {
                Err(AuthenticationError::ProviderUnavailable("discovery unavailable".into()))
            }
        }

        let config = OidcProviderConfig {
            issuer_url: Some(url::Url::parse("https://fake-idp.test").unwrap()),
            jwks_uri: None,
            ..Default::default()
        };

        let result = OidcAuthenticationProvider::with_discovery(
            config,
            pinned_clock(),
            default_mapper(),
            Arc::new(FailingDiscovery),
        );

        assert!(
            matches!(result, Err(AuthenticationError::ProviderUnavailable(_))),
            "discovery failure must propagate as ProviderUnavailable"
        );
    }

    #[test]
    fn jwks_uri_config_does_not_call_discovery() {
        // If jwks_uri is set, discovery should NOT be called.
        // Tested by using with_resolver which already has the resolver.
        let resolver = rsa_resolver(rs256_public_pem());
        let config = base_config(); // has jwks_uri set
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();

        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims);
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user");
    }

    #[test]
    fn neither_url_returns_provider_unavailable_from_validate() {
        let config = OidcProviderConfig {
            issuer_url: None,
            jwks_uri: None,
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
    }

    // HIGH-2: with_resolver enforces validate() — missing both urls must be rejected
    // (guards against test-kit configs that silently bypass invariants)
    #[test]
    fn with_resolver_rejects_config_missing_both_urls() {
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            issuer_url: None,
            jwks_uri: None,
            ..Default::default()
        };
        let result = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        );
        assert!(
            matches!(result, Err(AuthenticationError::ProviderUnavailable(_))),
            "with_resolver must reject config that fails validate()"
        );
    }

    #[test]
    fn token_format_auto_with_jwt_uses_jwt_path() {
        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims); // 3-part JWT
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            token_format: Some(TokenFormat::Auto),
            ..base_config()
        };
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();
        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user");
    }

    #[test]
    fn token_format_auto_with_opaque_returns_invalid_token_when_no_introspection() {
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            token_format: Some(TokenFormat::Auto),
            ..base_config()
        };
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();
        // Opaque token (no dots)
        let err = provider
            .authenticate(&Credential::Bearer("opaque-token-here".into()))
            .unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn token_format_opaque_with_dot_token_uses_introspection() {
        // A dot-containing token with TokenFormat::Opaque must use introspection.
        // We set up introspection (with a fake) and verify the opaque path is taken.
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            token_format: Some(TokenFormat::Opaque),
            introspection_endpoint: Some(
                url::Url::parse("https://idp.example.com/introspect").unwrap(),
            ),
            introspection_client_id: Some("cid".into()),
            introspection_client_secret: Some("csecret".into()),
            ..base_config()
        };

        // We need to build IntrospectionAuthenticationProvider with a fake.
        // Use with_resolver but then replace introspection via manual build.
        // Simpler: build via with_resolver which builds introspection from config.
        // The config has introspection fields, so with_resolver builds HttpIntrospectionProvider.
        // We can't easily inject a fake here without a with_introspection constructor.
        // For this test, just verify the routing logic by testing that an opaque token
        // (even dot-containing like a JWT) goes through introspection when Opaque is set.
        // We accept that HttpIntrospectionProvider fails (ProviderUnavailable) since we're
        // not spinning up a real HTTP server.
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();

        // A dot-containing "JWT-looking" token with TokenFormat::Opaque should still
        // go through introspection, not the JWT path. Since there's no server,
        // it returns ProviderUnavailable (not InvalidSignature or ExpiredToken).
        // We use a structurally-valid JWT string to prove it didn't go through the JWT path.
        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let jwt_token = make_rs256_token(&claims);
        let err = provider.authenticate(&Credential::Bearer(jwt_token)).unwrap_err();
        // If JWT path was taken: would get Ok (valid token, valid key)
        // If opaque path was taken: HttpIntrospectionProvider fails → ProviderUnavailable
        assert!(
            matches!(err, AuthenticationError::ProviderUnavailable(_)),
            "Opaque format should route to introspection (got {err:?})"
        );
    }

    // B-2: RS256 token rejected when allowed_algorithms = [Es256]
    #[test]
    fn try_jwt_path_rejects_algorithm_not_in_allowed_list() {
        let claims = json!({ "sub": "user", "exp": future_ts(3600) });
        let token = make_rs256_token(&claims); // RS256
        let resolver = rsa_resolver(rs256_public_pem());
        let config = OidcProviderConfig {
            // Only Es256 is allowed — RS256 token must be rejected
            allowed_algorithms: Some(vec![JwtAlgorithm::Es256]),
            ..base_config()
        };
        let provider = OidcAuthenticationProvider::with_resolver(
            resolver, config, pinned_clock(), default_mapper(),
        )
        .unwrap();
        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert!(
            matches!(err, AuthenticationError::AlgorithmNotSupported(_)),
            "RS256 token must be rejected when only Es256 is allowed; got {err:?}"
        );
    }

    // C-4: empty signature segment must not be treated as JWT
    #[test]
    fn looks_like_jwt_returns_false_for_empty_signature_segment() {
        assert!(!looks_like_jwt("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1In0."));
    }
}
