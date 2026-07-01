# Tasks: oauth2-oidc — OIDC Resource Server Framework

_Generated: 2026-06-28 | Status: ready-for-apply_

> Strict TDD Mode is active. Write the test first; make it pass; move on.
> All tasks target a single PR. Estimated line budget: ~1,800–2,500 lines
> (production + tests combined — see note at bottom).

---

## Dependency Order and Parallelism

```
T-01 (ego-domain: ClaimSet)
  └─ T-02 (security-sdk: PrincipalMapper trait)
       └─ T-03 (security-sdk: CredentialExtractor + RequestContext + built-in extractors)
            └─ T-04 (security-sdk: AuthenticationInterceptor)
  └─ T-05 (security-jwt: Cargo.toml — new deps)
       └─ T-06 (security-jwt: OidcProviderConfig + TokenFormat + MultiIssuerConfig)
            ├─ T-07 (security-jwt: JwksKeyResolver + JwksProvider)   ─┐
            ├─ T-08 (security-jwt: DiscoveryProvider)                  │ parallel
            └─ T-09 (security-jwt: IntrospectionProvider + IntrospectionAuthenticationProvider)
            └─ T-10 (security-jwt: DefaultPrincipalMapper)
                 ├─ T-11 (security-jwt: JwtValidationEngine refactor — inject PrincipalMapper)
                 └─ T-12 (security-jwt: OidcAuthenticationProvider)
                      └─ T-13 (security-jwt: MultiIssuerAuthenticationProvider + IssuerResolver)
                           └─ T-14 (security-jwt: TestKit — FakeIssuer, FakeDiscovery, FakeJwks, FakeIntrospection)
                                └─ T-15 (integration tests)
                                └─ T-16 (CI: no test-kit symbols in release build)
```

T-07, T-08, T-09, T-10 can be worked in parallel after T-06 is merged.
T-03 can be worked in parallel with T-05/T-06 after T-01 and T-02 are done.

---

## Phase 1: ego-domain

### T-01 — ClaimSet and ClaimValue value objects

**Crate**: `crates/domain`
**File**: `crates/domain/src/auth/claim_set.rs` (new)
**Spec req**: INV-8, US-006, design §ClaimSet value object

**Description**:
Add `ClaimSet { raw: BTreeMap<String, ClaimValue> }` and `ClaimValue` enum to `ego-domain`.
No `serde_json` dependency — this is a pure domain type. Also add the standard helper
methods: `subject()`, `roles()`, `tenant()`, `scope()`, `organization()`, `expiry()`,
`issuer()`, plus the raw access helpers `get_str`, `get_array`, `get_nested_array`, `get_i64`.

Add a `From<serde_json::Value> for ClaimValue` conversion (pub(crate) or pub — lives in
`security-jwt`'s boundary, but the conversion function can live in `security-jwt` rather than
domain to keep domain free of serde_json).

Wire the new module into `crates/domain/src/auth/mod.rs` and re-export from
`crates/domain/src/lib.rs`.

**Acceptance criteria**:
- `ClaimValue::as_str()` helper returns `Some(&str)` for the `String` variant.
- `ClaimSet::roles()` returns the first of `roles`, `realm_access.roles`, or `groups` that is present and non-empty (unit test covers all three paths + empty fallback).
- `ClaimSet::tenant()` tries `tenant_id`, then `tid`, then `tenant`.
- `ClaimSet::scope()` tries `scp`, then `scope`.
- `ClaimSet` and `ClaimValue` are `#[non_exhaustive]` on `ClaimValue` only (as specced).
- `cargo test -p ego-domain` passes.
- No `serde_json` dependency in `ego-domain/Cargo.toml`.

**TDD note**: write unit tests for `roles()` (all three claim paths + fallback), `tenant()`,
`scope()`, `get_nested_array`, and `ClaimValue` construction before implementing.

---

## Phase 2: security-sdk

### T-02 — PrincipalMapper trait

**Crate**: `crates/security-sdk`
**File**: `crates/security-sdk/src/principal_mapper.rs` (new, replaces `claims_mapper.rs` if it exists)
**Spec req**: US-006, AD-OIDC-002, INV-8, AD-OIDC-003

**Description**:
Add the `PrincipalMapper` trait to `security-sdk`:

```rust
pub trait PrincipalMapper: Send + Sync {
    fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError>;
}
```

Wire into `security-sdk/src/lib.rs` as a `pub` re-export.
`DefaultPrincipalMapper` is NOT re-exported from `security-sdk` — it lives in `security-jwt`.

**Acceptance criteria**:
- Trait is `pub` in `security-sdk`.
- A custom impl compiles without importing anything from `security-jwt`.
- `Arc<dyn PrincipalMapper>` is object-safe (compile-time assertion test).
- `serde_json` is NOT added to `security-sdk/Cargo.toml`.
- `cargo test -p ego-security-sdk` passes.

