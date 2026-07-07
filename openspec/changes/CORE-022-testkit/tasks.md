# Tasks: CORE-022 — TestKit

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 900–1100 |
| 400-line budget risk | High |
| Chained PRs recommended | Yes |
| Suggested split | PR 1 → PR 2 → PR 3 → PR 4 |
| Delivery strategy | chained PRs (owner decision, ask-on-risk resolved) |
| Chain strategy | stacked-to-main |

Decision needed before apply: Resolved — chained PRs, stacked-to-main
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Scaffold + Identity + Security + Context (Phases 1–4) | PR 1 | Foundation modules; crate compiles and tests pass standalone |
| 2 | Authorization + Config (Phases 5–6) | PR 2 | Independent modules built on PR 1's scaffold |
| 3 | Capturing Logger (Phase 7) | PR 3 | Isolated; needs its own grounding review against real kitlogger source (concern #2) |
| 4 | Fixtures + Assertions + Final Verification (Phases 8–10) | PR 4 | Wires everything via `RuntimeBuilder`; includes DI-path parity check (concern #1) and authn-stub privacy check (concern #3); crate becomes fully usable here |

---

## Phase 1: Workspace & Crate Scaffold

- [x] 1.1 Add `"crates/testkit"` to `members` in root `Cargo.toml`. Verify `cargo check --workspace` still compiles.
- [x] 1.2 Create `crates/testkit/Cargo.toml` — package `ego-testkit`, edition 2021. Deps: `async-trait`, `serde` (`derive`), `serde_json`, `ego-domain`, `ego-security-sdk`, `ego-service-sdk`, `kitlogger`/`kitlogger-formatter`/`console-exporter`/`kitlogger-log-domain` (git, `develop`). Feature `dev-providers = ["ego-security-sdk/test-helpers"]`, off by default. Dev-dependency: `ego-service-sdk-macros` (needed for the concern #1 macro-path test in Phase 8).
- [x] 1.3 Create `crates/testkit/src/lib.rs` — `#![deny(missing_docs)]`, crate-level doc stating the same-contract principle, empty `mod identity; mod security; mod context; mod authz; mod config; mod logger; mod fixtures; mod assertions;` declarations (stub files, no logic). `cargo check -p ego-testkit` compiles.

## Phase 2: Identity — `PrincipalBuilder` (AD-7)

- [x] 2.1 **[RED]** In `crates/testkit/src/identity.rs`: tests for default `PrincipalBuilder::new().build()` satisfying `Principal`/`SubjectId` invariants (default subject `"test:subject"`); `.role("admin")` override leaves other fields default; `principal()` == `PrincipalBuilder::new().build()`. Tests fail (not implemented).
- [x] 2.2 **[GREEN]** Implement `PrincipalBuilder` (kind/subject/tenant/roles/attributes) and `principal()` building a real `Principal::new(kind, SubjectId::new(..))`. Add `pub use identity::{PrincipalBuilder, principal};` to `lib.rs`. Tests pass.

## Phase 3: Security Helpers (AD-4)

- [x] 3.1 **[RED]** In `crates/testkit/src/security.rs`: tests for `authenticated(principal)` → `SecurityContext` with that principal, empty claims; `authenticated_with_claims(principal, claims)` → matching claims. Tests fail.
- [x] 3.2 **[GREEN]** Implement `authenticated`/`authenticated_with_claims` via `SecurityContext::empty`/`::new`. Re-export in `lib.rs`. Tests pass.

## Phase 4: `ServiceContext` Builder (AD-2, AD-4)

- [x] 4.1 **[RED]** In `crates/testkit/src/context.rs`: tests for `TestContextBuilder::new().security(sec).build()` attaching security; `.unauthenticated().build()` → `security` is `None` and `require_security()` → `Err(SecurityError::CapabilityNotEnabled)`; two independently built contexts with differing tenant/correlation don't leak state; `test_context()` returns an authenticated context for `principal()`. Tests fail.
- [x] 4.2 **[GREEN]** Implement `TestContextBuilder` (security/unauthenticated/logger/tenant/correlation/build) and `test_context()` over the real `ego_service_sdk::ServiceContext`. Re-export in `lib.rs`. Tests pass.

## Phase 5: Scripted Authorization Provider (AD-3)

- [ ] 5.1 **[RED]** In `crates/testkit/src/authz.rs`: tests for `allow_all()` → `Ok(Allow)` for any `(kind, action)`; `deny_all().allow(kind, action)` → allows only that pair, denies all else; `.deny(kind, action, reason)` → surfaces through `authorize_in_context` as `Err(SecurityError::AuthorizationDenied)`; compile-time assertion that `ScriptedAuthorizationProvider` is `Send + Sync` and object-safe as `Arc<dyn AuthorizationProvider>`. Tests fail.
- [ ] 5.2 **[GREEN]** Implement `ScriptedAuthorizationProvider` (default decision + `(kind, action)` rule map) implementing the real async `AuthorizationProvider` trait. Re-export ungated `DenyAllAuthorizationProvider`; re-export `AllowAllAuthorizationProvider` only behind `#[cfg(feature = "dev-providers")]`. Re-export in `lib.rs`. Tests pass.

## Phase 6: Test Configuration (AD-5)

- [ ] 6.1 **[RED]** In `crates/testkit/src/config.rs`: tests that `TestConfig::new().with_value(42u32).with_value("s".to_string())` collects both distinct-typed values without loss; `.set("k", json!(v))` is reflected only in `.provider()`'s JSON-subtree view and is a separate contract from `.with_value` (assert `.set()` alone leaves the typed-value collection empty). Tests fail. (The "observed via `resolve_config` by a real service" scenario is deferred to Phase 8, once `ServiceTestFixture` exists — do not fake DI here.)
- [ ] 6.2 **[GREEN]** Implement `TestConfig` (`root: serde_json::Value`, `typed: Vec<(TypeId, Arc<dyn Any + Send + Sync>)>`), `with_value::<C>`, `set`, `provider()` → `ConfigurationProvider::from_value`. Re-export in `lib.rs`. Tests pass.

## Phase 7: Capturing Logger (AD-6) — concern #2

- [ ] 7.1 **Grounding check.** Before writing any capture-parsing logic, read the real `kitlogger`/`kitlogger-formatter`/`kitlogger-log-domain`/`console-exporter` `develop` source and confirm: (a) exact JSON keys `LogFormat::Json` emits for level/message/fields, (b) name/signature of the structured field-carrying entry point vs. `KITLogger::log(Severity, &str)`, (c) both paths serialize through the same configured exporter/formatter (AD-6's two documented entry points — no more, no less). Record findings as doc comments directly above the parsing code in `logger.rs`; do not guess field names. If the shape is unstable, fall back to `CapturedRecord::fields` holding the raw object under one top-level key, per AD-6.
- [ ] 7.2 **[RED]** In `crates/testkit/src/logger.rs`: tests that logging via the confirmed structured entry point at a level with fields → `records()` yields one `CapturedRecord` with matching level/message/fields; `KITLogger::log(Severity, &str)` → captured record has level+message, empty `fields`; two independent `CapturingLogger` instances never cross-capture. Tests fail.
- [ ] 7.3 **[GREEN]** Implement `CapturingLogger` (`Arc<KITLogger>` + `Arc<Mutex<Vec<u8>>>` buffer via `ConsoleExporterImpl::set_writers` + `with_exporter_and_format(_, LogFormat::Json)`) and `CapturedRecord { level, message, fields }`, parsing buffered JSON using the field names confirmed in 7.1. Re-export in `lib.rs`. Tests pass.

## Phase 8: Service Test Fixture (AD-9, AD-10) — concern #1

- [ ] 8.1 **[RED]** In `crates/testkit/src/fixtures.rs`: a hand-rolled `Injectable` test service resolving a `ConfigValue<C>` field, built via `fixture.service::<S>()`, observes a value registered through `TestConfig::with_value::<C>` (closes AD-5's deferred scenario); unset `C` → `Err(DependencyNotFound)`, never a panic. `fixture.service::<S>()` returns a real instance whose async trait method yields the service's own `Result<T, ServiceError>`. `ServiceTestFixture::new()` is immediately usable; `FixtureBuilder` overriding only `.authorization(..)` leaves principal/config/logger at default. Tests fail.
- [ ] 8.2 **DI-path parity check (concern #1).** Add one more test in `fixtures.rs` defining a service struct annotated with the real `#[service(...)]` macro (pattern from `crates/service-sdk/examples/order_service.rs` / `crates/service-sdk/tests/authorize_codegen.rs`, using the `ego-service-sdk-macros` dev-dependency from 1.2), built via `fixture.service::<S>()`. Assert it constructs successfully and resolves config identically to the hand-rolled `Injectable` case in 8.1 — proving `fixture.service::<S>()` drives the SAME `Injectable::build` path the `#[service]` macro generates, not a shortcut that only works for hand-rolled impls.
- [ ] 8.3 **[GREEN]** Implement `ServiceTestFixture` (`runtime`, `context`, `logger`) and `FixtureBuilder` (`principal`/`unauthenticated`/`authorization`/`config`/`build`) wiring a real `RuntimeBuilder` (`with_security`, `with_logger`, `with_config::<C>`, `build`). Implement the AD-10 pairing authn stub as a **`pub(crate)`** type in `fixtures.rs` — never `pub`. Add `service::<S: Injectable>()` calling `S::build(self.runtime.inner())`. Re-export `ServiceTestFixture`/`FixtureBuilder` (NOT the authn stub) in `lib.rs`. Tests from 8.1–8.2 pass.
- [ ] 8.4 **Privacy check (concern #3).** Run `rg "pub " crates/testkit/src/fixtures.rs` and inspect `lib.rs`'s `pub use` block. Confirm the AD-10 authn stub's type name is declared `pub(crate)` (or module-private) and does not appear anywhere in `lib.rs`'s public re-exports. Record as a passing check.

## Phase 9: Assertion Helpers (AD-8)

- [ ] 9.1 **[RED]** In `crates/testkit/src/assertions.rs`: tests that `assert_authorized` passes when `authorize_in_context` returns `Ok`, panics with a clear message otherwise; `assert_denied` passes only on `Err(SecurityError::AuthorizationDenied)`; `assert_service_error!(result, Variant { .. })` passes on matching variant and `#[should_panic]`s on a non-matching one regardless of message text. Tests fail.
- [ ] 9.2 **[GREEN]** Implement `assert_authorized`/`assert_denied` (calling the real `authorize_in_context`) and `#[macro_export] assert_service_error!` (`matches!`-based). Re-export functions in `lib.rs` (the macro is crate-root via `#[macro_export]`). Tests pass.

## Phase 10: Final Assembly & Verification

- [ ] 10.1 Review `crates/testkit/src/lib.rs`'s full `pub use` surface against design.md's Interfaces/Contracts section — every listed public type/fn is exported, nothing extra (especially not the AD-10 stub, per concern #3).
- [ ] 10.2 Run `cargo test --workspace` — all `ego-testkit` tests plus existing workspace tests green, no regressions.
- [ ] 10.3 Run `cargo clippy -p ego-testkit -- -D warnings` — zero warnings.
- [ ] 10.4 Run `cargo doc -p ego-testkit --no-deps` — `#![deny(missing_docs)]` builds clean.
- [ ] 10.5 Run `cargo build --workspace` (default features) and confirm `ego-security-sdk/test-helpers` is NOT enabled — `dev-providers` stays opt-in (AD-3, Migration section).
- [ ] 10.6 Run `cargo fmt --check` — no formatting drift.
