# Domain Authentication Module — Canonical Specification

**Introduced by**: CORE-011 JWT Authentication Provider (2026-06-23)
**Last updated**: CORE-013 JWT Providers + KeyResolver (2026-06-25)
**Decision Record**: ADR-009A (sync authentication boundary)
**Status**: Production

## Overview

The domain authentication module (`crates/domain/src/auth/`) defines the synchronous authentication data model for ego-rs. It separates the authenticated principal from raw token claims (`Claims`). The canonical principal type (`Principal`) lives in `security-sdk`; domain keeps pure data models only (`Claims`, `Credential`, `AuthenticationError`).

## Core Types

### StandardClaims

The IANA-registered JWT claims (RFC 7519 Section 4.1).

```rust
pub struct StandardClaims {
    pub exp: Option<DateTime<Utc>>,    // Expiration time
    pub nbf: Option<DateTime<Utc>>,    // Not-before time
    pub iat: Option<DateTime<Utc>>,    // Issued-at time
    pub jti: Option<String>,            // JWT ID
    pub iss: Option<String>,            // Issuer
    pub aud: Option<Vec<String>>,       // Audience(s)
}
```

**Invariants**:
- All fields are `Option` (claims are technically optional per RFC 7519)
- Implementations validate the subset they require (e.g., HS256 provider may require `exp`, `iat`, `iss`)

### Claims

Combined standard + custom claims.

```rust
pub struct Claims {
    pub standard: StandardClaims,                  // Standard registered claims
    pub custom: BTreeMap<String, serde_json::Value>, // Application-specific claims
}
```

**Invariants**:
- Custom claims are never merged into StandardClaims
- Custom map uses BTreeMap for deterministic ordering
- Identity fields (`sub`, `roles`, `tenant_id`/`tid`) are extracted into Principal (from `security-sdk`) and also preserved in custom map if their types are wrong (graceful degradation per CLAR-005)

### Credential

The authentication credential supplied by the caller.

```rust
#[non_exhaustive]
pub enum Credential {
    Basic { username: String, secret: String },
    Bearer(String),
    Custom { scheme: String, payload: Vec<u8> },
}
```

**Invariants**:
- `#[non_exhaustive]` allows future variants without breaking existing match arms
- `Bearer` carries the raw token string (without the `"Bearer "` prefix)
- `Debug` impl redacts `Bearer` and `Basic.secret` — token material never appears in logs
- Credential is consumed by value to ensure sensitive material is dropped after validation

### AuthenticationError

Errors that can occur during authentication.

```rust
pub enum AuthenticationError {
    InvalidToken(String),           // Malformed token, invalid structure, or security claim with unexpected type (see CLAR-005)
    ExpiredToken,                    // exp claim indicates token has expired
    MissingClaim(String),            // Required claim is absent (e.g., 'sub')
    InvalidSignature,                // Signature validation failed
    AlgorithmNotSupported(String),   // Token algorithm not supported by provider
    ProviderUnavailable(String),     // Backing store / verifier is unreachable
}
```

**Invariants**:
- Each variant carries enough context for logging without additional state
- Implementations MUST NOT panic; all errors are recoverable
- `InvalidToken` is used for both structural failures and wrong-type `sub` (see CLAR-005)
- `ProviderUnavailable` MUST be used when the failure is transient infrastructure, not a bad token

### Clock

