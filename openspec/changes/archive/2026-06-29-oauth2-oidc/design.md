# Design: oauth2-oidc — OIDC Resource Server Framework

Technical HOW for the proposal `sdd/oauth2-oidc/proposal`. Extends `security-jwt`
(infrastructure) and `security-sdk` (contracts); adds `ClaimSet` to `ego-domain`.
All ADRs (AD-OIDC-001..009) are honored. Layer rule respected: domain value objects
in `ego-domain`, contracts in `security-sdk`, impls in `security-jwt`.

> **Scope note**: Resource Server authentication only (validating tokens). OAuth2
> Client flows (Client Credentials, Authorization Code, PKCE, Refresh Token) are
> deferred to CORE-021.

## Technical Approach

Composite providers, all implementing the existing sync
`AuthenticationProvider::authenticate(&Credential) -> Result<SecurityContext>`.
No change to that signature. Async I/O (JWKS fetch, discovery, introspection)
stays off the hot path via the proven `RESOLVER_POOL` bridge
(`spawn_ok` + `mpsc::channel`, already in `authenticator.rs`). JWT path reuses
`JwtValidationEngine::validate` unchanged; only claim mapping is extracted into
an injected `PrincipalMapper` operating on `ClaimSet`.

## Open Question Resolutions (ADR-style)

| OQ | Decision | Rationale | Rejected |
|----|----------|-----------|----------|
| OQ-1 | `PrincipalMapper` trait lives **directly in `security-sdk`**; `DefaultPrincipalMapper` impl in `security-jwt`, NOT re-exported into sdk | sdk already holds traits only (`AuthenticationProvider`, `RoleStore`); trait→contracts, impl→infra is the existing convention. AD-OIDC-002/003 require it. | Re-export from jwt: leaks impl crate into contract API |
| OQ-2 | **Format heuristic**, with config override. `TokenFormat::Auto\|Jwt\|Opaque`; `Auto` = 2-dot base64url ⇒ JWT, else opaque | Zero-config for the common case; false-positive risk ~nil (opaque tokens lack 2 dots). Override exists for the rare ambiguous IdP. Edge case: JWT-looking opaque with explicit `Opaque` mode works correctly (US-003b). | Config-only flag: forces config for the 99% case |
| OQ-3 | Introspection cache **default OFF**, opt-in short TTL (`introspection_cache_ttl_seconds`), keyed by SHA-256 of token | Caching `active:true` serves revoked tokens until TTL; correctness default beats latency default. Opt-in covers high-QPS callers. | Always-on: stale-revocation security risk. Never: HTTP per call, but acceptable as default |
| OQ-4 | `AuthenticationInterceptor` depends on `Arc<dyn CredentialExtractor>` + `Arc<dyn AuthenticationProvider>` — lives in `security-sdk` as a transport-agnostic authentication engine | Authentication interception is a security concern, not a transport concern. Extractor SPI lets callers inject `BearerExtractor`, `BasicExtractor`, `ApiKeyExtractor`, or custom. Transport adapters wrap the engine and handle protocol mapping. | In `transport`: couples transport to security internals, wrong layer |
| OQ-5 | Discovery doc missing `jwks_uri` ⇒ **fail-fast at startup** (provider construction error) | OIDC Core marks `jwks_uri` REQUIRED; absence = non-compliant IdP. Silent fallback hides misconfiguration. AD-OIDC-005. | Fallback: masks broken IdP, fails later at first request |
| OQ-6 | **`IssuerResolver` trait** with `StaticIssuerResolver` default. `MultiIssuerAuthenticationProvider` takes `Arc<dyn IssuerResolver>` | HashMap is frozen static only; resolver trait allows hostname, tenant, realm, path-based routing without a new provider type. Static registration remains the default path. | Concrete HashMap field: blocks non-static resolution patterns |

## Architecture Decisions

### Decision: `ClaimSet` value object in `ego-domain`
**Choice**: Introduce `ClaimSet { raw: BTreeMap<String, ClaimValue> }` and `ClaimValue`
enum in `crates/domain/src/auth/claim_set.rs`. `PrincipalMapper::map` takes `&ClaimSet`.
**Rationale**: Decouples `security-sdk` from `serde_json::Value`. `serde_json` is an
infrastructure concern; the domain layer must not depend on it. `ClaimValue` mirrors
JSON's type lattice but is an owned domain type — convertible from `serde_json::Value`
in `security-jwt` without leaking the conversion into `security-sdk`.
**Alternatives**: Keep `BTreeMap<String, Value>` in the trait (simpler, already worked).
**Rejected because**: INV-8 — `security-sdk` must not depend on `serde_json`.

