# Tasks: CORE-011A — Key Resolver Architecture

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 480–580 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 (Phase 1–3: types + refactor) → PR 2 (Phase 4–6: test migration + new tests + exports) |
| Delivery strategy | ask-on-risk |
| Chain strategy | stacked-to-main |

Decision needed before apply: Yes (resolved: PR 1 = Phases 1–4, stacked-to-main)
Chained PRs recommended: Yes (override: single PR with scope expansion)
Chain strategy: stacked-to-main
400-line budget risk: High (mitigated by user choice)

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Foundation + new types + authenticator refactor + test migration | PR 1 | Expanded scope per user decision |
| 2 | New behavioral tests + public API exports | PR 2 | Targets main after PR 1 merges |

---

## Phase 1: Foundation — Cargo deps + JwtAlgorithm split

- [x] 1.1 **Add `futures-executor` to `crates/security-jwt/Cargo.toml`** — add `futures-executor = "0.3"` under `[dependencies]`; add `async-trait = { workspace = true }` (already in workspace, just wire it in). **File**: `crates/security-jwt/Cargo.toml`. **TDD**: RED — crate fails to compile until async-trait import is used; GREEN — `cargo check` passes.

- [x] 1.2 **Strip key material from `JwtAlgorithm` in `config.rs`** — change `JwtAlgorithm` from struct variants (`Hs256 { secret: Vec<u8> }`, `Rs256 { public_key_pem: String }`) to a plain discriminant enum (`Hs256`, `Rs256`). Derive `Clone, Copy, PartialEq, Eq, Debug`. Remove field docs, update module doc. **File**: `crates/security-jwt/src/config.rs`. **TDD RED**: all existing tests that pattern-match `JwtAlgorithm::Hs256 { secret }` or `JwtAlgorithm::Rs256 { public_key_pem }` fail to compile. **Acceptance**: FR-015, spec §MODIFIED/JwtAlgorithm.

- [x] 1.3 **Remove key material from `JwtConfig`** — `JwtConfig` retains `algorithm: JwtAlgorithm`, `expected_iss: Option<String>`, `expected_aud: Option<Vec<String>>`. No raw bytes or PEM fields. Update module and field docs. **File**: `crates/security-jwt/src/config.rs`. **TDD RED**: existing config constructors with inline secrets fail compile. **Acceptance**: spec §MODIFIED/JwtConfig, AC-011.

---

## Phase 2: New Types — `key_resolver.rs`

- [x] 2.1 **Create `crates/security-jwt/src/key_resolver.rs` skeleton** — empty module file with `#![deny(missing_docs)]` and `//!` module doc. Add `mod key_resolver;` to `lib.rs`. **Files**: `src/key_resolver.rs`, `src/lib.rs`. **TDD RED**: compile error from empty module until types added.

- [x] 2.2 **Define `KeyResolverError` enum** — variants `KeyNotFound { kid: Option<String> }`, `AlgorithmMismatch { expected: JwtAlgorithm, requested: JwtAlgorithm }`, `InvalidKeyMaterial(String)`. Derive `Debug, Clone, PartialEq, Eq`, `#[derive(thiserror::Error)]`. Full doc coverage. **File**: `src/key_resolver.rs`. **TDD**: Write test asserting `KeyResolverError::KeyNotFound { kid: Some("k1") }` round-trips debug repr → RED (type undefined) → GREEN after definition. **Acceptance**: spec §ADDED/KeyResolverError, FR-015.

- [x] 2.3 **Define `VerificationKey` enum** — `#[non_exhaustive]`, variants `Hmac(Vec<u8>)`, `RsaPem(String)`. Derive `Debug, Clone`. Full doc coverage. **File**: `src/key_resolver.rs`. **TDD**: Write test asserting `VerificationKey::Hmac(vec![1,2,3])` stores bytes correctly → RED → GREEN. **Acceptance**: spec §ADDED/VerificationKey, NFR-009.

