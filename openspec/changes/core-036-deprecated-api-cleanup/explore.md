# Exploration — CORE-036: Pre-v0.1 Deprecated API Cleanup

**Change:** `core-036-deprecated-api-cleanup`
**Phase:** explore (investigation + classification — no implementation)
**Status:** ready for proposal

## Why this change exists

`PRD.md:140` (Design Constraints §7) states a hard policy:

> **No shims** — when a public API is removed, it is removed. No deprecated aliases in pre-stable crates.

The workspace currently ships **three live `#[deprecated]` attributes**, all inside pre-stable
(`version = "0.1.0"`) crates — direct, standing violations of that policy. Two supporting reports
flagged a wider set of "suspicious surface." This exploration verifies every flagged item against
the actual source and **classifies each one** so the proposal/spec/tasks can execute a bounded,
zero-ambiguity cleanup rather than another inventory.

## Ground truth: the complete `#[deprecated]` surface

`rg '#\[deprecated' crates/` returns **exactly three** hits — the full deprecated API surface:

| # | Symbol | Location | `since` | External refs |
|---|--------|----------|---------|---------------|
| 1 | `TokioExecutionBackend` | `crates/persistent-entity/src/execution_backend_tokio.rs:27-32` | `0.2.0` | 0 |
| 2 | `SyncTestBackend` | `crates/persistent-entity/src/execution_backend_tokio.rs:66-71` | `0.2.0` | 0 |
| 3 | `ServiceContext::is_cross_tenant_allowed()` | `crates/service-sdk/src/context/mod.rs:339-348` | (no `since`) | tests + docs only |

The `ExecutionBackend` **trait** (`crates/persistent-entity/src/execution_backend.rs:20`) is not
itself `#[deprecated]`, but it is imported under `#[allow(deprecated)]` and its only two
implementors are #1 and #2; the module doc (`execution_backend_tokio.rs:1-12`) states the hot path
uses **no** `ExecutionBackend` implementation. It is dead-with-the-pair.

Both `persistent-entity` and `service-sdk` are `version = "0.1.0"` → pre-stable → the "no deprecated
aliases in pre-stable crates" clause applies with no grandfather exception.

---

## Item-by-item inventory & classification

### Item 1 — `TokioExecutionBackend` → **REMOVE (before v0.1)**

`crates/persistent-entity/src/execution_backend_tokio.rs:21-60`. `#[deprecated(since="0.2.0", note="Use EntityActor directly; block_on has been removed")]`.

- Module doc (`:1-12`): *"The `block_on` path has been removed. Command execution now happens
  directly inside the spawned actor task via `.await`. This module is kept only in case external
  consumers still reference `TokioExecutionBackend`."*
- The `execute()` impl (`:43-59`) is a **stub that always returns `EntityError::Internal("TokioExecutionBackend is deprecated…")`** — it cannot execute anything; it exists only to satisfy the trait.
- **Reference scan** (`rg 'TokioExecutionBackend'`, excluding `openspec/**`): every hit is inside
  `execution_backend_tokio.rs` itself (declaration, self-impl, doc links). **Zero external callers,
  zero tests, zero examples, zero docs.**
- **Replacement:** `EntityActor` awaits handler methods directly (`crate::actor`); there is no
  successor type — the concept was deleted, not renamed.
- **Classification: REMOVE.** A stub that only errors, kept purely "in case," is exactly the shim
  `PRD.md:140` forbids.

### Item 2 — `SyncTestBackend` → **REMOVE (before v0.1)**

`crates/persistent-entity/src/execution_backend_tokio.rs:62-90`. `#[deprecated(since="0.2.0", note="Use EntityActor with InMemory stores directly; block_on has been removed")]`.

- Delegates to `TokioExecutionBackend::execute` (`:88`) — inherits the always-error behavior.
- **Reference scan:** only its own declaration + the doc line at `:9`. **Zero external references.**
- **Replacement:** drive an `EntityActor` with in-memory stores directly in tests.
- **Classification: REMOVE.** Same rationale as Item 1; it is a thin wrapper over an already-dead stub.

### Item 3 — `ExecutionBackend` trait → **REMOVE (before v0.1)**

`crates/persistent-entity/src/execution_backend.rs:20`. Not attributed `#[deprecated]`, but:

- Its **only** implementors are Items 1 & 2 (both removed).
- It is imported into `execution_backend_tokio.rs:16-17` under `#[allow(deprecated)]`.
- Module doc explicitly: *"no `ExecutionBackend` implementation is used on the hot path."*
- **Reference scan** (`rg 'ExecutionBackend'`): all hits are the trait def, the two doomed impls,
  and doc links — **no live consumer** once Items 1 & 2 are gone.