### Decision: Reuse `JwtValidationEngine`, inject `PrincipalMapper`
**Choice**: Keep `JwtValidationEngine::validate` as the signature+time+iss/aud
gate. Move only the claim→`(Principal, Claims)` extraction behind a `PrincipalMapper`
passed to `validate`, operating on a `ClaimSet` assembled from the decoded claims.
`DefaultPrincipalMapper` reproduces today's exact behavior.
**Alternatives**: New OIDC-only validation engine (duplicates verified logic).
**Rationale**: One validation path = one place to audit. Non-breaking: existing
providers inject `DefaultPrincipalMapper`.

### Decision: `JwksKeyResolver` implements existing `KeyResolver`
**Choice**: `JwksKeyResolver` is just another `KeyResolver` impl. The composite
JWT provider routes through the SAME `authenticate_inner` path that
Rs256/Es256 already use. No new auth code for the JWT branch.
**Rationale**: `VerificationKey` is `#[non_exhaustive]` and already has
`RsaPem`/`EcPem`. JWK→PEM conversion produces those variants. Maximum reuse.

### Decision: Sync bridge = existing RESOLVER_POOL only
**Choice**: Discovery, JWKS forced-refresh, and introspection all cross sync→async
via the same `mpsc::channel` + `resolver_pool().spawn_ok` pattern. Background
JWKS refresh runs on its own `tokio::spawn` interval task (NOT the pool).
**Rationale**: One concurrency pattern, already reviewed (B-2 fix).

### Decision: `JwksProvider` and `IntrospectionProvider` are public traits; `DiscoveryProvider` is internal
**Choice**: `JwksProvider` and `IntrospectionProvider` are `pub trait` in `security-jwt`. HTTP implementations
(`HttpJwksProvider`, `HttpIntrospectionProvider`) are the defaults but not the only option.
Custom implementations (Vault, k8s secrets, file) are first-class for these two SPIs.
`DiscoveryProvider` and `HttpDiscoveryProvider` are `pub(crate)` — internal implementation detail.
**Rationale**: Discovery is called once at construction time; it is not a caller-facing extension point.
Exposing it as a public SPI adds API surface without benefit. `OidcEndpoints` (the discovery result type)
remains a minimal public struct (`jwks_uri: Url`, `introspection_endpoint: Option<Url>`) because callers
may receive or inspect it; the provider producing it is internal.

### Decision: `AuthenticationInterceptor` in `security-sdk` (transport-agnostic authentication engine)
**Choice**: Move from `transport` to `crates/security-sdk/src/authentication/interceptor.rs`.
The interceptor constructor takes `Arc<dyn CredentialExtractor>` +
`Arc<dyn AuthenticationProvider>` — no direct dependency on `BearerExtractor`.
**Rationale**: Authentication interception is a security concern, not a transport concern.
`transport` should not know about authentication logic. HTTP, gRPC, Axum, Actix, and any
future transport each provide their own adapter crate (`security-http`, `security-grpc`,
`security-axum`, `security-actix`, etc.) that converts native request types to `RequestContext`
and maps `AuthenticationError` to protocol responses; the interceptor engine never sees
transport-specific types. The `CredentialExtractor` SPI lets operators compose extractors
without touching the interceptor.

### AD-OIDC-012: AuthenticationProvider as a Composite-capable interface

**Decision**: `AuthenticationProvider` is the uniform interface for both leaf providers
(single-algorithm implementations) and composite providers (routers, chains, fallbacks).

Leaf providers:
- `Hs256AuthenticationProvider`
- `Rs256AuthenticationProvider`
- `Es256AuthenticationProvider`
- `BasicAuthenticationProvider` (existing)
- `OidcAuthenticationProvider` (this change)
- `IntrospectionAuthenticationProvider` (this change)

Composite providers:
- `MultiIssuerAuthenticationProvider` (this change) — routes by unverified `iss`
- `ChainAuthenticationProvider` (future) — tries each provider in order
- `FallbackAuthenticationProvider` (future) — primary + fallback on `ProviderUnavailable`
- `AnonymousProvider` (future) — grants a fixed anonymous `SecurityContext`

