# Specification: oauth2-oidc — OIDC Resource Server Framework

_Date: 2026-06-28 | Status: ready-for-design_

> **Scope note**: This capability covers Resource Server authentication only (validating tokens issued by an external IdP). OAuth2 Client flows (Client Credentials, Authorization Code, PKCE, Refresh Token) are deferred to CORE-021.

---

## Part 1: Contracts

### 1.1 `ego-domain` — New Value Object

```rust
// Crate: ego-domain | Layer: domain | File: crates/domain/src/auth/claim_set.rs

/// Domain value object that carries the raw claims from a verified token.
/// Decouples `security-sdk` and consumers from `serde_json::Value`.
pub struct ClaimSet {
    pub raw: BTreeMap<String, ClaimValue>,
}

/// Raw claim value type. Supports the full range of claim structures including
/// nested maps (used by Cognito, Entra ID). Marked #[non_exhaustive] to allow
/// adding new variants without breaking existing implementations.
#[non_exhaustive]
pub enum ClaimValue {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<ClaimValue>),
    Map(BTreeMap<String, ClaimValue>),
    Null,
}

impl ClaimSet {
    // --- Raw access (base layer) ---
    pub fn get_str(&self, key: &str) -> Option<&str> { ... }
    pub fn get_array(&self, key: &str) -> Option<&[ClaimValue]> { ... }
    /// Returns an array nested one level deep: `outer.inner` (e.g. `realm_access.roles`).
    pub fn get_nested_array(&self, outer: &str, inner: &str) -> Option<&[ClaimValue]> { ... }
    /// Returns a numeric claim as `i64` (truncates fractional part).
    pub fn get_i64(&self, key: &str) -> Option<i64> { ... }

    /// Standard claim helpers — these cover every claim path read by `DefaultPrincipalMapper`.
    /// Custom `PrincipalMapper` implementations can use `get_str` / `get_array` for any
    /// vendor-specific claim not listed here.

    // --- Standard identity helpers (all claims handled by DefaultPrincipalMapper) ---

    /// Subject identifier (`sub` claim). Always present in valid OIDC tokens.
    pub fn subject(&self) -> Option<&str> { self.get_str("sub") }

    /// Roles from `roles`, `realm_access.roles` (Keycloak-style nested), or `groups` — first present wins.
    pub fn roles(&self) -> Vec<&str> {
        self.get_array("roles")
            .or_else(|| self.get_nested_array("realm_access", "roles"))
            .or_else(|| self.get_array("groups"))
            .map(|v| v.iter().filter_map(|c| c.as_str()).collect())
            .unwrap_or_default()
    }

    /// Tenant identifier from `tenant_id`, `tid` (Entra ID), or `tenant`.
    pub fn tenant(&self) -> Option<&str> {
        self.get_str("tenant_id")
            .or_else(|| self.get_str("tid"))
            .or_else(|| self.get_str("tenant"))
    }

    /// OAuth2 scopes from `scp` (Azure/Entra ID) or `scope`.
    pub fn scope(&self) -> Option<&str> {
        self.get_str("scp").or_else(|| self.get_str("scope"))
    }

    /// Organization identifier from `organization` or `org_id`.
    pub fn organization(&self) -> Option<&str> {
        self.get_str("organization").or_else(|| self.get_str("org_id"))
    }

    /// Token expiry timestamp (`exp` claim).
    pub fn expiry(&self) -> Option<i64> { self.get_i64("exp") }

    /// Token issuer (`iss` claim).
    pub fn issuer(&self) -> Option<&str> { self.get_str("iss") }
}
```

---

### 1.2 `security-sdk` — New Public Traits

```rust
// Crate: security-sdk | Layer: port (domain-facing) | Visibility: pub

/// Extracts a Credential from an incoming request.
/// Decouples AuthenticationInterceptor from any specific credential scheme.
pub trait CredentialExtractor: Send + Sync {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError>;
}

/// Deliberately minimal — evolves as transports are added (cookie, peer_certificate, path_param).
/// Implementations: HttpRequestContext (http feature), TonicRequestContext (grpc feature, future).
pub trait RequestContext: Send + Sync {
    fn header(&self, name: &str) -> Option<&str>;
    fn metadata(&self, key: &str) -> Option<&str>;
    fn query_param(&self, name: &str) -> Option<&str>;
}

// Built-in extractors shipped with security-sdk:
pub struct BearerExtractor;    // Authorization: Bearer <token>
pub struct BasicExtractor;     // Authorization: Basic <b64>
pub struct ApiKeyExtractor { pub header_name: String }

/// Maps a verified ClaimSet to a (Principal, Claims) pair.
/// Input is a domain ClaimSet — no serde_json::Value in this signature.
pub trait PrincipalMapper: Send + Sync {
    fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError>;
}
```

`DefaultPrincipalMapper` is implemented in `security-jwt` and is NOT re-exported from `security-sdk`.
No OIDC protocol type (`JwkSet`, `OidcConfiguration`, `IntrospectionResponse`, etc.) MUST appear in any `pub` API of `security-sdk`.

---

### 1.3 `security-sdk` — `AuthenticationInterceptor` (authentication engine)

