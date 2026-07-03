# Tasks: API Key Authentication (CORE-021)

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 320–420 |
| 400-line budget risk | Medium |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | single-pr |
| Chain strategy | size-exception |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: size-exception
400-line budget risk: Medium

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Full `security-apikey` crate + workspace wire | PR 1 | All modules, all tests; additive, no existing callers |

---

## Phase 1: Workspace & Crate Scaffold

- [x] 1.1 Add `"crates/security-apikey"` to `members` in root `Cargo.toml`. Verify `cargo check --workspace` still compiles (no new errors on existing crates).
- [x] 1.2 Create `crates/security-apikey/Cargo.toml` — package name `security-apikey`, edition 2021. Deps: `ego-domain` (path), `ego-security-sdk` (path), `sha2` (workspace), `subtle = "2"`, `zeroize = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `thiserror = "2"`. Dev-deps: `mockall` (workspace).
- [x] 1.3 Create `crates/security-apikey/src/lib.rs` — `#![deny(missing_docs)]`, module declarations, pub re-exports for all public types. No logic here.

---

## Phase 2: Value Objects (TDD — write tests first)

- [x] 2.1 **[RED]** Write tests in `key_id.rs` (`#[cfg(test)]`): valid id accepted, empty rejected, forbidden char (`@`, `.`, space) rejected, length 128 accepted, length 129 rejected. All tests must fail (file not yet impl).
- [x] 2.2 **[GREEN]** Implement `ApiKeyId(String)` in `crates/security-apikey/src/key_id.rs`: `ApiKeyId::new(s: &str) -> Result<Self, AuthenticationError>` — validates non-empty, charset `[a-zA-Z0-9_-]`, max 128 chars. `as_str(&self) -> &str`. Derive `Hash`, `Eq`, `PartialEq`, `Clone` (needed for HashMap key in resolver). Tests pass.
- [x] 2.3 **[RED]** Write tests in `secret.rs`: `as_bytes()` roundtrip, `#[derive(Zeroize, ZeroizeOnDrop)]` compile check.
- [x] 2.4 **[GREEN]** Implement `Secret(Vec<u8>)` in `crates/security-apikey/src/secret.rs`: `#[derive(Zeroize, ZeroizeOnDrop)]`, `pub fn as_bytes(&self) -> &[u8]`, no `String` exposure. Tests pass.
- [x] 2.5 **[RED]** Write tests in `key_hash.rs`: `verify(matching)` returns true, `verify(non-matching)` returns false (constant-time path asserted by running to completion without panic).
- [x] 2.6 **[GREEN]** Implement `ApiKeyHash` in `crates/security-apikey/src/key_hash.rs`: opaque struct (private `[u8; 32]` digest). `pub fn sha256(digest: [u8; 32]) -> Self`. `pub fn verify(&self, secret: &[u8]) -> bool` — SHA-256 hash of `secret` then `subtle::ConstantTimeEq` compare with stored digest. Tests pass.

---

## Phase 3: Parser (TDD)

- [x] 3.1 **[RED]** Write tests in `parser.rs`: `"id.secret"` parses to `(ApiKeyId("id"), Secret(b"secret"))`, `"id.sec.ret"` parses to secret `"sec.ret"` (dots after first kept), no dot → `InvalidToken`, empty id half → `InvalidToken`, empty secret half → `InvalidToken`. The parser assumes the caller has already enforced the size bound; no oversized test here (that contract lives in the provider — phase 5).
- [x] 3.2 **[GREEN]** Implement `ApiKeyParser: Send + Sync` trait and `DefaultApiKeyParser` in `crates/security-apikey/src/parser.rs`. `split_once('.')` — empty halves → `InvalidToken`. Tests pass. (`MAX_KEY_BYTES` lives in `authenticator.rs`, not here — the parser assumes the caller has already enforced the size bound.)

---

## Phase 4: Resolver (TDD)

- [x] 4.1 **[RED]** Write tests in `resolver.rs`: known key returns `Ok(Some(record))`, unknown key returns `Ok(None)`, dual-key coexistence (two `ApiKeyId`s for same principal both resolve independently), `Arc<dyn ApiKeyResolver>` object-safety compile assertion, `mockall` automock compile check.
- [x] 4.2 **[GREEN]** Implement in `crates/security-apikey/src/resolver.rs`:
  - `ApiKeyResolverError::Backend(String)` (`thiserror`)
  - `ApiKeyRecord { principal, scopes, expires_at, metadata, key_hash }` (derive `Clone`)
  - `trait ApiKeyResolver: Send + Sync` with `#[cfg_attr(test, mockall::automock)]`
  - `InMemoryApiKeyResolver` (HashMap-backed, `pub fn insert(key_id, record)`)
  - `pub trait LocalApiKeyResolver: ApiKeyResolver {}` (empty marker; `impl LocalApiKeyResolver for InMemoryApiKeyResolver {}`) — see design.md AD-8
  - Tests pass.

---

## Phase 5: Authenticator (TDD)

- [x] 5.1 **[RED]** Write tests in `authenticator.rs` using `InMemoryApiKeyResolver` + fixed-clock stub:
  - Happy path → `Ok(SecurityContext)` with correct principal
  - Scopes in `claims.custom["scopes"]` as JSON array
  - Non-Bearer credential → `InvalidToken`
  - Oversized raw credential (> `MAX_KEY_BYTES`) → `InvalidToken` (before parse)
  - Malformed key (no dot) → `InvalidToken`
  - Unknown key id → `InvalidToken`
  - Expired record (`expires_at < now`) → `InvalidToken`
  - Hash mismatch → `InvalidToken`
  - `Send + Sync` compile assertion: `fn assert_send_sync<T: Send + Sync>() {}`
  - `Arc<dyn AuthenticationProvider>` object-safety compile assertion
  - All failure paths collapse to `InvalidToken` (no variant differentiation)
- [x] 5.2 **[GREEN]** Implement `ApiKeyAuthenticationProvider` in `crates/security-apikey/src/authenticator.rs`:
  - `new(resolver: Arc<dyn ApiKeyResolver>, clock: Arc<dyn Clock>) -> Self` (uses `DefaultApiKeyParser`)
  - `with_parser(self, parser: Arc<dyn ApiKeyParser>) -> Self`
  - `const MAX_KEY_BYTES: usize = 1024` defined here (not in parser)
  - `impl AuthenticationProvider`: strict validation order — Bearer extract → MAX_KEY_BYTES guard on raw → parse → lookup → hash verify (dummy hash when unknown, timing-safe) → expiry → `SecurityContext` (must extract raw first before measuring its length)
  - Scopes → `claims.custom["scopes"]` as `serde_json::Value::Array`
  - All failures return `AuthenticationError::InvalidToken`
  - Tests pass.

---

## Phase 6: Final Verification

- [x] 6.1 Run `cargo test --workspace` — all tests green, no regressions in existing crates.
- [x] 6.2 Run `cargo clippy -p security-apikey -- -D warnings` — zero warnings in security-apikey (pre-existing warning in ego-security-sdk excluded; confirmed pre-existing via git stash).
- [x] 6.3 Confirm `#![deny(missing_docs)]` builds clean (`cargo doc -p security-apikey --no-deps`).
- [x] 6.4 Run `cargo fmt --check` — no formatting drift.
