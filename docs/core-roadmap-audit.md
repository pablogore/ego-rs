# CORE Roadmap Implementation Audit

Lead-architect audit comparing the current repository against the CORE roadmap. This is not a code review and not a proposal review — it is a verification of actual implementation status against merged proposals, designs, and specs. Every finding below is based on direct source reads, `cargo test` runs, and cross-referencing openspec documents against `crates/`, not on trusting "archived" status alone.

---

## CORE-000 — Release Engineering & CI/CD

### Status: 🔴 Not Implemented

**Complete:** `.github/workflows/claude.yml` (mention-bot) and `claude-code-review.yml` (AI review commentary on PR open/sync) exist. A real `Makefile` at root defines legitimate local gates: `make test` (`cargo test --workspace` + jwt feature + contract tests), `make test-cov` (tarpaulin, 95% fail-under), `make clippy` (`-D warnings`), `make buf-deterministic`.

**Partial:** The quality tooling is real but entirely disconnected from automated enforcement — nothing in `.github/workflows/` invokes it.

**Still missing:** No CI job runs `cargo test`/`clippy`/`fmt --check`. No `[workspace.lints]`, no `deny.toml`. No release automation, no CHANGELOG, all 15 crates frozen at `0.1.0` with no bump history. No documented release process anywhere.

**Architecture Closed?** NO — a PR that fails to compile can merge today; there is no mechanized gate at all.

**Tech Debt:** `.github/workflows/` needs a real build/test/lint job; no versioning strategy despite internal crate interdependencies.

**Confidence:** High

---

## CORE-006 — Persistent Entity Runtime

### Status: ⚠️ Needs Refactor

**Complete:** Real load→execute→persist→apply→snapshot→publish lifecycle (`crates/persistent-entity/src/actor.rs`), concrete `EntityTriple` identity, bounded FIFO mailbox with single-writer ordering *within one actor handle's lifetime*, optimistic concurrency with replay/snapshot fallback (`persistence.rs`). Compiles clean, tests pass.

**Partial:** Single-writer guarantee does not span independent `entity_ref()` calls for the same entity ID — `TokioEntityRef::new` spawns a fresh actor unconditionally, no active-sender lookup.

**Still missing:** The headline guarantee — the "final consistency lock" / single-flight activation — is **dead code**. `activation.rs::SharedActivation` is a real `Mutex` that is never constructed by `runtime.rs` or `entity_ref_tokio.rs`. The `get_active_sender` function the archived `tasks.md` names as the core mechanism doesn't exist. `EntityRegistry.active_entities` is a `HashSet<String>`, which literally can't detect duplicate concurrent activations. `runtime_verification_suite.rs` claims to verify this but only checks a pure function and two config fields — zero concurrency coverage. Two orphaned files (`supervisor.rs`, `types.rs`) are excluded from the build via a missing `pub mod`. `RecoveryManager` methods are hardcoded "not yet implemented." `crates/runtime`/`crates/runtime-tokio` message delivery is a no-op — `persistent-entity` bypasses it with a hand-rolled mailbox. Archived `tasks.md` checkboxes are 100% unchecked.

**Architecture Closed?** NO — `final-consistency-lock/spec.md` self-certifies "STABLE" via doc cross-consistency only; the actual guarantee is absent from the dispatch path. Concurrent double-activation of the same entity is not prevented today.

**Confidence:** High (corroborated by direct reads + `cargo test`)

---

## CORE-007 — Reactive Projection Engine

> Naming drift: `ARCHITECTURE.md:~304` has a stale, unrelated "Cluster Model" entry under this same number from before renumbering — never reconciled.

### Status: 🟡 Partial

**Complete:** `crates/ego-scheduler` is fully wired — tag/entity routing, bounded backpressure (`mpsc::channel(4096)`, drop policies), a real reactive cycle (`ingest→detect→route→reduce→evaluate→emit`), deterministic round-robin backed by a 1000+-case property test. 6 test files, none ignored.

**Partial:** Read-side projection primitives exist (`EventTag`, `DedupStore`, `OffsetStore`, `ReadSideSession::execute()`) — but live in `crates/domain`/`crates/runtime`, **not** `crates/event-adapter` where openspec says this work belongs. `TagSchedulerImpl::start_projection()` compiles but nothing drives it live. `DropOldest` silently behaves as `DropNewest` (tracked TODO).

