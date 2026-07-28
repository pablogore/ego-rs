# Design: CORE-036 — Pre-v0.1 Deprecated API Cleanup

## Technical Approach

Delete four deprecated symbols (three in `persistent-entity`, one in `service-sdk`), migrate the
four in-repo test call sites of the one method that has callers, delete the orphaned
`#[allow(deprecated)]` suppressors and the one deprecated-mention doc line, and add a single
source-scan test that makes the `PRD.md:140` "no shims in pre-stable crates" policy a
`cargo test --workspace` participant. The compiler is the authoritative zero-reference proof — a
dangling reference cannot build — and a grep gate plus the lint test corroborate and lock it. Items
5-7 are retained unchanged with justifications recorded in the specs so future audits do not
re-flag them.

## Architecture Decisions

### ADR-1 (DECISION 1): Per-item remove/keep classification → **4 remove, 3 keep**

**Choice:** classify each flagged item explicitly rather than treating the inventory as uniformly
"deprecated surface." Verified against source in `explore.md`.

| # | Item | Decision | Why |
|---|------|----------|-----|
| 1 | `TokioExecutionBackend` | REMOVE | `#[deprecated]`; stub that only errors; **0** external refs; hot path uses no impl |
| 2 | `SyncTestBackend` | REMOVE | `#[deprecated]`; delegates to #1; **0** external refs |
| 3 | `ExecutionBackend` trait | REMOVE | only implementors are #1/#2; dead once they go; doc says hot path uses none |
| 4 | `is_cross_tenant_allowed()` | REMOVE + migrate | `#[deprecated]`; documents a security foot-gun; safe `_for` replacement exists; refs are tests/docs only |
| 5 | macro-visibility hatches | KEEP | not `#[deprecated]`; required `pub` for `ego-service-sdk-macros` codegen; `#[doc(hidden)]` |
| 6 | testkit `log(Severity,&str)` | KEEP | external `kitlogger` API, not ego-owned; testkit only *covers* it |
| 7 | legacy `trace_id` mirror | KEEP + document | not `#[deprecated]`; authoritative-by-construction under `TraceContext` (PROD-003 ADR-4) |

**Rejected:** (a) *keep* `is_cross_tenant_allowed()` with a longer deprecation window — rejected:
pre-stable crate, zero external callers, and it is an active security foot-gun, so the no-shims
policy applies fully. (b) *remove* the macro hatches or the `trace_id` mirror to "clean up all
`pub` surface" — rejected: neither is deprecated; the hatches are a load-bearing codegen contract
and the mirror is a PROD-003-owned compat field. Over-removing would break the build (hatches) or
reopen a settled PROD-003 decision (mirror).

**Rationale:** the policy target is *deprecated shims*, precisely the three `#[deprecated]` symbols
(+ the `ExecutionBackend` trait they strand). Items 5-7 superficially resemble "extra surface" but
none is deprecated; conflating them would cause incorrect removals.

### ADR-2 (DECISION 2): `is_cross_tenant_allowed()` — remove vs. keep

**Choice:** REMOVE the method and migrate all four in-repo test references to
`is_cross_tenant_allowed_for(&TenantId)`.
**Rejected:** retain as a deprecated alias.

| Option | Tradeoff | Verdict |
|--------|----------|---------|
| Remove + migrate | Four mechanical test edits; deletes a security foot-gun; satisfies `PRD.md:140` | **Chosen** |
| Keep deprecated | Leaves a method whose own doc warns it can mis-authorize cross-tenant access; violates no-shims | Rejected |

**Migration equivalence:** the removed method is `self.allow_cross_tenant.is_some()`; the
replacement is `is_some_and(|g| g.destination() == destination)`. For the migrated tests the truth
value is preserved: the two "no permit" tests (`smoke.rs`, `cross_tenant_access_contract.rs`) assert
`false` and `_for(&any_dest)` is also `false`; the two "permit for `tenant-b`" tests
(`context/mod.rs`) assert `true` and `_for(&destination)` — with `destination = tenant-b` already in
scope — is also `true`. No test loses coverage; each in fact gains the stronger destination-scoped
assertion.

### ADR-3 (DECISION 3): Spec placement — MODIFIED deltas + one NEW capability

**Choice:** capability-specific removal/retention requirements go into **MODIFIED deltas** on the
owning capabilities (`specs/persistent-entity/spec.md`, `specs/service-sdk/spec.md`); the
cross-cutting "no shims / zero-reference verification" policy goes into a **NEW** thin capability
`specs/api-surface-hygiene/spec.md`.

