# Design: CORE-011A — Key Resolver Architecture

## 1. Overview

Introduce a `KeyResolver` async trait in `security-jwt` that owns key retrieval. `JwtAlgorithm` becomes a pure marker enum (no embedded material); key material moves into `VerificationKey` returned by the resolver. `JwtConfig` keeps only functional config. `JwtAuthenticator::new(config, resolver, clock)` injects an `Arc<dyn KeyResolver>`. The authenticator parses the JWT header, resolves the key via the async trait bridged to sync with `futures::executor::block_on`, then verifies. `LocalKeyResolver` wraps static material so today's behavior is preserved and tests need no live runtime.

## 2. Module Structure

| File | Action | Description |
|------|--------|-------------|
| `crates/security-jwt/src/key_resolver.rs` | Create | `KeyResolver` trait, `VerificationKey`, `KeyResolverError`, `LocalKeyResolver` |
| `crates/security-jwt/src/config.rs` | Modify | `JwtAlgorithm` becomes marker enum; `JwtConfig` drops key material |
| `crates/security-jwt/src/authenticator.rs` | Modify | `new` takes resolver; `authenticate` resolves key via bridge |
| `crates/security-jwt/src/lib.rs` | Modify | Export new types; update doc example |
| `crates/security-jwt/Cargo.toml` | Modify | Add `async-trait`, `futures` |
| `crates/domain/**` | Unchanged | AD-009 / FR-016 — no domain edits |

## 3. Type Designs

```rust
// config.rs — marker enum, no material (BREAKING vs current embedded-key enum)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JwtAlgorithm { Hs256, Rs256 }

pub struct JwtConfig {
    pub algorithm: JwtAlgorithm,
    pub expected_iss: Option<String>,
    pub expected_aud: Option<Vec<String>>,
}

// key_resolver.rs
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyResolverError {
    #[error("key not found (kid: {kid:?})")]
    KeyNotFound { kid: Option<String> },
    #[error("algorithm mismatch: expected {expected:?}, requested {requested:?}")]
    AlgorithmMismatch { expected: JwtAlgorithm, requested: JwtAlgorithm },
    #[error("invalid key material: {0}")]
    InvalidKeyMaterial(String),
}

#[non_exhaustive]
pub enum VerificationKey { Hmac(Vec<u8>), RsaPem(String) }

#[async_trait::async_trait]
pub trait KeyResolver: Send + Sync {
    async fn resolve(&self, kid: Option<&str>, algorithm: JwtAlgorithm)
        -> Result<VerificationKey, KeyResolverError>;
}

pub struct LocalKeyResolver { algorithm: JwtAlgorithm, key: VerificationKey }
impl LocalKeyResolver {
    pub fn new(algorithm: JwtAlgorithm, key: VerificationKey) -> Self { /* .. */ }
}
#[async_trait::async_trait]
impl KeyResolver for LocalKeyResolver {
    async fn resolve(&self, _kid: Option<&str>, algorithm: JwtAlgorithm)
        -> Result<VerificationKey, KeyResolverError> {
        if algorithm != self.algorithm {
            return Err(KeyResolverError::AlgorithmMismatch {
                expected: self.algorithm, requested: algorithm });
        }
        Ok(self.key.clone()) // VerificationKey derives Clone
    }
}
```

**Structure decision (Challenge 2):** single `(JwtAlgorithm, VerificationKey)` pair, not a `BTreeMap`. Multi-key resolution is JWKS territory (CORE-011B), and AD-012 forbids over-engineering. Constructor is `new(algorithm, key)` (not `from_config`) so the resolver is decoupled from `JwtConfig` — the JWKS resolver will not take a `JwtConfig` either.

**`kid` handling (CLAR-009):** `LocalKeyResolver` ignores `kid` (advisory) and returns its single key whenever the algorithm matches. `kid` stays in the signature (AD-010) for CORE-011B.

## 4. Async→Sync Bridge Decision (Challenge 1)

