# Proposal: CORE-011A — Key Resolver Architecture

## Metadata

| Field | Value |
|-------|-------|
| Change ID | CORE-011A |
| Title | Key Resolver Architecture |
| Type | Amendment to CORE-011 (JWT Authentication Provider) |
| Date | 2026-06-23 |
| Parent | CORE-011 (complete) |
| Enables | CORE-011B (JWKS remote key resolver) |
| Status | PROPOSING |

## Problem Statement

`JwtConfig` embeds key material directly (secret bytes or PEM string), and `JwtAuthenticator` reads that material from its own fields. Key retrieval is therefore coupled to the authenticator. There is no seam to introduce JWKS fetching, key rotation, or multi-issuer resolution without modifying both `JwtAuthenticator` and its configuration type. This violates dependency inversion at the infrastructure boundary: a stable verification engine should not change when the key source changes.

## Goals

- **G1 (FR-015)**: `JwtAuthenticator` depends on a `KeyResolver` abstraction, not on raw key material.
- **G2 (FR-016)**: `AuthenticationProvider` public API stays unchanged — synchronous, same signature.
- **G3 (FR-017)**: Provide `LocalKeyResolver`, an in-memory resolver backed by static key material.
- **G4 (FR-018)**: Future resolvers (JWKS) are pluggable with no change to `JwtAuthenticator`.

## Non-Goals (deferred to CORE-011B)

- Caching layer (`CachingKeyResolver`)
- JWKS / remote HTTP key resolution
- OIDC discovery
- ES256 / EdDSA algorithm support

## Proposed Solution

Introduce a `KeyResolver` abstraction in `security-jwt` that owns key retrieval. `JwtConfig` is split: it keeps functional configuration (algorithm, optional issuer/audience, clock skew) and loses key material. Key material moves behind `Arc<dyn KeyResolver>`, injected at construction. The shipped `LocalKeyResolver` wraps static key material so today's behavior is preserved. The resolver is async so CORE-011B can do network I/O without reshaping the trait; the authenticator bridges async resolution internally and keeps `authenticate()` synchronous (AD-007 intact).

## Architecture Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| AD-008 | `KeyResolver` is an async trait (`#[async_trait]`) | `LocalKeyResolver` is trivially sync, but CORE-011B (JWKS) needs network I/O. The async boundary belongs at the resolver, not at `AuthenticationProvider`. `authenticate()` stays sync (AD-007); the authenticator bridges async internally. |
| AD-009 | `KeyResolver`, `VerificationKey`, `JwtAlgorithm` live in `security-jwt`, not domain | Domain must stay ignorant of JWT, key IDs, algorithms, and crypto material. Domain only knows `Credential → AuthenticationProvider → SecurityContext`. |
| AD-010 | `kid` is in the resolver signature from day 1 | Adding `kid` later would be a breaking trait change. Marginal cost now is zero; JWKS requires it. |
| AD-011 | Split `JwtConfig` into functional config + injected resolver | Functional config (algorithm, iss, aud, clock skew) stays in `JwtConfig`; key material becomes `Arc<dyn KeyResolver>` passed to the constructor. |
| AD-012 | CORE-011A scope = trait + `LocalKeyResolver` only | No caching, no remote resolver. Over-engineering a component that does not yet exist is prohibited. |
| AD-013 | `KeyResolver` implementations MUST be cache-first | `AuthenticationProvider` is synchronous. `futures::executor::block_on` is safe inside `authenticate()` ONLY because `KeyResolver::resolve` must return from locally available state. Remote key acquisition (JWKS, database) MUST happen outside the auth path — via background refresh, explicit warm-up, or scheduled sync. This contract is a prerequisite for FR-019–021. |

## Interface Sketch

> Contract shapes only — not implementation. All types live in `crates/security-jwt`.