**Depends on**: T-01 (needs `ClaimSet` from `ego-domain`)

**TDD note**: write an object-safety assertion test and a minimal stub impl test before adding the trait.

---

### T-03 — CredentialExtractor, RequestContext, built-in extractors

**Crate**: `crates/security-sdk`
**File**: `crates/security-sdk/src/credential_extractor.rs` (new)
**Spec req**: US-001, AD-OIDC-011, INV-9

**Description**:
Add to `security-sdk`:

```rust
pub trait RequestContext: Send + Sync {
    fn header(&self, name: &str) -> Option<&str>;
    fn metadata(&self, key: &str) -> Option<&str>;
    fn query_param(&self, name: &str) -> Option<&str>;
}

pub trait CredentialExtractor: Send + Sync {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError>;
}

pub struct BearerExtractor;    // Authorization: Bearer <token>
pub struct BasicExtractor;     // Authorization: Basic <b64>
pub struct ApiKeyExtractor { pub header_name: String }
```

`BearerExtractor::extract` reads `Authorization: Bearer <t>` → `Some(Credential::Bearer(t))`.
`BasicExtractor::extract` reads `Authorization: Basic <b64>` → `Some(Credential::Basic { username, secret })` (base64 decode user:pass).
`ApiKeyExtractor::extract` reads `header_name` header → `Some(Credential::Bearer(value))`.

Wire into `security-sdk/src/lib.rs`.

**Acceptance criteria**:
- `BearerExtractor` correctly parses `Authorization: Bearer tok-123` → `Some(Credential::Bearer("tok-123"))`.
- `BearerExtractor` returns `None` when `Authorization` header is absent.
- `BearerExtractor` returns `Err(InvalidToken)` when header is malformed (present but not `Bearer ...`).
- `BasicExtractor` decodes a valid base64 `user:pass` pair correctly.
- `ApiKeyExtractor` reads from the configured header name.
- All three implement `Send + Sync` (compile-time assertion).
- `cargo test -p ego-security-sdk` passes.

**Depends on**: T-01 (for `AuthenticationError` from domain — already there; no new dep needed)

**TDD note**: write extraction tests with a minimal `MockRequestContext` (struct with a `HashMap` of headers) before implementing.

**Parallel with**: T-05/T-06 after T-02 is merged.

---

### T-04 — AuthenticationInterceptor

**Crate**: `crates/security-sdk`
**File**: `crates/security-sdk/src/authentication/interceptor.rs` (new)
**Spec req**: US-001, INV-9, AD-OIDC-011

**Description**:
Add the transport-agnostic authentication engine to `security-sdk`:

```rust
pub struct AuthenticationInterceptor {
    extractor: Arc<dyn CredentialExtractor>,
    provider: Arc<dyn AuthenticationProvider>,
}

impl AuthenticationInterceptor {
    pub fn new(extractor: Arc<dyn CredentialExtractor>, provider: Arc<dyn AuthenticationProvider>) -> Self;

    pub fn intercept(
        &self,
        ctx: &dyn RequestContext,
        service_ctx: &mut ServiceContext,
    ) -> Result<(), AuthenticationError>;
}
```

Logic: extract → if `Some(credential)` → authenticate → populate `service_ctx.security`;
if `None` → pass through; on error → propagate (never map to HTTP).

Remove `AuthenticationInterceptor` from `transport` crate if it exists there.

Wire into `security-sdk/src/authentication/mod.rs` and `security-sdk/src/lib.rs`.

**Acceptance criteria**:
- `intercept` populates `service_ctx` on a valid credential.
- `intercept` passes through (no error) when no credential is present.
- `intercept` propagates `AuthenticationError` from the provider — does NOT return an HTTP response type.
- The interceptor is `Send + Sync`.
- `cargo test -p ego-security-sdk` passes.

**Depends on**: T-02, T-03

**TDD note**: mock both `CredentialExtractor` and `AuthenticationProvider` in the test module before implementing.

---

## Phase 3: security-jwt — Infrastructure

### T-05 — Cargo.toml: add new dependencies

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/Cargo.toml`
**Spec req**: design §Build & Feature Flags

**Description**:
Add to `[dependencies]`:
```toml
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
tokio   = { version = "1", features = ["rt", "sync", "time"] }
url     = "2"
sha2    = "0.10"
base64  = "0.22"
```

Add to `[features]`:
```toml
test-kit = []
```

Verify workspace `Cargo.toml` has `reqwest` as a workspace dep (add if absent).

**Acceptance criteria**:
- `cargo check -p security-jwt` compiles with the new deps.
- `cargo check -p security-jwt --no-default-features` compiles (no test-kit symbols bleed in).
- No duplicate `tokio` version conflict in `cargo tree`.

**Parallel with**: T-02, T-03 (no code dependency)

---

### T-06 — OidcProviderConfig, TokenFormat, MultiIssuerConfig

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/oidc_config.rs` (new)
**Spec req**: US-002, US-003b, INV-10, INV-11, design §Configuration