**Still missing:** `crates/event-adapter` is an empty 4-line shell — directly contradicting the openspec doc's location claim. Zero tests for read-side projections despite ~40 planned test tasks in `tasks.md` (unchecked). No dead-letter handling, no replay/rebuild wiring.

**Architecture Closed?** NO — scheduler half is closed; read-side-projection half is not wired live, lives in the wrong crate, and has no test coverage.

**Confidence:** High

---

## CORE-008 — Service SDK

### Status: 🟡 Partial

**Complete:** `ServiceRegistry` (type+version keyed, semver resolution), `#[service]`/`#[operation]` macros generating tag/proxy/`Resolvable`/`ServiceContract`, interceptor pipeline (`Interceptor`/`InterceptorChain`) fully wired into generated proxies, `RuntimeBuilder` DI container, context propagation (`ServiceContext`, 54 call sites). 90+ tests pass.

**Partial:** DI primitives (`ProjectionRef`/`AdapterRef`/`ConfigValue`) are thin wrappers; the `Injectable` trait has **zero implementers** anywhere in the codebase. Tenant enforcement (`RuntimeInner::enforce_tenant`) is an explicit no-op stub, called unconditionally on every `#[operation]` invocation — a guaranteed pass-through today.

**Still missing:** `TracingInterceptor` is dead/aspirational — `interceptor/builtin/mod.rs` contains only a commented-out `pub use tracing::TracingInterceptor;` referencing a `tracing.rs` file that doesn't exist. No built-in interceptors ship at all.

**Architecture Closed?** NO — interceptor mechanism is complete but its intended "built-in interceptors" are empty, and DI has no production implementer.

**Tech Debt:** dead `interceptor/builtin/mod.rs` reference; unused `Injectable` trait; silent tenant no-op.

**Confidence:** High

---

## CORE-009 — Security SDK

### Status: 🟢 Completed

Real trait boundary (`Principal`, `Credential`, `AuthenticationProvider`, `AuthorizationProvider`, `SecurityContext`, `SecurityError`), genuinely load-bearing across the workspace — `security-jwt` and `security-apikey` implement `AuthenticationProvider` against it; `service-sdk` wires `AuthorizationProvider` into `#[authorize(...)]` codegen. `security-sdk` depends on nothing but `ego-domain`, preserving the intended one-way dependency direction.

**Partial:** Spec drifted from the *archived proposal* (not from current code) — `AuthenticationProvider` went async→sync, `Principal` gained `tenant_id`, `CapabilityNotEnabled` was added later via CORE-012 (see Cross-Cutting section below). Code matches the current merged spec.

**Architecture Closed?** YES.

**Tech Debt:** Archived `design.md`/`tasks.md` are truncated placeholders, not real records; `providers/{allow_all,deny_all}` have no consumers outside the crate itself.

**Confidence:** High

---

## CORE-009D — Optional Security Capability

### Status: 🟢 Completed

`SecurityError::CapabilityNotEnabled`, `authorize_in_context()` returning it when security is absent, `ServiceContext::require_security()` (Result, no panic), `RuntimeBuilder` building with or without security configured, macro codegen using `ok_or_else(...)?` with no `unwrap`/`panic!`. No global/static security state anywhere.

**Architecture Closed?** YES.

**Tech Debt:** `RuntimeInner::new()`/`Default` still bypass `RuntimeBuilder` (self-flagged TASK-014); `enforce_tenant`/`issue_cross_tenant_permit` no-ops (same TASK-014, shared debt with CORE-008).

**Confidence:** High

---

## CORE-010A — Remove Ambient ServiceContext

### Status: 🟢 Completed

`ServiceContext` is a plain `Clone` struct, forwarded explicitly as a generated first argument — verified against actual macro-expanded output and golden snapshots, not just prose. Workspace-wide grep for `thread_local!`/`lazy_static`/`task_local!`/`Lazy` context state: **zero hits** relevant to context/session/principal/tenant. `COOKBOOK.md`'s claim was independently verified true.

**Architecture Closed?** YES.

