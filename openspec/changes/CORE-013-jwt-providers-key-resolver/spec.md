# Spec: CORE-013 — JWT Providers + KeyResolver

## Purpose

Replace the monolithic `JwtAuthenticator`/`JwtConfig` with three single-algorithm
`AuthenticationProvider` implementations (HS256, RS256, ES256) over the existing
`KeyResolver` abstraction, and extract shared claim/time validation into an internal
`JwtValidationEngine`. This spec is a MODIFIED delta against the `security-jwt` crate.

---

## REMOVED Requirements

### Requirement: JwtAuthenticator — Single-Authenticator Contract

(Reason: Monolithic authenticator couples algorithm selection to a runtime config field,
blocking per-algorithm validation strategy and ES256 support. AD-013, AD-014.)
(Migration: Replace with `Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`,
or `Es256AuthenticationProvider`. No shims — pre-stable leaf crate with no external consumers.)

### Requirement: JwtConfig — Runtime Algorithm Selection

(Reason: Algorithm is now a compile-time property of each provider type, not a runtime field.
AD-014, AD-017.)
(Migration: Use the matching `Hs256Config`, `Rs256Config`, or `Es256Config` struct for the
chosen provider. Fields `expected_iss` and `expected_aud` carry over unchanged.)

---

## ADDED Requirements

### FR-019 — JwtAlgorithm::Es256 Variant

`JwtAlgorithm` MUST include an `Es256` variant. The enum MUST remain a closed set
(not `#[non_exhaustive]`) so providers can exhaustively match on it.

#### Scenario: Es256 variant round-trips through JwtAlgorithm

- GIVEN `JwtAlgorithm::Es256` is constructed
- WHEN compared with `==` against another `JwtAlgorithm::Es256` value
- THEN the comparison MUST return `true`
- AND the variant MUST be `Copy`, `Clone`, `PartialEq`, `Eq`, `Debug`

---

### FR-020 — VerificationKey::EcPem Variant

`VerificationKey` MUST include an `EcPem(String)` variant carrying an EC public key
in PEM format (beginning `-----BEGIN PUBLIC KEY-----`).
The enum is `#[non_exhaustive]` — this is an additive extension.

#### Scenario: EcPem variant stores PEM string

- GIVEN a valid EC P-256 public key PEM string
- WHEN wrapped in `VerificationKey::EcPem`
- THEN the stored string MUST equal the input

---

### FR-021 — Hs256AuthenticationProvider

`Hs256AuthenticationProvider` MUST implement `AuthenticationProvider` with algorithm HS256.
It MUST accept `Hs256Config`, an `Arc<dyn KeyResolver>`, and an `Arc<dyn Clock>`.
It MUST be `Send + Sync`.

#### Scenario: HS256 happy path — valid token returns SecurityContext

- GIVEN an `Hs256AuthenticationProvider` with a `LocalKeyResolver` holding an HMAC secret
- AND a `Bearer` credential containing a valid HS256-signed JWT with `sub`, valid `exp`
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Ok(SecurityContext)` with `principal.subject_id` equal to the `sub` claim

#### Scenario: HS256 wrong HMAC secret returns InvalidSignature

- GIVEN an `Hs256AuthenticationProvider` whose resolver holds a different HMAC secret
- AND a `Bearer` credential containing a valid HS256-signed JWT
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Err(AuthenticationError::InvalidSignature)`

#### Scenario: HS256 provider rejects RS256-signed token

- GIVEN an `Hs256AuthenticationProvider`
- AND a `Bearer` credential containing a valid RS256-signed JWT
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Err(AuthenticationError::AlgorithmNotSupported(_))`

#### Scenario: HS256 non-Bearer credential returns InvalidToken

- GIVEN an `Hs256AuthenticationProvider`
- WHEN `authenticate` is called with a non-`Bearer` credential variant
- THEN it MUST return `Err(AuthenticationError::InvalidToken(_))`

---

### FR-022 — Rs256AuthenticationProvider

`Rs256AuthenticationProvider` MUST implement `AuthenticationProvider` with algorithm RS256.
It MUST accept `Rs256Config`, `Arc<dyn KeyResolver>`, and `Arc<dyn Clock>`.
It MUST be `Send + Sync`.

#### Scenario: RS256 happy path — valid PEM key returns SecurityContext

- GIVEN an `Rs256AuthenticationProvider` with a `LocalKeyResolver` holding a valid RSA public key PEM
- AND a `Bearer` credential containing a valid RS256-signed JWT with `sub`, valid `exp`
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Ok(SecurityContext)` with `principal.subject_id` equal to the `sub` claim

