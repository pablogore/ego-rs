//! TestKit — in-process fakes for OIDC testing without live IdPs.
//!
//! Gated behind `#[cfg(feature = "test-kit")]` (INV-5, AD-OIDC-007).
//! All types in this module are zero-network — they operate entirely in-process.
//!
//! # Usage
//!
//! ```rust,no_run
//! # #[cfg(feature = "test-kit")] {
//! use std::sync::Arc;
//! use std::collections::BTreeMap;
//! use security_jwt::test_kit::{FakeIssuer, FakeJwks};
//! use ego_domain::auth::{ClaimValue, Clock, SystemClock};
//!
//! let clock: Arc<dyn Clock> = Arc::new(SystemClock);
//! let issuer = FakeIssuer::new(Arc::clone(&clock));
//! let mut claims = BTreeMap::new();
//! claims.insert("sub".to_string(), ClaimValue::String("user-1".into()));
//! claims.insert("exp".to_string(), ClaimValue::Integer(9_999_999_999));
//! let token = issuer.issue_token(claims);
//! # }
//! ```

#[cfg(feature = "test-kit")]
use std::collections::{BTreeMap, HashMap};
#[cfg(feature = "test-kit")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "test-kit")]
use std::time::Duration;

#[cfg(feature = "test-kit")]
use async_trait::async_trait;
#[cfg(feature = "test-kit")]
use ego_domain::auth::{AuthenticationError, ClaimSet, ClaimValue, Clock};
#[cfg(feature = "test-kit")]
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

#[cfg(feature = "test-kit")]
use crate::config::JwtAlgorithm;
#[cfg(feature = "test-kit")]
use crate::discovery::{DiscoveryProvider, OidcEndpoints};
#[cfg(feature = "test-kit")]
use crate::introspection::{ClientCredentials, IntrospectionProvider, IntrospectionResult};
#[cfg(feature = "test-kit")]
use crate::jwks::{JwksKeyResolver, JwksProvider};
#[cfg(feature = "test-kit")]
use crate::key_resolver::VerificationKey;
#[cfg(feature = "test-kit")]
use crate::principal_mapper::claim_value_to_json;

// ---------------------------------------------------------------------------
// FakeIssuer
// ---------------------------------------------------------------------------

/// An in-process JWT issuer backed by a real keypair.
///
/// Issues signed tokens via `issue_token`. Provides `jwks_resolver()` for
/// plugging into `OidcAuthenticationProvider` without HTTP.
#[cfg(feature = "test-kit")]
pub struct FakeIssuer {
    algorithm: JwtAlgorithm,
    encoding_key: EncodingKey,
    verification_key: VerificationKey,
    /// The fake JWKS URI — stable string used as the key in the cache map.
    pub jwks_uri: url::Url,
}

