# Archive Report: API Key Authentication (CORE-021)

**Status**: CLOSED
**Date Archived**: 2026-07-03
**Test Status**: ✓ 42 tests pass, 0 CRITICAL findings, 0 open WARNING/SUGGESTION

---

## Executive Summary

CORE-021 implements first-class API key authentication for ego-rs. The new `security-apikey` crate is production-ready, follows SDK conventions, and passes all tests with zero clippy warnings. The implementation is additive — no existing code's behavior was changed, rollback is trivial (delete crate, remove workspace member). Merged as [PR #112](https://github.com/pablogore/ego-rs/pull/112) into `develop`.

Two rounds of external code review ran against the open PR before merge, catching a real timing side-channel and an expiry boundary bug that the initial implementation missed. Both were fixed and re-verified before merge; see "Post-Merge-Readiness Fixes" below.

---

## What Was Built

### New Crate: `crates/security-apikey`

A synchronous API key authentication provider implementing `AuthenticationProvider` with:

1. **Value Objects** (`ApiKeyId`, `Secret`, `ApiKeyHash`)
   - `ApiKeyId`: validated, charset-constrained (128 char max), hashable for HashMap keys
   - `Secret`: zeroized-on-drop via `#[derive(Zeroize, ZeroizeOnDrop)]`
   - `ApiKeyHash`: opaque digest wrapper; constant-time verification via `subtle::ConstantTimeEq`; `sha256`/`of` constructors are `pub` so external resolvers can build `ApiKeyRecord`

2. **Parser SPI** (`ApiKeyParser` trait + `DefaultApiKeyParser`)
   - Default splits on first `.` separator: `{key_id}.{secret}` (secret may itself contain further dots)
   - Callers can supply custom parsers for different wire formats

3. **Resolver SPI** (`ApiKeyResolver` trait + `LocalApiKeyResolver` marker + `InMemoryApiKeyResolver`)
   - `ApiKeyResolver::lookup` is synchronous, cache-first, no I/O
   - `LocalApiKeyResolver: ApiKeyResolver {}` — empty marker trait the provider requires instead of bare `ApiKeyResolver`, an explicit (not compiler-verified) opt-in assertion that an implementation is local/no-I/O. See AD-8.
   - Returns `Option<Arc<ApiKeyRecord>>` with principal, scopes, expiry, `Arc<HashMap<String,String>>` metadata, hash
   - `InMemoryApiKeyResolver` supports dual-key coexistence (rotation pattern), implements `LocalApiKeyResolver`
   - Object-safe: storable as `Arc<dyn ApiKeyResolver>` / `Arc<dyn LocalApiKeyResolver>`

4. **Authentication Provider** (`ApiKeyAuthenticationProvider`)
   - Validation flow: extract Bearer → `MAX_KEY_BYTES` guard → parse → lookup → **unconditional hash-verify** (dummy digest when the key id is unknown) → expiry check (`now >= expires_at` rejects, including the exact-tie instant) → accept only if found AND hash matched AND not expired
   - The unconditional hash-verify step is the timing-side-channel fix: an unknown key id and a known key id with the wrong secret cost the same
   - All failures return `AuthenticationError::InvalidToken` (uniform error, no cause disclosure)
   - Scopes propagated to `SecurityContext.claims.custom["scopes"]` as JSON array
   - Clock injection for deterministic testing
   - Configurable parser via builder method

5. **Test Coverage** (strict TDD, 42 tests)
   - Happy path (valid key, matching hash, principal + scopes propagated)
   - Failure modes: malformed, unknown key, expired (including the exact-tie boundary), hash mismatch, oversized, non-Bearer
   - Structural regression test asserting no early return exists between the resolver lookup and the dummy-hash step (guards the timing fix against reintroduction)
   - Parser determinism test (same input always produces the same output)
   - Constant-time verification (oracle-free stub confirms no early exit)
   - Object-safety and Send+Sync compile assertions for both `ApiKeyResolver` and `LocalApiKeyResolver`
   - `mockall` automock for `ApiKeyResolver`
   - Dual-key coexistence scenario

### Workspace Changes

- Added `"crates/security-apikey"` member to root `Cargo.toml`
- New dependencies: `subtle = "2"`, `zeroize = { version = "1", features = ["derive"] }`, `chrono = "0.4"`, `tracing = "0.1"`
- Reused existing dependencies: `sha2`, `serde_json`, `thiserror`

---

