//! Integration tests for the OIDC resource server framework.
//!
//! Covers all US-001 through US-008 acceptance criteria using the TestKit
//! (no live IdP or network access). Requires `feature = "test-kit"`.

#![cfg(feature = "test-kit")]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ego_domain::auth::{AuthenticationError, ClaimSet, ClaimValue};
use ego_security_sdk::{AuthenticationProvider, Credential, PrincipalMapper, Principal, Claims};
use security_jwt::{
    JwtAlgorithm, OidcAuthenticationProvider, MultiIssuerAuthenticationProvider,
    StaticIssuerResolver, OidcProviderConfig, TokenFormat,
    IntrospectionAuthenticationProvider,
};
use security_jwt::principal_mapper::DefaultPrincipalMapper;
use security_jwt::test_kit::{FakeIssuer, FakeIntrospection, FakeJwks};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn pinned_now() -> chrono::DateTime<chrono::Utc> {
    chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2025, 6, 1, 12, 0, 0).unwrap()
}

fn fixed_clock(dt: chrono::DateTime<chrono::Utc>) -> Arc<dyn ego_domain::auth::Clock> {
    struct C(chrono::DateTime<chrono::Utc>);
    impl ego_domain::auth::Clock for C {
        fn now(&self) -> chrono::DateTime<chrono::Utc> { self.0 }
    }
    Arc::new(C(dt))
}

fn future_ts(secs: i64) -> i64 {
    (pinned_now() + chrono::Duration::seconds(secs)).timestamp()
}

#[allow(dead_code)]
fn past_ts(secs: i64) -> i64 {
    (pinned_now() - chrono::Duration::seconds(secs)).timestamp()
}

fn default_mapper() -> Arc<dyn PrincipalMapper> {
    Arc::new(DefaultPrincipalMapper)
}

fn make_claims(sub: &str, exp: i64) -> BTreeMap<String, ClaimValue> {
    let mut m = BTreeMap::new();
    m.insert("sub".into(), ClaimValue::String(sub.into()));
    m.insert("exp".into(), ClaimValue::Integer(exp));
    m.insert("iss".into(), ClaimValue::String("https://fake-issuer.test".into()));
    m
}

