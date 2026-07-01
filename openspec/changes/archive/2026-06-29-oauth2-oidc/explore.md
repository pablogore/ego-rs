# Exploration: oauth2-oidc — OAuth2/OIDC Authentication Framework

_Date: 2026-06-28 | Change: oauth2-oidc_

## 1. Existing Security Stack

### ego-domain (crates/domain/src/auth/)

| Symbol | File | Notes |
|---|---|---|
| `AuthenticationError` | `error.rs` | `#[non_exhaustive]`, variants: `InvalidToken`, `ExpiredToken`, `AlgorithmNotSupported`, `MissingClaim`, `InvalidSignature`, `ProviderUnavailable` |
| `Credential` | `credential.rs` | `#[non_exhaustive]`: `Bearer(String)`, `Basic{..}`, `Custom{..}` |
| `Claims` | `claims.rs` | `{ standard: StandardClaims, custom: BTreeMap<String, Value> }` |
| `StandardClaims` | `claims.rs` | `exp`, `nbf`, `iat`, `jti`, `iss`, `aud` — all `Option` |
| `Clock` | `clock.rs` | Injectable time trait: `fn now() -> DateTime<Utc>` |
| `SystemClock` | `clock.rs` | Production `Clock` impl |

### ego-security-sdk (crates/security-sdk/)

| Symbol | File | Notes |
|---|---|---|
| `AuthenticationProvider` | `authentication/mod.rs` | **Sync** trait. `fn authenticate(&Credential) -> Result<SecurityContext, AuthenticationError>` |
| `SecurityContext` | `context/mod.rs` | `{ principal: Principal, claims: Claims }` — no ambient state |
| `AuthorizationProvider` | `authorization/mod.rs` | Async trait |
| `Principal` | `principal/principal.rs` | `{ kind, subject_id, tenant_id, roles: BTreeSet<Role>, attributes }` |
| `BasicAuthenticationProvider` | `providers/basic/mod.rs` | In-memory Basic auth only |
| `authorize_in_context` | `authorization/mod.rs` | Shared helper used by `#[authorize]` macro |

### security-jwt (crates/security-jwt/)

| Symbol | File | Notes |
|---|---|---|
| `KeyResolver` | `key_resolver.rs` | Async trait: `resolve(kid, algorithm) -> VerificationKey`. Cache-first required (AD-013) |
| `LocalKeyResolver` | `key_resolver.rs` | Single static key. Satisfies AD-013 |
| `VerificationKey` | `key_resolver.rs` | `#[non_exhaustive]`: `Hmac`, `RsaPem`, `EcPem` |
| `JwtValidationEngine` | `validation.rs` | `pub(crate)`. Clock-injected exp/nbf/iss/aud/sub validation |
| `JwtProviderConfig` | `config.rs` | `{ expected_iss, expected_aud }`. Derives `serde::Deserialize` |
| `JwtAlgorithm` | `config.rs` | `Hs256`, `Rs256`, `Es256` |
| `Hs256AuthenticationProvider` | `authenticator.rs` | Single-algorithm JWT provider |
| `Rs256AuthenticationProvider` | `authenticator.rs` | Single-algorithm JWT provider |
| `Es256AuthenticationProvider` | `authenticator.rs` | Single-algorithm JWT provider |
| `RESOLVER_POOL` | `authenticator.rs` | Static `OnceLock<ThreadPool>` — 4-thread bridge for async→sync key resolution |

## 2. Integration Points

- `ServiceContext.security: Option<Arc<SecurityContext>>` — explicit propagation, no ambient state
- `RuntimeBuilder::with_security(authn, authz)` — single provider slot; multi-issuer needs a composite
- `Interceptor` trait (`on_request/on_response/on_error`) — correct seam for authentication interceptor
- No bearer token extraction exists in transport today
- No `#[authenticate]` macro — authentication is manual at the transport/interceptor boundary

## 3. Configuration Model

- `JwtProviderConfig` already derives `serde::Deserialize` — ready for `kit-config`
- Missing: `OidcProviderConfig`, `MultiIssuerConfig`
- `ConfigValue<T>` DI primitive available in `ego-service-sdk`

## 4. Gap Analysis per US