```rust
// Crate: security-sdk | File: src/authentication/interceptor.rs

/// Transport-agnostic authentication interceptor.
/// Extracts credentials, authenticates, and populates ServiceContext.
/// Never produces transport-specific responses — propagates AuthenticationError to caller.
/// Transport adapters (security-http, security-grpc, etc.) map AuthenticationError
/// to the appropriate protocol response (HTTP 401, gRPC UNAUTHENTICATED, etc.).
pub struct AuthenticationInterceptor {
    extractor: Arc<dyn CredentialExtractor>,
    provider: Arc<dyn AuthenticationProvider>,
}

impl AuthenticationInterceptor {
    pub fn new(
        extractor: Arc<dyn CredentialExtractor>,
        provider: Arc<dyn AuthenticationProvider>,
    ) -> Self;

    /// Returns Ok(()) with SecurityContext populated in `service_ctx`, or Err(AuthenticationError).
    /// Does NOT map errors to HTTP responses. Callers (transport adapters) handle mapping.
    pub fn intercept(
        &self,
        ctx: &dyn RequestContext,
        service_ctx: &mut ServiceContext,
    ) -> Result<(), AuthenticationError>;
}
```

Authentication interception is a security concern, not a transport concern — `AuthenticationInterceptor` does NOT live in the `transport` crate. Transport adapters (`security-http`, `security-grpc`, `security-axum`, `security-actix`, etc.) each wrap the interceptor and map `AuthenticationError` to their protocol-specific response.

**Transport wiring**: `AuthenticationInterceptor` does NOT implement the `Interceptor` trait. Adapter crates call `intercept(ctx, &mut service_ctx)` directly while they still own the mutable `ServiceContext` — before handing off to the interceptor chain. The sequence is: `Adapter → AuthenticationInterceptor::intercept(&mut ServiceContext) → InterceptorChain`. No interior mutability, no `UnsafeCell`. This constraint is the adapter's responsibility, not `security-sdk`'s.

---

### 1.4 `security-jwt` — New Public API Surface