fn oidc_provider_with_issuer(issuer: &FakeIssuer) -> OidcAuthenticationProvider {
    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    OidcAuthenticationProvider::with_resolver(
        resolver,
        config,
        fixed_clock(pinned_now()),
        default_mapper(),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// US-001: Bearer extraction + OIDC validation via AuthenticationInterceptor
// ---------------------------------------------------------------------------

#[test]
fn us001_bearer_extractor_with_oidc_provider_returns_ok() {
    use ego_security_sdk::{AuthenticationInterceptor, BearerExtractor};

    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let token = issuer.issue_token(make_claims("user-001", future_ts(3600)));

    let extractor = Arc::new(BearerExtractor);
    let provider = Arc::new(oidc_provider_with_issuer(&issuer));
    let interceptor = AuthenticationInterceptor::new(extractor, provider);

    // Minimal RequestContext with just the Authorization header
    struct SimpleCtx(String);
    impl ego_security_sdk::RequestContext for SimpleCtx {
        fn header(&self, name: &str) -> Option<&str> {
            if name.eq_ignore_ascii_case("authorization") {
                Some(&self.0)
            } else {
                None
            }
        }
        fn metadata(&self, _: &str) -> Option<&str> { None }
        fn query_param(&self, _: &str) -> Option<&str> { None }
    }

    let ctx = SimpleCtx(format!("Bearer {token}"));
    let mut security_ctx: Option<ego_security_sdk::SecurityContext> = None;
    interceptor
        .intercept(&ctx, |sc| security_ctx = Some(sc))
        .unwrap();

    let sc = security_ctx.expect("security context should be set");
    assert_eq!(sc.principal.subject_id.as_str(), "user-001");
}

// ---------------------------------------------------------------------------
// US-001: nbf in future rejected
// ---------------------------------------------------------------------------

#[test]
fn us001_nbf_in_future_returns_invalid_token() {
    // Clock fixed at T=1000. Token has nbf = T+100 (not yet valid).
    let clock = fixed_clock(chrono::DateTime::from_timestamp(1000, 0).unwrap());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = BTreeMap::new();
    claims.insert("sub".to_string(), ClaimValue::String("user-1".to_string()));
    claims.insert("exp".to_string(), ClaimValue::Integer(9_999_999_999));
    claims.insert("nbf".to_string(), ClaimValue::Integer(1100)); // T + 100
    claims.insert("iss".to_string(), ClaimValue::String("https://fake-issuer.test".to_string()));
    let token = issuer.issue_token(claims);
    let config = OidcProviderConfig {
        jwks_uri: Some(url::Url::parse("https://fake.example.com/jwks").unwrap()),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        Arc::new(issuer.jwks_resolver()),
        config,
        clock,
        Arc::new(DefaultPrincipalMapper),
    )
    .unwrap();
    let credential = Credential::Bearer(token);
    // note: claims also need iss for the validator; but nbf rejection fires before iss check
    let result = provider.authenticate(&credential);
    assert!(
        matches!(result, Err(AuthenticationError::InvalidToken(_))),
        "expected InvalidToken for nbf in future, got {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// US-002: Discovery path vs direct jwks_uri
// ---------------------------------------------------------------------------

#[test]
fn us002_jwks_uri_config_no_discovery_called() {
    // OidcAuthenticationProvider::with_resolver bypasses discovery entirely.
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        issuer_url: None, // explicit: no discovery
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider =
        OidcAuthenticationProvider::with_resolver(resolver, config, fixed_clock(pinned_now()), default_mapper())
        .unwrap();

    let token = issuer.issue_token(make_claims("u-jwks-direct", future_ts(3600)));
    let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "u-jwks-direct");
}

#[test]
fn us002_neither_url_returns_construction_error() {
    let config = OidcProviderConfig {
        issuer_url: None,
        jwks_uri: None,
        ..Default::default()
    };
    let err = config.validate().unwrap_err();
    assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
}

// ---------------------------------------------------------------------------
// US-003: JWT validation (RS256, ES256, invalid sig, iss mismatch)
// ---------------------------------------------------------------------------

#[test]
fn us003_rs256_valid_jwt_returns_security_context() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let token = issuer.issue_token(make_claims("rs256-user", future_ts(3600)));
    let ctx = oidc_provider_with_issuer(&issuer)
        .authenticate(&Credential::Bearer(token))
        .unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "rs256-user");
}

#[test]
fn us003_es256_valid_jwt_returns_security_context() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::with_algorithm(Arc::clone(&clock), JwtAlgorithm::Es256);
    let token = issuer.issue_token(make_claims("es256-user", future_ts(3600)));
    let ctx = oidc_provider_with_issuer(&issuer)
        .authenticate(&Credential::Bearer(token))
        .unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "es256-user");
}

#[test]
fn us003_tampered_signature_returns_error() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut token = issuer.issue_token(make_claims("user", future_ts(3600)));
    token.push_str("TAMPERED");
    let err = oidc_provider_with_issuer(&issuer)
        .authenticate(&Credential::Bearer(token))
        .unwrap_err();
    assert!(
        matches!(err, AuthenticationError::InvalidSignature | AuthenticationError::InvalidToken(_)),
        "unexpected {err:?}"
    );
}

#[test]
fn us003_iss_mismatch_post_signature_returns_invalid_token() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = make_claims("user", future_ts(3600));
    claims.insert("iss".into(), ClaimValue::String("rogue-issuer".into()));
    let token = issuer.issue_token(claims);

    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        expected_iss: Some("https://real-issuer.example.com".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver, config, fixed_clock(pinned_now()), default_mapper(),
    )
    .unwrap();

    let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
    assert!(matches!(err, AuthenticationError::InvalidToken(_)));
}

