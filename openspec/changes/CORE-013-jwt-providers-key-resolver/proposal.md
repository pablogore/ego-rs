# Proposal: CORE-013 — JWT Providers + KeyResolver

## Intent

`security-jwt` ships one monolithic `JwtAuthenticator` whose algorithm is a runtime `JwtConfig.algorithm` field. This couples HS256/RS256 in one type, blocks per-algorithm validation strategy, and offers no ES256.

Replace it with three single-algorithm `AuthenticationProvider`s over the existing `KeyResolver`, completing the CORE-011A direction. Pre-stable: no shims (AD-013).

## Scope

### In Scope

- REMOVE `JwtAuthenticator`, `JwtConfig` (struct + module).
- Promote `KeyResolver`/`VerificationKey`/`KeyResolverError`/`JwtAlgorithm` to a stable public surface; add `JwtAlgorithm::Es256` and `VerificationKey::EcPem` (`#[non_exhaustive]`, already prepared).
- Add `Hs256AuthenticationProvider`+`Hs256Config`, `Rs256AuthenticationProvider`+`Rs256Config`, `Es256AuthenticationProvider`+`Es256Config` (AD-014, AD-017).
- Extract shared claim/time validation (exp/nbf/iss/aud strict, sub strict, roles/tenant graceful — CLAR-005) into one internal module reused by all three; preserve `Clock` injection.
- `kid`-driven resolve in all providers via `(kid, algorithm)` lookup (AD-018).
- Keep sync `authenticate()` with internal `block_on` async bridge (AD-016, cache-first AD-013).
- Migrate internal references (lib.rs docs, authenticator tests) to the new providers.
- Strict TDD; `cargo test --workspace` green.

### Out of Scope

- `CompositeAuthenticationProvider` — deferred.
- `#[authorize]`/authz (CORE-014).
- JWKS/Vault/KMS resolvers, token issuance, refresh, OAuth2, SAML (future).

## Capabilities

### Modified Capabilities

- `security-jwt`: replace single-authenticator contract with per-algorithm providers + resolver lookup key `(kid, algorithm)`.

## Approach

`jsonwebtoken` v9 natively verifies HS256/RS256/ES256 — no `rsa`/`p256` crates needed. Each provider: decode header → derive `kid` + assert header alg == provider alg (reject mismatch, AD-017) → `block_on(resolver.resolve(kid, alg))` → build `DecodingKey` from the matching `VerificationKey` variant → delegate to the shared validator. ES256 adds `VerificationKey::EcPem` + `DecodingKey::from_ec_pem`. No multi-alg dispatcher.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `security-jwt/src/authenticator.rs` | Removed | Split into per-alg providers + shared validator |
| `security-jwt/src/config.rs` | Modified | Drop `JwtConfig`; add 3 configs; add `Es256` |
| `security-jwt/src/key_resolver.rs` | Modified | Add `EcPem`; keep trait/LocalKeyResolver |
| `security-jwt/src/lib.rs` | Modified | New exports, updated rustdoc example |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Validation logic duplicated across 3 providers | High | Single shared validator module; providers only key-build |
| ES256 PEM/curve handling subtleties | Med | EC P-256 fixtures; reuse jsonwebtoken `from_ec_pem` |
| `block_on` reentrancy if resolver does I/O | Low | Enforce cache-first AD-013 in docs+tests |

## Rollback Plan

Single leaf crate, no downstream consumers (verified: only internal refs). Revert the CORE-013 commits/branch; `security-jwt` returns to `JwtAuthenticator`.

## Dependencies

No new crates: `jsonwebtoken` v9 covers HS256/RS256/ES256. Add EC test fixtures.

## Success Criteria

- [ ] `JwtAuthenticator`/`JwtConfig` absent from the public API.
- [ ] Three providers verify their algorithm; cross-alg tokens rejected (AD-017).
- [ ] ES256 valid/invalid/mismatched-key paths covered.
- [ ] Shared validator: exp/nbf/iss/aud/sub/roles/tenant parity with CORE-012.
- [ ] `kid` forwarded to resolver; `cargo test --workspace` green.

## Decision Log (Accepted)

- AD-008: async resolver / sync authenticate
- AD-009: JWT types in security-jwt
- AD-010: resolve signature
- AD-011: config/key split
- AD-012: trait + LocalKeyResolver only
- AD-013: no transitional APIs (remove, no shims)
- AD-014: separate provider per algorithm (Open/Closed)
- AD-015: single KeyResolver (no KeyStore)
- AD-016: sync authenticate + async bridge
- AD-017: one algorithm per provider instance
- AD-018: resolver lookup key `(algorithm, kid)`, issuer stays in provider