```rust
pub enum KeyResolverError {
    KeyNotFound { kid: Option<String> },
    AlgorithmMismatch { expected: JwtAlgorithm, requested: JwtAlgorithm },
    InvalidKeyMaterial(String),
}

pub enum VerificationKey {
    Hmac(Vec<u8>),     // HS256 shared secret
    RsaPem(String),    // RS256 public key (PEM)
}

#[async_trait]
pub trait KeyResolver: Send + Sync {
    async fn resolve(
        &self,
        kid: Option<&str>,
        algorithm: JwtAlgorithm,
    ) -> Result<VerificationKey, KeyResolverError>;
}

pub struct LocalKeyResolver { /* static key material */ }

// JwtConfig: algorithm, optional issuer, optional audience, clock skew — NO key material.
impl JwtAuthenticator {
    pub fn new(config: JwtConfig, resolver: Arc<dyn KeyResolver>) -> Self;
}
```

## Migration Path

1. Remove key-material fields from `JwtConfig`; keep algorithm, optional iss/aud, clock skew.
2. Add `KeyResolver`, `VerificationKey`, `KeyResolverError`, `LocalKeyResolver` to `security-jwt`.
3. Change `JwtAuthenticator::new` to take `(JwtConfig, Arc<dyn KeyResolver>)`.
4. Internally resolve keys via the resolver, bridging async→sync inside `authenticate()`.
5. Update existing call sites/tests to wrap current key material in `LocalKeyResolver` (AC-012).

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Async→sync bridge inside `JwtAuthenticator` blocks/deadlocks (block_on on a running runtime) | Medium | For `LocalKeyResolver` resolution is in-memory and immediate; document the bridge strategy and constrain to non-reentrant runtime use. Remote resolvers (CORE-011B) will pre-resolve via cache instead of blocking. |
| New `tokio` runtime dependency in `security-jwt` for the bridge | Medium | Scope runtime usage to the resolver bridge; keep `LocalKeyResolver` free of real I/O so tests need no live runtime. |
| `async_trait` trait-object safety / boxing overhead | Low | `async_trait` produces object-safe `dyn KeyResolver`; overhead is a heap allocation per resolve, negligible vs. signature verification. |
| Layer violation (key/JWT concepts leaking into domain) | Low | AD-009 keeps all key types in `security-jwt`; enforced by `layers.toml` / `verify-layers.sh`. |

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/security-jwt/` | Modified | New `KeyResolver` trait, `VerificationKey`, `KeyResolverError`, `LocalKeyResolver`; `JwtConfig` and `JwtAuthenticator::new` changed |
| `crates/domain/src/auth/` | Unchanged | `AuthenticationProvider` API preserved (FR-016 / AD-007) |
| `openspec/specs/domain/auth.md` | Reference update | Infrastructure reference notes resolver-based key access |

## Capabilities

### New Capabilities
- None

### Modified Capabilities
- `domain/auth`: infrastructure reference for `security-jwt` changes — key material removed from `JwtConfig`, key access goes through the `KeyResolver` port. Domain authentication contract itself is unchanged; only the documented infrastructure implementation reference is updated.

## Rollback Plan

Revert is a single-crate change confined to `security-jwt`. Restore key-material fields on `JwtConfig`, restore the prior `JwtAuthenticator::new` signature, and drop the `KeyResolver` types. No domain or runtime changes to unwind; `AuthenticationProvider` never changed.

## Dependencies

- **Blocks on**: CORE-011 (complete).
- **Enables**: CORE-011B (JWKS remote key resolver, caching).

## Success Criteria

- [ ] **AC-011**: `JwtAuthenticator` holds no direct key material (no raw bytes, no PEM strings in its fields).
- [ ] **AC-012**: All existing JWT tests pass using `LocalKeyResolver` via the new constructor.
- [ ] **AC-013**: A JWKS resolver can be added implementing `KeyResolver` with no change to `AuthenticationProvider` or `JwtAuthenticator`.
- [ ] **FR-016**: `AuthenticationProvider` signature and synchronous boundary remain unchanged.