**Rationale**: A single interface for both levels allows `RuntimeBuilder` and
`AuthenticationInterceptor` to remain unaware of composition depth. New composition
strategies can be added without touching existing providers or framework code.

**Constraint**: Composite providers MUST NOT short-circuit security invariants. Each leaf
provider is responsible for its own signature verification, expiry checks, and claim
mapping. Composites only route or chain — they never bypass leaf validation.

### AD-OIDC-013: ClaimValue as a domain Value Object

**Decision**: `ClaimSet` and `ClaimValue` are pure ego-domain types with no `serde_json` dependency.

**Rationale**:
- Decouples `security-sdk` (and callers) from `serde_json::Value` permanently (INV-8).
- `ClaimValue`'s shape (`String`, `Integer`, `Float`, `Bool`, `Array`, `Map`, `Null`) is a domain
  contract, not a JSON artifact. The serialization layer (`serde_json`) converts to
  `ClaimValue` at the boundary inside `security-jwt`; it never leaks into `security-sdk`
  or `ego-domain`.
- Future token formats (CBOR, Protobuf, MessagePack) convert to `ClaimValue` at their
  own boundary without changing any `security-sdk` or `ego-domain` contracts.

**Constraint**: The `serde_json::Value` → `ClaimValue` conversion MUST live in `security-jwt`
(the infrastructure boundary), not in `ego-domain` or `security-sdk`. The direction is
always inward: `serde_json::Value` → `ClaimValue`, never `ClaimValue` → `serde_json::Value`
in the domain layer.

## Module Structure

### `ego-domain` (new)

| File | Public exports |
|------|---------------|
| `crates/domain/src/auth/claim_set.rs` | `ClaimSet`, `ClaimValue` |

### `security-sdk` (new/changed)

| File | Public exports |
|------|---------------|
| `src/credential_extractor.rs` | `CredentialExtractor`, `RequestContext`, `BearerExtractor`, `BasicExtractor`, `ApiKeyExtractor` |
| `src/principal_mapper.rs` | `PrincipalMapper` (replaces `claims_mapper.rs`) |
| `src/authentication/interceptor.rs` | `AuthenticationInterceptor` |

### `security-jwt` (new/changed)

| File | Public exports | Internal (`pub(crate)`/private) | Key deps |
|------|----------------|---------------------------------|----------|
| `src/jwks.rs` | `JwksKeyResolver`, `JwksProvider`, `HttpJwksProvider` | `jwk_to_verification_key`, refresh task | `reqwest`, `tokio::sync::RwLock`, `jsonwebtoken::jwk::JwkSet`, `url` |
| `src/discovery.rs` | `OidcEndpoints` | `DiscoveryProvider` (pub(crate)), `HttpDiscoveryProvider` (pub(crate)), `OidcConfiguration` (serde, pub(crate)) | `reqwest`, `url` |
| `src/introspection.rs` | `IntrospectionProvider`, `HttpIntrospectionProvider`, `IntrospectionAuthenticationProvider` | `IntrospectionResponse` (serde, pub(crate)), token-hash cache | `reqwest`, `sha2` |
| `src/oidc_provider.rs` | `OidcAuthenticationProvider` | token-type detection | `JwtValidationEngine`, `JwksKeyResolver` |
| `src/multi_issuer.rs` | `MultiIssuerAuthenticationProvider`, `IssuerResolver`, `StaticIssuerResolver` | `UnverifiedIss`, `unverified_iss()` | `serde_json`, `base64` |
| `src/oidc_config.rs` | `OidcProviderConfig`, `MultiIssuerConfig`, `TokenFormat` | — | `serde`, `url` |
| `src/principal_mapper.rs` | `DefaultPrincipalMapper` | — | `ego-security-sdk::PrincipalMapper`, `ego-domain::ClaimSet` |
| `src/test_kit/mod.rs` | `FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection` (all `#[cfg(feature="test-kit")]`) | — | `jsonwebtoken` encode |

`transport`: `AuthenticationInterceptor` is REMOVED — no longer lives here.

## Interfaces / Contracts