// ---------------------------------------------------------------------------
// US-003b: TokenFormat routing
// ---------------------------------------------------------------------------

#[test]
fn us003b_auto_jwt_uses_jwt_path() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let token = issuer.issue_token(make_claims("user-auto-jwt", future_ts(3600)));

    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        token_format: Some(TokenFormat::Auto),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver, config, fixed_clock(pinned_now()), default_mapper(),
    )
    .unwrap();

    let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "user-auto-jwt");
}

#[test]
fn us003b_auto_no_dots_uses_opaque_path_or_invalid_token() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        token_format: Some(TokenFormat::Auto),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver, config, fixed_clock(pinned_now()), default_mapper(),
    )
    .unwrap();

    // No introspection configured → InvalidToken for opaque path
    let err = provider
        .authenticate(&Credential::Bearer("no-dots-opaque-token".into()))
        .unwrap_err();
    assert!(matches!(err, AuthenticationError::InvalidToken(_)));
}

#[test]
fn us003b_opaque_format_with_dotted_token_uses_introspection() {
    // Introspection configured; TokenFormat::Opaque; token looks like JWT → goes to introspection
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let token = issuer.issue_token(make_claims("user", future_ts(3600)));
    // token is a valid JWT but with Opaque mode it should go to introspection
    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        token_format: Some(TokenFormat::Opaque),
        expected_iss: Some("https://fake-issuer.test".into()),
        introspection_endpoint: Some(
            url::Url::parse("https://fake.test/introspect").unwrap(),
        ),
        introspection_client_id: Some("cid".into()),
        introspection_client_secret: Some("csec".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver, config, fixed_clock(pinned_now()), default_mapper(),
    )
    .unwrap();
    // HttpIntrospectionProvider will fail (no server); this proves introspection path was taken
    let err = provider.authenticate(&Credential::Bearer(token)).unwrap_err();
    assert!(matches!(err, AuthenticationError::ProviderUnavailable(_)));
}

// ---------------------------------------------------------------------------
// US-004: Introspection
// ---------------------------------------------------------------------------

fn make_introspection_config(ttl: Option<u64>) -> OidcProviderConfig {
    OidcProviderConfig {
        jwks_uri: Some(url::Url::parse("https://fake.test/jwks").unwrap()),
        introspection_endpoint: Some(
            url::Url::parse("https://fake.test/introspect").unwrap(),
        ),
        introspection_client_id: Some("cid".into()),
        introspection_client_secret: Some("csec".into()),
        introspection_cache_ttl_seconds: ttl,
        ..Default::default()
    }
}

#[test]
fn us004_active_true_introspection_returns_ok() {
    let mut fake = FakeIntrospection::new();
    let mut raw = BTreeMap::new();
    raw.insert("sub".into(), ClaimValue::String("introspected-user".into()));
    raw.insert("exp".into(), ClaimValue::Integer(9_999_999_999));
    fake.set_active_response("tok", ClaimSet::new(raw));

    let provider = IntrospectionAuthenticationProvider::with_provider(
        make_introspection_config(None),
        fixed_clock(pinned_now()),
        default_mapper(),
        Arc::new(fake),
    )
    .unwrap();

    let ctx = provider.authenticate(&Credential::Bearer("tok".into())).unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "introspected-user");
}

#[test]
fn us004_active_false_introspection_returns_invalid_token() {
    let mut fake = FakeIntrospection::new();
    fake.set_inactive_response("tok");

    let provider = IntrospectionAuthenticationProvider::with_provider(
        make_introspection_config(None),
        fixed_clock(pinned_now()),
        default_mapper(),
        Arc::new(fake),
    )
    .unwrap();

    let err = provider.authenticate(&Credential::Bearer("tok".into())).unwrap_err();
    assert!(matches!(err, AuthenticationError::InvalidToken(_)));
}