| Option | Tradeoff | Verdict |
|--------|----------|---------|
| (a) `Handle::current().block_on` | Requires ambient tokio runtime; **deadlocks** if called on a runtime worker thread | Reject — forces runtime, breaks non-tokio callers/tests |
| (b) `futures::executor::block_on` | No runtime needed; drives the future on the current thread to completion | **Choose** |
| (c) custom `Ready` executor | Reinvents (b) | Reject |
| (d) extra sync `resolve_cached` | Two methods on the port → leaks impl detail, violates FR-018 one-trait | Reject |
| (e) split sync/async traits | Directly violates FR-018 | Reject |

**Chosen: (b) `futures::executor::block_on`.** `authenticate()` stays synchronous (AD-007). For `LocalKeyResolver` the future is immediately ready, so `block_on` returns without parking — zero runtime requirement, tests run as plain `#[test]`.

`block_on` is acceptable here ONLY because AD-013 guarantees the future completes immediately from local state. For any `KeyResolver` implementation that performs remote I/O, the resolver MUST pre-populate a local cache before being passed to `JwtAuthenticator`. The `authenticate()` path never waits for network operations.

**Behavior under CORE-011B:** a remote JWKS resolver MUST NOT do blocking network I/O inside `resolve` while driven by `block_on` on a runtime worker (would stall that worker). The documented contract for CORE-011B is **cache-first**: `resolve` returns from an in-memory cache synchronously-fast; cache refresh happens out-of-band (background task / explicit warm-up), never inside the `authenticate` hot path. This keeps the trait unchanged (FR-018) and the bridge safe. Document this constraint on the `KeyResolver` trait now.

## 4a. Cache-First Contract (AD-013)

**Invariant**: `KeyResolver::resolve` MUST return from locally available state on every call made during `authenticate()`. This is the foundational safety contract that makes the async→sync bridge with `futures::executor::block_on` correct.

**How `LocalKeyResolver` satisfies it**: trivially — all key material is stored in memory at construction time. There is no I/O path; `resolve` is always immediately ready.

**How CORE-011B (JWKS) must satisfy it**: the JWKS resolver MUST maintain a pre-warmed, in-memory key cache. Key refresh (fetching the JWKS endpoint) MUST happen in a background task or via an explicit `warm_up()` call before the resolver is passed to `JwtAuthenticator`. The resolver returns from its cache synchronously-fast inside `resolve`; it never initiates a network request on the `authenticate` hot path.

**What happens if the contract is violated**: violating AD-013 turns `authenticate()` into a blocking I/O operation. If called on a tokio worker thread, `block_on` will park the worker while the network request is in flight, stalling the runtime. This contract is documented, not compiler-enforced — the type system cannot distinguish "cache-backed future" from "network-initiating future". Correctness depends on discipline at the `KeyResolver` implementation boundary.

## 4b. VerificationKey Extensibility (NFR-009)

`VerificationKey` is marked `#[non_exhaustive]` (see type designs in section 3). This attribute ensures that downstream code matching on `VerificationKey` variants must include a wildcard arm, which allows new variants to be added in future changes without breaking the `KeyResolver` trait signature or `AuthenticationProvider`.

**Why this matters**: adding a new key type in the future (e.g., ES256 or an OIDC JWK) does not require any change to the `KeyResolver` trait or `AuthenticationProvider`. The trait still returns `Result<VerificationKey, KeyResolverError>` — only the set of valid variants grows.

**Intended expansion path**:
1. **CORE-011A**: `Hmac(Vec<u8>)` and `RsaPem(String)` — covers HS256 and RS256.
2. **CORE-011A follow-on**: `Es256(Vec<u8>)` — raw EC public key bytes for ES256, if needed before CORE-011B.
3. **CORE-011B**: `Jwk(JsonWebKey)` or a structured key type — backed by a JWKS-fetched key object.

**Implementation concern**: `JwtAuthenticator`'s internal match on `VerificationKey` (step 6 of the authenticator flow) will require updating when new variants are added. This is an implementation concern scoped to `security-jwt`, not a public contract concern. Because `VerificationKey` is `#[non_exhaustive]`, the compiler will require a `_` arm or explicit variant coverage, surfacing the update requirement at compile time.