## Architecture Decisions (AD-1 through AD-8)

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **AD-1: Resolver async?** | Synchronous `lookup` | AD-004 forbids I/O in `authenticate`; API keys have no async upstream (unlike JWT's JWKS refresh) |
| **AD-2: Hash internal representation** | Fixed `[u8; 32]` digest, single hardcoded algorithm (SHA-256) | Minimal deps, fully opaque public API; no algorithm-tag field since only one algorithm is supported |
| **AD-3: Hash algorithm (reference)** | SHA-256 | Fast, already in workspace, correct for high-entropy keys; resolver implementations may use stronger algorithms |
| **AD-4: Secret zeroization** | Local newtype `#[derive(Zeroize, ZeroizeOnDrop)]` | Minimal surface (`as_bytes` only), zero crypto-adjacent unsafe; mirrors `zeroize` primitive |
| **AD-4b: Clock injection** | `Arc<dyn Clock>` constructor param | Deterministic tests (mirrors JWT pattern) |
| **AD-5: Parser default** | Split on first `.` | `{key_id}.{secret}` → secret may contain dots; empty halves rejected |
| **AD-6: `ApiKeyId` max length** | 128 chars | Fits UUIDs, ULIDs, prefixed ids; bounds HashMap key size; rejects pathological input |
| **AD-7: Scopes carrier** | `Claims.custom["scopes"]` as JSON array | Request-scoped assertions belong in Claims, not Principal.attributes; no SDK change needed |
| **AD-8: `LocalApiKeyResolver` opt-in marker** | Empty marker trait `LocalApiKeyResolver: ApiKeyResolver {}`; provider requires it instead of bare `ApiKeyResolver` | Rust can't enforce "no I/O" at compile time; a marker trait forces an explicit, auditable opt-in from resolver authors instead of silently satisfying an unconstrained trait. See design.md for full rationale. |

---

## Key Design Contracts

1. **Uniform Error**: All failures (unknown, expired, malformed, hash-mismatched) return `AuthenticationError::InvalidToken`. No cause differentiation — permanent security invariant.

2. **Timing-Safe Validation**: `authenticate()` always performs a hash-verify (real digest if the key was found, a fixed dummy digest otherwise) before deciding accept/reject, and treats the exact expiry-tie instant as expired. This closes a timing side-channel where an unknown key id would otherwise return faster than a known key id with the wrong secret.

3. **`LocalApiKeyResolver` Contract** (AD-004, AD-8): `ApiKeyResolver::lookup` must return from locally available state without I/O, on every path including the not-found path — a database-backed or HTTP-backed resolver would reopen the timing side-channel regardless of how `key_hash.verify` behaves. `ApiKeyAuthenticationProvider` requires `Arc<dyn LocalApiKeyResolver>` (an opt-in marker), not bare `Arc<dyn ApiKeyResolver>`, as a soft (non-compiler-verified) enforcement of this contract. `InMemoryApiKeyResolver` is the canonical reference implementation.

4. **Constant-Time Verification**: `ApiKeyHash::verify` is guaranteed constant-time via `subtle::ConstantTimeEq`. The resolver owns algorithm selection; the provider never touches it directly.

5. **Scope Opaqueness**: Scopes are opaque strings (`Vec<String>`). No validation, transformation, or filtering in the provider. Format convention is caller's concern.

6. **Object-Safety**: `ApiKeyResolver`, `LocalApiKeyResolver`, and `ApiKeyAuthenticationProvider` are all object-safe and storable as `Arc<dyn T>`, enabling dynamic wiring in runtime builders.

---

## Known Deferrals (Out of Scope for CORE-021)

### D-1: External Resolver Hash Construction — Resolved

`ApiKeyHash::verify` is public, and `sha256`/`of` constructors are `pub`, so external resolvers (Postgres, Redis, Vault) can construct `ApiKeyHash` directly. No follow-up needed.

### D-2: SecurityContext::scopes Accessor

The proposal discusses "scopes on SecurityContext" — this maps to `Claims.custom["scopes"]`. If a first-class `SecurityContext::scopes` field is desired later, that's an additive SDK change (AD-7 follow-up), not part of CORE-021.

**Deferred because** the current design (Claims.custom) is sufficient and requires no SDK changes.

### D-3: Remote Resolver SPI (RemoteApiKeySource or similar)

External review (M1/H1, see below) correctly identified that `LocalApiKeyResolver` only provides a soft, author-asserted contract — Rust's type system cannot prevent an I/O-performing resolver from implementing the marker. A future, more structurally distinct SPI for remote-backed resolvers (with its own explicit latency/consistency contract) is a real possible evolution, tracked here rather than built speculatively (YAGNI: no remote resolver implementation exists yet to design against).

**Deferred because** no consumer needs a remote resolver today; building the split now would be speculative.

---

## Post-Merge-Readiness Fixes (found via external PR review, fixed before merge)

Two rounds of review against the open PR found and fixed:

1. **Timing side-channel** (HIGH): the resolver contract ("no I/O") was documentation-only, unenforceable by the type system, and `authenticate()` originally short-circuited on an unknown key id before any hash-verify, creating a timing oracle. Fixed by making the hash-verify step unconditional (dummy digest fallback) and adding `LocalApiKeyResolver` as an explicit opt-in marker (AD-8).
2. **Expiry tie-break**: `expires_at == now` was originally treated as still valid; fixed to reject the exact-tie instant, with a dedicated boundary test.
3. **`ApiKeyRecord.metadata` allocation**: changed from `HashMap<String, String>` to `Arc<HashMap<String, String>>` since the provider never reads it and resolvers sharing metadata across records now pay one allocation instead of one per record.
4. **Deleted determinism test restored**: `determinism_same_input_same_output` in `parser.rs` had been dropped during a rewrite; restored.
5. Minor: stale `pub(crate)` visibility on `ApiKeyHash::sha256`/`of` promoted to `pub` (closing D-1); spec/design docs corrected to match the final shipped ordering and signatures throughout.

Full detail: [PR #112 review threads](https://github.com/pablogore/ego-rs/pull/112).

---

## Testing & Verification

### Test Results (final, at merge)
- ✓ `cargo test -p security-apikey`: 42 tests pass
- ✓ `cargo test --workspace`: all pass, no regressions
- ✓ `cargo clippy -p security-apikey --all-targets -- -D warnings`: 0 warnings
- ✓ `cargo fmt -p security-apikey`: clean
- ✓ `cargo doc -p security-apikey --no-deps`: all public items documented

Workspace-wide `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` have pre-existing, unrelated failures in `ego-service-sdk-macros`, `persistent-entity`, and `security-jwt` — not introduced by and out of scope for this change. Tracked separately: `openspec/changes/CORE-023-workspace-hygiene`.

### Verification Report (from sdd-verify)
- **Status**: PASSED
- **CRITICAL**: 0 findings
- **WARNING**: 0 open (all resolved pre-merge)
- **SUGGESTION**: 0 open (D-1 resolved; D-3 filed as a deliberate future-evolution deferral, not an open action item)

---

## Integration Points

### service-sdk Runtime Builder

Construction is transparent — the runtime builder already accepts `Arc<dyn AuthenticationProvider>`:

```rust
let resolver: Arc<dyn LocalApiKeyResolver> = Arc::new(InMemoryApiKeyResolver::new());
let clock = Arc::new(SystemClock::new());
let provider = Arc::new(ApiKeyAuthenticationProvider::new(resolver, clock));
```

**Note**: `service-sdk`'s `RuntimeBuilder` currently accepts a single `AuthenticationProvider` slot — there is no multi-scheme wiring API (e.g. a `with_authentication_provider(Scheme::Bearer, provider)` method) to run this alongside `security-jwt` in one runtime today. Composing multiple providers is a real gap, not addressed by this change.

### Credential Pipeline

`ApiKeyExtractor` (already in codebase) maps HTTP Bearer header to `Credential::Bearer(raw_key)`. The provider consumes this and performs the validation flow described above.

---

## Rollback Plan

CORE-021 is fully additive. Rollback:
1. Remove `"crates/security-apikey"` from root `Cargo.toml` members
2. Delete `crates/security-apikey/` directory
3. No existing code's behavior was changed — zero regressions expected

---

## Files Changed

See [PR #112](https://github.com/pablogore/ego-rs/pull/112) for the full commit history (3 commits: initial implementation, timing/metadata hardening, `LocalApiKeyResolver` marker trait) and diff.

New crate: `crates/security-apikey/` (`Cargo.toml`, `lib.rs`, `key_id.rs`, `key_hash.rs`, `secret.rs`, `parser.rs`, `resolver.rs`, `authenticator.rs`). Modified: root `Cargo.toml`/`Cargo.lock` (workspace member).

---

## Artifacts & Traceability

- **Proposal**: `openspec/changes/archive/2026-07-03-CORE-021-api-key-auth/proposal.md`
- **Specification**: `openspec/changes/archive/2026-07-03-CORE-021-api-key-auth/spec.md` (+ delta spec at `specs/security-apikey/spec.md`)
- **Design**: `openspec/changes/archive/2026-07-03-CORE-021-api-key-auth/design.md`
- **Tasks**: `openspec/changes/archive/2026-07-03-CORE-021-api-key-auth/tasks.md`
- **Archive Report**: `openspec/changes/archive/2026-07-03-CORE-021-api-key-auth/archive-report.md` (this file)
- **Implementation**: `crates/security-apikey/` (entire crate)
- **Pull Request**: [pablogore/ego-rs#112](https://github.com/pablogore/ego-rs/pull/112)
- **Follow-up (filed, not started)**: `openspec/changes/CORE-023-workspace-hygiene` (pre-existing workspace-wide fmt/clippy debt, unrelated to this change)

---

## Change Metadata

| Field | Value |
|-------|-------|
| Change ID | CORE-021 |
| Title | API Key Authentication |
| Type | Feature / Core SDK |
| Status | CLOSED |
| Merged PR | [#112](https://github.com/pablogore/ego-rs/pull/112) (develop) |
| Test Coverage | 42/42 passing |
| Clippy Warnings | 0 |
| Breaking Changes | None (additive) |
| SDK Changes | None (crate is independent) |

---

## Sign-Off

**Verification**: PASSED (42 tests, 0 CRITICAL, 0 open WARNING/SUGGESTION)
**Quality**: Production-ready — zero clippy warnings on the crate, full documentation, strict TDD, timing side-channel closed and regression-tested
**Rollback Risk**: Trivial — additive crate, no existing callers
**Recommendation**: MERGED AND CLOSED

CORE-021 is now archived and considered complete.