An injectable time source for deterministic testing.

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}
```

**Invariants**:
- AuthenticationProvider implementations MUST use Clock::now() instead of Utc::now() directly
- Tests inject fixed-time Clock implementations for deterministic exp/nbf validation
- Implementations are Send + Sync for use in concurrent contexts

## Architectural Invariants

1. **Synchronous boundary**: All authentication is synchronous. Async wrappers (e.g., async task spawning) are left for higher layers or future work.

2. **No ambient context**: Authentication carries no implicit state. Principal and claims are always explicit in the SecurityContext return value.

3. **Deterministic ordering**: All collection types in Claims use BTreeMap. HashMap and HashSet MUST NOT appear in public API.

4. **Thread safety**: All public types and traits have Send + Sync bounds, enabling safe concurrent use via Arc.

5. **Clock injection**: All time-sensitive validation uses an injected Clock abstraction, enabling deterministic testing without mocking system calls.

6. **Claim classification**: Claims are split into three categories with distinct wrong-type behaviors. The `sub` claim is a required identity anchor — absent or non-string values reject the token immediately. Optional identity claims (`tenant_id`/`tid`, `roles`) degrade gracefully — the nominal field receives its zero value and the raw claim is preserved in `Claims.custom`. Security claims (`exp`, `nbf`, `iat`, `jti`, `iss`, `aud`) fail immediately with `AuthenticationError::InvalidToken` — no raw value is preserved. See CLAR-005.

7. **No type merging**: StandardClaims and Claims.custom are separate. Standard claims are never merged into a flat map.

## Clarifications

### CLAR-005 — Claim Classification

JWT claims fall into three categories with distinct wrong-type behaviors:

**`sub` (required identity anchor)**

- Absent → `Err(AuthenticationError::MissingClaim("sub"))`.
- Present but not a string → `Err(AuthenticationError::InvalidToken("sub claim is not a string"))`.
- Present but empty string → `Err(AuthenticationError::InvalidToken("invalid subject id"))`.
- Rationale: `sub` is the only claim that uniquely identifies an authenticated principal. An unknown or unrepresentable subject cannot form a valid `SecurityContext`.

**Optional identity claims**: `tenant_id`, `tid`, `roles`

- Wrong type MUST NOT fail authentication.
- Implementation MUST degrade gracefully: the nominal field receives its zero value (`None` for `Option<String>`, empty set for `BTreeSet<Role>`).
- The original raw claim value MUST be preserved in `Claims.custom` under its original key.
- Rationale: these are enrichment fields; a token with an unexpected encoding is still a valid, authenticated token — the principal is still identified by `sub`.

**Security claims**: `exp`, `nbf`, `iat`, `jti`, `iss`, `aud`

- Wrong type MUST fail authentication with `AuthenticationError::InvalidToken`.
- The raw value is NOT preserved; the request is rejected immediately before `SecurityContext` is produced.
- Rationale: these claims govern time-bound validity and trust constraints. Accepting a non-parseable value would silently bypass security checks.

**Scenario — `sub` absent**

Given a JWT with no `sub` key  
When an `AuthenticationProvider::authenticate` is called  
Then `Err(AuthenticationError::MissingClaim("sub"))` is returned

**Scenario — `sub` present but wrong type**

Given a JWT with `{ "sub": 123, "exp": 9999999999 }` (sub as integer, exp valid)  
When an `AuthenticationProvider::authenticate` is called  
Then `Err(AuthenticationError::InvalidToken("sub claim is not a string"))` is returned

**Scenario — security claim, wrong type**

Given a JWT with `{ "sub": "user-1", "exp": "not-a-number" }` (exp as string)  
When an `AuthenticationProvider::authenticate` is called  
Then `Err(AuthenticationError::InvalidToken("exp claim is not a valid integer"))` is returned

**Scenario — optional identity claim, wrong type**

Given a JWT with `{ "sub": "user-1", "tenant_id": 999, "exp": 9999999999 }`  
When an `AuthenticationProvider::authenticate` is called  
Then `Ok(SecurityContext)` is returned with:
- `principal.tenant_id == None` (graceful degradation)
- `claims.custom["tenant_id"] == 999` (raw value preserved)
- `claims.custom` does NOT contain `"exp"` (security claim values are never preserved in `Claims.custom`)

---

### CLAR-006 — Required Claim Semantics: `sub`

`sub` is the sole required identity claim. All three failure modes reject the token:

| Scenario | Example payload | Result |
|---|---|---|
| `sub` absent | `{}` | `Err(MissingClaim("sub"))` |
| `sub` present, not a string | `{"sub": 123}` | `Err(InvalidToken("sub claim is not a string"))` |
| `sub` present, empty string | `{"sub": ""}` | `Err(InvalidToken("invalid subject id"))` |

**Rationale**: `sub` is the only claim that uniquely identifies an authenticated principal. If it is absent, malformed, or unrepresentable as a non-empty `SubjectId`, no authenticated identity exists and the token is rejected. This is distinct from optional identity claims (`tenant_id`, `tid`, `roles`) which degrade gracefully per CLAR-005.

Absent optional claims (`tenant_id`, `tid`, `roles`) are simply not present; no error is produced.

---

## Infrastructure Implementation Reference

The `crates/security-jwt` crate provides three single-algorithm `AuthenticationProvider` implementations over a `KeyResolver` abstraction (CORE-013):

- **JwtAlgorithm** enum: `Hs256`, `Rs256`, `Es256` (closed set — `#[non_exhaustive]` removed so providers can exhaustively match). Key material lives in `VerificationKey` behind the `KeyResolver`.
- **Hs256AuthenticationProvider** / **Rs256AuthenticationProvider** / **Es256AuthenticationProvider**: three concrete types each implementing `AuthenticationProvider`. Accept their matching config type (`Hs256Config`, `Rs256Config`, `Es256Config` — each carries `expected_iss: Option<String>` and `expected_aud: Option<Vec<String>>`), an `Arc<dyn KeyResolver>`, and an `Arc<dyn Clock>`. All are `Send + Sync`.
- **JwtValidationEngine** (`pub(crate)`, in `src/validation.rs`): internal shared engine for claim/time/iss/aud/sub/roles/tenant validation. Never re-exported. Providers delegate to it after resolving the key.
- **KeyResolver** trait (`crates/security-jwt`): async trait returning `Result<VerificationKey, KeyResolverError>`. Accepts `kid: Option<&str>` and `algorithm: JwtAlgorithm`. Resolver MUST be cache-first — network I/O MUST occur outside the auth hot path (AD-013).
- **VerificationKey** enum (`crates/security-jwt`): `Hmac(Vec<u8>)` for HS256, `RsaPem(String)` for RS256, `EcPem(String)` for ES256. `#[non_exhaustive]` — additive extension.
- **LocalKeyResolver** struct (`crates/security-jwt`): concrete `KeyResolver` holding a single `(algorithm, VerificationKey)` pair. Ignores `kid` (advisory). Satisfies the cache-first contract trivially.