```rust
// Crate: security-jwt | Layer: adapter (infrastructure)

// --- Configuration ---

/// Full OIDC provider configuration. Derives Deserialize for kit-config integration.
/// All URL fields are `url::Url` — validated at construction time, not at use time.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OidcProviderConfig {
    /// Either issuer_url (Discovery) or jwks_uri (manual) MUST be present.
    pub issuer_url: Option<url::Url>,
    pub jwks_uri: Option<url::Url>,
    pub expected_iss: Option<String>,
    pub expected_aud: Option<Vec<String>>,
    /// Seconds of clock-skew tolerance. Default 0.
    pub clock_skew_seconds: Option<u64>,
    /// JWKS background refresh interval in seconds. Default 300.
    pub jwks_refresh_ttl_seconds: Option<u64>,
    /// Token format detection mode.
    pub token_format: Option<TokenFormat>,
    /// Introspection endpoint URL. Present when opaque token support is required.
    pub introspection_endpoint: Option<url::Url>,
    pub introspection_client_id: Option<String>,
    pub introspection_client_secret: Option<String>,
    /// Introspection response cache TTL in seconds. `None` = cache disabled (default: off).
    pub introspection_cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MultiIssuerConfig {
    /// Map of issuer string → OidcProviderConfig.
    pub issuers: HashMap<String, OidcProviderConfig>,
}

/// Controls how the provider decides whether a token is a JWT or opaque.
#[derive(Debug, Clone, serde::Deserialize)]
pub enum TokenFormat {
    /// Always validate as JWT.
    Jwt,
    /// Always use introspection.
    Opaque,
    /// Detect by format: two base64url segments separated by dots = JWT; else opaque.
    Auto,
}

// --- Infrastructure SPI Traits ---

/// Fetches OIDC discovery configuration for a given issuer URL.
/// Internal implementation detail — NOT part of the public SPI.
/// Custom discovery strategies are not a supported extension point.
pub(crate) trait DiscoveryProvider: Send + Sync {
    async fn fetch_configuration(&self, issuer_url: &url::Url) -> Result<OidcEndpoints, AuthenticationError>;
}

/// Minimal — only the endpoints needed for token validation.
/// Public: callers receive OidcEndpoints as the result of discovery; this struct is part of the public API.
/// Discovery document fields beyond jwks_uri and introspection_endpoint are not exposed.
pub struct OidcEndpoints {
    pub jwks_uri: url::Url,
    pub introspection_endpoint: Option<url::Url>,
}

/// Fetches and parses a JWKS from a given URI.
/// Custom implementations can load from Vault, files, k8s secrets, etc.
pub trait JwksProvider: Send + Sync {
    async fn fetch_jwks(&self, jwks_uri: &url::Url) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError>;
}

/// Credentials used to authenticate the resource server against the introspection endpoint.
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

/// Result of an RFC 7662 introspection call.
/// `active: false` maps to `Err(AuthenticationError::InvalidToken)`.
/// Protocol-specific fields beyond `active` and `claims` are NOT exposed.
/// If `active` is true, `claims` MUST be `Some`; an `active: true, claims: None`
/// result is a protocol error and MUST be treated as `AuthenticationError::ProviderUnavailable`.
pub struct IntrospectionResult {
    pub active: bool,
    /// Claims extracted from the introspection response; only present when `active` is true.
    pub claims: Option<ClaimSet>,
}

/// Calls an RFC 7662 introspection endpoint.
pub trait IntrospectionProvider: Send + Sync {
    async fn introspect(
        &self,
        token: &str,
        endpoint: &url::Url,
        credentials: &ClientCredentials,
    ) -> Result<IntrospectionResult, AuthenticationError>;
}

// --- HTTP implementations (the defaults) ---
pub(crate) struct HttpDiscoveryProvider { /* opaque: reqwest::Client */ }  // internal
pub struct HttpJwksProvider { /* opaque: reqwest::Client */ }
pub struct HttpIntrospectionProvider { /* opaque: reqwest::Client */ }

impl DiscoveryProvider for HttpDiscoveryProvider { ... }
impl JwksProvider     for HttpJwksProvider     { ... }
impl IntrospectionProvider for HttpIntrospectionProvider { ... }

// --- IssuerResolver SPI ---

/// Resolves an AuthenticationProvider for a given issuer string.
/// Allows dynamic or tenant-based routing beyond a static HashMap.
pub trait IssuerResolver: Send + Sync {
    fn resolve(&self, issuer: &str) -> Option<Arc<dyn AuthenticationProvider>>;
}

/// Default static implementation — providers registered at startup.
pub struct StaticIssuerResolver {
    providers: HashMap<String, Arc<dyn AuthenticationProvider>>,
}

impl StaticIssuerResolver {
    pub fn new(providers: HashMap<String, Arc<dyn AuthenticationProvider>>) -> Self;
}

// --- SPI Visibility Summary ---
//
// | SPI                    | Visibility              |
// |------------------------|-------------------------|
// | PrincipalMapper        | pub                     |
// | CredentialExtractor    | pub                     |
// | RequestContext         | pub                     |
// | JwksProvider           | pub                     |
// | IntrospectionProvider  | pub                     |
// | IssuerResolver         | pub                     |
// | DiscoveryProvider      | pub(crate) — internal   |

// --- Providers ---

/// Authenticates Credential::Bearer (JWT and opaque composite).
/// Implements AuthenticationProvider (sync).
pub struct OidcAuthenticationProvider { /* opaque */ }

impl OidcAuthenticationProvider {
    pub fn new(
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Self, AuthenticationError>;
}

/// `OidcAuthenticationProvider` MUST construct `ClientCredentials { client_id: introspection_client_id, client_secret: introspection_client_secret }`
/// from the corresponding `OidcProviderConfig` fields when instantiating the `IntrospectionAuthenticationProvider` for the opaque token path.
/// Both fields MUST be present; if either is absent and `token_format` includes the opaque path, construction MUST fail at startup.

/// Routes Bearer tokens to the sub-provider matching the unverified iss claim.
/// Implements AuthenticationProvider (sync).
pub struct MultiIssuerAuthenticationProvider { /* opaque */ }

impl MultiIssuerAuthenticationProvider {
    pub fn new(resolver: Arc<dyn IssuerResolver>) -> Self;
}

/// Validates opaque tokens via RFC 7662 introspection.
/// Implements AuthenticationProvider (sync).
pub struct IntrospectionAuthenticationProvider { /* opaque */ }

impl IntrospectionAuthenticationProvider {
    pub fn new(
        config: OidcProviderConfig,
        clock: Arc<dyn Clock>,
        mapper: Arc<dyn PrincipalMapper>,
    ) -> Result<Self, AuthenticationError>;
}

// --- Key Resolution ---

/// Resolves verification keys from a JWKS endpoint with in-memory cache.
/// Implements KeyResolver (async).
pub struct JwksKeyResolver { /* opaque */ }

impl JwksKeyResolver {
    /// Creates a resolver with the default HTTP JWKS provider.
    pub fn new(
        jwks_uri: url::Url,
        cache_ttl: std::time::Duration,
    ) -> Self;

    /// Creates a resolver with a custom JwksProvider (use in tests with FakeJwks).
    pub fn with_provider(
        jwks_uri: url::Url,
        cache_ttl: std::time::Duration,
        provider: Arc<dyn JwksProvider>,
    ) -> Self;
}

// --- Default Principal Mapper ---

/// Maps standard OIDC claims to Principal and Claims.
///
/// Claim paths read (in priority order per field):
/// - `principal.subject_id` ← `sub`
/// - `principal.roles`      ← `roles` | `realm_access.roles` (Keycloak) | `groups`
/// - `principal.tenant_id`  ← `tenant_id` | `tid` (Entra ID) | `tenant`
/// - `claims["scope"]`      ← `scp` (Azure/Entra ID) | `scope`
/// - `claims["organization"]` ← `organization` | `org_id`
///
/// These paths correspond exactly to the `ClaimSet` standard helpers above.
/// Custom `PrincipalMapper` implementations handle any claim not listed here.
pub struct DefaultPrincipalMapper;

impl PrincipalMapper for DefaultPrincipalMapper { /* ... */ }
```

---

### 1.5 `security-jwt` — TestKit (feature = "test-kit" only)