#[cfg(feature = "test-kit")]
impl FakeIssuer {
    /// Create a `FakeIssuer` backed by the test RSA key fixtures.
    ///
    /// Uses the shared test PEM files from `tests/fixtures/`.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_algorithm(clock, JwtAlgorithm::Rs256)
    }

    /// Create with a specific algorithm. Only `Rs256` and `Es256` are supported.
    pub fn with_algorithm(_clock: Arc<dyn Clock>, algorithm: JwtAlgorithm) -> Self {
        let (encoding_key, verification_key) = match algorithm {
            JwtAlgorithm::Rs256 => {
                let priv_pem = include_str!("../../tests/fixtures/test_rsa_private.pem");
                let pub_pem = include_str!("../../tests/fixtures/test_rsa_public.pem");
                (
                    EncodingKey::from_rsa_pem(priv_pem.as_bytes())
                        .expect("test RSA private key invalid"),
                    VerificationKey::RsaPem(pub_pem.to_string()),
                )
            }
            JwtAlgorithm::Es256 => {
                let priv_pem = include_str!("../../tests/fixtures/test_ec_private.pem");
                let pub_pem = include_str!("../../tests/fixtures/test_ec_public.pem");
                (
                    EncodingKey::from_ec_pem(priv_pem.as_bytes())
                        .expect("test EC private key invalid"),
                    VerificationKey::EcPem(pub_pem.to_string()),
                )
            }
            JwtAlgorithm::Hs256 => {
                panic!("FakeIssuer does not support Hs256 (OIDC issuers use RSA/EC)");
            }
        };

        Self {
            algorithm,
            encoding_key,
            verification_key,
            jwks_uri: url::Url::parse("https://fake-issuer.test/jwks").unwrap(),
        }
    }

    /// Sign a token with the given claims.
    ///
    /// Claims must include `sub` and `exp` (as `ClaimValue::Integer`).
    pub fn issue_token(&self, claims: BTreeMap<String, ClaimValue>) -> String {
        let alg = match self.algorithm {
            JwtAlgorithm::Rs256 => Algorithm::RS256,
            JwtAlgorithm::Es256 => Algorithm::ES256,
            JwtAlgorithm::Hs256 => unreachable!("Hs256 not supported"),
        };
        let header = Header::new(alg);

        // Convert ClaimValue → serde_json::Value for encoding
        let json_claims: serde_json::Map<String, serde_json::Value> = claims
            .into_iter()
            .map(|(k, v)| (k, claim_value_to_json(&v)))
            .collect();
        let value = serde_json::Value::Object(json_claims);

        encode(&header, &value, &self.encoding_key)
            .expect("FakeIssuer failed to encode token")
    }

    /// Returns a `JwksKeyResolver` pre-loaded with this issuer's public key.
    ///
    /// No HTTP involved — backed by `FakeJwks`.
    pub fn jwks_resolver(&self) -> JwksKeyResolver {
        JwksKeyResolver::with_provider(
            self.jwks_uri.clone(),
            Duration::MAX,
            Arc::new(FakeJwks::new(self)),
        )
    }
}

// ---------------------------------------------------------------------------
// FakeJwks
// ---------------------------------------------------------------------------

/// Returns the `FakeIssuer`'s public key.
///
/// Implements `JwksProvider` — plug into `JwksKeyResolver::with_provider`.
#[cfg(feature = "test-kit")]
pub struct FakeJwks {
    key: VerificationKey,
}

#[cfg(feature = "test-kit")]
impl FakeJwks {
    /// Create from an existing `FakeIssuer`.
    pub fn new(issuer: &FakeIssuer) -> Self {
        Self { key: issuer.verification_key.clone() }
    }
}

#[cfg(feature = "test-kit")]
#[async_trait]
impl JwksProvider for FakeJwks {
    async fn fetch_jwks(
        &self,
        _jwks_uri: &url::Url,
    ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
        Ok(vec![(None, self.key.clone())])
    }
}

// ---------------------------------------------------------------------------
// FakeDiscovery
// ---------------------------------------------------------------------------

/// Returns `OidcEndpoints` pointing at the `FakeIssuer`'s fake JWKS URI.
///
/// Implements `DiscoveryProvider` (pub(crate)) — used internally within `security-jwt`
/// to test the `issuer_url`-based construction path of `OidcAuthenticationProvider`.
#[cfg(feature = "test-kit")]
pub struct FakeDiscovery {
    endpoints: OidcEndpoints,
}

#[cfg(feature = "test-kit")]
impl FakeDiscovery {
    /// Create from an existing `FakeIssuer`.
    pub fn new(issuer: &FakeIssuer) -> Self {
        Self {
            endpoints: OidcEndpoints {
                jwks_uri: issuer.jwks_uri.clone(),
                introspection_endpoint: None,
            },
        }
    }
}

