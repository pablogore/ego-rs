# Proposal: oauth2-oidc — OIDC Resource Server Framework

_Date: 2026-06-28 | Author: SDD pipeline | Status: approved_

> **Scope note**: Covers Resource Server authentication only. OAuth2 Client flows (Authorization Code, PKCE, Client Credentials, Refresh Token) are deferred to CORE-021.

---

## 1. Executive Summary

ego-rs has a JWT authentication stack (`security-jwt`) that handles statically-configured single-algorithm providers but has no ability to speak OIDC: no discovery, no JWKS resolution, no opaque token introspection, and no multi-issuer routing. This proposal extends `security-jwt` and `security-sdk` to add a complete, provider-neutral OAuth2/OIDC authentication layer — from Discovery through claims mapping — while preserving the existing sync `AuthenticationProvider` contract and zero-vendor-lock-in philosophy. The output of every authentication call is a uniform `SecurityContext`; no OIDC protocol types are exposed to application code.

---

## 2. Problem Statement

### What is broken today

| Gap | Impact |
|---|---|
| No JWKS resolution or cache | Cannot validate RS256/ES256 tokens from any real IdP (Keycloak, Cognito, Auth0, Entra ID, etc.) |
| No OIDC Discovery | Every deployment requires manual key configuration; breaks on key rotation |
| No opaque token introspection | OAuth2 opaque tokens (common in client-credentials flows) are unvalidatable |
| No multi-issuer routing | A service cannot authenticate users from more than one IdP simultaneously |
| No claims mapper contract | `groups`, `roles`, `scp`, `organization`, `tenant` → `Principal` mapping is hardcoded in `JwtValidationEngine`; downstream apps cannot customize it |
| No transport bearer extraction | No code extracts `Authorization: Bearer <token>` from HTTP; authentication must be wired manually by each service author |
| No public TestKit | Tests that exercise authentication must run against live IdPs or maintain hand-rolled stubs |

### Business impact

Services built on ego-rs cannot integrate with any standard enterprise identity provider today. Every team that needs authentication writes their own ad-hoc JWT validation, creating inconsistent security postures and duplicated maintenance surface. Adding first-class OIDC support removes the gap between ego-rs and production-grade service frameworks.

---

## 3. Goals and Non-Goals

### Goals

- Authenticate `Credential::Bearer` tokens (JWT and opaque) via a uniform `AuthenticationProvider` API.
- Support OIDC Discovery automatically; support manual JWKS URI configuration as an equal first-class path.
- Validate JWT signatures, issuer, audience, expiry (clock-injected), and `nbf` against JWKS-resolved keys.
- Support opaque token validation via RFC 7662 Token Introspection.
- Resolve, cache, and background-refresh JWKS; retry with forced refresh on `InvalidSignature`.
- Expose `PrincipalMapper` as a public contract in `security-sdk` so application code can customize claim → `SecurityContext` mapping.
- Route incoming tokens to the correct sub-provider based on unverified `iss` claim; signature is the trust gate.
- Provide a feature-flagged TestKit (`FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`) so tests require no external services.
- Add an `AuthenticationInterceptor` in `security-sdk` that extracts credentials via a `CredentialExtractor` SPI, authenticates, and populates `ServiceContext.security`. The interceptor is transport-agnostic; transport adapters (`security-http`, `security-grpc`, `security-axum`, `security-actix`, etc.) wrap it and map `AuthenticationError` to protocol responses.

### Non-Goals (explicit, deferred or permanently out of scope)

- RBAC / authorization rules — `AuthorizationProvider` is out of scope; output is `SecurityContext` only.
- Session management, cookies, refresh token flows.
- UI / login screens, OAuth2 authorization code redirect flows.
- SCIM, MFA, SAML, Social Login.
- Device Flow, Token Exchange, fine-grained authorization.
- User management APIs.
- Vendor-specific SDK integrations (Keycloak admin API, Cognito user pools, etc.).
- `#[authenticate]` macro (deferred; transport interceptor covers the primary use case).
- RS384, RS512, PS256, PS384, PS512, EdDSA algorithm variants (deferred; RS256 + ES256 cover all confirmed providers).

---

## 4. User Stories and Acceptance Criteria

### US-001 — Authenticate Bearer Tokens

> As a developer, I want to authenticate OAuth2/OIDC Bearer tokens to obtain a reliable SecurityContext.