**Description**:
Add:
```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub enum TokenFormat { Auto, Jwt, Opaque }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OidcProviderConfig {
    pub issuer_url: Option<url::Url>,
    pub jwks_uri: Option<url::Url>,
    pub expected_iss: Option<String>,
    pub expected_aud: Option<Vec<String>>,
    pub clock_skew_seconds: Option<u64>,
    pub jwks_refresh_ttl_seconds: Option<u64>,
    pub token_format: Option<TokenFormat>,
    pub introspection_endpoint: Option<url::Url>,
    pub introspection_client_id: Option<String>,
    pub introspection_client_secret: Option<String>,
    pub introspection_cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MultiIssuerConfig {
    pub issuers: HashMap<String, OidcProviderConfig>,
}
```

Validation: `OidcProviderConfig::validate()` returns `Err(AuthenticationError::ProviderUnavailable)` when
both `issuer_url` and `jwks_uri` are `None`. Called at provider construction time.

INV-11: `introspection_endpoint`, if present, must be `https` scheme unless host is `localhost` or `127.0.0.1`.

Wire into `security-jwt/src/lib.rs`.

**Acceptance criteria**:
- `OidcProviderConfig` with neither URL field returns `ProviderUnavailable` from `validate()`.
- `jwks_uri` wins when both are set (test: `validate()` returns `Ok`; the `jwks_uri` field is `Some`).
- HTTP introspection endpoint (non-localhost) fails validation.
- localhost HTTP introspection endpoint passes validation.
- `serde::Deserialize` round-trip test for `TokenFormat::Auto`, `Jwt`, `Opaque`.
- `cargo test -p security-jwt` passes.

**Depends on**: T-05

---

### T-07 — JwksKeyResolver, JwksProvider, HttpJwksProvider

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/jwks.rs` (new)
**Spec req**: US-005, INV-7, AD-OIDC-009

**Description**:
Add the public SPI trait:
```rust
pub trait JwksProvider: Send + Sync {
    async fn fetch_jwks(&self, jwks_uri: &url::Url) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError>;
}
pub struct HttpJwksProvider { /* reqwest::Client */ }
impl JwksProvider for HttpJwksProvider { ... }
```

Add `JwksKeyResolver`:
- State: `cache: Arc<tokio::sync::RwLock<HashMap<Option<String>, VerificationKey>>>`,
  `jwks_uri: url::Url`, `provider: Arc<dyn JwksProvider>`.
- `new(jwks_uri, cache_ttl)`: uses `HttpJwksProvider`; warm-up fetch via RESOLVER_POOL at construction.
- `with_provider(jwks_uri, cache_ttl, provider)`: injectable for tests.
- `resolve(kid, alg)`: read-lock hit → return clone; miss → one forced RESOLVER_POOL refresh, re-read; still missing → `KeyNotFound`.
- Background refresh: `tokio::spawn` + `tokio::time::interval(cache_ttl)`. Failure → `warn!`, keep stale (exponential backoff on consecutive failures is a stretch goal, not blocking).

Implement `KeyResolver` for `JwksKeyResolver` so it plugs into the existing `authenticate_inner` path.

JWK → `VerificationKey` conversion: use `jsonwebtoken::jwk::JwkSet` from the existing `jsonwebtoken = "9"` dep; convert to `VerificationKey::RsaPem` / `VerificationKey::EcPem` via PEM re-encoding.

**Acceptance criteria**:
- Cache hit returns key without HTTP (use `FakeJwks` in tests — implement a minimal inline `FakeJwks` here for unit tests; the full test-kit version is T-14).
- Cache miss triggers exactly one fetch (assert call count with a counting fake).
- Stale cache is retained when refresh fails (set fake to fail, verify old key still returned).
- 10 concurrent `resolve` calls with a populated cache produce no deadlock (simple thread-spawn test).
- INV-7: write lock is only held during cache replacement — unit-test comment + `RwLock` usage confirms it.
- `cargo test -p security-jwt` passes.

**Depends on**: T-06
**Parallel with**: T-08, T-09, T-10

---

### T-08 — DiscoveryProvider, HttpDiscoveryProvider, OidcEndpoints

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/discovery.rs` (new)
**Spec req**: US-002