- **Classification: REMOVE.** With both implementors deleted the trait is dead code; the whole file
  `execution_backend.rs` and `execution_backend_tokio.rs` are deleted, and the two `pub mod` lines
  (`crates/persistent-entity/src/lib.rs:40-41`) removed.

### Item 4 — `ServiceContext::is_cross_tenant_allowed()` → **REMOVE + migrate callers**

`crates/service-sdk/src/context/mod.rs:339-348`. `#[deprecated(note="checks 'is any permit
attached', not 'is access allowed to the tenant actually being accessed' … Use
is_cross_tenant_allowed_for(destination) instead (CORE-008A AD-008).")]`.

- The deprecation note documents a **security hazard**, not mere style: gating a real decision on
  this method would let a permit for one destination authorize a *different* one. This is a
  correctness/security foot-gun kept alive as a shim.
- **Replacement already exists and is safe:** `is_cross_tenant_allowed_for(&TenantId)`
  (`context/mod.rs:357-361`) — destination-scoped, closes the permit-reuse hole (CORE-008A AD-008).
- **Complete reference scan** (`rg 'is_cross_tenant_allowed\b'`, excluding `_for`):
  | Ref | Location | Kind | Migration |
  |-----|----------|------|-----------|
  | own def | `context/mod.rs:346` | production def | delete the method + its `#[deprecated]` |
  | unit test | `context/mod.rs:560` (`with_cross_tenant_access_sets_flag`, `#[allow(deprecated)]` at `:551`) | test | `assert!(ctx.is_cross_tenant_allowed_for(&destination))` (`destination` already in scope), drop `#[allow(deprecated)]` |
  | unit test | `context/mod.rs:574` (`clone_preserves_cross_tenant_flag`, `#[allow(deprecated)]` at `:564`) | test | `assert!(cloned.is_cross_tenant_allowed_for(&destination))`, drop `#[allow(deprecated)]` |
  | integration test | `crates/service-sdk/tests/smoke.rs:210` (`test_tenant_isolation`, `#[allow(deprecated)]` at `:203`) | test | `assert!(!a.is_cross_tenant_allowed_for(&TenantId::new("tenant-b").unwrap()))`, drop `#[allow(deprecated)]` |
  | contract test | `crates/service-sdk/tests/cross_tenant_access_contract.rs:7` (`is_cross_tenant_allowed_defaults_to_false`, `#[allow(deprecated)]` at `:4`) | test | assert `!ctx.is_cross_tenant_allowed_for(&dest)` for an arbitrary destination, drop `#[allow(deprecated)]`; rename to `…_for_defaults_to_false` |
  | doc | `COOKBOOK.md:422` | docs | delete the parenthetical *"`is_cross_tenant_allowed()` still exists but is **deprecated**…"* |
- **No production (non-test) caller exists.** Every live reference is a test or a doc line.
- **Classification: REMOVE + migrate.** All four test references migrate mechanically to the
  destination-scoped replacement; the doc line is deleted.

### Item 5 — `#[doc(hidden)] pub` macro hatches → **KEEP (false positive)**

- `RuntimeInner::logger()` (`crates/service-sdk/src/runtime/runtime_builder.rs:403-406`)
- `RuntimeInner::authorization_provider()` (`:504-507`)
- `RuntimeInner::record_security_denial()` (`:530-536`)
- `pub use async_trait` (`crates/service-sdk/src/lib.rs:33-34`)
- `pub use ego_security_sdk as security` (`:37-38`)

Each carries an explicit **"Accessibility contract (macro-visibility)"** doc block: these are `pub`
**solely** so code generated by the separate `ego-service-sdk-macros` proc-macro crate can reach
items that would otherwise be `pub(crate)`. `#[doc(hidden)]` keeps them out of rustdoc; the doc
says application code MUST NOT call them. They are **not deprecated**, carry **no `#[deprecated]`**,
and are a required, intentional part of the codegen contract.

- **Classification: KEEP (false positive).** Not a deprecated surface — deliberate macro-visibility.
  No action beyond recording the justification so future audits don't re-flag them.

### Item 6 — testkit back-compat `log(Severity, &str)` path → **KEEP (false positive)**

- `crates/testkit/src/logger.rs` grounding note (c) (`:55-85`) documents that `KITLogger::log(Severity, &str)` is a **"back-compat"** path that bypasses the JSON formatter.
- `crates/service-sdk/examples/logging_bootstrap.rs:16` documents it as *"back-compat"*.

`KITLogger::log` belongs to the **external `kitlogger` crate** (git dependency), **not** to any
`ego-*` crate. `testkit` merely *tests* both entry points; the example *demonstrates* both. There is
no ego-owned deprecated symbol here to remove — the "back-compat" label describes upstream's API.
Removing or altering it is out of ego's control and out of scope for a pre-v0.1 ego surface cleanup.