**Acceptance criteria:**
- `Credential::Bearer(token)` passed to any OIDC provider returns `Ok(SecurityContext)` for a valid token.
- The returned `SecurityContext` contains `principal.subject_id` mapped from `sub`.
- An expired token returns `Err(AuthenticationError::ExpiredToken)`.
- An invalid signature returns `Err(AuthenticationError::InvalidSignature)`.
- The provider is agnostic to JWT vs. opaque format — the caller passes `Credential::Bearer`; the provider routes internally.
- No OIDC-specific type (e.g., `IdToken`, `BearerToken`, `JwtClaims`) appears in the return type or error type visible to application code.

### US-002 — Discover Identity Providers

> As a developer, I want to automatically resolve OIDC configuration via Discovery to avoid manual configuration.

**Acceptance criteria:**
- Given an `issuer_url`, the provider fetches `{issuer_url}/.well-known/openid-configuration` at startup and extracts `jwks_uri` (and `introspection_endpoint` if present).
- Discovery is attempted once at startup; failures are surfaced as a startup error (not a runtime panic).
- A manual `jwks_uri` override bypasses Discovery entirely — no HTTP call is made.
- Discovery result is cached in memory; no re-fetch happens per request.
- Both Discovery and manual-JWKS paths produce identical runtime behaviour.

### US-003 — Validate JWT Tokens

> As a developer, I want to validate JWTs (signature, issuer, audience, expiration) to guarantee authenticity.

**Acceptance criteria:**
- RS256 and ES256 signed tokens are validated against JWKS-resolved keys.
- HS256 tokens are validated against a pre-shared key (existing behaviour preserved).
- `exp` is validated against the injected `Clock` — not `SystemTime::now()` directly.
- `nbf` is validated when present.
- `iss` is validated against the configured expected issuer after signature verification.
- `aud` is validated when configured.
- A token with an unknown `kid` triggers a one-time forced JWKS refresh before returning `InvalidSignature`.
- Tokens with unsupported algorithms return `Err(AuthenticationError::AlgorithmNotSupported)`.

### US-004 — Validate Opaque Tokens

> As a developer, I want to support Introspection to validate opaque tokens.

**Acceptance criteria:**
- Given a `Credential::Bearer(opaque_token)`, the introspection provider calls the RFC 7662 `/introspect` endpoint.
- An `active: true` response produces a `SecurityContext` from the introspection claims.
- An `active: false` response returns `Err(AuthenticationError::InvalidToken)`.
- Network errors return `Err(AuthenticationError::ProviderUnavailable(...))`.
- The introspection HTTP call is made synchronously via the RESOLVER_POOL bridge (cache-first where applicable).
- Application code sees only `Credential::Bearer` → `SecurityContext`; no introspection response type is public.

### US-005 — Resolve and Cache JWKS

> As a developer, I want to automatically resolve and cache JWKS to support key rotation.

**Acceptance criteria:**
- On provider startup, JWKS is fetched from the resolved `jwks_uri` and stored in an `Arc<RwLock<HashMap<kid, VerificationKey>>>`.
- `KeyResolver::resolve(kid, algorithm)` reads from the in-memory cache (no HTTP call on the hot path).
- A background task refreshes the cache on a configurable TTL (default 5 minutes).
- When `resolve()` cannot find a `kid`, it triggers one synchronous forced refresh before returning `KeyNotFound`.
- Refresh failures are logged and the stale cache remains valid (no availability interruption).
- The cache is thread-safe; concurrent reads do not block each other.

### US-006 — Map Claims to SecurityContext

> As a developer, I want to transform OIDC claims into a uniform SecurityContext.

**Acceptance criteria:**
- A default `DefaultPrincipalMapper` maps: `sub` → `principal.subject_id`; `roles`/`realm_access.roles` → `principal.roles`; `groups` → `principal.roles`; `scp`/`scope` → `claims.custom["scope"]`; `tid`/`tenant_id`/`tenant` → `principal.tenant_id`; `organization` → `claims.custom["organization"]`.
- `PrincipalMapper` is a public trait in `security-sdk` with signature `fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError>`.
- Application code can provide a custom `PrincipalMapper` implementation and inject it into the provider.
- The `JwtValidationEngine` no longer hardcodes claim mapping — it delegates to the injected `PrincipalMapper`.

