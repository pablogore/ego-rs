# Design: CORE-013 — JWT Providers + JwtValidationEngine

## Technical Approach

Replace the monolithic `JwtAuthenticator`/`JwtConfig` with three single-algorithm `AuthenticationProvider`s (HS256/RS256/ES256) over the existing `KeyResolver`. Each provider owns ONLY: alg enforcement, `kid` extraction, key resolution via the async→sync bridge, and `DecodingKey` build. All claim/time/iss/aud/sub/roles/tenant logic is extracted verbatim from `authenticator.rs` into one internal `JwtValidationEngine` (AD-014), preserving CORE-012/CLAR-005 semantics (sub strict, roles/tenant graceful). No new crates: `jsonwebtoken` v9 covers all three algorithms.

## Component Architecture

```
Credential::Bearer
   ↓ decode_header → kid + header.alg
Hs256AuthenticationProvider | Rs256AuthenticationProvider | Es256AuthenticationProvider
   ├─ assert header.alg == provider alg (else AlgorithmNotSupported) [AD-017]
   ├─ block_on(resolver.resolve(kid, alg)) [AD-016/AD-018]
   ├─ build DecodingKey from matching VerificationKey variant
   ↓ delegate
JwtValidationEngine (internal, not pub) — decode+verify, exp/nbf/iss/aud/sub/roles/tenant
   ↓
SecurityContext
```

## Architecture Decisions

| ID | Choice | Rejected | Rationale |
|----|--------|----------|-----------|
| AD-013 | Remove JwtAuthenticator/JwtConfig, no shims | Deprecated aliases | Pre-stable, leaf crate, zero external consumers (verified) |
| AD-014 | One internal `JwtValidationEngine`, providers do only key-build | Duplicate logic per provider; trait default methods | DRY; single CLAR-005 source of truth |
| AD-015 | Single `pub trait KeyResolver` | Separate KeyStore | Already exists, sufficient |
| AD-016 | sync `authenticate()` + internal `block_on` | async trait | Cache-first resolver → bridge never parks |
| AD-017 | One algorithm per provider; reject header alg mismatch | Runtime alg field | Open/Closed; explicit per-alg validation; Composite deferred |
| AD-018 | Resolver lookup key `(algorithm, kid)`; issuer in engine | Issuer-aware resolver | Resolver issuer-agnostic; iss is a validation concern |
| AD-019 | `JwtValidationEngine` is `pub(crate)`; MUST NOT be re-exported | Public engine type | Providers are the only integration point; crate boundary enforces this |

## Public API

```rust
// config.rs — JwtAlgorithm gains Es256
pub enum JwtAlgorithm { Hs256, Rs256, Es256 }

pub struct Hs256Config { pub expected_iss: Option<String>, pub expected_aud: Option<Vec<String>> }
pub struct Rs256Config { pub expected_iss: Option<String>, pub expected_aud: Option<Vec<String>> }
pub struct Es256Config { pub expected_iss: Option<String>, pub expected_aud: Option<Vec<String>> }

pub struct Hs256AuthenticationProvider { /* config, resolver, clock */ }
pub struct Rs256AuthenticationProvider { /* config, resolver, clock */ }
pub struct Es256AuthenticationProvider { /* config, resolver, clock */ }

impl<Each> {
    pub fn new(config: XConfig, resolver: Arc<dyn KeyResolver>, clock: Arc<dyn Clock>) -> Self
}
impl AuthenticationProvider for <Each> {
    fn authenticate(&self, &Credential) -> Result<SecurityContext, AuthenticationError>
}
```

`lib.rs` exports: three providers + three configs + `JwtAlgorithm`, `KeyResolver`, `KeyResolverError`, `LocalKeyResolver`, `VerificationKey`. Remove `JwtAuthenticator`, `JwtConfig`.

## Internal API — JwtValidationEngine (`src/validation.rs`, `pub(crate)`)

```rust
pub(crate) struct ValidationParams<'a> {
    expected_iss: Option<&'a str>,
    expected_aud: Option<&'a [String]>,
}

pub(crate) struct JwtValidationEngine;

impl JwtValidationEngine {
    // Receives the already-resolved decoding key + algorithm; performs decode,
    // signature verify (validate_exp/nbf/aud disabled), then clock-injected
    // exp/nbf, iss/aud, sub (strict), roles/tenant (graceful), builds Claims+Principal.
    pub(crate) fn validate(
        token: &str,
        key: &DecodingKey,
        alg: Algorithm,
        params: ValidationParams,
        clock: &dyn Clock,
    ) -> Result<SecurityContext, AuthenticationError>;
}
```

Moves these existing fns from `authenticator.rs` verbatim: `RawClaims`, `build_standard_claims`, `extract_subject`, `extract_tenant_id`, `extract_roles`, `remove_standard_keys`, plus the exp/nbf/iss/aud/sub blocks.

## VerificationKey Extension

Add `EcPem(String)` variant (it is `#[non_exhaustive]` — additive). Build via `DecodingKey::from_ec_pem(pem.as_bytes())`. Each provider's match handles only its own variant; other variants → `AuthenticationError::InvalidToken`.