```rust
#[cfg(feature = "test-kit")]
pub mod test_kit {
    /// Generates real RS256-signed JWTs using an in-process key pair.
    pub struct FakeIssuer { /* opaque */ }

    impl FakeIssuer {
        pub fn new(clock: Arc<dyn Clock>) -> Self;
        /// Creates a FakeIssuer using the specified algorithm (e.g. `JwtAlgorithm::Es256`).
        pub fn with_algorithm(clock: Arc<dyn Clock>, algorithm: JwtAlgorithm) -> Self;
        /// Signs a token with the provided claims map.
        pub fn issue_token(&self, claims: BTreeMap<String, ClaimValue>) -> String;
        /// Returns a JwksKeyResolver backed by this issuer's public key.
        pub fn jwks_resolver(&self) -> JwksKeyResolver;
    }

    /// Returns a hard-coded OidcEndpoints without HTTP.
    pub struct FakeDiscovery { /* opaque */ }

    impl FakeDiscovery {
        pub fn new(issuer: &FakeIssuer) -> Self;
    }

    /// Returns the JwkSet for FakeIssuer's public key without HTTP.
    pub struct FakeJwks { /* opaque */ }

    impl FakeJwks {
        pub fn new(issuer: &FakeIssuer) -> Self;
    }

    /// Returns configurable introspection responses without HTTP.
    pub struct FakeIntrospection { /* opaque */ }

    impl FakeIntrospection {
        pub fn new() -> Self;
        /// Configures response for a specific raw token string.
        /// Use for `active: false` scenarios. For `active: true`, use `set_active_response`
        /// to supply the required `ClaimSet`.
        pub fn set_response(&mut self, token: &str, active: bool);
        /// Configures the fake to return an active introspection result with the given claims.
        /// Use this when `active: true` — `set_response(token, true)` alone is insufficient
        /// because a real `IntrospectionResult` requires `claims: Some(...)`.
        pub fn set_active_response(&mut self, token: &str, claims: ClaimSet);
    }
}
```

---

## Part 2: Requirements and Scenarios

---

### Requirement: Bearer Token Authentication (US-001)

The system MUST authenticate `Credential::Bearer(token)` and return `Ok(SecurityContext)` for a valid token.
The system MUST return `Err(AuthenticationError::ExpiredToken)` when `exp` is in the past relative to the injected `Clock`.
The system MUST return `Err(AuthenticationError::InvalidSignature)` for a tampered or unsigned token.
The system MUST NOT expose any OIDC-specific type in the return value or error visible to application code.
`authenticate()` MUST be synchronous — no async boundary crossed on the call path.
The `AuthenticationInterceptor` MUST use an injected `CredentialExtractor` — it MUST NOT hardcode Bearer extraction.

#### Scenario: Valid JWT extracted via BearerExtractor and authenticated

- GIVEN an `AuthenticationInterceptor` wired with a `BearerExtractor` and an `OidcAuthenticationProvider`
- AND a request carrying `Authorization: Bearer <valid_token>`
- WHEN the interceptor processes the request
- THEN `BearerExtractor::extract` returns `Some(Credential::Bearer(...))` and `authenticate` returns `Ok(SecurityContext)`

#### Scenario: Valid JWT returns SecurityContext

- GIVEN a `FakeIssuer` with a known key pair and an `OidcAuthenticationProvider` configured with its `JwksKeyResolver`
- WHEN `authenticate(&Credential::Bearer(valid_token))` is called
- THEN the result is `Ok(SecurityContext)` with `principal.subject_id` matching the token `sub` claim

#### Scenario: Expired token returns ExpiredToken

- GIVEN a `FakeIssuer` that issued a token with `exp = T` and a `FixedClock` returning `T + 1s`
- WHEN `authenticate` is called
- THEN the result is `Err(AuthenticationError::ExpiredToken)`

#### Scenario: Tampered signature returns InvalidSignature

- GIVEN a valid JWT token with its signature byte replaced
- WHEN `authenticate` is called
- THEN the result is `Err(AuthenticationError::InvalidSignature)`

#### Scenario: Not-yet-valid token (nbf in future) is rejected

- GIVEN a token with `nbf = T` and a `FixedClock` returning `T - 1s`
- WHEN `authenticate` is called
- THEN the result is `Err(AuthenticationError::InvalidToken("nbf not reached"))`

---

### Requirement: OIDC Discovery (US-002)

The system MUST fetch `{issuer_url}/.well-known/openid-configuration` at provider construction time when `issuer_url` is configured and `jwks_uri` is absent.
The system MUST use the explicit `jwks_uri` directly without any Discovery HTTP call when `jwks_uri` is provided.
The system MUST fail at construction (not at request time) when neither `issuer_url` nor `jwks_uri` is configured.
Discovery results MUST be cached in memory; re-fetch MUST NOT occur per `authenticate()` call.
The `DiscoveryProvider` trait is `pub(crate)` — it is an internal implementation detail; custom implementations are not a supported extension point for this SPI.

#### Scenario: issuer_url triggers Discovery at construction

- GIVEN an `OidcProviderConfig` with `issuer_url` set and `FakeDiscovery` serving the well-known endpoint
- WHEN `OidcAuthenticationProvider::new(...)` is called
- THEN the `jwks_uri` is resolved from the `OidcEndpoints` and no per-request HTTP occurs

#### Scenario: Manual jwks_uri bypasses Discovery

- GIVEN an `OidcProviderConfig` with `jwks_uri` set and no `issuer_url`
- WHEN `OidcAuthenticationProvider::new(...)` is called and then `authenticate` is invoked
- THEN no request is made to any `/.well-known/openid-configuration` endpoint

#### Scenario: Neither issuer_url nor jwks_uri fails at construction

- GIVEN an `OidcProviderConfig` with both `issuer_url` and `jwks_uri` absent
- WHEN `OidcAuthenticationProvider::new(...)` is called
- THEN the result is `Err(AuthenticationError::ProviderUnavailable("..."))` — construction fails, not the first request

#### Scenario: jwks_uri wins when both are provided