**Rejected:** (a) put everything in one new capability — rejected: the `ExecutionBackend` removal and
the `is_cross_tenant_allowed` removal are facts *about* those capabilities' surfaces and belong with
them, not floating in a hygiene capability. (b) put the policy inside `service-sdk` — rejected: the
no-shims/`#[deprecated]`-count-zero rule spans *every* pre-stable crate (persistent-entity too), so
scoping it to one capability would misrepresent it.

**Rationale:** cleanest ownership — each removal requirement lives where the removed symbol lived;
the genuinely cross-cutting policy (which no existing capability owns) becomes its own small,
independently-testable capability. This is the placement chosen; justification recorded here per the
task's instruction to justify the spec-shape choice.

### ADR-4 (DECISION 4): Zero-reference verification mechanism

**Choice:** three layered, observable checks — strongest first:

1. **Compilation (authoritative):** `cargo build --workspace` + `cargo test --workspace`. A removed
   symbol referenced anywhere fails to compile. This is the primary proof of "references reach zero."
2. **Grep gates (scriptable, human-auditable):** after removal, each of these MUST return zero —
   - `rg 'TokioExecutionBackend|SyncTestBackend|ExecutionBackend|execution_backend' crates/`
   - `rg 'is_cross_tenant_allowed\b' crates/ COOKBOOK.md` (excluding `_for`)
   - `rg '#\[deprecated' crates/`  (the no-shims policy gate)
   - `rg '#\[allow\(deprecated\)\]' crates/`  (no lingering suppressors)
3. **Source-scan lint test (`no_deprecated_shims_lint`):** a `cargo test --workspace` participant
   modeled on `crates/service-sdk/tests/tenant_scoped_lint.rs` — ascends from `CARGO_MANIFEST_DIR`
   to the `[workspace]` root, scans `crates/*/src/**`, and asserts **zero** `#[deprecated]`
   attributes in pre-stable crates. This turns `PRD.md:140` from prose into an enforced gate; a
   future re-introduced shim fails the standard test run.

**Rejected:** a standalone shell script in CI — rejected: not enforced by the project's actual gate
(`cargo test --workspace`), can drift, and this repo already prefers `cargo test` lint participants
(`tenant_scoped_lint.rs`, `otlp_boundary_lint.rs`, `transport_agnostic_lint.rs`).

**Rationale:** the compiler already guarantees zero references for removals; the grep + lint layers
make the guarantee *observable and durable* so the policy cannot silently regress.