- [x] 2.4 **Define `KeyResolver` async trait** — `#[async_trait::async_trait]`, `pub trait KeyResolver: Send + Sync`, single method `async fn resolve(&self, kid: Option<&str>, algorithm: JwtAlgorithm) -> Result<VerificationKey, KeyResolverError>`. Doc: cache-first contract (AD-013). **File**: `src/key_resolver.rs`. **TDD**: Write test that constructs a `dyn KeyResolver` reference (object-safety check) → RED → GREEN. **Acceptance**: spec §ADDED/KeyResolver, NFR-005, FR-018.

- [x] 2.5 **Implement `LocalKeyResolver` struct** — fields `algorithm: JwtAlgorithm`, `key: VerificationKey`. `pub fn new(algorithm: JwtAlgorithm, key: VerificationKey) -> Self`. Implement `KeyResolver`: return `Ok(self.key.clone())` on alg match, `Err(AlgorithmMismatch { .. })` on mismatch; ignore `_kid`. Doc coverage. **File**: `src/key_resolver.rs`. **TDD** (write tests first): (a) `resolve(None, Hs256)` on Hs256 resolver → `Ok(Hmac(_))`; (b) `resolve(Some("any"), Hs256)` → `Ok(Hmac(_))` (kid ignored); (c) `resolve(None, Rs256)` on Hs256 resolver → `Err(AlgorithmMismatch)`; (d) `resolve(None, Rs256)` on Rs256 resolver → `Ok(RsaPem(_))`. Use `futures_executor::block_on` in `#[test]` bodies. RED → GREEN → REFACTOR. **Acceptance**: spec §ADDED/LocalKeyResolver, CLAR-009, NFR-007.

---

## Phase 3: Authenticator Refactor

- [x] 3.1 **Update `JwtAuthenticator` struct to hold `Arc<dyn KeyResolver>`** — replace key-material access pattern with `resolver: Arc<dyn KeyResolver>`. Update struct definition and doc comment. **File**: `src/authenticator.rs`. **TDD RED**: current `match &self.config.algorithm { JwtAlgorithm::Hs256 { secret } => … }` blocks no longer compile after Phase 1. GREEN after this task wires resolver.

- [x] 3.2 **Update `JwtAuthenticator::new` signature** — `pub fn new(config: JwtConfig, resolver: Arc<dyn KeyResolver>, clock: Arc<dyn Clock>) -> Self`. Add `use crate::key_resolver::KeyResolver;`. **File**: `src/authenticator.rs`. **Acceptance**: spec §MODIFIED/JwtAuthenticator::new, AC-017, AC-018.

- [x] 3.3 **Implement key resolution in `authenticate()`** — replace old `match &self.config.algorithm` block (steps 1–2 of old flow) with: (a) `decode_header` to extract `kid` and `requested_alg`; (b) `futures_executor::block_on(self.resolver.resolve(kid, requested_alg))` with `KeyResolverError` mapping; (c) build `DecodingKey` from `VerificationKey` variants. Add `use futures_executor::block_on`. Keep steps 7–8 (decode + claims extraction) unchanged verbatim. **File**: `src/authenticator.rs`. **Acceptance**: spec §MODIFIED/JwtAuthenticator, design §5, FR-015, AC-014, AC-015.

- [x] 3.4 **Implement `KeyResolverError → AuthenticationError` mapping** — `KeyNotFound { .. }` → `InvalidSignature`; `AlgorithmMismatch { .. }` → `AlgorithmNotSupported(msg)`; `InvalidKeyMaterial(s)` → `InvalidToken(format!("key material: {s}"))`. **File**: `src/authenticator.rs`. **Acceptance**: spec §ADDED/ErrorMapping, design §6, CLAR-008.

---

## Phase 4: Test Migration (restores all 37 existing tests to GREEN)

- [x] 4.1 **Add test helper functions for new constructor pattern** — add `hs256_resolver() -> Arc<dyn KeyResolver>`, `rs256_resolver() -> Arc<dyn KeyResolver>`, `rs256_other_resolver() -> Arc<dyn KeyResolver>` helpers in the `#[cfg(test)]` block. Update `hs256_config()` and `rs256_config()` to remove embedded key material. **File**: `src/authenticator.rs`. **TDD RED**: all 37 existing `JwtAuthenticator::new(config, clock)` call sites fail to compile. GREEN: helpers compile.