#[test]
fn us004_token_over_8kib_rejected_before_io() {
    let mut fake = FakeIntrospection::new();
    fake.set_inactive_response("whatever");
    let provider = IntrospectionAuthenticationProvider::with_provider(
        make_introspection_config(None),
        fixed_clock(pinned_now()),
        default_mapper(),
        Arc::new(fake),
    )
    .unwrap();
    let huge = "x".repeat(8193);
    let err = provider.authenticate(&Credential::Bearer(huge)).unwrap_err();
    assert!(matches!(err, AuthenticationError::InvalidToken(_)));
}

// ---------------------------------------------------------------------------
// US-005: JWKS cache behavior
// ---------------------------------------------------------------------------

#[test]
fn us005_cache_hit_returns_key_without_second_fetch() {
    use security_jwt::{JwksKeyResolver, JwksProvider, KeyResolver, VerificationKey};
    use async_trait::async_trait;

    struct CountingFakeJwks {
        key: VerificationKey,
        count: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl JwksProvider for CountingFakeJwks {
        async fn fetch_jwks(
            &self,
            _: &url::Url,
        ) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(vec![(None, self.key.clone())])
        }
    }

    let key_pem = include_str!("fixtures/test_rsa_public.pem");
    let count = Arc::new(AtomicUsize::new(0));
    let fake = CountingFakeJwks {
        key: VerificationKey::RsaPem(key_pem.to_string()),
        count: Arc::clone(&count),
    };

    let resolver = JwksKeyResolver::with_provider(
        url::Url::parse("https://fake.test/jwks").unwrap(),
        std::time::Duration::from_secs(300),
        Arc::new(fake),
    );

    // The warm-up fetch counts as 1.
    let fetch_count_after_warmup = count.load(Ordering::SeqCst);
    assert!(fetch_count_after_warmup >= 1, "warm-up should fetch");

    // Additional resolves hit cache — no new fetches.
    futures_executor::block_on(resolver.resolve(None, JwtAlgorithm::Rs256)).unwrap();
    futures_executor::block_on(resolver.resolve(None, JwtAlgorithm::Rs256)).unwrap();
    let fetch_count_after_resolves = count.load(Ordering::SeqCst);
    assert_eq!(
        fetch_count_after_resolves, fetch_count_after_warmup,
        "cache hits should not trigger additional fetches"
    );
}

// ---------------------------------------------------------------------------
// US-006: DefaultPrincipalMapper + custom mapper
// ---------------------------------------------------------------------------

#[test]
fn us006_default_mapper_maps_all_standard_claims() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = make_claims("std-user", future_ts(3600));
    claims.insert("roles".into(), ClaimValue::Array(vec![ClaimValue::String("admin".into())]));
    claims.insert("tid".into(), ClaimValue::String("tenant-99".into()));
    let token = issuer.issue_token(claims);

    let ctx = oidc_provider_with_issuer(&issuer)
        .authenticate(&Credential::Bearer(token))
        .unwrap();

    assert_eq!(ctx.principal.subject_id.as_str(), "std-user");
    assert!(ctx.principal.roles.iter().any(|r| r.0 == "admin"));
    assert_eq!(ctx.principal.tenant_id.as_deref(), Some("tenant-99"));
}

struct PreferredUsernameToPrincipalMapper;

impl PrincipalMapper for PreferredUsernameToPrincipalMapper {
    fn map(
        &self,
        claims: &ClaimSet,
    ) -> Result<(Principal, Claims), AuthenticationError> {
        // Use preferred_username as subject_id instead of sub
        let sub = claims
            .get_str("preferred_username")
            .or_else(|| claims.get_str("sub"))
            .ok_or_else(|| AuthenticationError::MissingClaim("sub".into()))?;
        let principal = ego_security_sdk::Principal::new(
            ego_security_sdk::PrincipalKind::User,
            ego_security_sdk::SubjectId::new(sub).unwrap(),
        );
        let claims_out = ego_domain::auth::Claims {
            standard: ego_domain::auth::StandardClaims::default(),
            custom: Default::default(),
        };
        Ok((principal, claims_out))
    }
}