**Description**:
```rust
// OidcEndpoints is pub — callers receive it as the result of discovery.
pub struct OidcEndpoints {
    pub jwks_uri: url::Url,
    pub introspection_endpoint: Option<url::Url>,
}

// DiscoveryProvider is pub(crate) — internal implementation detail, not a public SPI.
pub(crate) trait DiscoveryProvider: Send + Sync {
    async fn fetch_configuration(&self, issuer_url: &url::Url) -> Result<OidcEndpoints, AuthenticationError>;
}

// Internal serde type — pub(crate) only
#[derive(serde::Deserialize)]
pub(crate) struct OidcConfiguration {
    pub jwks_uri: Option<url::Url>,
    pub introspection_endpoint: Option<url::Url>,
}

// HttpDiscoveryProvider is pub(crate) — internal default impl, not exposed as an extension point.
pub(crate) struct HttpDiscoveryProvider { /* reqwest::Client */ }
impl DiscoveryProvider for HttpDiscoveryProvider {
    // GETs {issuer_url}/.well-known/openid-configuration
    // Returns ProviderUnavailable on HTTP error
    // Returns ProviderUnavailable("jwks_uri absent") if OidcConfiguration.jwks_uri is None (AD-OIDC-005, OQ-5)
}
```

**Acceptance criteria**:
- `OidcConfiguration` with `jwks_uri: None` → `HttpDiscoveryProvider` returns `ProviderUnavailable`.
- `OidcEndpoints` has only `jwks_uri` and `introspection_endpoint` — no other discovery fields are public (INV-2).
- `OidcConfiguration` is `pub(crate)` only.
- `DiscoveryProvider` trait is `pub(crate)` only — not re-exported from `security-jwt`'s public API.
- `HttpDiscoveryProvider` is `pub(crate)` only.
- Unit test: a `FakeDiscovery` inline stub (also `pub(crate)`) returning a known `OidcEndpoints` is used to verify the `OidcAuthenticationProvider` construction path (full FakeDiscovery in T-14, under `#[cfg(feature = "test-kit")]`).
- `cargo test -p security-jwt` passes.

**Depends on**: T-06
**Parallel with**: T-07, T-09, T-10

---

### T-09 — IntrospectionProvider, HttpIntrospectionProvider, IntrospectionAuthenticationProvider

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/introspection.rs` (new)
**Spec req**: US-004, INV-4, INV-6

**Description**:
```rust
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

pub struct IntrospectionResult {
    pub active: bool,
    pub claims: Option<ClaimSet>,
}

pub trait IntrospectionProvider: Send + Sync {
    async fn introspect(
        &self,
        token: &str,
        endpoint: &url::Url,
        credentials: &ClientCredentials,
    ) -> Result<IntrospectionResult, AuthenticationError>;
}

pub struct HttpIntrospectionProvider { /* reqwest::Client */ }
impl IntrospectionProvider for HttpIntrospectionProvider {
    // RFC 7662: POST form body token=<token>, Basic auth with client_id:client_secret
    // active:false → IntrospectionResult { active: false, claims: None }
    // active:true → IntrospectionResult { active: true, claims: Some(ClaimSet from response fields) }
    // HTTP error → ProviderUnavailable
}

// Internal serde type — pub(crate)
#[derive(serde::Deserialize)]
pub(crate) struct IntrospectionResponse {
    pub active: bool,
    #[serde(flatten)]
    pub claims: BTreeMap<String, serde_json::Value>,
}

pub struct IntrospectionAuthenticationProvider {
    // provider: Arc<dyn IntrospectionProvider>
    // endpoint: url::Url
    // credentials: ClientCredentials
    // mapper: Arc<dyn PrincipalMapper>
    // cache: Option<(u64, Arc<tokio::sync::RwLock<HashMap<[u8;32], (i64, SecurityContext)>>>)>
    // clock: Arc<dyn Clock>
}

impl IntrospectionAuthenticationProvider {
    pub fn new(config: OidcProviderConfig, clock: Arc<dyn Clock>, mapper: Arc<dyn PrincipalMapper>) -> Result<Self, AuthenticationError>;
}

impl AuthenticationProvider for IntrospectionAuthenticationProvider {
    // Pre-check: token.len() > 8192 → InvalidToken
    // Cache lookup (if enabled): SHA-256 key, clock-based TTL
    // Miss: introspect via RESOLVER_POOL
    // active:false → InvalidToken; active:true + claims:None → ProviderUnavailable
    // success: serde_json::Value → ClaimValue → ClaimSet → mapper.map → SecurityContext
}
```

INV-6: cache TTL comparison uses `clock.now()`, not `Instant`.

**Acceptance criteria**:
- `active:true` → `Ok(SecurityContext)`.
- `active:false` → `Err(InvalidToken)`.
- `active:true, claims:None` → `Err(ProviderUnavailable)` (protocol error per spec).
- Network failure → `Err(ProviderUnavailable)`.
- Token > 8 KiB → `Err(InvalidToken)` before any I/O.
- Cache disabled by default: two calls to `authenticate` with a counting `FakeIntrospection` make two HTTP calls.
- Cache enabled: second call within TTL returns cached result with zero additional introspection calls (use `FixedClock` from test helpers).
- `IntrospectionResponse` is `pub(crate)` only.
- `cargo test -p security-jwt` passes.

**Depends on**: T-06, T-02 (for `PrincipalMapper`)
**Parallel with**: T-07, T-08, T-10

---

### T-10 — DefaultPrincipalMapper

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/principal_mapper.rs` (new)
**Spec req**: US-006, AD-OIDC-002, INV-3