```rust
// ego-domain — domain value object (no serde_json dependency)
pub struct ClaimSet { pub raw: BTreeMap<String, ClaimValue> }
pub enum ClaimValue { String(String), Integer(i64), Float(f64), Bool(bool), Array(Vec<ClaimValue>), Map(BTreeMap<String, ClaimValue>), Null }

// security-sdk — public contracts
pub trait CredentialExtractor: Send + Sync {
    fn extract(&self, request: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError>;
}
pub trait RequestContext: Send + Sync {
    fn header(&self, name: &str) -> Option<&str>;
    fn metadata(&self, key: &str) -> Option<&str>;
    fn query_param(&self, name: &str) -> Option<&str>;
}
pub trait PrincipalMapper: Send + Sync {  // replaces ClaimsMapper
    fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError>;
}

// security-jwt — public SPI traits
pub(crate) trait DiscoveryProvider: Send + Sync { async fn fetch_configuration(&self, issuer_url: &Url) -> Result<OidcEndpoints, AuthenticationError>; }  // internal
pub trait JwksProvider: Send + Sync { async fn fetch_jwks(&self, jwks_uri: &Url) -> Result<Vec<(Option<String>, VerificationKey)>, AuthenticationError>; }
pub trait IntrospectionProvider: Send + Sync { async fn introspect(&self, token: &str, endpoint: &Url, credentials: &ClientCredentials) -> Result<IntrospectionResult, AuthenticationError>; }
pub trait IssuerResolver: Send + Sync { fn resolve(&self, issuer: &str) -> Option<Arc<dyn AuthenticationProvider>>; }

// security-jwt internal protocol types — pub(crate) only (AD-OIDC-003)
#[derive(serde::Deserialize)] pub(crate) struct OidcConfiguration { jwks_uri: Option<Url>, introspection_endpoint: Option<Url>, /* ... */ }
#[derive(serde::Deserialize)] pub(crate) struct IntrospectionResponse { active: bool, #[serde(flatten)] claims: BTreeMap<String, Value> }
pub(crate) struct UnverifiedIss(String); // newtype: "unverified" is in the type

// JwkSet: REUSE jsonwebtoken::jwk::JwkSet (no wrapper) — converted to VerificationKey
// Conversion: serde_json::Value → ClaimValue happens at the jwt/introspection boundary, not in sdk
```

### JwksKeyResolver
- State: `cache: Arc<RwLock<HashMap<Option<String>, VerificationKey>>>` (key = `kid`),
  `jwks_uri: Url`, `provider: Arc<dyn JwksProvider>`, `algorithm: JwtAlgorithm`.
- `resolve(kid, alg)`: read cache → hit returns clone. Miss ⇒ ONE forced refresh
  via RESOLVER_POOL, re-read; still missing ⇒ `KeyNotFound`.
- Startup: sync warm-up fetch in constructor via RESOLVER_POOL (cache-first
  contract requires populated cache before first `authenticate`).
- Background: `tokio::spawn` + `tokio::time::interval(ttl)` (default 300s);
  failure ⇒ `warn!` + keep stale (US-005, AD-OIDC-009). Exponential backoff on
  consecutive failures.
- `provider` defaults to `HttpJwksProvider`; custom `JwksProvider` injectable for testing.

### OidcAuthenticationProvider
- State: `jwks: JwksKeyResolver`, `introspection: Option<IntrospectionAuthenticationProvider>`,
  `mapper: Arc<dyn PrincipalMapper>`, `clock: Arc<dyn Clock>`, `config: OidcProviderConfig`.
- `authenticate`:
  - Pre-check: if `token.len() > 8192`, return `InvalidToken` immediately.
  - detect format (OQ-2, `TokenFormat` enum) → JWT branch calls the
    existing `authenticate_inner` with `JwksKeyResolver` + mapper; opaque branch
    delegates to `introspection` (or `InvalidToken` if none configured).
- `serde_json::Value` → `ClaimValue` conversion happens here, producing a `ClaimSet` passed to mapper.

### MultiIssuerAuthenticationProvider
- State: `resolver: Arc<dyn IssuerResolver>` (replaces concrete `HashMap`).
- `authenticate`: Pre-check: if `token.len() > 8192`, return `InvalidToken` immediately — no base64 decode, no JSON parse. Then `unverified_iss()` = base64url-decode segment[1] using `base64::engine::general_purpose::URL_SAFE_NO_PAD` (RFC 4648 §5 — standard base64 corrupts tokens containing `-` or `_`), then `serde_json`
  parse `iss` — NO `jsonwebtoken::decode`, no signature. Route via `resolver.resolve(iss)`
  → sub-provider does full validation (re-checks `iss` post-signature).
  Unknown iss ⇒ `InvalidToken` (AD-OIDC-004).
