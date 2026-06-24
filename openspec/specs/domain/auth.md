# Domain Authentication Module — Canonical Specification

**Introduced by**: CORE-011 JWT Authentication Provider (2026-06-23)
**Decision Record**: ADR-009A (sync authentication boundary)
**Status**: Production

## Overview

The domain authentication module (`crates/domain/src/auth/`) defines the synchronous authentication contract for ego-rs. It separates the authenticated principal (Identity) from raw token claims (Claims), and exposes a trait-based authentication port (AuthenticationProvider) for infrastructure implementations.

All authentication happens synchronously; async wrappers are scoped to future capabilities (e.g., CORE-012).

## Core Types

### Identity

The authenticated principal extracted from a credential.

```rust
pub struct Identity {
    pub subject: String,                    // JWT 'sub' claim
    pub tenant_id: Option<String>,          // Optional org/tenant scope
    pub roles: BTreeSet<String>,            // Role assignments (deterministic order)
    pub attributes: BTreeMap<String, String>, // Arbitrary metadata (deterministic order)
}
```

**Invariants**:
- `subject` is never empty (guaranteed by AuthenticationProvider implementations)
- `roles` and `attributes` use BTreeSet/BTreeMap for deterministic iteration order
- No HashMap or HashSet used anywhere in public API

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
- Identity fields (`sub`, `roles`, `tenant_id`/`tid`) are extracted into Identity and also preserved in custom map if their types are wrong (graceful degradation per CLAR-003)

### SecurityContext

The resolved, authenticated execution context.

```rust
pub struct SecurityContext {
    pub identity: Identity,
    pub claims: Claims,
}
```

**Invariants**:
- Always produced by successful AuthenticationProvider::authenticate call
- Is a concrete struct (not a trait object)
- Implements Clone, Debug, PartialEq

### Credential

The authentication credential supplied by the caller.

```rust
#[non_exhaustive]
pub enum Credential {
    BearerToken(String),
    // Future variants: ApiKey(String), ClientCertificate(Vec<u8>), etc.
}
```

**Invariants**:
- #[non_exhaustive] allows future variants without breaking existing implementors
- BearerToken is the initial variant for JWT/OIDC tokens
- Credential is consumed by value to ensure sensitive material is dropped after validation

### AuthenticationError

Errors that can occur during authentication.

```rust
pub enum AuthenticationError {
    InvalidToken(String),           // Malformed token or other structural issue
    ExpiredToken,                    // exp claim indicates token has expired
    MissingClaim(String),            // Required claim is absent (e.g., 'sub')
    InvalidSignature,                // Signature validation failed
    AlgorithmNotSupported,           // Token algorithm not supported by provider
}
```

**Invariants**:
- Each variant carries enough context for logging without additional state
- Implementations MUST NOT panic; all errors are recoverable
- InvalidToken carries a message for context

### AuthenticationProvider

The synchronous authentication port.

```rust
pub trait AuthenticationProvider: Send + Sync {
    fn authenticate(
        &self,
        credential: Credential,
    ) -> Result<SecurityContext, AuthenticationError>;
}
```

**Invariants**:
- Synchronous: no async I/O (JWT verification is CPU-bound)
- Send + Sync: implementations can be shared across threads via Arc<dyn AuthenticationProvider>
- Credential is consumed by value (no references)
- No authentication state is passed into the trait; implementations are stateless ports

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

2. **No ambient context**: The AuthenticationProvider trait carries no implicit state. Identity and claims are always explicit in the SecurityContext return value.

3. **Deterministic ordering**: All collection types in Identity and Claims use BTreeSet/BTreeMap. HashMap and HashSet MUST NOT appear in public API.

4. **Thread safety**: All public types and traits have Send + Sync bounds, enabling safe concurrent use via Arc.

5. **Clock injection**: All time-sensitive validation uses an injected Clock abstraction, enabling deterministic testing without mocking system calls.

6. **Claim classification**: Claims are split into two categories with different wrong-type behaviors. Identity claims (`sub`, `tenant_id`/`tid`, `roles`) degrade gracefully — the nominal field receives an empty/null value and the raw claim is preserved in `Claims.custom`. Security claims (`exp`, `nbf`, `iat`, `iss`, `aud`) fail immediately with `AuthenticationError::InvalidToken` — no raw value is preserved. See CLAR-005.