**Description**:
```rust
pub struct DefaultPrincipalMapper;

impl PrincipalMapper for DefaultPrincipalMapper {
    fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError> {
        // sub (required) → subject_id; absent → MissingClaim("sub")
        // roles() → principal.roles
        // tenant() → principal.tenant_id
        // scope() → claims.custom["scope"]
        // organization() → claims.custom["organization"]
        // All remaining fields → claims.custom (via serde_json::Value → ClaimValue conversion)
    }
}
```

`serde_json::Value` → `ClaimValue` conversion: a `pub(crate) fn value_to_claim_value(v: serde_json::Value) -> ClaimValue` helper lives in this crate (not domain). Used here and in T-09 and T-12.

**Acceptance criteria**:
- `sub = "user-1"` maps to `principal.subject_id = "user-1"`.
- `roles = ["admin"]` maps to `principal.roles` containing `"admin"`.
- `realm_access.roles = ["editor"]` (Keycloak nested) maps to `principal.roles`.
- `groups = ["viewers"]` maps to `principal.roles` (fallback path).
- `tid = "tenant-42"` maps to `principal.tenant_id = Some("tenant-42")`.
- `scp = "read write"` maps to `claims.custom["scope"]`.
- `organization = "acme"` maps to `claims.custom["organization"]`.
- Missing `sub` → `Err(MissingClaim("sub"))`.
- `cargo test -p security-jwt` passes.

**Depends on**: T-02 (for `PrincipalMapper` trait), T-01 (for `ClaimSet`)
**Parallel with**: T-07, T-08, T-09

---

## Phase 4: security-jwt — Providers

### T-11 — JwtValidationEngine: inject PrincipalMapper

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/validation.rs` (modify existing)
**Spec req**: US-006, INV-3, design §Reuse JwtValidationEngine

**Description**:
Refactor `JwtValidationEngine::validate` to accept `mapper: &dyn PrincipalMapper` and
a `ClaimSet` produced from the decoded `BTreeMap<String, Value>`. Extract the inline
`extract_subject`/`extract_roles`/`extract_tenant_id` block into `DefaultPrincipalMapper::map`.

The engine still owns: signature verification, `exp`/`nbf`/`iss`/`aud` checks, and `ClaimSet`
assembly (via `value_to_claim_value`). The `(Principal, Claims)` pair now comes from the
injected mapper.

`Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`, `Es256AuthenticationProvider`:
gain an optional `mapper` field (default: `Arc<DefaultPrincipalMapper>`). Constructors
keep their 3-arg form; add a `with_mapper` builder method. Existing tests pass unchanged
because `DefaultPrincipalMapper` reproduces the existing behavior verbatim.

**Acceptance criteria**:
- All existing tests in `validation.rs` and `authenticator.rs` pass without modification (behavior is identical for existing providers using `DefaultPrincipalMapper`).
- A new test confirms that a custom `PrincipalMapper` injected into `Rs256AuthenticationProvider::with_mapper(...)` is called exactly once per `authenticate` call (tracking mapper).
- `cargo test -p security-jwt` passes.

**Depends on**: T-10

---

### T-12 — OidcAuthenticationProvider

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/oidc_provider.rs` (new)
**Spec req**: US-001, US-002, US-003, US-003b, INV-1, INV-6

**Description**:
```rust
pub struct OidcAuthenticationProvider { /* opaque */ }

impl OidcAuthenticationProvider {
    pub fn new(
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Self, AuthenticationError>;
    // Construction:
    // 1. validate config (T-06)
    // 2. if jwks_uri → use directly; if issuer_url only → call DiscoveryProvider via RESOLVER_POOL
    // 3. build JwksKeyResolver (warm-up in constructor)
    // 4. if introspection fields present → build IntrospectionAuthenticationProvider
}

impl AuthenticationProvider for OidcAuthenticationProvider {
    fn authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
        // Pre-check: token.len() > 8192 → InvalidToken
        // detect TokenFormat (config or Auto)
        //   Jwt  → authenticate_inner(..., JwksKeyResolver, mapper)
        //   Opaque → introspection provider (or InvalidToken if absent)
        //   Auto → detect by 2-dot heuristic
    }
}
```