- A unit test MUST cover a token whose payload segment contains URL-safe characters (`-` or `_`).
- `StaticIssuerResolver` wraps `HashMap<String, Arc<dyn AuthenticationProvider>>`; frozen after build (OQ-6).

### IntrospectionAuthenticationProvider
- State: `provider: Arc<dyn IntrospectionProvider>`, `endpoint: Url`,
  `credentials: ClientCredentials`, `mapper: Arc<dyn PrincipalMapper>`,
  optional `cache: Option<(ttl_seconds: u64, RwLock<HashMap<[u8;32], (i64, SecurityContext)>>)>` — where the `i64` is the insertion timestamp from `clock.now().timestamp()`, compared at lookup via `clock.now().timestamp() - inserted_at > ttl_seconds`.
- Cache lookup uses `Arc<dyn Clock>` for TTL comparison — not `std::time::Instant` — to remain testable with `FixedClock`.
- `authenticate`:
  - Pre-check: if `token.len() > 8192`, return `InvalidToken` immediately.
  - cache lookup (if on) → else `provider.introspect(token, endpoint, credentials)`
    via RESOLVER_POOL → `active:false`⇒`InvalidToken`, net error⇒`ProviderUnavailable`,
    `active:true` → convert claims to `ClaimSet` → `mapper.map(claims)`.
- `provider` defaults to `HttpIntrospectionProvider`; custom `IntrospectionProvider` injectable.

### AuthenticationInterceptor (`security-sdk`, authentication engine)
- `pub struct AuthenticationInterceptor { extractor: Arc<dyn CredentialExtractor>, provider: Arc<dyn AuthenticationProvider> }`.
- Does NOT implement the `Interceptor` trait. Adapter crates call `intercept` directly while they still own `&mut ServiceContext` — before handing off to the interceptor chain. Sequence: `Adapter → AuthenticationInterceptor::intercept(&mut ServiceContext) → InterceptorChain`. No interior mutability, no `UnsafeCell` required.
- `fn intercept(&self, ctx: &dyn RequestContext, service_ctx: &mut ServiceContext) -> Result<(), AuthenticationError>`:
  1. `credential = extractor.extract(ctx)?` → `Option<Credential>`
  2. If `Some(credential)`: `context = provider.authenticate(&credential)?` → populate `service_ctx.security`
  3. If `None` (no credential present): pass through (no security set)
  4. On `AuthenticationError`: propagate to caller — DO NOT map to HTTP responses
- `BearerExtractor` reads `Authorization: Bearer <t>` and returns `Some(Credential::Bearer(t))`.
- `BasicExtractor` reads `Authorization: Basic <b64>`, decodes to `Credential::Basic(user, pass)`.

**Note**: `AuthenticationInterceptor` NEVER produces HTTP responses. It returns
`Result<(), AuthenticationError>`. The caller (transport adapter) decides what
HTTP 401, gRPC UNAUTHENTICATED, etc. looks like.

## Transport Adapter Extension Model

`security-sdk` is transport-agnostic. The `AuthenticationInterceptor` in `security-sdk`
defines the authentication _execution_ — not the protocol _response_.

Transport adapters (future crates) wrap the interceptor:

- `security-http` → `HttpAuthenticationMiddleware` → HTTP 401
- `security-grpc` → `GrpcAuthenticationInterceptor` → gRPC UNAUTHENTICATED
- `security-graphql` → `GraphqlAuthenticationExtension` → GraphQL error
- `security-axum` → `AuthenticationLayer` → Axum middleware layer
- `security-actix` → `AuthenticationMiddleware` → Actix-web middleware

Pattern for each adapter:
```
NativeRequest → RequestContextImpl → AuthenticationInterceptor → AuthenticationError → ProtocolResponse
```

## Configuration

```rust
#[derive(Deserialize)]
pub enum TokenFormat { Auto, Jwt, Opaque }  // replaces OidcTokenType; default Auto

#[derive(Deserialize)]
pub struct OidcProviderConfig {
    pub issuer_url: Option<Url>,            // url::Url — validated at construction
    pub jwks_uri: Option<Url>,             // url::Url — jwks_uri wins (AD-OIDC-005)
    pub expected_iss: Option<String>,
    pub expected_aud: Option<Vec<String>>,
    pub clock_skew_seconds: Option<u64>,              // default 0
    pub token_format: Option<TokenFormat>,            // default Auto
    pub introspection_endpoint: Option<Url>, // url::Url
    pub introspection_client_id: Option<String>,
    pub introspection_client_secret: Option<String>,
    pub jwks_refresh_ttl_seconds: Option<u64>,           // default 300
    pub introspection_cache_ttl_seconds: Option<u64>,    // None = cache off (OQ-3)
}

#[derive(Deserialize)]
pub struct MultiIssuerConfig { pub issuers: HashMap<String, OidcProviderConfig> }
```