- [x] 4.2 **Migrate all 37 existing tests** — update every `JwtAuthenticator::new(config, clock)` call to `JwtAuthenticator::new(config, resolver, clock)`. Update every `JwtConfig { algorithm: JwtAlgorithm::Hs256 { secret: … } }` inline construction to use `JwtAlgorithm::Hs256` + `hs256_resolver()` / `rs256_resolver()`. For tests using a non-default key (e.g., `hs256_wrong_secret`, `rs256_other_public_key_pem`), construct a `LocalKeyResolver` inline with the specific `VerificationKey`. **File**: `src/authenticator.rs`. **TDD GREEN**: `cargo test -p security-jwt` exits zero. **Acceptance**: NFR-006, AC-012.

---

## Phase 5: New Behavioral Tests

- [x] 5.1 **Test: `kid` from JWT header passed to resolver** — define an in-test `CapturingResolver` that records the `kid` it receives. Sign a JWT with `"kid": "primary-key"` in the header. Assert resolver received `kid = Some("primary-key")`. **File**: `src/authenticator.rs` tests or `tests/key_resolver_integration.rs`. **Acceptance**: spec §KeyResolver/scenario 1.

- [x] 5.2 **Test: `kid = None` when JWT has no kid field** — sign a JWT without `kid` header claim. Assert resolver receives `kid = None`. **File**: same as 5.1. **Acceptance**: spec §KeyResolver/scenario 2.

- [x] 5.3 **Test: `KeyNotFound` error mapping** — define `FailingResolver` (in-test) that always returns `Err(KeyResolverError::KeyNotFound { kid: None })`. Assert `authenticate(...)` returns `Err(AuthenticationError::InvalidSignature)`. **File**: `src/authenticator.rs` tests. **Acceptance**: spec §ErrorMapping/scenario 1, CLAR-008.

- [x] 5.4 **Test: `AlgorithmMismatch` error mapping** — `FailingResolver` returns `AlgorithmMismatch`. Assert `AuthenticationError::AlgorithmNotSupported(_)`. **File**: `src/authenticator.rs` tests. **Acceptance**: spec §ErrorMapping/scenario 2.

- [x] 5.5 **Test: `InvalidKeyMaterial` error mapping** — `FailingResolver` returns `InvalidKeyMaterial("bad pem")`. Assert `AuthenticationError::InvalidToken(_)`. **File**: `src/authenticator.rs` tests. **Acceptance**: spec §ErrorMapping/scenario 3.

- [x] 5.6 **Test: shared resolver (AC-019)** — create one `Arc<dyn KeyResolver>` from `LocalKeyResolver`. Construct two `JwtAuthenticator` instances with `Arc::clone`. Assert both authenticate the same valid HS256 token successfully. **File**: `src/authenticator.rs` tests. **Acceptance**: AC-019, NFR-010.

- [x] 5.7 **Test: `LocalKeyResolver::resolve` is runtime-free** — call `futures_executor::block_on(resolver.resolve(None, JwtAlgorithm::Hs256))` from a plain `#[test]` with no `#[tokio::test]` annotation. Assert it returns `Ok(VerificationKey::Hmac(_))`. **File**: `src/key_resolver.rs` tests. **Acceptance**: NFR-007, design §8.

---

## Phase 6: Public API Surface + Docs

- [x] 6.1 **Export new types from `lib.rs`** — add `pub use key_resolver::{KeyResolver, KeyResolverError, LocalKeyResolver, VerificationKey};`. Update crate-level doc example to show `LocalKeyResolver::new(JwtAlgorithm::Hs256, VerificationKey::Hmac(...))` usage. **File**: `src/lib.rs`. **Acceptance**: NFR-008 (public API accessible), AC-011.

- [x] 6.2 **Verify `#[deny(missing_docs)]` compliance** — confirm `cargo doc -p security-jwt` emits zero warnings for all new public items (`KeyResolver`, `VerificationKey`, `KeyResolverError`, `LocalKeyResolver` and all their variants/methods). Fix any missing doc strings. **Files**: `src/key_resolver.rs`, `src/lib.rs`. **Acceptance**: NFR-008.

- [x] 6.3 **Final `cargo test --workspace` clean run** — all crates build and all tests pass with zero failures, zero warnings on new code. **Acceptance**: NFR-006, AC-012.
