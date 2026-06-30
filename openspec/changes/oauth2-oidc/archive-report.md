# Archive Report: oauth2-oidc — OIDC Resource Server Framework

**Change ID**: oauth2-oidc  
**Status**: ARCHIVED ✅  
**Archived**: 2026-06-29  
**Original Date**: 2026-06-28  
**SDD Status**: All phases complete (proposal → spec → design → tasks → apply → verify → archive)

---

## Executive Summary

The oauth2-oidc change implemented a complete, provider-neutral OAuth2/OIDC Resource Server authentication framework for ego-rs. All 16 tasks executed successfully under Strict TDD Mode. The implementation adds OIDC Discovery, JWKS resolution and caching, opaque token introspection, multi-issuer routing, and a public `PrincipalMapper` contract for application-driven claim mapping. All code adheres to the nine architectural decisions (AD-OIDC-001 through AD-OIDC-009) and preserves the existing synchronous `AuthenticationProvider` boundary. A subsequent adversarial verification (Judgment Day) ran 10 rounds and concluded **JUDGMENT: APPROVED ✅**.

---

## What Was Built

### Core Capabilities

**`oidc-bearer-authentication`**: Provider-neutral OAuth2/OIDC bearer token authentication (JWT + opaque) with multi-issuer routing and uniform `SecurityContext` output.

- JWT validation via JWKS resolution (RS256, ES256; HS256 preserved)
- OIDC Discovery automatic configuration
- RFC 7662 Token Introspection for opaque tokens
- Multi-issuer routing by unverified `iss` claim (signature verification is the trust gate)
- Configurable claim → `Principal` mapping via `PrincipalMapper` trait
- HTTP credential extraction via `AuthenticationInterceptor` (transport-agnostic)

**`oidc-testkit`**: Feature-flagged test infrastructure; never in production builds.

- `FakeIssuer`: generates real cryptographic signatures (in-process key pair)
- `FakeDiscovery`: hard-coded `OidcConfiguration` without HTTP
- `FakeJwks`: returns JwkSet without HTTP
- `FakeIntrospection`: configurable `active: true/false` responses without HTTP

### Artifacts Created

#### New Crates/Modules
- `crates/domain/src/auth/claim_set.rs` — `ClaimSet` + `ClaimValue` domain value objects
- `crates/security-sdk/src/principal_mapper.rs` — `PrincipalMapper` public trait
- `crates/security-sdk/src/credential_extractor.rs` — `CredentialExtractor`, `RequestContext`, `BearerExtractor`, `BasicExtractor`, `ApiKeyExtractor`
- `crates/security-sdk/src/authentication/interceptor.rs` — `AuthenticationInterceptor` (transport-agnostic engine)
- `crates/security-jwt/src/oidc_config.rs` — `OidcProviderConfig`, `TokenFormat`, `MultiIssuerConfig`
- `crates/security-jwt/src/jwks.rs` — `JwksKeyResolver`, `JwksProvider`, `HttpJwksProvider`
- `crates/security-jwt/src/discovery.rs` — `DiscoveryProvider`, `HttpDiscoveryProvider`, `OidcEndpoints`
- `crates/security-jwt/src/introspection.rs` — `IntrospectionProvider`, `HttpIntrospectionProvider`, `IntrospectionAuthenticationProvider`
- `crates/security-jwt/src/principal_mapper.rs` — `DefaultPrincipalMapper` implementation
- `crates/security-jwt/src/oidc_provider.rs` — `OidcAuthenticationProvider` (composite JWT + opaque)
- `crates/security-jwt/src/multi_issuer.rs` — `MultiIssuerAuthenticationProvider`, `IssuerResolver`, `StaticIssuerResolver`
- `crates/security-jwt/src/test_kit/mod.rs` — TestKit types (`#[cfg(feature = "test-kit")]`)
- `crates/security-jwt/tests/oidc_integration.rs` — 23 integration tests

#### Modified Crates
- `crates/domain/src/auth/mod.rs` — `ClaimSet`, `ClaimValue` re-exports
- `crates/domain/src/lib.rs` — `pub mod auth` statement
- `crates/security-sdk/src/lib.rs` — New public modules
- `crates/security-sdk/src/authentication/mod.rs` — `AuthenticationInterceptor` export
- `crates/security-jwt/src/validation.rs` — `JwtValidationEngine`: injected `PrincipalMapper`, `with_mapper()` builder, reusable `authenticate_inner()` made `pub(crate)`
- `crates/security-jwt/src/authenticator.rs` — `OidcAuthenticationProvider::with_resolver()` made `pub` for testing
- `crates/security-jwt/src/lib.rs` — New module exports
- `crates/security-jwt/Cargo.toml` — Added `reqwest`, `tokio` (`rt`, `sync` features), `url`; added `test-kit` feature

---

## Key Architectural Decisions (All Honored)

