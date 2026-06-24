# Delta Spec: CORE-011A — Key Resolver Architecture

**Change ID**: CORE-011A
**Parent spec**: `openspec/specs/domain/auth.md`
**Affected crate**: `crates/security-jwt`
**Status**: SPEC

---

## Frozen Invariants (MUST NOT change)

| Type | Location | Unchanged |
|---|---|---|
| `AuthenticationProvider` trait | `ego-domain` | signature, sync boundary, Send+Sync |
| `Credential` enum | `ego-domain` | variants, non_exhaustive attribute |
| `SecurityContext`, `Identity`, `Claims` | `ego-domain` | all fields and invariants |
| `AuthenticationError` variants | `ego-domain` | all five variants |
| `Clock` trait | `ego-domain` | signature and invariants |
| `layers.toml` assignments | workspace | `security-jwt` remains infrastructure |

---

## ADDED Requirements

### Requirement: KeyResolverError enum

`KeyResolverError` MUST be defined in `crates/security-jwt` with `pub` visibility and `#[deny(missing_docs)]` doc coverage.

| Variant | Fields | Meaning |
|---|---|---|
| `KeyNotFound` | `kid: Option<String>` | No key available for the requested kid |
| `AlgorithmMismatch` | `expected: JwtAlgorithm, requested: JwtAlgorithm` | Resolver is configured for a different algorithm |
| `InvalidKeyMaterial` | `(String)` | Key bytes or PEM content failed to parse |

The enum MUST derive `Debug` and implement `std::error::Error`.

#### Scenario: KeyNotFound carries the kid

- GIVEN a resolver that has no key registered for a given kid
- WHEN it returns `Err(KeyResolverError::KeyNotFound { kid: Some("k1") })`
- THEN the caller receives the kid value unmodified for diagnostic logging

#### Scenario: AlgorithmMismatch carries both algorithm values

- GIVEN a resolver configured for HS256
- WHEN it is called with `algorithm = RS256`
- THEN `Err(KeyResolverError::AlgorithmMismatch { expected: HS256, requested: RS256 })` is returned

---

### Requirement: VerificationKey enum

`VerificationKey` MUST be defined in `crates/security-jwt` with `pub` visibility and `#[deny(missing_docs)]` doc coverage.

| Variant | Inner type | Represents |
|---|---|---|
| `Hmac` | `Vec<u8>` | Shared HMAC-SHA256 secret bytes |
| `RsaPem` | `String` | PEM-encoded RSA public key |

#### Scenario: Hmac variant holds raw bytes

- GIVEN a `VerificationKey::Hmac(bytes)` value
- WHEN the bytes are inspected
- THEN they equal the original secret byte slice passed at construction

#### Scenario: RsaPem variant holds PEM string

- GIVEN a `VerificationKey::RsaPem(pem)` value
- WHEN the string is inspected
- THEN it begins with `-----BEGIN PUBLIC KEY-----` or `-----BEGIN RSA PUBLIC KEY-----`

---

### Requirement: KeyResolver trait

`KeyResolver` MUST be an `#[async_trait]` trait defined in `crates/security-jwt` with `pub` visibility and `#[deny(missing_docs)]` doc coverage.

```
pub trait KeyResolver: Send + Sync {
    async fn resolve(
        &self,
        kid: Option<&str>,
        algorithm: JwtAlgorithm,
    ) -> Result<VerificationKey, KeyResolverError>;
}
```

- The trait MUST be object-safe (`dyn KeyResolver` MUST compile).
- The `kid` parameter MUST be the JWT `kid` header claim if present, or `None` if absent. The authenticator MUST NOT validate or filter by kid — it passes the value through unchanged.

#### Scenario: Resolver invoked with kid from JWT header

- GIVEN a JWT whose header contains `"kid": "primary-key"`
- WHEN `JwtAuthenticator::authenticate` is called
- THEN `KeyResolver::resolve` is called with `kid = Some("primary-key")`

#### Scenario: Resolver invoked with None when JWT has no kid

- GIVEN a JWT whose header does not contain a `kid` field
- WHEN `JwtAuthenticator::authenticate` is called
- THEN `KeyResolver::resolve` is called with `kid = None`

---

### Requirement: LocalKeyResolver struct

`LocalKeyResolver` MUST be defined in `crates/security-jwt` with `pub` visibility and `#[deny(missing_docs)]` doc coverage. It holds a single `VerificationKey` and the `JwtAlgorithm` it corresponds to, entirely in memory. It MUST implement `KeyResolver`.