- GIVEN an `OidcProviderConfig` with both `issuer_url` and `jwks_uri` present
- WHEN `OidcAuthenticationProvider::new(...)` is called
- THEN `jwks_uri` is used directly without calling the discovery endpoint

---

### Requirement: JWT Signature and Claims Validation (US-003)

The system MUST validate RS256 and ES256 tokens against JWKS-resolved keys.
The system MUST validate HS256 tokens against a pre-shared symmetric key (existing behaviour preserved).
The system MUST validate `exp` against the injected `Clock` (MUST NOT call `SystemTime::now()` directly).
The system MUST validate `nbf` when the claim is present.
The system MUST validate `iss` AFTER signature verification.
The system MUST trigger one forced JWKS refresh when the token's `kid` is absent from cache before returning `Err(AuthenticationError::InvalidSignature)`.
The system MUST reject any token whose total byte length exceeds 8 KiB (8,192 bytes) with `Err(AuthenticationError::InvalidToken(...))` before performing any base64 decoding or JWT parsing.

#### Scenario: RS256 valid token passes validation

- GIVEN a `FakeIssuer` (RS256) and an `OidcAuthenticationProvider` wired with its resolver
- WHEN a valid RS256 token is authenticated
- THEN result is `Ok(SecurityContext)`

#### Scenario: ES256 valid token passes validation

- GIVEN a `FakeIssuer` configured for ES256 and a corresponding provider
- WHEN a valid ES256 token is authenticated
- THEN result is `Ok(SecurityContext)`

#### Scenario: Unknown kid triggers one forced refresh

- GIVEN a `JwksKeyResolver` with a populated cache containing key `kid-A`
- AND a token with `kid = "kid-unknown"` not present in cache
- WHEN `authenticate` is called
- THEN the resolver fetches the JWKS endpoint exactly once more before returning `Err(AuthenticationError::InvalidSignature)` (if kid remains absent)

#### Scenario: iss mismatch after valid signature is rejected

- GIVEN a token with a valid RS256 signature but `iss` not matching `expected_iss`
- WHEN `authenticate` is called
- THEN result is `Err(AuthenticationError::InvalidToken("iss mismatch"))`

---

### Requirement: Token Format Detection (US-003b)

The system MUST support explicit `TokenFormat::Jwt`, `TokenFormat::Opaque`, and `TokenFormat::Auto` modes.
In `Auto` mode the system MUST classify a token as JWT when it contains exactly two dots and both segments are valid base64url; otherwise it MUST treat the token as opaque.
A token that superficially looks like a JWT (contains dots) but is actually opaque MUST be handled correctly when `TokenFormat::Opaque` is set explicitly.

#### Scenario: Auto detects JWT correctly

- GIVEN `token_format = Auto` and a well-formed JWT (`header.payload.signature`)
- WHEN `authenticate` is called
- THEN the JWT validation path is taken

#### Scenario: Auto detects opaque correctly

- GIVEN `token_format = Auto` and a token with no dots
- WHEN `authenticate` is called
- THEN the introspection path is taken (or `InvalidToken` if introspection not configured)

#### Scenario: JWT-looking opaque token handled with explicit Opaque mode

- GIVEN a token that contains two dots but is not a valid JWT (opaque IdP artifact)
- AND `token_format = Opaque`
- WHEN `authenticate` is called
- THEN the introspection path is taken; the Auto heuristic is not consulted

#### Scenario: Auto JWT-looking opaque returns InvalidToken

- GIVEN `token_format = Auto` and a token that has two dots but fails JWT decode (opaque artifact)
- AND no introspection provider configured
- WHEN `authenticate` is called
- THEN result is `Err(AuthenticationError::InvalidToken(...))`

---

### Requirement: Opaque Token Introspection (US-004)

The system MUST call the configured RFC 7662 introspection endpoint for opaque tokens.
The system MUST return `Ok(SecurityContext)` when the introspection response contains `active: true`.
The system MUST return `Err(AuthenticationError::InvalidToken(...))` when `active: false`.
The system MUST return `Err(AuthenticationError::ProviderUnavailable(...))` on network error.
No `IntrospectionResponse` type MUST be visible to application code.
The system MUST reject any token whose total byte length exceeds 8 KiB (8,192 bytes) with `Err(AuthenticationError::InvalidToken(...))` before passing it to the introspection endpoint.

#### Scenario: active:true returns SecurityContext

- GIVEN a `FakeIntrospection` configured with `set_response("opaque-token", true)` and a `DefaultPrincipalMapper`
- WHEN `authenticate(&Credential::Bearer("opaque-token"))` is called
- THEN result is `Ok(SecurityContext)`

#### Scenario: active:false returns InvalidToken

- GIVEN a `FakeIntrospection` configured with `set_response("revoked-token", false)`
- WHEN `authenticate(&Credential::Bearer("revoked-token"))` is called
- THEN result is `Err(AuthenticationError::InvalidToken(...))`

#### Scenario: Network error returns ProviderUnavailable

- GIVEN an `IntrospectionAuthenticationProvider` whose endpoint URL is unreachable
- WHEN `authenticate` is called with any opaque token
- THEN result is `Err(AuthenticationError::ProviderUnavailable(...))`

#### Scenario: IntrospectionResponse is invisible to caller

- GIVEN any successful introspection flow
- WHEN the result is inspected by application code
- THEN the type is `SecurityContext` — no introspection-specific struct is present