Auto heuristic: count exactly two dots + both adjacent segments are valid base64url → JWT; else opaque.

INV-11: construction must fail when `introspection_endpoint` is http:// non-localhost and either
`introspection_client_id` or `introspection_client_secret` is absent.

**Acceptance criteria**:
- Valid RS256 JWT → `Ok(SecurityContext)` with correct subject_id (use inline FakeJwks resolver).
- Valid ES256 JWT → `Ok(SecurityContext)`.
- Expired JWT → `Err(ExpiredToken)` (use `FixedClock`).
- Tampered signature → `Err(InvalidSignature)`.
- Token > 8 KiB → `Err(InvalidToken)`.
- `issuer_url` config → discovery called at construction (use inline `FakeDiscovery`).
- `jwks_uri` config → no discovery call.
- Both fields → `jwks_uri` wins, no discovery call.
- Neither field → `Err(ProviderUnavailable)` at construction.
- `TokenFormat::Auto` with a well-formed JWT string → JWT path.
- `TokenFormat::Auto` with no dots → opaque path (or `InvalidToken` if no introspection).
- `TokenFormat::Opaque` with a dot-containing token → introspection path.
- `cargo test -p security-jwt` passes.

**Depends on**: T-07, T-08, T-11

---

### T-13 — MultiIssuerAuthenticationProvider, IssuerResolver, StaticIssuerResolver

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/multi_issuer.rs` (new)
**Spec req**: US-007, INV-4, AD-OIDC-004

**Description**:
```rust
pub trait IssuerResolver: Send + Sync {
    fn resolve(&self, issuer: &str) -> Option<Arc<dyn AuthenticationProvider>>;
}

pub struct StaticIssuerResolver {
    providers: HashMap<String, Arc<dyn AuthenticationProvider>>,
}
impl StaticIssuerResolver {
    pub fn new(providers: HashMap<String, Arc<dyn AuthenticationProvider>>) -> Self;
}
impl IssuerResolver for StaticIssuerResolver { ... }

pub struct MultiIssuerAuthenticationProvider {
    resolver: Arc<dyn IssuerResolver>,
}
impl MultiIssuerAuthenticationProvider {
    pub fn new(resolver: Arc<dyn IssuerResolver>) -> Self;
}

impl AuthenticationProvider for MultiIssuerAuthenticationProvider {
    fn authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
        // Pre-check: token.len() > 8192 → InvalidToken
        // Extract raw bearer string
        // base64url-decode segment[1] (URL_SAFE_NO_PAD, RFC 4648 §5)
        // serde_json parse → extract "iss" (no jsonwebtoken decode, no signature)
        // resolver.resolve(iss) → Some(provider) → provider.authenticate(credential)
        //                       → None → Err(InvalidToken)
    }
}

// pub(crate) helpers
pub(crate) fn unverified_iss(token: &str) -> Option<String>;  // extracts iss from payload
```

INV-4: `iss` used only for routing; sub-provider re-validates `iss` post-signature.

**Acceptance criteria**:
- Token from issuer A routes to provider A and returns `Ok(SecurityContext)` (use two `FakeIssuer`-backed `Rs256AuthenticationProvider` instances).
- Token with unknown `iss` → `Err(InvalidToken)`.
- Token with `iss = "issuer-A"` but signed by issuer B → routes to provider A, fails signature → `Err(InvalidSignature)`.
- `Arc<MultiIssuerAuthenticationProvider>` is usable as `Arc<dyn AuthenticationProvider>` (compile-time test).
- A token whose payload base64url segment contains `-` or `_` is decoded correctly (table test for `unverified_iss`).
- Token > 8 KiB → `Err(InvalidToken)` before any decode.
- `cargo test -p security-jwt` passes.

**Depends on**: T-12

---

## Phase 5: TestKit

### T-14 — TestKit: FakeIssuer, FakeDiscovery, FakeJwks, FakeIntrospection

**Crate**: `crates/security-jwt`
**File**: `crates/security-jwt/src/test_kit/mod.rs` (new)
**Spec req**: US-008, INV-5, AD-OIDC-007

**Description**:
All types gated behind `#[cfg(feature = "test-kit")]`.