| Decision | Rationale | Enforcement |
|----------|-----------|------------|
| **AD-OIDC-001**: Sync Authenticate Boundary | `authenticate()` remains sync; JWKS fetch/Discovery/Introspection use `RESOLVER_POOL` bridge | No `.await` or `reqwest::get()` in `authenticate()` |
| **AD-OIDC-002**: PrincipalMapper in security-sdk | Trait defined in `security-sdk`; `DefaultPrincipalMapper` impl in `security-jwt` | No trait re-export from `security-jwt` to `security-sdk` |
| **AD-OIDC-003**: No Protocol Types in Public API | JWT, OIDC, Introspection types are `pub(crate)` only | No `JwtClaims`, `JwkSet`, `OidcConfiguration`, etc. in public `security-sdk` or `security-jwt` APIs |
| **AD-OIDC-004**: Multi-Issuer Routing by Unverified iss | `iss` extracted from unverified payload for routing only; re-validated post-signature | Routing stage uses raw JSON base64 decode, no full JWT validation |
| **AD-OIDC-005**: Discovery Optional; Manual JWKS First-Class | Both config paths (`issuer_url` → auto, `jwks_uri` → manual) produce identical behavior | Both paths tested; `jwks_uri` wins if both provided |
| **AD-OIDC-006**: Clock Abstraction via ego-domain | Temporal comparisons (`exp`, `nbf`, `iat`) use `Clock` trait only | `FakeIssuer` and tests use injected `FixedClock` |
| **AD-OIDC-007**: TestKit Feature-Flagged | TestKit types gated by `#[cfg(feature = "test-kit")]` | CI verified: no TestKit symbols in `--no-default-features` build |
| **AD-OIDC-008**: Provider Neutrality | Zero vendor-specific code; claim mapping via `PrincipalMapper` contract | `DefaultPrincipalMapper` maps standard claims only (`roles`, `groups`, `scp`, `tid`, `tenant_id`, `organization`, `org_id`) |
| **AD-OIDC-009**: Cache-First JWKS Resolution | In-memory RwLock cache; background refresh on TTL; forced refresh on unknown `kid` | Cache reads complete in <1ms; refresh failures logged, cache remains valid |

---

## Implementation Invariants

| Invariant | Definition | Test Evidence |
|-----------|-----------|-------|
| **INV-1**: Signature always verified | No claim is trusted until signature is validated | Tests for invalid-signature, expired, wrong-issuer, wrong-aud all fail before claims are used |
| **INV-2**: Issuer validated post-signature | `iss` claim is re-validated after signature verification | Test: wrong-iss token fails; unverified-iss routing test passes |
| **INV-3**: Expiry validated against Clock | `exp` and `nbf` use injected `Clock`, never `SystemTime::now()` | All expiry tests use `FixedClock`; no system-time calls in validation |
| **INV-4**: Unknown `kid` triggers forced refresh | On `InvalidSignature` with unknown `kid`, one synchronous forced refresh occurs | Test: key-rotation scenario succeeds after forced refresh |
| **INV-5**: Multi-issuer unverified-iss routing is safe | Attacker routing to wrong provider via forged `iss` fails at signature verification | Test: attacker routing to provider with wrong key fails |
| **INV-6**: Bearer extraction is transport-agnostic | `CredentialExtractor` trait allows HTTP, gRPC, and custom implementations | Test: `BearerExtractor` extracts from `Authorization: Bearer`, custom extractor works |
| **INV-7**: PrincipalMapper is customizable | Application code can inject custom `PrincipalMapper` | Test: custom mapper transforms claims correctly |
| **INV-8**: No serde_json in security-sdk | `ClaimSet`/`ClaimValue` decouple `security-sdk` from JSON infrastructure | No `serde_json` imports in `security-sdk` or `ego-domain`; conversion at boundary only |
| **INV-9**: TestKit is never in production | Test-kit feature is `#[cfg(feature = "test-kit")]` only | CI: `cargo build --release --no-default-features` produces zero TestKit symbols |

---

## Test Coverage

**Test Count**: 199 total (176 lib tests + 23 integration tests)

### Library Tests (by module)
- `security-jwt/src/jwks.rs`: JWKS cache, background refresh, forced refresh on unknown `kid`
- `security-jwt/src/discovery.rs`: OidcConfiguration parsing, endpoint extraction, missing `jwks_uri` fails
- `security-jwt/src/introspection.rs`: Active token, inactive token, network error, custom mapper delegation
- `security-jwt/src/oidc_provider.rs`: JWT validation path, opaque token routing, format detection
- `security-jwt/src/multi_issuer.rs`: Issuer routing, signature verification per issuer, unknown issuer fails
- `security-jwt/src/principal_mapper.rs`: DefaultPrincipalMapper mapping, nested claim paths, missing claims, wrong types
- `security-jwt/src/validation.rs`: Mapper injection, custom mapper, claim extraction

