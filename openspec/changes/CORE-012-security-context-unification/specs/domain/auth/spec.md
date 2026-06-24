# Delta for domain/auth

## REMOVED Core Types

### SecurityContext

The canonical `SecurityContext` type moves to `security-sdk`. Domain MUST NOT define a `SecurityContext`.

(Reason: AD-001 assigns sole ownership of `SecurityContext` to `security-sdk`. The dual model broke the authN→authZ pipeline — `JwtAuthenticator` produced a domain `SecurityContext` that `AuthorizationProvider` could not consume.)

(Migration: All references to `domain::auth::SecurityContext` MUST be updated to `ego_security_sdk::SecurityContext`. The new `SecurityContext` uses `Principal` in place of `Identity` and carries a `claims: Claims` field.)

#### Scenario: No domain SecurityContext type remains

- GIVEN the `crates/domain/src/auth/` module
- WHEN `grep -rn "pub struct SecurityContext" crates/domain/src/auth/` is executed
- THEN zero matches are returned

#### Scenario: JwtAuthenticator produces security-sdk SecurityContext

- GIVEN a valid JWT credential
- WHEN `JwtAuthenticator::authenticate(credential)` is called
- THEN the result is `Ok(ego_security_sdk::SecurityContext { principal, claims })`

### AuthenticationProvider trait

The `AuthenticationProvider` trait moves to `security-sdk`. Its signature changes from:

```rust
fn authenticate(&self, credential: Credential) -> Result<SecurityContext, AuthenticationError>;
```

to the `security-sdk` variant (sync, returns `Result<SecurityContext, AuthenticationError>`).

(Reason: AD-004 and AD-001 unify the authN and authZ contracts under `security-sdk` ownership. All authentication providers (JWT, Basic) MUST implement the `security-sdk` trait.)

(Migration: Existing `impl AuthenticationProvider for JwtAuthenticator` MUST move to implement `ego_security_sdk::AuthenticationProvider`. The return type changes from `Result<domain::auth::SecurityContext, AuthenticationError>` to `Result<ego_security_sdk::SecurityContext, ego_security_sdk::AuthenticationError>`.)

#### Scenario: No AuthenticationProvider trait in domain

- GIVEN the `crates/domain/src/auth/` module
- WHEN `grep -rn "trait AuthenticationProvider" crates/domain/src/auth/` is executed
- THEN zero matches are returned

#### Scenario: JwtAuthenticator implements security-sdk AuthenticationProvider

- GIVEN the `crates/security-jwt/src/authenticator.rs` file
- WHEN inspected
- THEN it imports and implements `ego_security_sdk::AuthenticationProvider`, NOT `domain::auth::AuthenticationProvider`

### Identity type

`Identity` is removed. `Principal` (from `security-sdk`) is the sole identity model. All authN and authZ components operate on `Principal`.

(Reason: Q8 — `Identity` (subject, tenant_id, roles, attributes) and `Principal` (kind, subject, tenant_id, roles, claims, attributes) are semantically equivalent. Keeping both creates duplication and conversion debt. `Principal` is the canonical identity type.)

(Migration: All references to `domain::auth::Identity` MUST be replaced with `ego_security_sdk::Principal`. JWT claim extraction (subject, tenant_id, roles) now maps directly to `Principal` fields.)

#### Scenario: No Identity type remains in domain

- GIVEN the `crates/domain/src/auth/` module
- WHEN `grep -rn "pub struct Identity" crates/domain/src/auth/` is executed
- THEN zero matches are returned

#### Scenario: JwtAuthenticator produces Principal directly

- GIVEN a valid JWT credential
- WHEN `JwtAuthenticator::authenticate(credential)` extracts identity claims
- THEN `sub` maps to `Principal.subject`, `roles` to `Principal.roles`, `tenant_id`/`tid` to `Principal.tenant_id`

## MODIFIED Sections

### Overview

The domain authentication module (`crates/domain/src/auth/`) defines the synchronous authentication data model for ego-rs. It separates the authenticated principal from raw token claims (`Claims`). The canonical principal type (`Principal`) lives in `security-sdk`; domain keeps pure data models only (`Claims`, `Credential`, `AuthenticationError`).

(Previously: mentioned AuthenticationProvider as a domain-defined trait. The trait now lives in `security-sdk`.)

### Dependency Graph

```
crates/domain/auth (data models: Claims, Credential, AuthenticationError)
  ↑ (Claims re-exported)
crates/security-sdk (authN + authZ traits, SecurityContext, Principal)
  ↑
crates/security-jwt (HS256/RS256 implementation)
```

(Previously: showed `domain/auth (port) → security-jwt` and `Identity` as a domain model. Identity removed per Q8; `Principal` lives in `security-sdk`. The port moves to `security-sdk`; `domain/auth` becomes a pure-model crate with no traits.)

### Future Capabilities

- **CORE-011B**: JWKS remote key resolver (cache-backed, OIDC discovery, multi-issuer routing)
- **Future**: ES256/EdDSA algorithm support (extend JwtAlgorithm and VerificationKey)
- **Future**: Additional domain auth types as needed by new providers

(Previously: listed CORE-012 as a future capability. This change implements it.)

## KEPT (no change)

All other core types remain in `domain/auth` unchanged:

| Type | Role |
|------|------|
| `StandardClaims` | RFC 7519 Section 4.1 registered claims |
| `Claims` | Combined standard + custom claims (re-exported by security-sdk) |
| `Credential` | Authentication input (BearerToken variant) |
| `AuthenticationError` | Authentication failure variants |
| `Clock` | Injectable time source for deterministic testing |

**Removed**: `Identity` (replaced by `ego_security_sdk::Principal`), `SecurityContext` (replaced by `ego_security_sdk::SecurityContext`), `AuthenticationProvider` (trait moved to `ego_security_sdk::authentication::AuthenticationProvider`).