### US-007 — Support Multiple Identity Providers

> As a developer, I want to authenticate users from multiple Identity Providers.

**Acceptance criteria:**
- `MultiIssuerAuthenticationProvider` accepts an `IssuerResolver` trait implementation (default: `StaticIssuerResolver` backed by `HashMap<String, Arc<dyn AuthenticationProvider>>`) mapping `issuer_url → provider`.
- On `authenticate(Credential::Bearer(token))`, the router extracts `iss` from the unverified token payload and looks up the corresponding provider.
- If no provider is found for the `iss`, returns `Err(AuthenticationError::InvalidToken)`.
- The selected provider performs full validation including signature verification.
- `MultiIssuerAuthenticationProvider` itself implements `AuthenticationProvider` — wraps as `Arc<dyn AuthenticationProvider>`, no `RuntimeBuilder` API change needed.
- The unverified `iss` is used only for routing — it is NOT a trust assertion.

### US-008 — Test Authentication Without External Services

> As a developer, I want an OIDC TestKit to run tests without depending on external services.

**Acceptance criteria:**
- `security-jwt` exposes a `test-kit` Cargo feature (never enabled in production profiles).
- `FakeIssuer`: signs JWT tokens with an in-process key; configurable `sub`, `iss`, `aud`, custom claims.
- `FakeDiscovery`: returns a hard-coded `OidcConfiguration` without HTTP.
- `FakeJwks`: returns a `JwkSet` for the `FakeIssuer`'s key without HTTP.
- `FakeIntrospection`: returns configurable `active: true/false` responses without HTTP.
- TestKit types use `FixedClock` (already in `test_helpers`) for deterministic expiry testing.
- All 8 US acceptance criteria can be covered by tests that use only the TestKit (no live IdP required).

---

## 5. Architectural Decisions

### AD-OIDC-001 — Sync Authenticate Boundary

**Decision**: `AuthenticationProvider::authenticate()` MUST remain synchronous. All async I/O (JWKS fetch, Discovery, Introspection) MUST be performed off the hot path.

**Rationale**: AD-004 (pre-existing) establishes that `authenticate` is CPU-bound and performs no I/O. Changing it to async would break every existing provider and all call sites.

**Constraint**: The existing `RESOLVER_POOL` pattern (`OnceLock<ThreadPool>` in `authenticator.rs`) is the approved sync-async bridge. JWKS refresh runs in a background Tokio task; `authenticate()` reads from the in-memory cache only. Introspection calls are dispatched through the same bridge.

**Violation criteria**: Any `reqwest::get()` or `.await` inside `authenticate()` is an architectural violation.

---

### AD-OIDC-002 — PrincipalMapper is a Contract in security-sdk

**Decision**: `PrincipalMapper` is a public trait defined in `security-sdk`, not an internal detail of `security-jwt`. Its signature operates on `&ClaimSet` (a domain type), keeping `security-sdk` free of `serde_json::Value`.

**Rationale**: Application code needs to customize claim → `Principal` mapping (e.g., map a vendor-specific `custom:groups` claim). The trait must be on the application-layer side of the dependency boundary. `security-jwt` implementations depend on `security-sdk`, never the reverse.

**Layer**: `security-sdk` (application-adjacent contract layer). `security-jwt` provides `DefaultPrincipalMapper` as the production default. `DefaultPrincipalMapper` is NOT re-exported from `security-sdk`.

---

### AD-OIDC-003 — No Protocol-Specific Types in Public SDK API

**Decision**: Types such as `JwtClaims`, `Jwk`, `JwkSet`, `OidcConfiguration`, `DiscoveryDocument`, `BearerToken`, `IdToken`, `IntrospectionResponse`, `AccessToken` MUST NOT appear in any public API surface of `security-sdk` or in any type that application code imports.

**Rationale**: Application code must be portable across providers. Exposing protocol types creates vendor coupling. The boundary is `Credential → AuthenticationProvider → SecurityContext`. Everything else is `pub(crate)` in `security-jwt`.

**Enforcement**: All OIDC parsing types live in `security-jwt` with `pub(crate)` or module-private visibility. The `PrincipalMapper` trait signature uses `&ClaimSet` (a domain value object) — no OIDC types or `serde_json` types leak through it.

---

### AD-OIDC-004 — Multi-Issuer Routing by Unverified iss