All URL fields are `url::Url` — parse and validate at config construction time, not at use time.
Build-time validation: neither `issuer_url` nor `jwks_uri` ⇒ construction error (AD-OIDC-005).

## Build & Feature Flags

```toml
# security-sdk/Cargo.toml
[features]
http = ["dep:http"]

# security-jwt/Cargo.toml
[features]
test-kit = []

[dependencies]
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
tokio   = { version = "1", features = ["rt", "sync", "time", "macros"] }
url = "2"
sha2 = "0.10"   # introspection cache key
base64 = "0.22" # unverified iss decode
```

CI guard: `cargo build --release --no-default-features` then assert no
`FakeIssuer`/`test_kit` symbols (success metric). TestKit only in `[dev-dependencies]`/feature.

## Refactoring Scope (non-breaking)

`JwtValidationEngine::validate` gains a `mapper: &dyn PrincipalMapper` + `claims: &ClaimSet` param.
The inline `extract_subject`/`extract_roles`/`extract_tenant_id`/custom block
moves verbatim into `DefaultPrincipalMapper::map`. Signature/time/iss/aud checks
stay in the engine. `Hs256/Rs256/Es256AuthenticationProvider` gain a
`DefaultPrincipalMapper` default (constructor unchanged via `..new` keeping current
3-arg form + a `with_mapper` builder). Existing tests pass unchanged.

`ClaimsMapper` → `PrincipalMapper` rename is a breaking API change within this PR only
(no external callers yet; this is a greenfield crate addition).

`transport` crate: remove `AuthenticationInterceptor` — zero external callers at this stage.

## Data Flow

    Incoming request
        │  AuthenticationInterceptor (security-sdk — authentication engine)
        │    └─ CredentialExtractor::extract (BearerExtractor / BasicExtractor / custom)
        ▼
    Credential
        │  MultiIssuerAuthenticationProvider
        │    └─ IssuerResolver::resolve(unverified_iss)
        ▼
    OidcAuthenticationProvider
        │  detect TokenFormat (Auto/Jwt/Opaque)
        ├─ JWT ──→ authenticate_inner → JwksKeyResolver(cache) → JwtValidationEngine
        │                                         → serde_json::Value→ClaimValue → ClaimSet
        │                                         → PrincipalMapper::map → SecurityContext
        └─ opaque → IntrospectionAuthenticationProvider
                        → IntrospectionProvider::introspect → /introspect
                        → IntrospectionResponse → ClaimSet → PrincipalMapper::map → SecurityContext

    JWKS background task ──interval refresh──→ Arc<RwLock<cache>> ◄── forced refresh (RESOLVER_POOL)
    JwksProvider (Http or custom) ──────────────────────────────────────────────────────────────────┘

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Unit | token-type detect, unverified_iss, jwk→PEM, claim mapping, ClaimSet construction, config validation | pure fns, table tests |
| Unit | JWKS cache hit/miss/forced-refresh/stale-on-fail | inject `FakeJwks`/seeded cache |
| Unit | `ClaimValue` round-trip from `serde_json::Value` | table tests in `security-jwt` |
| Unit | `CredentialExtractor` impls (Bearer, Basic, ApiKey) | pure extraction tests |
| Integration | full JWT + opaque auth, multi-issuer (2 IdPs), discovery vs manual jwks parity | `FakeIssuer` real signatures, `FakeDiscovery`, `FakeIntrospection` |
| Integration | TokenFormat::Auto edge cases (JWT-looking opaque) | `FakeIntrospection` + malformed JWT headers |
| CI | no test-kit symbols in release | `--no-default-features` build check |

Strict TDD: `cargo test --workspace`, tests first.

## Migration / Rollout

Additive. Existing providers/`Basic` untouched. Interceptor opt-in. `transport`
interceptor removal is non-breaking (no external callers). Rollback =
revert security-jwt + security-sdk + domain additions; no caller breaks (rollback plan).

## Open Questions

- [ ] None — all 6 OQs resolved above.