#[test]
fn us006_custom_mapper_preferred_username_as_subject_id() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = make_claims("internal-id", future_ts(3600));
    claims.insert(
        "preferred_username".into(),
        ClaimValue::String("external-name".into()),
    );
    let token = issuer.issue_token(claims);

    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver,
        config,
        fixed_clock(pinned_now()),
        Arc::new(PreferredUsernameToPrincipalMapper),
    )
    .unwrap();

    let ctx = provider.authenticate(&Credential::Bearer(token)).unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "external-name");
}

#[test]
fn us006_missing_sub_returns_missing_claim() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = BTreeMap::new();
    claims.insert("exp".into(), ClaimValue::Integer(future_ts(3600)));
    claims.insert("iss".into(), ClaimValue::String("https://fake-issuer.test".into()));
    // no sub
    let token = issuer.issue_token(claims);
    let err = oidc_provider_with_issuer(&issuer)
        .authenticate(&Credential::Bearer(token))
        .unwrap_err();
    assert!(matches!(err, AuthenticationError::MissingClaim(ref s) if s == "sub"));
}

struct TrackingMapper {
    inner: DefaultPrincipalMapper,
    count: Arc<AtomicUsize>,
}

impl PrincipalMapper for TrackingMapper {
    fn map(
        &self,
        claims: &ClaimSet,
    ) -> Result<(Principal, Claims), AuthenticationError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.inner.map(claims)
    }
}

#[test]
fn us006_mapper_called_exactly_once_per_authenticate() {
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let token = issuer.issue_token(make_claims("user", future_ts(3600)));

    let count = Arc::new(AtomicUsize::new(0));
    let mapper = Arc::new(TrackingMapper {
        inner: DefaultPrincipalMapper,
        count: Arc::clone(&count),
    });

    let resolver = Arc::new(issuer.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer.jwks_uri.clone()),
        expected_iss: Some("https://fake-issuer.test".into()),
        ..Default::default()
    };
    let provider = OidcAuthenticationProvider::with_resolver(
        resolver, config, fixed_clock(pinned_now()), mapper,
    )
    .unwrap();

    provider.authenticate(&Credential::Bearer(token)).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// US-007: Multi-issuer routing
// ---------------------------------------------------------------------------

#[test]
fn us007_known_issuer_routes_correctly() {
    let clock = fixed_clock(pinned_now());
    let issuer_a = FakeIssuer::new(Arc::clone(&clock));

    let resolver = Arc::new(issuer_a.jwks_resolver());
    let config = OidcProviderConfig {
        jwks_uri: Some(issuer_a.jwks_uri.clone()),
        expected_iss: Some("https://issuer-a.test".into()),
        ..Default::default()
    };
    let provider_a = Arc::new(
        OidcAuthenticationProvider::with_resolver(
            resolver,
            config,
            fixed_clock(pinned_now()),
            default_mapper(),
        )
        .unwrap(),
    ) as Arc<dyn AuthenticationProvider>;

    let mut issuers: HashMap<String, Arc<dyn AuthenticationProvider>> = HashMap::new();
    issuers.insert("https://issuer-a.test".into(), provider_a);

    let multi = MultiIssuerAuthenticationProvider::new(Arc::new(StaticIssuerResolver::new(issuers)));

    let mut claims = make_claims("user-a", future_ts(3600));
    claims.insert("iss".into(), ClaimValue::String("https://issuer-a.test".into()));
    let token = issuer_a.issue_token(claims);

    let ctx = multi.authenticate(&Credential::Bearer(token)).unwrap();
    assert_eq!(ctx.principal.subject_id.as_str(), "user-a");
}