| US-ID | What Exists | What's Missing | Affected Crates |
|---|---|---|---|
| US-001 Bearer Auth | `Credential::Bearer`, `AuthenticationProvider`, `JwtValidationEngine` | `OidcAuthenticationProvider` composite; `MultiIssuerAuthenticationProvider` with `IssuerResolver`; `CredentialExtractor` SPI; `AuthenticationInterceptor` in `security-sdk` (transport-agnostic authentication engine) | `security-jwt` (extend), `security-sdk` |
| US-002 OIDC Discovery | `JwtProviderConfig.expected_iss` | `DiscoveryProvider` public trait; `HttpDiscoveryProvider` impl; `OidcEndpoints { jwks_uri, introspection_endpoint }` | New in `security-jwt` |
| US-003 JWT Validation | Full exp/nbf/iss/aud/sub; RS256/ES256 | `JwksProvider` public trait; `HttpJwksProvider` impl; `JwksKeyResolver` with `Arc<RwLock<>>` cache | `security-jwt` (`JwksKeyResolver`) |
| US-004 Opaque Introspection | `AuthenticationProvider` trait seam | `IntrospectionProvider` public trait; `HttpIntrospectionProvider` impl; `IntrospectionAuthenticationProvider` | New in `security-jwt` |
| US-005 JWKS Cache | `KeyResolver` trait | `JwksKeyResolver` cache + background refresh via Tokio; `StartupMode { FailFast, Lazy }` | New in `security-jwt` |
| US-006 Claims Mapping | `JwtValidationEngine` maps `sub`, `roles`, `tenant_id` | `ClaimSet` + `ClaimValue` in `ego-domain`; `PrincipalMapper` trait in `security-sdk`; `DefaultPrincipalMapper` in `security-jwt`; `ClaimSet` helpers: `subject()`, `roles()`, `tenant()`, `scope()` | `security-sdk` (trait), `security-jwt` (impl), `ego-domain` (value object) |
| US-007 Multi-Provider | Single authn slot in `RuntimeBuilder` | `IssuerResolver` trait; `StaticIssuerResolver`; `MultiIssuerAuthenticationProvider` | New in `security-jwt` |
| US-008 TestKit | `test_helpers` (`pub(crate)`) | `FakeIssuer/FakeDiscovery/FakeJwks/FakeIntrospection` in `security-jwt` (feature: `test-kit`) | `security-jwt` (`test-kit` feature) |

## 5. Architecture Recommendations (updated to reflect final design)

All OIDC implementation belongs in `security-jwt` (infrastructure layer).

New types added to `ego-domain`:
- `ClaimSet`, `ClaimValue` — domain value object, decouples security-sdk from serde_json

New contracts added to `security-sdk`:
- `PrincipalMapper` trait — maps `&ClaimSet` → `(Principal, Claims)`
- `CredentialExtractor` trait — extracts `Credential` from `&dyn RequestContext`
- `RequestContext` trait — transport-agnostic request abstraction
- `AuthenticationInterceptor` — transport-agnostic authentication engine; wires extractor + provider; HTTP, gRPC, Axum, Actix and any future transport each provide a thin adapter crate

New public traits in `security-jwt`:
- `DiscoveryProvider` — fetch OIDC discovery doc
- `JwksProvider` — fetch/parse JWKS
- `IntrospectionProvider` — call RFC 7662 introspection endpoint
- `IssuerResolver` — resolve `AuthenticationProvider` by issuer string

New structs in `security-jwt`:
- `OidcAuthenticationProvider` — JWT + opaque composite
- `JwksKeyResolver` — cache-first JWKS key resolution
- `MultiIssuerAuthenticationProvider` — routes by unverified `iss`
- `IntrospectionAuthenticationProvider` — opaque token validation
- `StaticIssuerResolver` — `HashMap`-backed `IssuerResolver`
- `OidcProviderConfig`, `MultiIssuerConfig` — `serde::Deserialize`
- `TokenFormat`, `StartupMode`, `OidcEndpoints` — supporting types
- `HttpDiscoveryProvider`, `HttpJwksProvider`, `HttpIntrospectionProvider` — HTTP impls
- TestKit (feature: `test-kit`): `FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`

No breaking changes to existing contracts required.

## 6. Dependency Gaps

| Dep | Why Needed |
|---|---|
| `reqwest` (0.12, tokio feature) | JWKS fetch, Discovery, Introspection HTTP calls |
| `tokio` (rt, sync features) | Background refresh task, `RwLock` for key cache |
| `url` (2) | Discovery URL construction and validation |
| `base64ct` | JWK `n`/`e` modulus decode (if not handled by `jsonwebtoken::jwk`) |

`jsonwebtoken = "9"` already includes `jsonwebtoken::jwk::JwkSet` — covers JWKS parsing.

## 7. Risk Flags

- **HIGH**: `AuthenticationProvider::authenticate` is sync (AD-004). All OIDC I/O must be pre-loaded (cache-first). Violating this blocks thread safety.
- **MEDIUM**: JWKS cache staleness during key rotation — retry with forced refresh on `InvalidSignature`
- **MEDIUM**: Discovery unavailability at startup — manual JWKS URI override required as escape hatch
- **MEDIUM**: Multi-issuer routing parses unverified `iss` — must not be used as trust assertion; signature verification is the trust gate
- **LOW**: `test-kit` feature must not reach production builds