**Decision**: `MultiIssuerAuthenticationProvider` extracts `iss` from the unverified JWT payload solely to select the sub-provider. The `iss` value at this step is NOT trusted.

**Rationale**: To verify a token you need the correct key; to find the correct key you need the issuer; the issuer is in the token. This is an unavoidable bootstrap. Safety is maintained because: (a) the selected provider re-validates `iss` after signature verification, (b) an attacker forging `iss` can only route to a provider that will fail their fake token on signature check.

**Constraint**: `iss` parsing at routing stage uses `serde_json::from_str` on the raw base64-decoded payload — no full JWT decode, no signature step.

---

### AD-OIDC-005 — Discovery is Optional; Manual JWKS URI is First-Class

**Decision**: `OidcProviderConfig` MUST support two fully equivalent configuration paths: (a) `issuer_url` → automatic Discovery, and (b) `jwks_uri` (explicit) → skip Discovery entirely. Both paths produce the same runtime state.

**Rationale**: Some deployments cannot reach the IdP's discovery endpoint at startup (air-gapped, firewall rules). Others do not want the Discovery round-trip latency. Neither path should be second-class.

**Constraint**: If both `issuer_url` and `jwks_uri` are provided, `jwks_uri` wins (no Discovery call). If only `issuer_url` is provided, Discovery is attempted. If neither is provided, provider construction MUST fail at build time (not at first authenticate call).

---

### AD-OIDC-006 — Clock Abstraction via ego-domain Clock Trait

**Decision**: All temporal comparisons (`exp`, `nbf`, `iat`) MUST use the `Clock` trait from `ego-domain`. Direct calls to `SystemTime::now()`, `Utc::now()`, or `chrono::Utc::now()` are prohibited in authentication logic.

**Rationale**: Deterministic testing requires time injection. The `Clock` trait already exists and `JwtValidationEngine` already uses it. Consistency is mandatory.

**Constraint**: `FakeIssuer` in the TestKit MUST accept a `Clock` to generate tokens with deterministic expiry. Test assertions on expiry MUST use the same `FixedClock` instance.

---

### AD-OIDC-007 — TestKit is Feature-Flagged; Never in Production Builds

**Decision**: All TestKit types (`FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`) MUST be gated behind `#[cfg(feature = "test-kit")]`. The `test-kit` feature MUST NOT be listed in any workspace member's `[dependencies]` block — only in `[dev-dependencies]`.

**Rationale**: TestKit types exist only to make test code ergonomic. Including them in production binaries adds attack surface (a `FakeIssuer` in a production build can sign arbitrary tokens).

**Enforcement**: CI MUST build the workspace with `--no-default-features` and verify no TestKit symbols are in the resulting artifact. Pattern: same as `security-sdk`'s `dev-providers` / `test-helpers` features.

---

### AD-OIDC-008 — Provider Neutrality; Zero Vendor-Specific Code in Framework API

**Decision**: The framework contains zero vendor-specific dependencies, configuration keys, claim names, or API surfaces. Keycloak's `realm_access.roles`, Cognito's `cognito:groups`, Auth0's `permissions`, Entra ID's `tid` are handled ONLY through the `PrincipalMapper` contract — the application or a thin adapter provides the mapping.

**Rationale**: A framework that special-cases Keycloak forces every other provider to work around those special cases. The `PrincipalMapper` trait is the correct extension point.

**Constraint**: `DefaultPrincipalMapper` in `security-jwt` MAY map widely-used standard claim names (`groups`, `roles`, `scp`, `scope`, `tid`) because these appear in multiple providers' tokens as de-facto standards. It MUST NOT reference any vendor brand, vendor SDK, or vendor-proprietary claim name.

---

### AD-OIDC-009 — Cache-First JWKS Resolution (Extends AD-013)

**Decision**: `JwksKeyResolver::resolve()` MUST return from the in-memory cache without any I/O. A background task (Tokio `spawn`) is responsible for periodic refresh. A cache miss for an unknown `kid` MAY trigger one synchronous forced refresh via the RESOLVER_POOL bridge before returning `KeyNotFound`.

**Rationale**: The sync `authenticate()` contract (AD-OIDC-001) forbids blocking I/O. Cache-first is the only compatible model. The forced-refresh-on-miss is a controlled exception that tolerates key rotation without requiring manual intervention.