Each provider: decodes the JWT header → asserts `header.alg == provider algorithm` (else `AlgorithmNotSupported`) → `block_on(resolver.resolve(kid, alg))` → builds `DecodingKey` from the matching `VerificationKey` variant → delegates to `JwtValidationEngine` for signature verification and CLAR-005 claim validation.

**Removed** (CORE-013, AD-013): `JwtAuthenticator` and `JwtConfig` are no longer part of the public API. No shims — pre-stable leaf crate with no external consumers.

## Dependency Graph

```
crates/domain/auth (data models: Claims, Credential, AuthenticationError)
  ↑ (Claims re-exported)
crates/security-sdk (authN + authZ traits, SecurityContext, Principal)
  ↑
crates/security-jwt (HS256/RS256 implementation)
```

**Prohibited**: crates/ego-runtime MUST NOT depend on crates/security-jwt. Enforcement via layers.toml and scripts/verify-layers.sh.

## Future Capabilities

- **CORE-011B**: JWKS remote key resolver (cache-backed, OIDC discovery, multi-issuer routing)
- **CORE-014**: `#[authorize]` / authorization macro support
- **Future**: EdDSA algorithm support
- **Future**: Additional domain auth types as needed by new providers

## References

- RFC 7519: JSON Web Token (JWT)
- ADR-009A: Sync authentication boundary decision
- CORE-011: JWT Authentication Provider implementation
- CORE-013: JWT Providers + KeyResolver (replaced JwtAuthenticator/JwtConfig with three single-algorithm providers)
- CLAR-005–006: JWT Authentication clarifications (defined in this document)
- CLAR-001–004: Earlier JWT authentication clarifications (see change artifact history)