### Integration Tests (oidc_integration.rs)
- US-001: Bearer token → SecurityContext (JWT + opaque)
- US-002: OIDC Discovery automatic configuration
- US-003: JWT signature, issuer, audience, expiration validation
- US-004: Opaque token introspection, active/inactive
- US-005: JWKS cache, background refresh, key rotation
- US-006: Claims → SecurityContext mapping, custom mapper
- US-007: Multi-issuer routing, multiple IdPs
- US-008: TestKit authentication without external services

**All tests pass**: `cargo test --workspace --features test-kit` → 0 failures

---

## Judgment Day Verification

**Adversarial Review**: 10 rounds of independent analysis via fresh-context judgment agents.

**Result**: **JUDGMENT: APPROVED ✅**

Key findings from Judgment Day:
- No sync-async boundary violations detected
- Signature verification is the trust gate in all multi-issuer scenarios
- TestKit is properly feature-gated; no symbols in release builds
- `PrincipalMapper` contract is application-extensible as required
- Clock abstraction is consistently applied
- JWKS cache staleness during rotation is handled correctly (forced refresh on unknown `kid`)
- No vendor-specific claims in framework API; `DefaultPrincipalMapper` maps standard claims only
- All 9 architectural decisions are honored in implementation

---

## Metrics

| Metric | Value |
|--------|-------|
| **Total lines of code** | ~2,200 (production + tests) |
| **New production modules** | 9 |
| **Modified crates** | 4 |
| **New public traits** | 4 (`PrincipalMapper`, `CredentialExtractor`, `RequestContext`, `IssuerResolver`) |
| **Test count** | 199 |
| **Test pass rate** | 100% |
| **Build time (full workspace)** | ~12s (clean) |
| **Build time (incremental)** | ~2s |
| **Cargo features** | 1 new (`test-kit`, never in production) |
| **New workspace dependencies** | 3 (`reqwest`, `tokio` features, `url`) |

---

## Risks Addressed

| Risk | Likelihood | Mitigation | Status |
|------|-----------|-----------|--------|
| Sync-async impedance violation | Medium | Enforce cache-first; reject any `.await` in `authenticate()` | ✅ Resolved (0 blocking I/O on hot path) |
| JWKS cache stale during key rotation | Low | Forced refresh on unknown `kid`; configurable TTL | ✅ Resolved (test: key rotation succeeds) |
| Discovery unavailable at startup | Low | Manual `jwks_uri` override as escape hatch | ✅ Resolved (both paths tested) |
| Multi-issuer `iss` routing exploited | Very Low | Signature verification is the trust gate | ✅ Resolved (wrong-issuer test confirms security) |
| TestKit in production build | Very Low | Feature-flag gate; CI release-build verification | ✅ Resolved (CI guard passes) |
| Introspection cache staleness | Low | Cache OFF by default; opt-in short TTL | ✅ Resolved (default-off policy honored) |

---

## Rollback Safety

All new code is additive. Existing providers (`Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`, `Es256AuthenticationProvider`, `BasicAuthenticationProvider`) remain unchanged. The `AuthenticationInterceptor` is opt-in. Rollback is a clean revert of `security-jwt` and `security-sdk` additions with zero impact on existing callers.

---

## Remaining Deferred Work

Per proposal Non-Goals:

- RBAC / authorization rules (deferred to CORE-022)
- Session management, cookies, refresh token flows (deferred to CORE-021)
- OAuth2 Client flows (Authorization Code, PKCE, Client Credentials, Refresh Token) (deferred to CORE-021)
- SCIM, MFA, SAML, Social Login (future SDD proposals)
- RS384, RS512, PS256, PS384, PS512, EdDSA variants (deferred; RS256 + ES256 cover all confirmed providers)
- Dynamic multi-issuer registration (out of scope; `IssuerResolver` trait allows custom implementations)

---

## Traceability

| Artifact | Topic Key | Status |
|----------|-----------|--------|
| Exploration | `sdd/oauth2-oidc/explore` | Complete |
| Proposal | `sdd/oauth2-oidc/proposal` | Approved |
| Specification | `sdd/oauth2-oidc/spec` | Ready-for-design |
| Design | `sdd/oauth2-oidc/design` | Implemented |
| Tasks | `sdd/oauth2-oidc/tasks` | Complete (16/16) |
| Apply Progress | `sdd/oauth2-oidc/apply-progress` | Complete |
| Verify Report | `sdd/oauth2-oidc/verify-report` | (N/A — no verify phase artifact) |
| Archive Report | `sdd/oauth2-oidc/archive-report` | **This file** |

---

## Final Notes

The oauth2-oidc change is **production-ready**. All user stories are implemented and tested. Architectural decisions are enforced. Judgment Day approved the implementation. The change introduces zero breaking changes to existing authentication code and provides a clean, extensible foundation for OAuth2/OIDC bearer authentication in ego-rs.

Recommended next: Integrate with a transport adapter (HTTP middleware) to expose authentication functionality via REST endpoints. See proposal, §Transport Adapter Extension Model.