---

### Requirement: JWKS Cache and Key Resolution (US-005)

The system MUST load JWKS at provider startup and store keys in `Arc<RwLock<HashMap<kid, VerificationKey>>>`.
`KeyResolver::resolve()` MUST read from cache without HTTP on the hot path.
The system MUST refresh the cache on a configurable TTL (default 300 seconds) via a background task.
A cache miss MUST trigger at most one forced synchronous refresh before returning an error.
Refresh failures MUST NOT interrupt availability — stale cache MUST be kept and the error logged.

#### Scenario: Hot-path resolve uses cache only

- GIVEN a `JwksKeyResolver` whose cache is already populated with `kid-A`
- WHEN `resolve(Some("kid-A"), JwtAlgorithm::Rs256)` is called
- THEN the key is returned without any HTTP call

#### Scenario: Cache miss triggers one refresh

- GIVEN a `JwksKeyResolver` with an empty cache and a reachable JWKS endpoint returning `kid-B`
- WHEN `resolve(Some("kid-B"), JwtAlgorithm::Rs256)` is called
- THEN exactly one JWKS fetch occurs, the key is inserted into cache, and the key is returned

#### Scenario: Concurrent reads do not block each other

- GIVEN a `JwksKeyResolver` with a populated cache
- WHEN 10 threads call `resolve` concurrently
- THEN all succeed without deadlock or data race (verified under `cargo test` with MIRI or loom where applicable)

#### Scenario: Refresh failure keeps stale cache

- GIVEN a `JwksKeyResolver` with cached `kid-A` and an unreachable refresh endpoint
- WHEN the background refresh task fires
- THEN `resolve(Some("kid-A"), ...)` still returns the stale key successfully

---

### Requirement: Principal Mapping (US-006)

The system MUST expose `PrincipalMapper` as a public trait in `security-sdk`.
The trait signature uses `&ClaimSet` — NOT `&BTreeMap<String, Value>` — to keep `security-sdk` free of `serde_json`.
`DefaultPrincipalMapper` MUST map: `sub` → `principal.subject_id`; `roles` / `realm_access.roles` (Keycloak nested) / `groups` → `principal.roles`; `scp` / `scope` → `claims.custom["scope"]`; `tenant_id` / `tid` (Entra ID) / `tenant` → `principal.tenant_id`; `organization` / `org_id` → `claims.custom["organization"]`.
A custom `PrincipalMapper` MUST be injectable into any provider at construction time.
The system MUST call the injected `PrincipalMapper::map()` for every token — no hardcoded claim extraction path MUST exist in validation logic.
Absence of `sub` MUST produce `Err(AuthenticationError::MissingClaim("sub"))`.

#### Scenario: DefaultPrincipalMapper maps standard claims

- GIVEN a JWT with `sub = "user-1"`, `roles = ["admin"]`, `tid = "tenant-42"`
- WHEN authenticated with `DefaultPrincipalMapper`
- THEN `principal.subject_id = "user-1"`, `principal.roles` contains `"admin"`, `principal.tenant_id = Some("tenant-42")`

#### Scenario: Custom PrincipalMapper is used

- GIVEN a custom `PrincipalMapper` that maps `preferred_username` → `principal.subject_id`
- AND it is injected into `OidcAuthenticationProvider::new(..., mapper)`
- WHEN a token with `preferred_username = "alice"` is authenticated
- THEN `principal.subject_id = "alice"`

#### Scenario: Missing sub returns MissingClaim

- GIVEN a JWT without the `sub` claim, authenticated with `DefaultPrincipalMapper`
- WHEN `authenticate` is called
- THEN result is `Err(AuthenticationError::MissingClaim("sub"))`

#### Scenario: PrincipalMapper is always called (not bypassed)

- GIVEN a provider wired with a tracking `PrincipalMapper` that records invocations
- WHEN `authenticate` is called with a valid JWT
- THEN the tracker records exactly one invocation

---

### Requirement: Multi-Issuer Routing (US-007)

The system MUST route `Credential::Bearer` tokens to the sub-provider resolved by `IssuerResolver` for the `iss` claim extracted from the unverified payload.
Routing by unverified `iss` MUST NOT constitute a trust assertion — the selected sub-provider MUST re-validate `iss` after signature verification.
An unknown or absent `iss` MUST produce `Err(AuthenticationError::InvalidToken(...))` immediately.
`MultiIssuerAuthenticationProvider` MUST implement `AuthenticationProvider` and be usable wherever `Arc<dyn AuthenticationProvider>` is expected.
`MultiIssuerAuthenticationProvider` MUST take `Arc<dyn IssuerResolver>` — NOT a concrete `HashMap` — allowing custom resolvers (hostname, tenant, realm).
The system MUST reject any token whose total byte length exceeds 8 KiB (8,192 bytes) with `Err(AuthenticationError::InvalidToken(...))` before performing any base64 decoding or JSON parsing.

#### Scenario: Token from known issuer routes to correct provider

- GIVEN two `FakeIssuer` instances A and B, each registered under their respective `iss` strings in a `StaticIssuerResolver`
- WHEN a token signed by issuer A (with `iss = "issuer-A"`) is authenticated
- THEN provider A's `authenticate` is called and returns `Ok(SecurityContext)`

#### Scenario: Unknown iss returns InvalidToken