**Tech Debt:** macro doesn't typecheck that the first param is `ServiceContext` (convention, not compiler-enforced); stale "TaskLocal" mentions remain in `COOKBOOK.md`.

**Confidence:** High

---

## CORE-013 — JWT Authentication Providers

### Status: 🟢 Completed

Real crypto (HS256/RS256/ES256 via `jsonwebtoken`), a genuinely working **JWKS remote resolver** (`JwksKeyResolver`: HTTP fetch, background TTL refresh, cache), **real OIDC discovery** (`HttpDiscoveryProvider` hits `.well-known/openid-configuration`), multi-issuer support, RFC 7662 introspection. This is notably *more* than PRD.md credits — PRD.md still lists "CORE-011B JWKS remote key resolver" as unstarted "Next Up," but the `oauth2-oidc` change (2026-06-29) already delivered exactly that as an in-scope extension of the same stack.

**Architecture Closed?** YES — one coherent pipeline, `authenticator.rs → key_resolver.rs → jwks.rs → discovery.rs → multi_issuer.rs → oidc_provider.rs`.

**Confidence:** High

---

## CORE-014 — Authorization Providers

### Status: ⚠️ Needs Refactor

**Complete:** `DenyAllAuthorizationProvider`, `RbacProvider` (real role/permission matching, wildcard actions, resource wildcards correctly rejected), `AuthorizationProvider` SPI. Live-verified: 8/8 targeted tests pass, 117 pass with `dev-providers`, 113 on default features. The `#[authorize]` macro is correctly absent (deferred to CORE-015, not falsely credited here).

**Partial:** `AllowAllAuthorizationProvider` works but is silently feature-gated behind `dev-providers`/`test-helpers` — absent from default builds, and this gating is **not documented** in proposal/design/archive-report, which describe it as generally available.

**Architecture Closed?** NO — the feature-gate is a real API-shape decision never captured in any approved planning doc. Possibly the right safety call, but it's undocumented drift.

**Tech Debt:** archived `archive-report.md`'s "79 tests" figure is stale against the actual 113/117.

**Confidence:** High

---

## CORE-015 — Declarative Authorization & Service Security Integration

### Status: 🟢 Completed (as the authorization macro) — but see the naming collision below

**Complete:** `#[authorize(context = ctx, permission = "resource:action")]` is a real proc-macro attribute wired through `#[service]` expansion, calling `authorize_in_context(...)` **at runtime**, pulling the provider from `RuntimeInner`. Fails closed on missing context, dropped runtime, or disabled capability. 9 compile-time error classes covered by unit tests + 8 trybuild fixtures. Runtime enforcement live-tested: **7/7 pass** (allow path, deny path with body-not-executed, missing-context, dropped-runtime, capability-not-enabled, multi-method guards).

**Partial:** `AccessRequest::from_permission` — the "stable parsing target" CORE-014's spec named for this macro — is fully implemented but the macro bypasses it, hand-building `Resource`/`Action` directly instead. Functionally equivalent, minor undocumented drift.

**⚠️ Documentation collision (confirmed independently by two agents):** `PRD.md:169` labels "CORE-015" as an unstarted **Telemetry SDK** (`ego-telemetry-sdk`). But CORE-014's own archived proposal/archive-report, and the test files themselves (`authorize_codegen.rs`, `authorization_integration.rs`), self-identify as "CORE-015 — the `#[authorize]` macro." Two unrelated features share one CORE number in different documents. If PRD.md's meaning is taken at face value, **CORE-015-as-Telemetry-SDK is 🔴 Not Implemented — no `ego-telemetry-sdk` crate exists anywhere in the 15-crate workspace.**

**Architecture Closed?** YES for the macro. NO / N/A for telemetry — nothing exists.

**Confidence:** High

---

## CORE-012 — "Structured Logging Framework" (per audit brief)

### Status: 🔴 Not Implemented

There is no structured logging framework anywhere in the repo. `tracing` is declared as a dependency in 4 crates (`ego-scheduler`, `security-jwt`, `service-sdk`, `persistent-entity`) but is essentially unused — one stray log call exists in `security-jwt/src/oidc_config.rs`, and `tracing-subscriber` in `service-sdk` has **zero live call sites**: the only reference is a commented-out `pub use tracing::TracingInterceptor;` in `crates/service-sdk/src/interceptor/builtin/mod.rs` pointing at a `tracing.rs` file that doesn't exist (independently confirmed by the CORE-008 audit above). No subscriber initialization, no correlation IDs, no JSON output, nothing.