#### Scenario: RS256 mismatched public key returns InvalidSignature

- GIVEN an `Rs256AuthenticationProvider` whose resolver holds a different RSA public key
- WHEN `authenticate` is called with a JWT signed by the primary RS256 private key
- THEN it MUST return `Err(AuthenticationError::InvalidSignature)`

#### Scenario: RS256 provider rejects HS256-signed token

- GIVEN an `Rs256AuthenticationProvider`
- AND a `Bearer` credential containing a valid HS256-signed JWT
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Err(AuthenticationError::AlgorithmNotSupported(_))`

---

### FR-023 — Es256AuthenticationProvider

`Es256AuthenticationProvider` MUST implement `AuthenticationProvider` with algorithm ES256.
It MUST accept `Es256Config`, `Arc<dyn KeyResolver>`, and `Arc<dyn Clock>`.
It MUST be `Send + Sync`.
The resolver MUST return `VerificationKey::EcPem`; an unexpected variant MUST map to
`AuthenticationError::InvalidToken`.

#### Scenario: ES256 happy path — valid EC PEM key returns SecurityContext

- GIVEN an `Es256AuthenticationProvider` with a resolver returning a valid P-256 EC public PEM
- AND a `Bearer` credential containing a valid ES256-signed JWT with `sub`, valid `exp`
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Ok(SecurityContext)` with `principal.subject_id` equal to the `sub` claim

#### Scenario: ES256 invalid signature returns InvalidSignature

- GIVEN an `Es256AuthenticationProvider` whose resolver holds a different EC public key
- WHEN `authenticate` is called with a JWT signed by a different EC private key
- THEN it MUST return `Err(AuthenticationError::InvalidSignature)`

#### Scenario: ES256 provider rejects HS256-signed token

- GIVEN an `Es256AuthenticationProvider`
- AND a `Bearer` credential containing a valid HS256-signed JWT
- WHEN `authenticate(&credential)` is called
- THEN it MUST return `Err(AuthenticationError::AlgorithmNotSupported(_))`

---

### FR-024 — JwtValidationEngine (internal) — Clock-Injected Claim Validation

An internal `JwtValidationEngine` (not `pub`) MUST centralize all claim/time validation
shared by the three providers. It MUST preserve CLAR-005 semantics exactly:

| Claim | Rule |
|-------|------|
| `exp` | If present: MUST be integer; if `<= now` → `ExpiredToken` |
| `nbf` | If present: MUST be integer; if `> now` → `InvalidToken` |
| `iss` | If `expected_iss` configured: MUST match exactly; absent → `InvalidToken` |
| `aud` | If `expected_aud` configured: ≥1 overlap required; absent → `InvalidToken` |
| `sub` | MUST be present, MUST be non-empty string; absent → `MissingClaim("sub")`; non-string/empty → `InvalidToken` |
| `roles` | If present but wrong type or mixed array → skip (graceful), raw preserved in `Claims.custom` |
| `tenant_id`/`tid` | If present but wrong type → skip (graceful), raw preserved under original key |

#### Scenario: Expired token (exp <= now) returns ExpiredToken

- GIVEN a provider with a fixed clock returning time T
- AND a JWT whose `exp` equals T (boundary case)
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::ExpiredToken)`

#### Scenario: Token not yet valid (nbf > now) returns InvalidToken

- GIVEN a provider with a fixed clock returning time T
- AND a JWT whose `nbf` is T+300
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::InvalidToken(_))`

#### Scenario: Missing sub returns MissingClaim

- GIVEN a JWT with no `sub` claim
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::MissingClaim(s))` where `s == "sub"`

#### Scenario: Non-string sub returns InvalidToken

- GIVEN a JWT where `sub` is a JSON integer
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::InvalidToken(_))`

#### Scenario: Empty string sub returns InvalidToken