- `LocalKeyResolver::resolve` MUST complete without any I/O or blocking.
- `LocalKeyResolver` MUST successfully resolve when `kid = None` for its configured algorithm.
- `LocalKeyResolver` MAY successfully resolve when `kid = Some(_)` (treating kid as advisory).
- `LocalKeyResolver` MUST NOT enforce or validate kid values.

#### Scenario: HS256 resolution — kid absent

- GIVEN `LocalKeyResolver` configured with `VerificationKey::Hmac(secret_bytes)`
- WHEN `resolve(kid = None, algorithm = HS256)` is called
- THEN `Ok(VerificationKey::Hmac(secret_bytes))` is returned

#### Scenario: RS256 resolution — kid absent

- GIVEN `LocalKeyResolver` configured with `VerificationKey::RsaPem(pem_string)`
- WHEN `resolve(kid = None, algorithm = RS256)` is called
- THEN `Ok(VerificationKey::RsaPem(pem_string))` is returned

#### Scenario: Algorithm mismatch

- GIVEN `LocalKeyResolver` configured for HS256
- WHEN `resolve(kid = None, algorithm = RS256)` is called
- THEN `Err(KeyResolverError::AlgorithmMismatch { expected: HS256, requested: RS256 })` is returned

#### Scenario: HS256 resolution — kid present (advisory)

- GIVEN `LocalKeyResolver` configured with `VerificationKey::Hmac(secret_bytes)`
- WHEN `resolve(kid = Some("any-kid"), algorithm = HS256)` is called
- THEN `Ok(VerificationKey::Hmac(secret_bytes))` is returned (kid is ignored)

---

### Requirement: KeyResolverError → AuthenticationError mapping

`JwtAuthenticator` MUST map `KeyResolverError` to `AuthenticationError` at the authenticator boundary. The mapping MUST be:

| `KeyResolverError` variant | `AuthenticationError` variant |
|---|---|
| `KeyNotFound { .. }` | `InvalidSignature` |
| `AlgorithmMismatch { .. }` | `AlgorithmNotSupported(msg)` |
| `InvalidKeyMaterial(msg)` | `InvalidToken(msg)` |

#### Scenario: Resolver returns KeyNotFound → InvalidSignature

- GIVEN `JwtAuthenticator` constructed with a `LocalKeyResolver` that returns `KeyNotFound`
- WHEN `authenticate(BearerToken(some_jwt))` is called
- THEN `Err(AuthenticationError::InvalidSignature)` is returned

#### Scenario: Resolver returns AlgorithmMismatch → AlgorithmNotSupported

- GIVEN `JwtAuthenticator` constructed with a resolver that returns `AlgorithmMismatch`
- WHEN `authenticate(BearerToken(some_jwt))` is called
- THEN `Err(AuthenticationError::AlgorithmNotSupported(_))` is returned

#### Scenario: Resolver returns InvalidKeyMaterial → InvalidToken

- GIVEN `JwtAuthenticator` constructed with a resolver that returns `InvalidKeyMaterial("bad pem")`
- WHEN `authenticate(BearerToken(some_jwt))` is called
- THEN `Err(AuthenticationError::InvalidToken("bad pem"))` is returned

---

## MODIFIED Requirements

### Requirement: JwtConfig (updated — key material removed)

`JwtConfig` MUST retain functional validation parameters and MUST NOT contain key material.
(Previously: `JwtConfig` contained `algorithm: JwtAlgorithm` which embedded key bytes or PEM strings directly.)

Updated fields:

| Field | Type | Purpose |
|---|---|---|
| `algorithm` | `JwtAlgorithm` | Algorithm selection — HS256 or RS256 (no key bytes) |
| `expected_iss` | `Option<String>` | Optional issuer constraint (unchanged) |
| `expected_aud` | `Option<Vec<String>>` | Optional audience constraint (unchanged) |

`JwtAlgorithm` MUST be refactored to carry only the algorithm discriminant, not key material. Key material moves entirely to `VerificationKey` inside the resolver.

#### Scenario: JwtConfig constructed without key material compiles

- GIVEN `JwtConfig { algorithm: JwtAlgorithm::Hs256, expected_iss: None, expected_aud: None }`
- WHEN compiled
- THEN no raw byte fields or PEM string fields are present on `JwtConfig` or `JwtAlgorithm`

---

### Requirement: JwtAuthenticator::new signature (updated)

`JwtAuthenticator::new` MUST accept `(JwtConfig, Arc<dyn KeyResolver>, Arc<dyn Clock>)`.
(Previously: `JwtAuthenticator::new(config: JwtConfig, clock: Arc<dyn Clock>)` — key material was inside `JwtConfig`.)