## 5. JwtAuthenticator Flow

```
authenticate(credential):
  1. credential -> BearerToken or InvalidToken("unsupported credential type")
  2. decode_header(token) -> header   // jsonwebtoken::decode_header; map err -> InvalidToken
  3. requested_alg = map_header_alg(header.alg)  // HS256->Hs256, RS256->Rs256,
                                                 // else AlgorithmNotSupported
  4. kid = header.kid.as_deref()
  5. key = block_on(resolver.resolve(kid, requested_alg))  // map KeyResolverError (sec.6)
  6. (decoding_key, alg) = match key {
        Hmac(b)   => (DecodingKey::from_secret(&b), Algorithm::HS256),
        RsaPem(p) => (DecodingKey::from_rsa_pem(p.as_bytes())?, Algorithm::RS256) }
  7. decode + verify signature (unchanged jsonwebtoken path)
  8. exp / nbf / iss / aud checks + identity extraction (UNCHANGED from current code)
```

Steps 7–8 are lifted verbatim from the current implementation. Only steps 2–6 (header parse → resolve → build key) are new. The authenticator holds `config`, `resolver: Arc<dyn KeyResolver>`, `clock` — **no key material** (AC-011).

## 6. Error Mapping

| `KeyResolverError` | → `AuthenticationError` | Rationale |
|--------------------|-------------------------|-----------|
| `KeyNotFound { kid }` | `InvalidSignature` | No key ⇒ token cannot be trusted; avoid leaking key-store internals |
| `AlgorithmMismatch { .. }` | `AlgorithmNotSupported(msg)` | Header alg not served by resolver |
| `InvalidKeyMaterial(s)` | `InvalidToken(format!("key material: {s}"))` | Configuration-side material defect surfaced without secret bytes |

Header-parse failure (step 2) → `InvalidToken`. Unknown header alg (step 3) → `AlgorithmNotSupported`.

## 7. JwtConfig Migration

| Field | Before | After |
|-------|--------|-------|
| `algorithm` | `JwtAlgorithm` (owns `secret`/`pem`) | `JwtAlgorithm` (marker: `Hs256`/`Rs256`) |
| key material | inside `algorithm` | moved to `VerificationKey` via resolver |
| `expected_iss` | `Option<String>` | unchanged |
| `expected_aud` | `Option<Vec<String>>` | unchanged |

Constructor: plain `JwtAuthenticator::new(config: JwtConfig, resolver: Arc<dyn KeyResolver>, clock: Arc<dyn Clock>)`. No builder — three explicit args match the existing `(config, clock)` style and keep the diff minimal (AD-011).

**`Arc` over `Box` (NFR-010 / NFR-011)**: `JwtAuthenticator` stores `Arc<dyn KeyResolver>`, not `Box<dyn KeyResolver>`. This is a deliberate ownership decision: multiple authenticator instances MAY share the same resolver, and the resolver's lifecycle is external to any individual authenticator. `JwtAuthenticator::new` clones the `Arc` — it does not take ownership of a concrete type.

**Shared resolver example** (AC-019): two authenticators sharing one resolver are correct and supported:

```rust
let resolver: Arc<dyn KeyResolver> = Arc::new(LocalKeyResolver::new(
    JwtAlgorithm::Hs256,
    VerificationKey::Hmac(secret_bytes),
));
let auth_a = JwtAuthenticator::new(config_a, Arc::clone(&resolver), clock.clone());
let auth_b = JwtAuthenticator::new(config_b, Arc::clone(&resolver), clock.clone());
// Both auth_a and auth_b share the same resolver instance.
// Resolver lifecycle outlives both authenticators.
```

**CORE-011B implication**: CORE-011B's JWKS resolver will use this shared ownership to centralize the key cache. A single `Arc<JwksKeyResolver>` can be shared across multiple `JwtAuthenticator` instances (e.g., per-issuer authenticators) and across background refresh tasks, without duplicating cache state or network connections.