#[cfg(feature = "test-kit")]
#[async_trait]
impl DiscoveryProvider for FakeDiscovery {
    async fn fetch_configuration(
        &self,
        _issuer_url: &url::Url,
    ) -> Result<OidcEndpoints, AuthenticationError> {
        Ok(OidcEndpoints {
            jwks_uri: self.endpoints.jwks_uri.clone(),
            introspection_endpoint: self.endpoints.introspection_endpoint.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// FakeIntrospection
// ---------------------------------------------------------------------------

/// Configurable fake introspection provider for unit tests.
///
/// Pre-configure per-token responses with `set_response` and `set_active_response`.
/// Unknown tokens return `active: false`.
#[cfg(feature = "test-kit")]
pub struct FakeIntrospection {
    responses: Mutex<HashMap<String, IntrospectionResult>>,
}

#[cfg(feature = "test-kit")]
impl FakeIntrospection {
    /// Create with no pre-configured responses (all tokens inactive by default).
    pub fn new() -> Self {
        Self { responses: Mutex::new(HashMap::new()) }
    }

    /// Set a simple active/inactive response for a token.
    ///
    /// `active: false` → `IntrospectionResult { active: false, claims: None }`.
    /// `active: true` → `IntrospectionResult { active: true, claims: None }` (protocol error!
    /// use `set_active_response` to provide claims).
    pub fn set_response(&mut self, token: &str, active: bool) {
        let result = IntrospectionResult { active, claims: None };
        self.responses.lock().unwrap().insert(token.to_string(), result);
    }

    /// Set a full active response with claims for a token.
    pub fn set_active_response(&mut self, token: &str, claims: ClaimSet) {
        let result = IntrospectionResult { active: true, claims: Some(claims) };
        self.responses.lock().unwrap().insert(token.to_string(), result);
    }
}

#[cfg(feature = "test-kit")]
impl Default for FakeIntrospection {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "test-kit")]
#[async_trait]
impl IntrospectionProvider for FakeIntrospection {
    async fn introspect(
        &self,
        token: &str,
        _endpoint: &url::Url,
        _credentials: &ClientCredentials,
    ) -> Result<IntrospectionResult, AuthenticationError> {
        let guard = self.responses.lock().unwrap();
        if let Some(result) = guard.get(token) {
            Ok(IntrospectionResult {
                active: result.active,
                claims: result.claims.clone(),
            })
        } else {
            Ok(IntrospectionResult { active: false, claims: None })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (only compiled with feature = "test-kit")
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "test-kit"))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ego_domain::auth::{ClaimSet, ClaimValue};
    use ego_security_sdk::{AuthenticationProvider, Credential};

    use crate::config::JwtAlgorithm;
    use crate::oidc_config::OidcProviderConfig;
    use crate::oidc_provider::OidcAuthenticationProvider;
    use crate::principal_mapper::DefaultPrincipalMapper;
    use crate::test_helpers::fixed_clock;

    fn pinned_now() -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2025, 6, 1, 12, 0, 0).unwrap()
    }

    fn future_ts(secs: i64) -> i64 {
        (pinned_now() + chrono::Duration::seconds(secs)).timestamp()
    }

    fn make_claims(sub: &str, exp: i64) -> BTreeMap<String, ClaimValue> {
        let mut map = BTreeMap::new();
        map.insert("sub".to_string(), ClaimValue::String(sub.into()));
        map.insert("exp".to_string(), ClaimValue::Integer(exp));
        map
    }

    // -----------------------------------------------------------------------
    // FakeIssuer
    // -----------------------------------------------------------------------

    #[test]
    fn fake_issuer_rs256_token_passes_oidc_validation() {
        let clock = fixed_clock(pinned_now());
        let issuer = FakeIssuer::new(Arc::clone(&clock));
        let claims = make_claims("user-rs256", future_ts(3600));
        let token = issuer.issue_token(claims);

        let resolver = Arc::new(issuer.jwks_resolver());
        let config = OidcProviderConfig {
            jwks_uri: Some(issuer.jwks_uri.clone()),
            ..Default::default()
        };
        let mapper = Arc::new(DefaultPrincipalMapper);
        let provider =
            OidcAuthenticationProvider::with_resolver(resolver, config, clock, mapper).unwrap();

        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-rs256");
    }

    #[test]
    fn fake_issuer_es256_token_passes_oidc_validation() {
        let clock = fixed_clock(pinned_now());
        let issuer = FakeIssuer::with_algorithm(Arc::clone(&clock), JwtAlgorithm::Es256);
        let claims = make_claims("user-es256", future_ts(3600));
        let token = issuer.issue_token(claims);

        let resolver = Arc::new(issuer.jwks_resolver());
        let config = OidcProviderConfig {
            jwks_uri: Some(issuer.jwks_uri.clone()),
            ..Default::default()
        };
        let mapper = Arc::new(DefaultPrincipalMapper);
        let provider =
            OidcAuthenticationProvider::with_resolver(resolver, config, clock, mapper).unwrap();

        let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "user-es256");
    }