```rust
pub struct FakeIssuer { /* in-process RSA or EC key pair, clock */ }
impl FakeIssuer {
    pub fn new(clock: Arc<dyn Clock>) -> Self;  // defaults RS256
    pub fn with_algorithm(clock: Arc<dyn Clock>, algorithm: JwtAlgorithm) -> Self;
    pub fn issue_token(&self, claims: BTreeMap<String, ClaimValue>) -> String;
    pub fn jwks_resolver(&self) -> JwksKeyResolver;
}

pub struct FakeDiscovery { /* OidcEndpoints with FakeIssuer's jwks_uri */ }
impl FakeDiscovery {
    pub fn new(issuer: &FakeIssuer) -> Self;
}
impl DiscoveryProvider for FakeDiscovery { ... }

pub struct FakeJwks { /* returns FakeIssuer's public key */ }
impl FakeJwks {
    pub fn new(issuer: &FakeIssuer) -> Self;
}
impl JwksProvider for FakeJwks { ... }

pub struct FakeIntrospection { /* token → (active, Option<ClaimSet>) map */ }
impl FakeIntrospection {
    pub fn new() -> Self;
    pub fn set_response(&mut self, token: &str, active: bool);
    pub fn set_active_response(&mut self, token: &str, claims: ClaimSet);
}
impl IntrospectionProvider for FakeIntrospection { ... }
```

`FakeIssuer::issue_token` converts `BTreeMap<String, ClaimValue>` → `serde_json::Value` map,
signs with the in-process key via `jsonwebtoken::encode`.

`FakeIssuer::jwks_resolver()` returns a `JwksKeyResolver::with_provider(fake_uri, Duration::MAX, Arc::new(FakeJwks::new(self)))`.

**Acceptance criteria**:
- `FakeIssuer` tokens pass real RS256 JWKS validation through `OidcAuthenticationProvider` (no HTTP).
- `FakeIssuer::with_algorithm(clock, JwtAlgorithm::Es256)` produces tokens that pass ES256 validation.
- `FixedClock` with `exp = T - 1` → deterministically `Err(ExpiredToken)`.
- `FakeIntrospection::set_response("tok", false)` → `Err(InvalidToken)`.
- `FakeIntrospection::set_active_response("tok", claim_set)` → `Ok(SecurityContext)`.
- `cargo build --release --no-default-features -p security-jwt` produces zero `FakeIssuer`/`FakeDiscovery`/`FakeJwks`/`FakeIntrospection` symbols (verified in T-16).
- `cargo test --features test-kit -p security-jwt` passes.

**Depends on**: T-12, T-09, T-08

---

## Phase 6: Integration Tests and CI

### T-15 — Integration tests (all US acceptance criteria)

**Crate**: `crates/security-jwt/tests/` or `tests/` module in the crate
**Spec req**: All US-001 through US-008 acceptance criteria

**Description**:
Write integration tests that exercise the complete happy paths and critical error paths
using only the TestKit (no live IdP). Each test maps to a spec scenario.

Cover at minimum:
1. **US-001**: `AuthenticationInterceptor` with `BearerExtractor` + `OidcAuthenticationProvider` + `FakeIssuer` → `Ok(SecurityContext)`.
2. **US-002**: `issuer_url` → `FakeDiscovery` called at construction; `jwks_uri` → no discovery call; both → `jwks_uri` wins; neither → construction error.
3. **US-003**: RS256 valid; ES256 valid; unknown `kid` triggers one forced refresh; `iss` mismatch post-signature → `InvalidToken`.
4. **US-003b**: `Auto` + JWT-shaped → JWT path; `Auto` + no-dots → opaque path; `Opaque` + dotted → introspection; `Auto` + JWT-looking-but-failing → `InvalidToken`.
5. **US-004**: `active:true` → `Ok`; `active:false` → `Err(InvalidToken)`; network error → `Err(ProviderUnavailable)`; `IntrospectionResult` type invisible to caller.
6. **US-005**: hot-path resolve uses cache; miss triggers one fetch; 10 concurrent resolves succeed; refresh failure keeps stale cache.
7. **US-006**: `DefaultPrincipalMapper` maps all standard claims; custom mapper `preferred_username` → `subject_id`; mapper called exactly once; missing `sub` → `MissingClaim`.
8. **US-007**: known issuer routes correctly; unknown `iss` → `InvalidToken`; forged `iss` → `InvalidSignature`; `Arc<dyn AuthenticationProvider>` compile check.
9. **Multi-issuer end-to-end**: two FakeIssuers, tokens from each authenticated correctly.

**Acceptance criteria**:
- All 34 spec scenarios from the coverage table are covered by at least one test.
- `cargo test --workspace` passes.
- Zero tests require a live IdP or network access.

**Depends on**: T-14

---

### T-16 — CI guard: no test-kit symbols in release build

**Crate**: workspace
**Spec req**: US-008 scenario "No TestKit symbol in release build", INV-5, AD-OIDC-007

**Description**:
Add a shell script or Makefile target (or document a CI step) that runs:
```bash
cargo build --release --no-default-features -p security-jwt 2>&1
# Then verify no test-kit symbols:
nm -g target/release/libsecurity_jwt.rlib 2>/dev/null | grep -E "FakeIssuer|FakeDiscovery|FakeJwks|FakeIntrospection" && exit 1 || exit 0
```