- GIVEN a JWT where `sub` is `""`
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::InvalidToken(_))`

#### Scenario: Wrong-type roles degrades gracefully

- GIVEN a JWT where `roles` is a JSON string `"admin"` (not an array)
- WHEN `authenticate` succeeds (valid sub, sig, exp)
- THEN `principal.roles` MUST be empty
- AND `claims.custom["roles"]` MUST equal `"admin"` (raw preserved)

#### Scenario: Wrong-type tenant_id degrades gracefully, preserved under original key

- GIVEN a JWT where `tid` is a JSON integer `42`
- WHEN `authenticate` succeeds
- THEN `principal.tenant_id` MUST be `None`
- AND `claims.custom["tid"]` MUST equal `42`
- AND `claims.custom` MUST NOT contain key `"tenant_id"`

---

### FR-025 — kid Extraction and Forwarding

Each provider MUST extract the `kid` field from the JWT header via `decode_header`.
It MUST forward `kid` (as `Option<&str>`) and the provider's algorithm to `KeyResolver::resolve`.
A token with no `kid` header field MUST forward `None`.

#### Scenario: kid present in header is forwarded to resolver

- GIVEN an `Hs256AuthenticationProvider` backed by a capturing `KeyResolver`
- AND a JWT whose header includes `kid: "primary-key"`
- WHEN `authenticate` is called
- THEN the resolver MUST receive `kid = Some("primary-key")` and `algorithm = JwtAlgorithm::Hs256`

#### Scenario: kid absent in header forwards None to resolver

- GIVEN an `Hs256AuthenticationProvider` backed by a capturing `KeyResolver`
- AND a JWT whose header has no `kid` field
- WHEN `authenticate` is called
- THEN the resolver MUST receive `kid = None`

---

### FR-026 — KeyResolver Error Mapping (per provider)

Each provider MUST map `KeyResolverError` variants to `AuthenticationError` as follows:

| KeyResolverError | AuthenticationError |
|-----------------|---------------------|
| `KeyNotFound { .. }` | `InvalidSignature` |
| `AlgorithmMismatch { .. }` | `AlgorithmNotSupported(_)` |
| `InvalidKeyMaterial(_)` | `InvalidToken(_)` |

#### Scenario: KeyNotFound maps to InvalidSignature

- GIVEN a provider backed by a resolver that returns `KeyResolverError::KeyNotFound`
- WHEN `authenticate` is called with a structurally valid JWT
- THEN it MUST return `Err(AuthenticationError::InvalidSignature)`

#### Scenario: AlgorithmMismatch from resolver maps to AlgorithmNotSupported

- GIVEN a provider backed by a resolver that returns `KeyResolverError::AlgorithmMismatch`
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::AlgorithmNotSupported(_))`

#### Scenario: InvalidKeyMaterial maps to InvalidToken

- GIVEN a provider backed by a resolver that returns `KeyResolverError::InvalidKeyMaterial`
- WHEN `authenticate` is called
- THEN it MUST return `Err(AuthenticationError::InvalidToken(_))`

---

### FR-027 — Public API Surface of security-jwt

`lib.rs` MUST export exactly:
- `Hs256AuthenticationProvider`, `Hs256Config`
- `Rs256AuthenticationProvider`, `Rs256Config`
- `Es256AuthenticationProvider`, `Es256Config`
- `JwtAlgorithm` (with `Es256` variant)
- `KeyResolver`, `KeyResolverError`, `LocalKeyResolver`, `VerificationKey` (with `EcPem` variant)

`JwtAuthenticator` and `JwtConfig` MUST NOT appear in any public export, re-export, or
`pub use` statement in the crate's public API.

#### Scenario: JwtAuthenticator is absent from public exports

- GIVEN the compiled `security_jwt` crate
- WHEN any code attempts `use security_jwt::JwtAuthenticator`
- THEN it MUST fail to compile

#### Scenario: All three providers and configs are importable

- GIVEN the compiled `security_jwt` crate
- WHEN tests import all three providers and their configs
- THEN all imports MUST succeed

---

## Non-Functional Requirements

### NFR-013-01 — No New Runtime Dependencies

`security-jwt/Cargo.toml` MUST NOT add any new dependencies beyond those already present.
`jsonwebtoken` v9 covers HS256/RS256/ES256 natively.