**Constraint**: The background refresh task MUST use exponential backoff on failure. Refresh failures MUST be logged with `tracing::warn!` and MUST NOT crash the service. The stale cache remains valid until a successful refresh.

---

## 6. Scope

### Crates Affected

| Crate | Impact | Changes |
|---|---|---|
| `security-sdk` (`crates/security-sdk/`) | Modified | Add `PrincipalMapper`, `CredentialExtractor`, `RequestContext` public traits; `AuthenticationInterceptor` (authentication engine) |
| `security-jwt` (`crates/security-jwt/`) | Extended | New modules; new deps; feature flag |
| `ego-domain` | Modified | New `ClaimSet` + `ClaimValue` value object |
| `service-sdk` | None | `RuntimeBuilder::with_security` accepts `Arc<dyn AuthenticationProvider>` — composite pattern is compatible |

### New Modules in security-jwt

| Module | File | Purpose |
|---|---|---|
| JWKS resolver | `src/jwks.rs` | `JwksKeyResolver`: cache-first, background refresh, forced-refresh-on-miss |
| Discovery client | `src/discovery.rs` | `OidcDiscoveryClient` trait + `HttpOidcDiscoveryClient` impl |
| Introspection client | `src/introspection.rs` | `IntrospectionClient` trait + `HttpIntrospectionClient` impl + `IntrospectionAuthenticationProvider` |
| OIDC composite provider | `src/oidc_provider.rs` | `OidcAuthenticationProvider`: detects JWT vs opaque, delegates to correct path |
| Multi-issuer router | `src/multi_issuer.rs` | `MultiIssuerAuthenticationProvider`: routes by unverified `iss` |
| OIDC config | `src/oidc_config.rs` | `OidcProviderConfig`, `MultiIssuerConfig` — derive `serde::Deserialize` |
| TestKit | `src/test_kit/mod.rs` | `FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection` — `#[cfg(feature = "test-kit")]` |

### New Surface in security-sdk

| Symbol | Kind | Visibility |
|---|---|---|
| `PrincipalMapper` | trait | `pub` |
| `CredentialExtractor` | trait | `pub` |
| `RequestContext` | trait | `pub` |
| `BearerExtractor`, `BasicExtractor`, `ApiKeyExtractor` | struct | `pub` — built-in extractor impls |
| `AuthenticationInterceptor` | struct | `pub` |
| `DefaultPrincipalMapper` | struct | `pub` — in `security-jwt` only; NOT re-exported from `security-sdk` |

### security-sdk — Authentication Engine

| Symbol | Location | Purpose |
|---|---|---|
| `AuthenticationInterceptor` | `crates/security-sdk/src/authentication/interceptor.rs` | Depends on `Arc<dyn CredentialExtractor>` + `Arc<dyn AuthenticationProvider>`; extracts credentials, authenticates, and populates `ServiceContext.security`. Propagates `AuthenticationError` to the caller — never maps errors to protocol responses. Transport adapters are responsible for converting the native request to `RequestContext` and mapping `AuthenticationError` to the appropriate protocol response (HTTP 401, gRPC UNAUTHENTICATED, GraphQL error, etc.). |

## Transport Adapter Extension Model

`security-sdk` is transport-agnostic. Each transport provides its own thin adapter:

| Adapter | Location | Maps to |
|---|---|---|
| `HttpAuthenticationMiddleware` | `security-http` (future crate) | HTTP 401 Unauthorized |
| `GrpcAuthenticationInterceptor` | `security-grpc` (future crate) | gRPC UNAUTHENTICATED |
| `GraphqlAuthenticationExtension` | `security-graphql` (future crate) | GraphQL authentication error |
| `AuthenticationLayer` | `security-axum` (future crate) | Axum middleware layer |
| `AuthenticationMiddleware` | `security-actix` (future crate) | Actix-web middleware |

Each adapter:
1. Converts its native request type to a `RequestContext` implementation
2. Invokes `AuthenticationInterceptor` (from `security-sdk`)
3. Maps `AuthenticationError` to the protocol-specific failure response

`security-sdk` never performs this mapping. `AuthenticationInterceptor` is the transport-independent authentication engine — it belongs to the authentication subsystem, not to HTTP or any other transport.

### Dependency Additions (security-jwt/Cargo.toml)