## 8. Test Design (Strict TDD)

RED-able units in dependency order:

1. `key_resolver.rs` tests — `LocalKeyResolver::resolve` returns key on alg match; `AlgorithmMismatch` on mismatch; ignores `kid`. Async tests need an executor: use `futures::executor::block_on` in plain `#[test]` (no `#[tokio::test]`, stays runtime-free).
2. Error-mapping test — assert each `KeyResolverError` → `AuthenticationError` via a tiny in-test `FailingResolver` implementing the trait.
3. Bridge test — `authenticate` succeeds when driven from a plain `#[test]` (proves no runtime needed).

**Existing 37 tests migrate** by replacing the embedded-key config helpers. New helper pattern:

```rust
fn hs256_resolver() -> Arc<dyn KeyResolver> {
    Arc::new(LocalKeyResolver::new(JwtAlgorithm::Hs256,
        VerificationKey::Hmac(hs256_secret())))
}
fn hs256_config() -> JwtConfig {
    JwtConfig { algorithm: JwtAlgorithm::Hs256, expected_iss: None, expected_aud: None }
}
// call site:
let auth = JwtAuthenticator::new(hs256_config(), hs256_resolver(), now_clock());
```

Wrong-secret / mismatched-RS256-key tests swap only the `VerificationKey` inside the resolver. **No `MockKeyResolver` needed** for the happy paths — `LocalKeyResolver` covers them; a minimal in-test `FailingResolver` covers the error-mapping branch (`KeyNotFound`/`InvalidKeyMaterial`) that `LocalKeyResolver` cannot produce. This satisfies AC-012 (all existing tests pass via `LocalKeyResolver`) and AC-013 (a second trait impl proves pluggability).

## 9. Layer Compliance

All new types (`KeyResolver`, `VerificationKey`, `KeyResolverError`, `JwtAlgorithm` marker) live in `crates/security-jwt` (infrastructure). Domain is untouched — `AuthenticationProvider`, `AuthenticationError`, `Credential`, `Clock` unchanged. `layers.toml` / `verify-layers.sh` enforcement is preserved because no new cross-layer edge is introduced; `security-jwt → ego-domain` is the only ego edge, and `ego-runtime` still does not depend on `security-jwt` (NFR-001). No `ego-security-sdk` import (CLAR-004).

## 10. Dependency Changes

`crates/security-jwt/Cargo.toml`:
- `async-trait = { workspace = true }` — already in workspace deps; for the trait.
- `futures = "0.3"` — for `executor::block_on`. New external dep, executor-only feature surface; no tokio runtime pulled in.

No `tokio` dependency added (avoids the risk flagged in the proposal). `block_on` from `futures` needs no runtime.

## 11. AD Record

| AD | Status | Implementation note |
|----|--------|--------------------|
| AD-008 async trait | Confirmed | `#[async_trait]`; bridged via `futures::executor::block_on` (not tokio) |
| AD-009 types in security-jwt | Confirmed | New module `key_resolver.rs`; domain untouched |
| AD-010 `kid` in signature | Confirmed | `LocalKeyResolver` ignores it (CLAR-009 advisory); reserved for CORE-011B |
| AD-011 split JwtConfig | Confirmed + amended | Amendment: `JwtAlgorithm` ALSO splits into marker enum; material moves to `VerificationKey`. Necessary because today the enum owns the bytes. |
| AD-012 scope = trait + LocalKeyResolver | Confirmed | Single key/algorithm pair; no map, no cache, no remote |
| AD-013 cache-first resolver contract | Confirmed | `block_on` is safe only because `resolve` returns from local state; remote I/O MUST happen out-of-band; contract documented, not compiler-enforced |

## Open Questions

- [ ] `futures` vs hand-rolled minimal `block_on` — `futures` is the standard, low-risk choice; confirm no objection to the new external dep (alternative: pin to `futures-executor` only to shrink surface).