**⚠️ Naming collision:** the openspec archive actually numbered `CORE-012` is `2026-06-24-CORE-012-security-context-unification` — a real, already-shipped, unrelated capability: it unified the dual `SecurityContext` model between `domain::auth` and `security-sdk`, made `AuthenticationProvider` sync, added `tenant_id` to `Principal`, and added `SecurityError::CapabilityNotEnabled`. This is the "amendment" the CORE-009 audit found without initially knowing its source. This is genuinely completed, valuable, foundational work — but it has nothing to do with logging, and the roadmap brief's CORE-012 label is simply wrong/stale for what this number actually shipped.

**Architecture Closed?** NO — no structured logging exists to close.

**Confidence:** High

---

## Repository-Wide Audit Table

| CORE | Status | Architecture Closed | Confidence | Remaining Work |
|------|--------|---------------------|------------|----------------|
| CORE-000 | 🔴 Not Implemented | NO | High | Wire `make test`/`clippy`/`fmt` into actual CI; add versioning/release strategy |
| CORE-006 | ⚠️ Needs Refactor | NO | High | Wire single-flight activation guard (`SharedActivation` is dead); fix `HashSet` registry; delete/fix orphaned `supervisor.rs`/`types.rs`; implement `RecoveryManager` |
| CORE-007 | 🟡 Partial | NO | High | Move/build real event-adapter logic; wire live projection polling; add read-side test coverage (~40 planned tasks) |
| CORE-008 | 🟡 Partial | NO | High | Implement or remove `Injectable`; build or delete `TracingInterceptor`; replace tenant no-op stub |
| CORE-009 | 🟢 Completed | YES | High | Refresh stale archived proposal/design docs |
| CORE-009D | 🟢 Completed | YES | High | Wire `RuntimeInner::new()`/`Default` through `RuntimeBuilder` |
| CORE-010A | 🟢 Completed | YES | High | Cosmetic doc cleanup only |
| CORE-012 (logging) | 🔴 Not Implemented | NO | High | No logging framework exists; needs to be built from zero, or the CORE number needs correcting |
| CORE-013 | 🟢 Completed | YES | High | Update PRD.md — CORE-011B/JWKS is done, not "next up" |
| CORE-014 | ⚠️ Needs Refactor | NO | High | Document the `AllowAll` feature-gate decision in spec; refresh stale test counts |
| CORE-015 | 🟢 Completed (macro) / 🔴 (as PRD's "Telemetry SDK") | Split — see above | High | Resolve the CORE-015 naming collision; build telemetry SDK if still wanted |

---

## 🟢 Fully Completed

CORE-009 (Security SDK), CORE-009D (Optional Security Capability), CORE-010A (Remove Ambient ServiceContext), CORE-013 (JWT Authentication Providers), CORE-015-as-authorization-macro. These have closed architecture, real cross-crate consumers, and (where applicable) passing live test runs — not just merged docs.

## 🟡 Partially Completed

- **CORE-007**: scheduler half is genuinely solid; read-side projection half is unwired, misplaced, untested.
- **CORE-008**: registry/macros/DI container/interceptor pipeline work; `Injectable` and `TracingInterceptor` are dead weight; tenant enforcement is a stub.
- **CORE-014**: functionally solid and live-tested, but an undocumented feature-gate constitutes real scope drift from the approved spec.
- **CORE-006** also belongs conceptually here even though it's scored ⚠️: most of the lifecycle works, but its headline safety guarantee doesn't exist in the dispatch path.

## 🔴 Still Pending

CORE-000 (no CI/CD enforcement at all), CORE-012-as-logging (zero implementation), CORE-015-as-Telemetry-SDK (zero implementation, no crate exists).

---

## Cross-Cutting Analysis

- **CORE-012 "Security Context Unification"** is a real, completed, valuable piece of infrastructure — unifying `domain::auth::SecurityContext` and `security-sdk::SecurityContext` into one model — that never got a coherent name in the current roadmap brief because the number was reused for something unrelated. It deserves to be tracked as its own first-class item, separate from whatever "structured logging" ends up being numbered.
- **`ego-scheduler`'s reactive engine** is more mature and more tested (1000+-case property test) than its surrounding roadmap treatment suggests — it's arguably ready to be leaned on more heavily than the still-unwired read-side-projection half it's paired with under CORE-007.
- **The `#[authorize]` macro + `AuthorizationProvider` runtime wiring** (CORE-015) is a fully closed, live-tested loop — real production-grade capability hiding under a mislabeled roadmap slot.

## Architectural Gaps (proposal vs design vs implementation mismatches)

1. **CORE-006**: spec/tasks claim a "final consistency lock" is "STABLE — IMPLEMENTATION-READY"; the actual mechanism (`SharedActivation`, `get_active_sender`) is dead code never wired into the dispatch path.
2. **CORE-007**: openspec places read-side projection logic in `event-adapter`; it actually lives in `domain`/`runtime`, and `event-adapter` is an empty shell.
3. **CORE-014**: proposal/design never mention that `AllowAllAuthorizationProvider` would be feature-gated out of default builds — it is.
4. **CORE-015 / PRD.md**: two different features (declarative-authorization macro vs. Telemetry SDK) share the "CORE-015" label in different documents.
5. **CORE-012**: roadmap brief expects "Structured Logging Framework"; the actual shipped CORE-012 is Security Context Unification — total content mismatch.
6. **PRD.md** generally is stale: it lists CORE-011B/JWKS as "Next Up" when it's already shipped, and CORE-014's `#[authorize]` macro as future work when CORE-015 already delivered it.

## Forgotten Work

- `crates/persistent-entity/src/supervisor.rs` and `types.rs` — orphaned, excluded from the build via a missing `pub mod` (one has a type error, the other is an unused duplicate `EntityTriple`).
- `RecoveryManager` (`persistent-entity/src/recovery.rs`) — both methods hardcoded to return "not yet implemented."
- `crates/runtime` / `crates/runtime-tokio` — message delivery is a no-op in both; `persistent-entity` bypasses them entirely with its own mailbox, meaning two runtime crates in the workspace do essentially nothing.
- `crates/event-adapter` — 4-line empty shell despite openspec designating it as the read-side projection home.
- `TracingInterceptor` — commented-out reference to a file that was never created.
- `Injectable` trait in `service-sdk::di` — defined, never implemented anywhere.
- `enforce_tenant`/`issue_cross_tenant_permit` — no-op stubs called unconditionally on every operation invocation (TASK-014, referenced by both CORE-008 and CORE-009D audits).

---

## Final Recommendation

Ranked by architectural consistency → developer usability → production readiness → minimizing future refactors, based only on current repo state:

1. **Fix CORE-006's single-flight activation** — the most dangerous gap: the system's core safety property (no duplicate concurrent activation of an entity) does not exist in the dispatch path today. Wire `SharedActivation`, replace the `HashSet` registry, delete or fix the orphaned files.
2. **Stand up real CI (CORE-000)** — the `Makefile` commands already exist; wiring them into `.github/workflows/` is a small, high-leverage change that protects every subsequent fix from regressing silently.
3. **Resolve the CORE-012/CORE-015 numbering collisions in the roadmap docs** — cheap to fix, but until it's fixed nobody can safely plan against PRD.md without independently re-verifying, which is exactly what this audit had to do.
4. **Close out CORE-007's read-side projection half** — move/build the logic where the spec says it belongs, wire the polling loop live, and write the ~40 planned tests that don't exist yet.
5. **Decide the fate of `Injectable`, `TracingInterceptor`, and the tenant-enforcement stubs in CORE-008/009D** — either implement or delete; dead scaffolding invites accidental misuse (e.g., someone assuming tenant isolation is real).
6. **Document the CORE-014 `AllowAllAuthorizationProvider` feature-gate decision** — small doc fix, closes real architecture-vs-spec drift.
7. **Only then**, decide whether to actually build a structured logging framework and/or a Telemetry SDK — both are currently zero-code, and pick which one "CORE-012"/"CORE-015" mean going forward before writing more roadmap text that references them.

---

## Next Step

Want me to start on the top recommendation — wiring the CORE-006 activation lock — or something else from the list?