| Crate | Version | Reason |
|---|---|---|
| `reqwest` | 0.12 (features: `json`, `rustls-tls`) | JWKS fetch, Discovery, Introspection HTTP calls |
| `tokio` | 1 (features: `rt`, `sync`, `time`) | Background refresh task, `RwLock`, `sleep` |
| `url` | 2 | Discovery URL construction and validation |

Note: `base64ct` is NOT added — `jsonwebtoken = "9"` already handles JWK `n`/`e` decoding via `jsonwebtoken::jwk`.

---

## 7. Security Considerations

### Token Validation Requirements

- Signature MUST be verified before any claim is trusted.
- `iss` MUST be validated after signature verification against the configured expected issuer.
- `aud` MUST be validated when an expected audience is configured. Missing `aud` is a validation failure if `expected_aud` is set.
- `exp` MUST be validated using the injected `Clock`. Clock skew tolerance (if any) is a configuration parameter, not a hardcoded value.
- `nbf` MUST be validated when present.
- Tokens with no `exp` claim MUST be rejected unless a future explicit policy config allows it.

### JWKS Cache Staleness and Key Rotation

- Key rotation window: stale cache remains valid until the background refresh succeeds. Target refresh TTL: 5 minutes default, configurable.
- On `InvalidSignature` with a known `kid`, the provider MUST attempt a forced cache refresh exactly once before returning the error. This handles key rotation without service restart.
- On `InvalidSignature` with an unknown `kid`, the provider MUST attempt a forced cache refresh exactly once. If the key is still absent, return `KeyNotFound` (mapped to `InvalidToken`).
- JWKS responses MUST be validated for well-formedness before replacing the cache.

### Multi-Issuer Trust Boundary

- Unverified `iss` is used ONLY for provider selection (routing). It is extracted from the base64-decoded JWT payload WITHOUT signature verification.
- The selected sub-provider MUST re-validate `iss` after signature verification. A mismatch at post-verification is an `InvalidToken` result.
- An unknown `iss` (no matching provider) returns `InvalidToken` immediately — no attempt at validation is made.

### TestKit Isolation

- `test-kit` feature MUST be excluded from all production Cargo profiles.
- `FakeIssuer` generates real cryptographic signatures (in-process key pair) — it is not a stub that bypasses signature verification. Tests exercise the real validation path.
- CI must verify no `test-kit` symbols appear in release artifacts.

---

## 8. Resolved During Design

All open questions identified during planning were resolved in the design phase. No unresolved decisions remain before implementation.

| OQ | Question (summary) | Resolution |
|---|---|---|
| OQ-1 | ClaimsMapper in security-sdk vs re-export from security-jwt | AD-OIDC-002: `PrincipalMapper` trait defined directly in `security-sdk`. `DefaultPrincipalMapper` impl lives in `security-jwt`. No re-export. |
| OQ-2 | JWT vs opaque detection: format heuristic vs explicit config | `TokenFormat { Jwt, Opaque, Auto }` enum in `OidcProviderConfig`. `Auto` uses dot-count heuristic; explicit modes override it. |
| OQ-3 | Introspection result caching | Default off. Opt-in via `IntrospectionCacheConfig { enabled: bool, ttl_seconds: u64 }`. Cache key is SHA-256 of token. |
| OQ-4 | AuthenticationInterceptor: generic vs OIDC-specific | Generic — depends on `Arc<dyn CredentialExtractor>` + `Arc<dyn AuthenticationProvider>`. Located in `security-sdk` (authentication subsystem, transport-agnostic). |
| OQ-5 | Discovery succeeds but jwks_uri absent | Fail-fast at startup. OIDC Core spec requires `jwks_uri`; absent = non-compliant IdP = construction error. |
| OQ-6 | MultiIssuer: dynamic vs static registration | `IssuerResolver` is a public SPI — custom implementations can resolve by hostname, tenant, realm, etc. The only implementation shipped with this capability is `StaticIssuerResolver` (static map built at startup). Dynamic registration (mutating the resolver at runtime) is out of scope and deferred. The SPI exists for extensibility; no dynamic impl is promised. |

---

## 9. Success Metrics