#[test]
fn us007_unknown_iss_returns_invalid_token() {
    let multi = MultiIssuerAuthenticationProvider::new(Arc::new(StaticIssuerResolver::new(HashMap::new())));
    let clock = fixed_clock(pinned_now());
    let issuer = FakeIssuer::new(Arc::clone(&clock));
    let mut claims = make_claims("user", future_ts(3600));
    claims.insert("iss".into(), ClaimValue::String("https://unknown.test".into()));
    let token = issuer.issue_token(claims);
    let err = multi.authenticate(&Credential::Bearer(token)).unwrap_err();
    assert!(matches!(err, AuthenticationError::InvalidToken(_)));
}

#[test]
fn us007_arc_multi_issuer_usable_as_dyn_authentication_provider() {
    fn check(_: Arc<dyn AuthenticationProvider>) {}
    let multi = Arc::new(MultiIssuerAuthenticationProvider::new(Arc::new(
        StaticIssuerResolver::new(HashMap::new()),
    )));
    check(multi);
}

#[test]
fn us007_multi_issuer_end_to_end_two_issuers() {
    let clock = fixed_clock(pinned_now());
    let issuer_a = FakeIssuer::new(Arc::clone(&clock));
    let issuer_b = FakeIssuer::with_algorithm(Arc::clone(&clock), JwtAlgorithm::Es256);

    let provider_a = {
        let resolver = Arc::new(issuer_a.jwks_resolver());
        let config = OidcProviderConfig {
            jwks_uri: Some(issuer_a.jwks_uri.clone()),
            expected_iss: Some("issuer-a".into()),
            ..Default::default()
        };
        Arc::new(
            OidcAuthenticationProvider::with_resolver(
                resolver, config, fixed_clock(pinned_now()), default_mapper(),
            )
            .unwrap(),
        ) as Arc<dyn AuthenticationProvider>
    };

    let provider_b = {
        let resolver = Arc::new(issuer_b.jwks_resolver());
        let config = OidcProviderConfig {
            jwks_uri: Some(issuer_b.jwks_uri.clone()),
            expected_iss: Some("issuer-b".into()),
            ..Default::default()
        };
        Arc::new(
            OidcAuthenticationProvider::with_resolver(
                resolver, config, fixed_clock(pinned_now()), default_mapper(),
            )
            .unwrap(),
        ) as Arc<dyn AuthenticationProvider>
    };

    let mut map: HashMap<String, Arc<dyn AuthenticationProvider>> = HashMap::new();
    map.insert("issuer-a".into(), provider_a);
    map.insert("issuer-b".into(), provider_b);
    let multi = MultiIssuerAuthenticationProvider::new(Arc::new(StaticIssuerResolver::new(map)));

    // Token from issuer A
    let mut claims_a = make_claims("user-from-a", future_ts(3600));
    claims_a.insert("iss".into(), ClaimValue::String("issuer-a".into()));
    let token_a = issuer_a.issue_token(claims_a);
    let ctx_a = multi.authenticate(&Credential::Bearer(token_a)).unwrap();
    assert_eq!(ctx_a.principal.subject_id.as_str(), "user-from-a");

    // Token from issuer B
    let mut claims_b = make_claims("user-from-b", future_ts(3600));
    claims_b.insert("iss".into(), ClaimValue::String("issuer-b".into()));
    let token_b = issuer_b.issue_token(claims_b);
    let ctx_b = multi.authenticate(&Credential::Bearer(token_b)).unwrap();
    assert_eq!(ctx_b.principal.subject_id.as_str(), "user-from-b");
}

// ---------------------------------------------------------------------------
// US-008: TestKit features (compile check — types are usable)
// ---------------------------------------------------------------------------

#[test]
fn us008_fake_issuer_and_fake_introspection_are_accessible() {
    // Compile-time check: types exist and are constructible.
    let clock = fixed_clock(pinned_now());
    let _issuer = FakeIssuer::new(clock);
    let _fake_intro = FakeIntrospection::new();
    let fake_jwks_issuer = FakeIssuer::new(fixed_clock(pinned_now()));
    let _fake_jwks = FakeJwks::new(&fake_jwks_issuer);
}