### NFR-013-02 — Sync Authenticate Contract Preserved

`authenticate(&self, credential: &Credential) -> Result<SecurityContext, AuthenticationError>`
MUST remain a synchronous method on all three providers.

### NFR-013-03 — Send + Sync on All Providers

All three providers MUST implement `Send + Sync`.

#### Scenario: Providers are Send + Sync

- GIVEN any of the three provider structs
- WHEN used in a context requiring `T: Send + Sync`
- THEN it MUST compile without error

### NFR-013-05 — JwtValidationEngine is Internal-Only

`JwtValidationEngine` MUST be `pub(crate)`. It MUST NOT be re-exported from `lib.rs`
or any public module. The `validation` module MUST NOT be `pub`. Authentication providers
are the only supported integration point for validation logic.

#### Scenario: JwtValidationEngine is not accessible from outside the crate

- GIVEN the compiled `security_jwt` crate
- WHEN external code attempts `use security_jwt::JwtValidationEngine` or
  `use security_jwt::validation::JwtValidationEngine`
- THEN it MUST fail to compile

---

### NFR-013-04 — Deterministic Validation via Clock Injection

No provider or the engine MUST call `Utc::now()` directly.
All time-sensitive validation (exp, nbf) MUST go through the injected `Arc<dyn Clock>`.

---

## Key Scenarios (Cross-Cutting)

### S-1: HS256 happy path end-to-end
- GIVEN `Hs256AuthenticationProvider` with `LocalKeyResolver`, real clock
- AND a `Bearer` JWT signed with that secret, `sub: "u1"`, valid `exp`
- WHEN `authenticate` is called
- THEN returns `Ok(SecurityContext)` with `principal.subject_id == "u1"`

### S-2: RS256 happy path end-to-end
- GIVEN `Rs256AuthenticationProvider` with RSA PEM public key, real clock
- AND a `Bearer` JWT signed with matching RSA private key, `sub: "rs-user"`, valid `exp`
- WHEN `authenticate` is called
- THEN returns `Ok(SecurityContext)` with `principal.subject_id == "rs-user"`

### S-3: ES256 happy path end-to-end
- GIVEN `Es256AuthenticationProvider` with EC P-256 public PEM, real clock
- AND a `Bearer` JWT signed with matching EC private key, `sub: "ec-user"`, valid `exp`
- WHEN `authenticate` is called
- THEN returns `Ok(SecurityContext)` with `principal.subject_id == "ec-user"`

### S-4: Algorithm mismatch — RS256 token to Hs256 provider
- GIVEN `Hs256AuthenticationProvider`
- AND a `Bearer` JWT signed with RS256
- WHEN `authenticate` is called
- THEN returns `Err(AuthenticationError::AlgorithmNotSupported(_))`

### S-5: Expired token
- GIVEN any provider with a fixed clock at time T
- AND a JWT with `exp = T - 1`
- WHEN `authenticate` is called
- THEN returns `Err(AuthenticationError::ExpiredToken)`

### S-6: Missing sub claim
- GIVEN any provider, JWT with no `sub`, valid signature and exp
- WHEN `authenticate` is called
- THEN returns `Err(AuthenticationError::MissingClaim("sub"))`

### S-7: kid routing to resolver
- GIVEN `Hs256AuthenticationProvider` with capturing resolver
- AND a JWT with `kid: "k42"` in header
- WHEN `authenticate` is called
- THEN resolver receives `kid = Some("k42")` and `algorithm = Hs256`

### S-8: Unknown kid → InvalidSignature
- GIVEN `Hs256AuthenticationProvider` backed by resolver returning `KeyNotFound { kid: Some("unknown") }`
- WHEN `authenticate` is called
- THEN returns `Err(AuthenticationError::InvalidSignature)`

---

## Testing Strategy

Engine-level claim validation (exp/nbf/iss/aud/sub/roles/tenant) MUST be tested through
`Hs256AuthenticationProvider` only. RS256 and ES256 tests cover: valid signature,
invalid/mismatched key, algorithm mismatch. No claim-matrix re-duplication per algorithm.

All tests MUST pass under `cargo test --workspace` (Strict TDD gate).