If the project already has a CI Makefile/justfile, add the target there.
If not, document the command in a comment in `Cargo.toml`'s `[features]` section.

**Acceptance criteria**:
- `cargo build --release --no-default-features -p security-jwt` succeeds.
- No `FakeIssuer`, `FakeDiscovery`, `FakeJwks`, or `FakeIntrospection` symbol appears in the release artifact.

**Depends on**: T-14

---

## Line Budget Note

Estimated production code: **~1,800–2,500 lines** across all crates (excluding integration tests).

Primary contributors:
- `ClaimSet` + `ClaimValue` + all helpers: ~150 lines
- `PrincipalMapper`, `CredentialExtractor`, `RequestContext` traits + built-in impls: ~120 lines
- `AuthenticationInterceptor`: ~40 lines
- `OidcProviderConfig` + `MultiIssuerConfig` + validation: ~100 lines
- `JwksKeyResolver` + `JwksProvider` + background refresh: ~200 lines
- `DiscoveryProvider` (internal) + `HttpDiscoveryProvider`: ~80 lines
- `IntrospectionProvider` + `HttpIntrospectionProvider` + `IntrospectionAuthenticationProvider` + cache: ~250 lines
- `DefaultPrincipalMapper`: ~80 lines
- `JwtValidationEngine` refactor: ~60 lines
- `OidcAuthenticationProvider`: ~150 lines
- `MultiIssuerAuthenticationProvider` + `IssuerResolver`: ~100 lines
- TestKit (`FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`): ~250 lines
- Unit tests (per-module): ~600–900 lines
- Integration tests: ~200–400 lines additional

The previous estimate of ~420–450 LOC counted only new production code in the narrowest
sense. The real implementation scope — including config validation, background tasks,
cache logic, TestKit, and tests — is significantly larger. The single-PR delivery is still
appropriate given the coherent scope, but reviewers should plan accordingly.

---

## Conformance Gates for sdd-verify

Three gates must stay green throughout implementation. `sdd-verify` MUST check each one.

### Gate 1 — Public API
Compare the final public API surface against the SPI visibility table in spec.md Section 1.4.
Any `pub` type not in that table requires an explicit architectural justification; otherwise downgrade to `pub(crate)`.

### Gate 2 — Pipeline uniqueness
Every authentication path (JWT, Introspection, Multi-Issuer) MUST terminate through:
```
decode / verify
    ↓
ClaimSet
    ↓
PrincipalMapper
    ↓
SecurityContext
```
No provider may construct `SecurityContext` directly from raw claims. Grep for `SecurityContext {` and `SecurityContext::new` across all new code and verify each call site goes through a `PrincipalMapper`.

### Gate 3 — Sync boundary
`AuthenticationProvider::authenticate()` MUST be entirely synchronous. Verify:
- No `.await` expression inside any `authenticate()` implementation.
- All I/O (JWKS fetch, discovery, introspection) is either pre-loaded at construction or dispatched through `RESOLVER_POOL` / Tokio background tasks.
- `cargo clippy` with `async_fn_in_trait` lint enabled produces no new warnings on the auth hot path.

---

## Summary Table

| ID  | Phase         | Crate          | Parallel OK? | Blocks  |
|-----|--------------|----------------|-------------|---------|
| T-01 | ego-domain  | `domain`        | —           | T-02, T-05 |
| T-02 | security-sdk | `security-sdk` | after T-01  | T-03, T-04, T-09, T-10 |
| T-03 | security-sdk | `security-sdk` | after T-02  | T-04 |
| T-04 | security-sdk | `security-sdk` | after T-03  | T-15 |
| T-05 | security-jwt | `security-jwt` | after T-01  | T-06 |
| T-06 | security-jwt | `security-jwt` | after T-05  | T-07..T-10 |
| T-07 | security-jwt | `security-jwt` | with T-08..T-10 | T-12 |
| T-08 | security-jwt | `security-jwt` | with T-07, T-09, T-10 | T-12 |
| T-09 | security-jwt | `security-jwt` | with T-07, T-08, T-10 | T-14 |
| T-10 | security-jwt | `security-jwt` | with T-07, T-08, T-09 | T-11 |
| T-11 | security-jwt | `security-jwt` | after T-10  | T-12 |
| T-12 | security-jwt | `security-jwt` | after T-07, T-08, T-11 | T-13 |
| T-13 | security-jwt | `security-jwt` | after T-12  | T-15 |
| T-14 | security-jwt | `security-jwt` | after T-12, T-09, T-08 | T-15, T-16 |
| T-15 | integration  | `security-jwt` | after T-14  | — |
| T-16 | CI guard     | workspace      | after T-14  | — |