- [ ] All 8 US acceptance criteria are covered by tests that pass `cargo test --workspace`.
- [ ] Zero tests in the suite require a live IdP — all OIDC paths are covered using the TestKit (`FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`).
- [ ] A service using `MultiIssuerAuthenticationProvider` with two providers (e.g., one RS256 JWKS-backed, one introspection-backed) authenticates tokens from both and returns the correct `SecurityContext` for each, proven by a test.
- [ ] `cargo build --release --no-default-features` produces no TestKit symbols in the `security-jwt` artifact.
- [ ] No OIDC-specific type (`JwtClaims`, `Jwk`, `OidcConfiguration`, `IntrospectionResponse`, etc.) appears in any `pub` API of `security-sdk`.
- [ ] `AuthenticationProvider::authenticate` signature is unchanged — all existing providers (`Hs256`, `Rs256`, `Es256`, `BasicAuthenticationProvider`) continue to compile and pass their tests without modification.
- [ ] A custom `PrincipalMapper` implementation (e.g., mapping `custom:groups` → `Principal.roles`) can be injected into `OidcAuthenticationProvider` and is exercised by a test.
- [ ] Discovery path and manual-JWKS-URI path produce identical `SecurityContext` output in tests — no behavioural difference.

---

## Capabilities

### New Capabilities

- `oidc-bearer-authentication`: Provider-neutral OAuth2/OIDC bearer token authentication (JWT + opaque) with multi-issuer routing and uniform `SecurityContext` output. Covers US-001 through US-007.
- `oidc-testkit`: Feature-flagged test infrastructure for OIDC authentication (`FakeIssuer`, `FakeDiscovery`, `FakeJwks`, `FakeIntrospection`). Covers US-008.

### Modified Capabilities

- `jwt-authentication`: Existing `security-jwt` JWT validation is extended with JWKS-backed key resolution and `PrincipalMapper` delegation. Existing `Hs256/Rs256/Es256AuthenticationProvider` are not modified, but `JwtValidationEngine` is refactored to delegate claim mapping via `PrincipalMapper`.
- `security-contracts`: `security-sdk` gains the `PrincipalMapper`, `CredentialExtractor`, and `RequestContext` public traits.

---

## Affected Areas

| Area | Impact | Description |
|---|---|---|
| `crates/security-sdk/` | Modified | Add `PrincipalMapper`, `CredentialExtractor`, `RequestContext` traits; `AuthenticationInterceptor` (authentication engine, transport-agnostic) |
| `crates/security-jwt/` | Extended | 7 new modules, `test-kit` feature, 3 new deps |
| `crates/domain/` | Modified | New `ClaimSet` + `ClaimValue` value objects |
| `crates/security-jwt/Cargo.toml` | Modified | Add `reqwest`, `tokio`, `url` |
| `Cargo.toml` (workspace) | Modified | Add `reqwest` as workspace dep if other crates need it |

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Sync-async impedance violation | Medium | High — thread starvation, deadlock | Enforce cache-first discipline in code review; CI test that `authenticate()` completes in < 1ms with warm cache |
| JWKS cache stale during rapid key rotation | Low | Medium — auth failures for rotation window | Forced refresh on `InvalidSignature`; configurable TTL down to 1 minute |
| Discovery unavailable at startup | Low | Medium — service won't start | Manual `jwks_uri` override as escape hatch; retry with backoff on startup |
| Multi-issuer `iss` routing exploited | Very Low | Low — attacker can only route, not bypass signature | Signature verification is the trust gate; test coverage for wrong-issuer routing |
| TestKit in production build | Very Low | High — `FakeIssuer` in prod is a backdoor | Feature-flag gate; CI release-build verification |
| OQ-3 introspection cache risk | Low | Medium — stale `active: true` for revoked tokens | Resolve in design phase; short TTL or no cache by default |

---

## Rollback Plan

All new code is additive. Existing providers (`Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`, `Es256AuthenticationProvider`, `BasicAuthenticationProvider`) are not modified. Rollback is a revert of the `security-jwt` and `security-sdk` additions with no impact on existing callers. The `AuthenticationInterceptor` in `security-sdk` is opt-in — removing it restores prior manual wiring.

---

## Dependencies

- `security-jwt` depends on `security-sdk` (already) — no new cross-crate dependency direction.
- `reqwest = "0.12"` — new workspace dependency.
- `tokio = "1"` — already in workspace; needs `rt` and `sync` features in `security-jwt`.
- `url = "2"` — new in `security-jwt`.
- No new crate is added to the workspace. All new code lives in existing crates.