- GIVEN a `MultiIssuerAuthenticationProvider` with issuers A and B registered
- WHEN a token with `iss = "issuer-C"` is presented
- THEN result is `Err(AuthenticationError::InvalidToken(...))` without calling either sub-provider

#### Scenario: Forged iss is rejected by sub-provider post-signature

- GIVEN a token whose base64-decoded payload contains `iss = "issuer-A"` but whose signature was issued by issuer B
- WHEN authenticated against a `MultiIssuerAuthenticationProvider`
- THEN routing selects provider A, which fails signature verification, and result is `Err(AuthenticationError::InvalidSignature)`

#### Scenario: MultiIssuerAuthenticationProvider usable as Arc<dyn AuthenticationProvider>

- GIVEN a `MultiIssuerAuthenticationProvider` wrapped in `Arc`
- WHEN it is passed to code expecting `Arc<dyn AuthenticationProvider>`
- THEN it compiles and dispatches correctly (static assertion via type check)

---

### Requirement: TestKit (US-008)

All TestKit types MUST be gated behind `#[cfg(feature = "test-kit")]`.
The `test-kit` feature MUST NOT be included in production Cargo profiles.
`FakeIssuer` MUST produce real signed tokens; it defaults to RS256 and MAY support ES256 via an algorithm parameter.
`FixedClock` (from `ego-domain` or test-kit) MUST make temporal validation deterministic.
All 8 US acceptance criteria MUST be coverable without a live IdP.

**INV-TK: TestKit cryptographic integrity.** The TestKit MUST NOT implement a second cryptographic stack. `FakeIssuer` MUST produce tokens via `jsonwebtoken::encode` and the in-process key pair, which are then validated by the same `JwtValidationEngine` and `AuthenticationProvider` path that production code uses. No `FakeValidator`, no mock validation, no bypassing of the authentication pipeline. A test that passes through `FakeIssuer → jsonwebtoken::encode → JwksKeyResolver → JwtValidationEngine → authenticate()` is testing the real path. A test that bypasses any of these steps is not.

#### Scenario: FakeIssuer tokens pass real validation

- GIVEN a `FakeIssuer` and an `OidcAuthenticationProvider` wired with `FakeIssuer::jwks_resolver()`
- WHEN a token from `FakeIssuer::issue_token(claims)` is authenticated
- THEN real JWKS-backed RS256 validation succeeds and returns `Ok(SecurityContext)`

#### Scenario: FixedClock makes expiry deterministic

- GIVEN a `FixedClock` returning timestamp `T`
- AND a token with `exp = T - 1` (already expired)
- WHEN `authenticate` is called
- THEN result is deterministically `Err(AuthenticationError::ExpiredToken)` regardless of wall clock

#### Scenario: FakeIntrospection returns active:false

- GIVEN `FakeIntrospection` with `set_response("tok", false)` and an `IntrospectionAuthenticationProvider`
- WHEN `authenticate(&Credential::Bearer("tok"))` is called
- THEN result is `Err(AuthenticationError::InvalidToken(...))`

#### Scenario: No TestKit symbol in release build

- GIVEN a release build (`cargo build --release --no-default-features`)
- WHEN the resulting binary or rlib is inspected for `FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`
- THEN none of these symbols are present (verified by CI)

---

## Part 3: Architectural Decisions

### AD-OIDC-011 — AuthenticationInterceptor Depends on RequestContext, Not Concrete HTTP Types

**Decision**: `AuthenticationInterceptor` depends on `Arc<dyn CredentialExtractor>` and `Arc<dyn AuthenticationProvider>`. `CredentialExtractor` depends on `&dyn RequestContext`. No concrete HTTP type (`hyper::Request`, `axum::Request`, `actix::HttpRequest`) appears in the interceptor or extractor contracts.

**Rationale**: The interceptor runs authentication logic that is transport-agnostic. Depending on a concrete HTTP type would force a re-implementation for every transport (axum, tonic, actix, hyper). `RequestContext` is the abstraction boundary — transport adapters convert their concrete request type to `RequestContext`; the interceptor and extractors never see the transport.

**Future extensions**: `TonicRequestContext`, `AxumRequestContext`, etc. implement `RequestContext` without changing the interceptor.

---

## Part 4: Invariants

Every implementation MUST preserve all of the following. A violation of any invariant is a specification defect, not an implementation detail.