## Claims Validation Flow (engine, unchanged semantics)

1. `decode::<RawClaims>` with validate_exp/nbf/aud off, required_spec_claims empty → InvalidSignature/AlgorithmNotSupported/InvalidToken mapping.
2. exp: present & <= now → ExpiredToken; non-int → InvalidToken.
3. nbf: > now → InvalidToken; non-int → InvalidToken.
4. iss: if expected configured, must match (absent → InvalidToken).
5. aud: if expected configured, ≥1 overlap (absent → InvalidToken).
6. sub strict: absent→MissingClaim("sub"); non-string/empty→InvalidToken.
7. roles/tenant graceful: wrong type → skip, raw preserved in custom.
8. Build Principal + Claims → SecurityContext.

## Key Resolution Flow (per provider)

`decode_header(token)` → assert `header.alg == PROVIDER_ALG` else `AlgorithmNotSupported` → `block_on(resolver.resolve(header.kid, PROVIDER_ALG))` → map KeyResolverError (KeyNotFound→InvalidSignature, AlgorithmMismatch→AlgorithmNotSupported, InvalidKeyMaterial→InvalidToken) → match VerificationKey variant → DecodingKey → engine.

## File Changes

| File | Action | Detail |
|------|--------|--------|
| `src/validation.rs` | Create | `JwtValidationEngine` + extracted helpers (pub(crate)) |
| `src/providers.rs` | Create | Three providers; shared header/alg/key-build helper |
| `src/authenticator.rs` | Delete | Logic split into validation.rs + providers |
| `src/config.rs` | Modify | Drop JwtConfig; add Es256; add 3 configs |
| `src/key_resolver.rs` | Modify | Add `VerificationKey::EcPem` |
| `src/lib.rs` | Modify | New exports + rustdoc example rewrite |
| `tests/fixtures/test_ec_*.pem` | Create | EC P-256 priv/pub/other-pub fixtures for ES256 |
| `openspec/specs/domain/auth.md` | Modify | Update JwtAuthenticator/JwtConfig references |

## Migration Strategy

Leaf crate, no downstream consumers. Existing `authenticator.rs` tests are re-homed: HS256 tests → Hs256 provider; RS256 → Rs256 provider; resolver-error/kid/shared-resolver tests distributed. Engine-only invariants tested through Hs256 provider (no fixtures needed) to avoid 3x duplication.

## Testing Strategy (Strict TDD)

| Layer | What | How |
|-------|------|-----|
| security-jwt unit (engine) | exp/nbf/iss/aud/sub/roles/tenant matrix | Through Hs256 provider only (CLAR-005 parity) |
| security-jwt unit (providers) | alg enforcement, kid forwarding, resolver-error mapping, signature paths | Per provider: valid/wrong-key/mismatched-alg |
| ES256 | valid, invalid sig, mismatched key | New EC fixtures + jsonwebtoken from_ec_pem |

Engine logic NOT re-tested per algorithm — signature/key paths are; claim logic once.

## Sequence Diagrams

### HS256
```
caller → Hs256Provider.authenticate(Bearer)
  decode_header → alg=HS256 ✓, kid
  block_on(resolver.resolve(kid, Hs256)) → VerificationKey::Hmac(bytes)
  DecodingKey::from_secret(bytes), Algorithm::HS256
  → JwtValidationEngine.validate(token,key,HS256,params,clock)
      decode+verify sig → claims → time/iss/aud/sub/roles/tenant
  ← SecurityContext
```

### RS256
```
caller → Rs256Provider.authenticate(Bearer)
  decode_header → alg=RS256 ✓ (HS256/ES256 header → AlgorithmNotSupported)
  block_on(resolver.resolve(kid, Rs256)) → VerificationKey::RsaPem(pem)
  DecodingKey::from_rsa_pem(pem) (err→InvalidToken), Algorithm::RS256
  → JwtValidationEngine.validate(...) ← SecurityContext
```

## Risks & Mitigations

| Risk | Sev | Mitigation |
|------|-----|------------|
| Behavior drift moving logic out of authenticator.rs | WARNING | Move helpers verbatim; reuse existing test bodies against providers |
| ES256 EC PEM/curve subtleties | WARNING | P-256 fixtures; jsonwebtoken from_ec_pem; cover valid/invalid/mismatch |
| EC fixtures missing on disk | SUGGESTION | Generate + commit EC fixtures; verify RSA fixtures resolve before relying on them |
| block_on reentrancy if resolver does I/O | LOW | Enforce cache-first AD-013 in docs/tests |

## Decision Log

AD-008 async resolver/sync authenticate; AD-009 JWT types in security-jwt; AD-010 resolve signature; AD-011 config/key split; AD-012 trait+LocalKeyResolver only; AD-013 no shims (remove); AD-014 shared JwtValidationEngine; AD-015 single KeyResolver; AD-016 sync authenticate + async bridge; AD-017 one algorithm per provider; AD-018 resolver lookup key (algorithm, kid), issuer in engine; AD-019 JwtValidationEngine pub(crate) only, providers are the only integration point.