The authenticator MUST store the resolver and invoke it on every `authenticate` call to obtain the `VerificationKey` for the token's algorithm and kid.

#### Scenario: JwtAuthenticator uses resolver for key lookup

- GIVEN `JwtAuthenticator` constructed with `JwtConfig` (HS256 algorithm), `LocalKeyResolver` (HS256 key), and a fixed `Clock`
- WHEN `authenticate(BearerToken(valid_hs256_jwt))` is called
- THEN `Ok(SecurityContext)` is returned and the `LocalKeyResolver` was the source of key material

#### Scenario: All 37 existing security-jwt tests pass via LocalKeyResolver

- GIVEN each existing test that previously constructed `JwtConfig` with embedded key material
- WHEN the test is updated to wrap that key material in `LocalKeyResolver` and pass it to the new `JwtAuthenticator::new`
- THEN `cargo test --workspace` exits with zero failures (NFR-006)

---

## REMOVED Requirements

### Requirement: JwtAlgorithm key-material variants

(Reason: Key material is no longer embedded in `JwtAlgorithm`. The `Hs256 { secret: Vec<u8> }` and `Rs256 { public_key_pem: String }` associated data fields are removed; `JwtAlgorithm` becomes a plain discriminant enum.)
(Migration: Callers that referenced `JwtAlgorithm::Hs256 { secret }` or `JwtAlgorithm::Rs256 { public_key_pem }` MUST move key material to `LocalKeyResolver` and pass the resolver to `JwtAuthenticator::new`.)

---

## Out of Scope

The following are explicitly deferred to CORE-011B and MUST NOT be added in CORE-011A:

- `CachingKeyResolver`
- JWKS / remote HTTP resolver
- OIDC discovery
- kid validation against a trusted set
- ES256 / EdDSA algorithm support
- Multi-issuer routing

---

## NFR Summary

| ID | Requirement |
|---|---|
| NFR-005 | No new public API in `ego-domain`. `KeyResolver`, `VerificationKey`, `kid` MUST NOT appear in the domain crate. |
| NFR-006 | All 37 existing `security-jwt` tests MUST pass after the refactor using `LocalKeyResolver`. |
| NFR-007 | `LocalKeyResolver::resolve` MUST complete synchronously (no I/O, no blocking). |
| NFR-008 | `KeyResolver`, `VerificationKey`, `KeyResolverError`, `LocalKeyResolver` MUST have `#[deny(missing_docs)]` coverage. |
| NFR-009 | **VerificationKey Extensibility**: `VerificationKey` is intentionally extensible. Future variants (`Es256(Vec<u8>)`, `EdDsa(Vec<u8>)`, `Jwk(JsonWebKey)`) MAY be added without requiring changes to `AuthenticationProvider` or the `KeyResolver` trait signature. CORE-011A ships `Hmac(Vec<u8>)` and `RsaPem(String)` only. |
| NFR-010 | **Shared Resolver Instances**: Multiple `JwtAuthenticator` instances MAY share the same `Arc<dyn KeyResolver>`. Resolver lifecycle management is external to `JwtAuthenticator`. |
| NFR-011 | **Resolver Lifecycle is External**: `JwtAuthenticator` MUST NOT own or manage the resolver's lifecycle. The resolver is passed in as `Arc<dyn KeyResolver>` and may outlive any individual authenticator instance. |

---

## Functional Requirements (Cache-First)

> **FR-019**: `KeyResolver::resolve` MUST return from locally available state during `authenticate()`.
> **FR-020**: `KeyResolver::resolve` MUST NOT perform network I/O, HTTP calls, JWKS downloads, database access, or blocking filesystem operations during `authenticate()`.
> **FR-021**: Remote key acquisition MUST occur outside the authentication path (e.g., background refresh, explicit warm-up, or scheduled synchronization).

---

## Acceptance Criteria (Cache-First)

> **AC-014**: `AuthenticationProvider` remains synchronous after CORE-011A.
> **AC-015**: `authenticate()` never waits for remote key retrieval.
> **AC-016**: A future JWKS resolver can refresh keys asynchronously without changing `AuthenticationProvider` or `JwtAuthenticator`.

---

## Acceptance Criteria (Resolver Ownership)

> **AC-017**: `JwtAuthenticator` stores `Arc<dyn KeyResolver>`, not `Box<dyn KeyResolver>`.
> **AC-018**: Resolver ownership remains external — `JwtAuthenticator::new` takes `Arc<dyn KeyResolver>`, not ownership of a concrete type.
> **AC-019**: Shared resolver instances are explicitly tested: two `JwtAuthenticator` instances constructed with `Arc::clone` of the same resolver both authenticate successfully.