    #[test]
    fn fake_issuer_expired_token_returns_expired_token() {
        let clock = fixed_clock(pinned_now());
        let issuer = FakeIssuer::new(Arc::clone(&clock));
        let claims = make_claims("user", (pinned_now() - chrono::Duration::seconds(60)).timestamp());
        let token = issuer.issue_token(claims);

        let resolver = Arc::new(issuer.jwks_resolver());
        let config = OidcProviderConfig {
            jwks_uri: Some(issuer.jwks_uri.clone()),
            ..Default::default()
        };
        let mapper = Arc::new(DefaultPrincipalMapper);
        let provider =
            OidcAuthenticationProvider::with_resolver(resolver, config, clock, mapper).unwrap();

        let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
        assert_eq!(err, ego_domain::auth::AuthenticationError::ExpiredToken);
    }

    // -----------------------------------------------------------------------
    // FakeIntrospection
    // -----------------------------------------------------------------------

    #[test]
    fn fake_introspection_set_response_inactive_returns_invalid_token() {
        let mut fake = FakeIntrospection::new();
        fake.set_response("tok", false);

        let config = OidcProviderConfig {
            jwks_uri: Some(url::Url::parse("https://fake.test/jwks").unwrap()),
            introspection_endpoint: Some(
                url::Url::parse("https://fake.test/introspect").unwrap(),
            ),
            introspection_client_id: Some("cid".into()),
            introspection_client_secret: Some("csecret".into()),
            ..Default::default()
        };
        let clock = fixed_clock(pinned_now());
        let mapper = Arc::new(DefaultPrincipalMapper);
        let provider = crate::introspection::IntrospectionAuthenticationProvider::with_provider(
            config,
            clock,
            mapper,
            Arc::new(fake),
        )
        .unwrap();

        let err = provider
            .authenticate(&Credential::Bearer("tok".into()))
            .unwrap_err();
        assert!(matches!(err, ego_domain::auth::AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn fake_introspection_set_active_response_returns_ok() {
        let mut fake = FakeIntrospection::new();
        let mut raw = BTreeMap::new();
        raw.insert("sub".to_string(), ClaimValue::String("introspected-user".into()));
        raw.insert("exp".to_string(), ClaimValue::Integer(9_999_999_999));
        fake.set_active_response("tok", ClaimSet::new(raw));

        let config = OidcProviderConfig {
            jwks_uri: Some(url::Url::parse("https://fake.test/jwks").unwrap()),
            introspection_endpoint: Some(
                url::Url::parse("https://fake.test/introspect").unwrap(),
            ),
            introspection_client_id: Some("cid".into()),
            introspection_client_secret: Some("csecret".into()),
            ..Default::default()
        };
        let clock = fixed_clock(pinned_now());
        let mapper = Arc::new(DefaultPrincipalMapper);
        let provider = crate::introspection::IntrospectionAuthenticationProvider::with_provider(
            config,
            clock,
            mapper,
            Arc::new(fake),
        )
        .unwrap();

        let ctx = provider.authenticate(&Credential::Bearer("tok".into())).unwrap();
        assert_eq!(ctx.principal.subject_id.as_str(), "introspected-user");
    }
}