7. **No type merging**: Identity, StandardClaims, and Claims.custom are separate. Standard claims are never merged into a flat map.

## Clarifications

### CLAR-005 — Claim Classification

JWT claims fall into two categories with distinct wrong-type behaviors:

**Identity Claims**: `sub`, `tenant_id`, `tid`, `roles`

- Wrong type MUST NOT fail authentication.
- Implementation MUST degrade gracefully: the nominal identity field receives an empty/null value.
- The original raw claim value MUST be preserved in `Claims.custom` under its original key.
- Rationale: identity enrichment fields are not security-critical; a token with an unexpected claim encoding is still a valid, authenticated token.

**Security Claims**: `exp`, `nbf`, `iat`, `iss`, `aud`

- Wrong type MUST fail authentication with `AuthenticationError::InvalidToken`.
- The raw value is NOT preserved; the request is rejected immediately before `SecurityContext` is produced.
- Rationale: these claims govern time-bound validity and trust constraints. Accepting a non-parseable value would silently bypass security checks.

**Scenario — security claim, wrong type**

Given a JWT with `{ "sub": "user-1", "exp": "not-a-number" }` (exp as string)  
When `JwtAuthenticator::authenticate` is called  
Then `Err(AuthenticationError::InvalidToken("exp claim is not a valid integer"))` is returned

**Scenario — identity claim, wrong type**

Given a JWT with `{ "sub": 123, "exp": 9999999999 }` (sub as integer, exp valid)  
When `JwtAuthenticator::authenticate` is called  
Then `Ok(SecurityContext)` is returned with `identity.subject == ""` and `claims.custom["sub"] == 123`

---

### CLAR-006 — Required Claim Semantics: Absent vs. Wrong Type

"Absent claim" and "wrong-type claim" are distinct failure modes with different outcomes for identity claims:

| Scenario | Example payload | Result |
|---|---|---|
| Claim absent | `{}` (no `sub` key) | `Err(AuthenticationError::MissingClaim("sub"))` |
| Claim present, wrong type | `{"sub": 123}` | `Ok(SecurityContext)` with graceful degradation |

**Rationale**: a missing `sub` indicates a structurally invalid token — no authenticated identity was issued. A `sub` of the wrong type indicates a valid token with an unexpected encoding; the token is authenticated but the subject cannot be reliably extracted as a string.

**Scenario A — absent claim**

Given a JWT with no `sub` claim  
When `JwtAuthenticator::authenticate` is called  
Then `Err(AuthenticationError::MissingClaim("sub"))` is returned

**Scenario B — present claim, wrong type**

Given a JWT with `{ "sub": 123 }` (integer, not a string)  
When `JwtAuthenticator::authenticate` is called  
Then `Ok(SecurityContext)` is returned with:
- `identity.subject == ""` (empty string, not an error)
- `claims.custom["sub"] == 123` (raw value preserved under original key)

This behavior applies to all identity claims as defined in CLAR-005. CLAR-006 uses `sub` as the canonical example because it is the only required identity claim.

---

## Infrastructure Implementation Reference

The `crates/security-jwt` crate provides a reference implementation of AuthenticationProvider for JWT tokens:

- **JwtAlgorithm** enum: HS256 (HMAC-SHA256), RS256 (RSA-SHA256)
- **JwtConfig** struct: algorithm selection, key material (secret bytes or PEM), optional iss/aud constraints
- **JwtAuthenticator** struct: implements AuthenticationProvider for JWT tokens

JwtAuthenticator verifies signatures, validates exp/nbf time claims using the injected Clock, and extracts Identity fields with graceful degradation.

## Dependency Graph

```
crates/domain/auth (port)
  ↑
crates/security-jwt (HS256/RS256 implementation)
```

**Prohibited**: crates/ego-runtime MUST NOT depend on crates/security-jwt. Enforcement via layers.toml and scripts/verify-layers.sh.

## Future Capabilities

- **CORE-012**: Authorization provider (RBAC/ABAC based on Identity roles/attributes)
- **CORE-011A**: ES256/EdDSA algorithm support (extend JwtAlgorithm and JwtAuthenticator)
- **CORE-011B**: JWKS remote key resolver (decouple key material from JwtConfig)

## References

- RFC 7519: JSON Web Token (JWT)
- ADR-009A: Sync authentication boundary decision
- CORE-011: JWT Authentication Provider implementation
- CLAR-001–006: JWT Authentication clarifications