## Data Flow

    Deprecated surface (3 × #[deprecated] + stranded trait)
        │
        ├─ persistent-entity: delete execution_backend.rs + execution_backend_tokio.rs
        │                     drop pub mod lines (lib.rs:40-41)
        │
        ├─ service-sdk: delete is_cross_tenant_allowed() (context/mod.rs:339-348)
        │               migrate 4 test refs → is_cross_tenant_allowed_for(&dest)
        │               drop #[allow(deprecated)] × (2 unit + smoke + contract)
        │               delete COOKBOOK.md:422 deprecated mention
        │
        └─ add no_deprecated_shims_lint (cargo test participant)
        ▼
    Verification: cargo build/test --workspace (compile proof)
                + grep gates == 0  + no_deprecated_shims_lint green
        ▼
    #[deprecated] count in pre-stable crates == 0  (PRD.md:140 satisfied)

## File Changes

*(All production/test edits below are FUTURE apply-phase work — this design does not perform them.)*

| File | Action | Description |
|------|--------|-------------|
| `crates/persistent-entity/src/execution_backend.rs` | Delete | `ExecutionBackend` trait removed |
| `crates/persistent-entity/src/execution_backend_tokio.rs` | Delete | `TokioExecutionBackend` + `SyncTestBackend` removed |
| `crates/persistent-entity/src/lib.rs` | Modify | Remove `pub mod execution_backend;` + `pub mod execution_backend_tokio;` (`:40-41`) |
| `crates/service-sdk/src/context/mod.rs` | Modify | Delete `is_cross_tenant_allowed()` (`:339-348`); migrate 2 unit tests (`:551-575`) to `_for`; drop their `#[allow(deprecated)]` |
| `crates/service-sdk/tests/smoke.rs` | Modify | `:210` → `_for(&TenantId::new("tenant-b").unwrap())`; drop `#[allow(deprecated)]` (`:203`) |
| `crates/service-sdk/tests/cross_tenant_access_contract.rs` | Modify | `:7` → `_for(&dest)`; drop `#[allow(deprecated)]` (`:4`); rename test to `..._for_defaults_to_false` |
| `COOKBOOK.md` | Modify | Delete the `is_cross_tenant_allowed()` deprecated parenthetical (`:422`) |
| `crates/service-sdk/tests/no_deprecated_shims_lint.rs` | Create | Source-scan: assert `#[deprecated]` count == 0 in pre-stable crates |

## Interfaces / Contracts

```rust
// REMOVED (persistent-entity) — no successor; hot path awaits handlers directly:
//   pub trait ExecutionBackend { fn execute<C,E,S>(..) -> Result<(Vec<E>,S), EntityError>; }
//   pub struct TokioExecutionBackend;   // stub that only errored
//   pub struct SyncTestBackend;         // delegated to the stub

// REMOVED (service-sdk):
//   #[deprecated] pub fn is_cross_tenant_allowed(&self) -> bool  // permit-presence only

// RETAINED replacement (service-sdk) — unchanged, the migration target:
impl ServiceContext {
    pub fn is_cross_tenant_allowed_for(&self, destination: &TenantId) -> bool; // destination-scoped (CORE-008A AD-008)
}

// RETAINED (service-sdk) — Items 5 & 7, unchanged, justification recorded:
//   #[doc(hidden)] pub fn logger(&self) / authorization_provider(&self) / record_security_denial(..)  // macro-visibility
//   #[doc(hidden)] pub use async_trait;  pub use ego_security_sdk as security;                        // codegen re-exports
//   fn trace_id (private, authoritative-by-construction under TraceContext, PROD-003 ADR-4)

// NEW verification (service-sdk tests) — no production symbol:
//   no_deprecated_shims_lint: asserts count of #[deprecated] in pre-stable crate sources == 0
```

## Error Model

No new error paths. The removed `TokioExecutionBackend::execute` only ever returned
`EntityError::Internal("… deprecated …")`; deleting it removes an always-erroring dead path, not a
live one. No caller relied on that error.

## Observability

None. No logging, metrics, or tracing surface is added or removed. (The macro-visibility
`record_security_denial` hatch — Item 5 — is retained unchanged.)

## Testing Strategy

| Layer | What | Approach |
|-------|------|----------|
| Compile | Zero dangling references to removed symbols | `cargo build --workspace` + `cargo test --workspace` must be green |
| Unit | Migrated `context/mod.rs` tests assert `is_cross_tenant_allowed_for(&destination)` | Edit in place; `destination` already in scope |
| Integration | `smoke.rs` + `cross_tenant_access_contract.rs` migrated to `_for` | Edit in place; drop `#[allow(deprecated)]` |
| Lint (source-scan) | `#[deprecated]` count == 0 in pre-stable crates | New `no_deprecated_shims_lint.rs`, model on `tenant_scoped_lint.rs`; workspace-root anchored via `CARGO_MANIFEST_DIR` |
| Gate (grep) | All four zero-reference greps return 0 | Explicit `rg` commands embedded in tasks |
| Gate (workspace) | fmt / clippy / test / build | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace` |

## Threat Matrix

N/A — no routing, shell, subprocess, VCS/PR automation, or executable-file classification is added.
CORE-036 *reduces* security exposure by deleting `is_cross_tenant_allowed()`, a method whose own
`#[deprecated]` note documents a cross-tenant mis-authorization foot-gun; the migration moves every
call site to the destination-scoped, foot-gun-free `is_cross_tenant_allowed_for`.

## Migration / Rollout / Compatibility

Pre-stable (`0.1.0`) crates with **zero external callers** for every removed symbol — no deprecation
window, alias, or re-export is created (`PRD.md:140`). Compatibility impact is limited to in-repo
tests/docs, all migrated within this change. No schema, data, or runtime-behavior change. Rollback =
`git revert`. Items 5-7 are byte-unchanged, so no compat surface shifts for macro codegen, testkit
consumers, or `TraceContext` users.

## Open Questions

None blocking. All four removals have zero external callers (verified in `explore.md`); all three
retentions have recorded justifications; the migration target (`is_cross_tenant_allowed_for`) and
the enforcement mechanism (`no_deprecated_shims_lint`) already have working precedents in-repo.