| # | Invariant |
|---|-----------|
| INV-1 | `AuthenticationProvider::authenticate()` is always synchronous — no `.await` inside its call body |
| INV-2 | No OIDC protocol type (`JwtClaims`, `Jwk`, `JwkSet`, `OidcConfiguration`, `DiscoveryDocument`, `IntrospectionResponse`, `BearerToken`, `IdToken`, `AccessToken`) appears in any `pub` re-export of `security-sdk` |
| INV-3 | `PrincipalMapper::map` is called for every successfully verified token — no provider may hardcode claim extraction logic |
| INV-4 | Unverified `iss` is used ONLY for sub-provider routing in `MultiIssuerAuthenticationProvider` — it MUST NOT be used as a trust or authorization assertion. The selected sub-provider MUST re-validate `iss` after signature verification before trusting any claim value. For `IntrospectionAuthenticationProvider`, the trust gate is endpoint authentication (HTTPS + `ClientCredentials`) rather than signature verification; `iss` re-validation does not apply to the opaque token path. |
| INV-5 | The `test-kit` feature MUST NOT be transitively enabled in production builds; `cargo build --release --no-default-features` MUST produce zero TestKit symbols |
| INV-6 | All temporal comparisons (`exp`, `nbf`) MUST use the injected `Clock` trait — direct `SystemTime::now()`, `Utc::now()`, or equivalent MUST NOT appear in validation logic |
| INV-7 | `JwksKeyResolver::resolve()` acquires a read lock during lookup — the write lock is acquired only during cache refresh, never on the `authenticate()` hot path |
| INV-8 | `security-sdk` MUST NOT introduce a new direct dependency on `serde_json`. Pre-existing `ego-domain` types that contain `serde_json::Value` (e.g. `Claims.custom`) are inherited by composition and do not violate this invariant. New types added to `security-sdk` in this change MUST NOT add `serde_json` directly. |
| INV-9 | `AuthenticationInterceptor` MUST depend on `Arc<dyn CredentialExtractor>` — never directly on `BearerExtractor` or any concrete extractor |
| INV-10 | All URL fields in `OidcProviderConfig` MUST be `url::Url` — raw `String` URL fields are not permitted |
| INV-11 | `introspection_endpoint`, when present, MUST use the `https` scheme. `OidcAuthenticationProvider` construction MUST return `Err(...)` if an `http://` URL is provided (exception: `http://localhost` and `http://127.0.0.1` are permitted for test environments). |
| INV-TK | `FakeIssuer` MUST produce tokens via `jsonwebtoken::encode` and the in-process key pair. No `FakeValidator`, no mock validation, no bypassing of `JwtValidationEngine` or `AuthenticationProvider`. Tests that bypass any step of the real pipeline are not valid acceptance tests. |

---

## Scope Table

| Component | Crate | What changes |
|-----------|-------|-------------|
| `ClaimSet` + `ClaimValue` | `ego-domain` | New value object (domain layer) |
| `CredentialExtractor`, `RequestContext` | `security-sdk` | New SPI traits |
| `BearerExtractor`, `BasicExtractor`, `ApiKeyExtractor` | `security-sdk` | Built-in extractor impls |
| `PrincipalMapper` | `security-sdk` | New trait (replaces `ClaimsMapper`) |
| `AuthenticationInterceptor` | `security-sdk` (authentication subsystem) | Moved from `transport`; depends on `CredentialExtractor`; transport-agnostic |
| `DiscoveryProvider` (`pub(crate)`), `JwksProvider`, `IntrospectionProvider` | `security-jwt` | `DiscoveryProvider` is internal; `JwksProvider` and `IntrospectionProvider` are public SPI traits |
| `OidcEndpoints` | `security-jwt` | New minimal public struct |
| `HttpDiscoveryProvider` (`pub(crate)`), `HttpJwksProvider`, `HttpIntrospectionProvider` | `security-jwt` | `HttpDiscoveryProvider` is internal; `HttpJwksProvider` and `HttpIntrospectionProvider` are public defaults |
| `IssuerResolver`, `StaticIssuerResolver` | `security-jwt` | New resolver SPI + static impl |
| `TokenFormat` enum | `security-jwt` | New (replaces `OidcTokenType`) |
| `OidcProviderConfig` (URL fields) | `security-jwt` | All URL fields become `url::Url` |
| `OidcAuthenticationProvider` | `security-jwt` | Takes `Arc<dyn PrincipalMapper>` |
| `MultiIssuerAuthenticationProvider` | `security-jwt` | Takes `Arc<dyn IssuerResolver>` |
| `IntrospectionAuthenticationProvider` | `security-jwt` | Takes `Arc<dyn PrincipalMapper>` |
| `JwksKeyResolver` | `security-jwt` | `jwks_uri: url::Url` |
| `DefaultPrincipalMapper` | `security-jwt` | Replaces `DefaultClaimsMapper` |
| TestKit | `security-jwt` | Updated for `ClaimSet`, `PrincipalMapper` |
| `transport` interceptor | `transport` | REMOVED — interceptor lives in `security-sdk` |

OAuth2 Client flows (Client Credentials, Authorization Code, PKCE, Refresh Token) are out of scope — deferred to CORE-021.

---

## Coverage Summary

| Domain | Type | Requirements | Scenarios |
|--------|------|-------------|-----------|
| ego-domain: ClaimSet | New | — | — |
| security-sdk: CredentialExtractor | New | 1 (US-001) | 1 |
| security-sdk: PrincipalMapper | New | 1 (US-006) | 4 |
| security-sdk: AuthenticationInterceptor (authentication engine) | New | 1 (US-001) | 1 |
| security-jwt: OIDC Discovery | New | 1 (US-002) | 4 |
| security-jwt: JWT Validation | New | 1 (US-003) | 4 |
| security-jwt: Token Format Detection | New | 1 (US-003b) | 4 |
| security-jwt: Introspection | New | 1 (US-004) | 4 |
| security-jwt: JWKS Cache | New | 1 (US-005) | 4 |
| security-jwt: Multi-Issuer + IssuerResolver | New | 1 (US-007) | 4 |
| security-jwt: TestKit | New | 1 (US-008) | 4 |
| **Total** | — | **9 explicit + contracts** | **34** |

Happy paths: covered (all US).
Edge cases: covered (nbf, kid rotation, stale cache, forged iss, release symbol check, Auto heuristic ambiguity).
Error states: covered (ExpiredToken, InvalidSignature, MissingClaim, InvalidToken, ProviderUnavailable).