- **Classification: KEEP (false positive).** External API, not an ego deprecated surface. Retain the
  testkit coverage and example verbatim; record the justification.

### Item 7 — legacy flat `trace_id` mirror on `ServiceContext` → **KEEP + document**

`crates/service-sdk/src/context/mod.rs:69-83`. The field is **private** and, per its doc block and
PROD-003 ADR-4, is **authoritative-by-construction** under `TraceContext`: `with_trace_context`
sets it from `trace_context().trace_id()`, and `with_trace_id` writes the legacy value **only when
no `TraceContext` is present**. It carries **no `#[deprecated]` attribute** — it is a *legacy
compatibility mirror*, deliberately retained, kept in sync by construction, and covered by PROD-003.

`with_trace_id` / `trace_id` are still referenced by live tests and examples (`smoke.rs`,
`context_propagation.rs`, `context_explicit_propagation.rs`, `security_integration.rs`,
`examples/order_service.rs`, `crates/transport/src/propagation.rs`, `crates/domain/src/tracer.rs`,
`crates/infrastructure/*`) — an active, supported source-compat surface, not a shim awaiting removal.

- **Classification: KEEP + document.** Not deprecated; removing it is a PROD-003 concern, not a
  cleanup target. Record the retention justification so it is not re-flagged as a "no-shims" breach.

---

## Decision matrix (summary)

| # | Item | Owning capability | Decision | Replacement / Justification |
|---|------|-------------------|----------|------------------------------|
| 1 | `TokioExecutionBackend` | persistent-entity | **REMOVE** | `EntityActor` awaits handlers directly (no successor type) |
| 2 | `SyncTestBackend` | persistent-entity | **REMOVE** | `EntityActor` + in-memory stores in tests |
| 3 | `ExecutionBackend` trait | persistent-entity | **REMOVE** | Dead once #1/#2 gone; hot path uses no impl |
| 4 | `ServiceContext::is_cross_tenant_allowed()` | service-sdk | **REMOVE + migrate** | `is_cross_tenant_allowed_for(&TenantId)` (destination-scoped, CORE-008A AD-008) |
| 5 | `#[doc(hidden)] pub` macro hatches | service-sdk | **KEEP (false positive)** | Intentional macro-visibility for `ego-service-sdk-macros`; not deprecated |
| 6 | testkit `log(Severity,&str)` back-compat | testkit | **KEEP (false positive)** | External `kitlogger` API, not ego-owned; testkit only covers it |
| 7 | legacy flat `trace_id` mirror | service-sdk | **KEEP + document** | Not deprecated; authoritative-by-construction under `TraceContext` (PROD-003 ADR-4) |

**Net effect:** 4 removals (Items 1-4), 3 documented retentions (Items 5-7). After removal, the
workspace-wide `#[deprecated]` count is **0**, satisfying `PRD.md:140` with a verifiable check.

## Verification mechanisms discovered (for design/tasks)

- **Compilation gate (strongest):** removed symbols referenced anywhere fail `cargo build --workspace`
  / `cargo test --workspace` — a dangling reference cannot compile. This is the primary zero-reference
  proof; a grep gate corroborates it.
- **Grep gates (observable, scriptable):** after removal —
  - `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/` → **0**
  - `rg 'is_cross_tenant_allowed\b' crates/ COOKBOOK.md` (excluding `_for`) → **0**
  - `rg '#\[deprecated' crates/` → **0** (the no-shims policy gate)
  - `rg '#\[allow\(deprecated\)\]' crates/` → **0** (no lingering suppressors)
- **Source-scan lint-test precedent:** `crates/service-sdk/tests/tenant_scoped_lint.rs`,
  `otlp_boundary_lint.rs`, `crates/runtime/tests/transport_agnostic_lint.rs` — a `cargo test`
  participant that ascends from `CARGO_MANIFEST_DIR` to the `[workspace]` root and scans sources.
  This is the model for a `no_deprecated_shims_lint` test that asserts `#[deprecated]` count == 0 in
  pre-stable crates, making the `PRD.md:140` policy enforceable inside the standard test gate rather
  than an unenforced shell script.

## Constraints / notes

- Both target crates are `0.1.0` (pre-stable) — the no-shims clause applies with no exception.
- No dedicated open issue tracks this debt.
- Removals are **public-surface changes** but affect **zero external callers** (all references are
  in-repo tests/docs), so migration is fully contained in this workspace.
- Item 3's trait removal must delete two files and two `pub mod` lines; Items 1/2 live in the same
  file as Item 3's implementors, so the persistent-entity removal is a coherent single deletion.
