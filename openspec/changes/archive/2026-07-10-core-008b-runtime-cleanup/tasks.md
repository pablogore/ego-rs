# Tasks: CORE-008B — Runtime Cleanup & API Consolidation

No design.md — both scope decisions (delete deprecated accessors now; delete
the orphaned `ExecutionContext` types now) were resolved during proposal
review. Tasks below implement the spec delta
(`specs/service-sdk/spec.md`) directly.

Strict TDD Mode is active (`cargo test --workspace`). This change removes
obsolete APIs rather than adding behavior, so "test-first" here means: the
existing tests are the RED/GREEN gate — migrate/delete tests together with
the code they exercise, never leave a deprecated accessor's caller migrated
without immediately being able to run its test green, and never delete a
method while a test still references it.

Ordering rule: real callers must be migrated off `tenant_id()`/`has_tenant()`
**before** the methods are deleted, or the workspace won't compile
mid-task. Same principle for the orphaned `ExecutionContext` types — verify
zero callers immediately before deleting.

**Important boundary found during task breakdown**: `crates/domain/src/context.rs`
is NOT purely the `ExecutionContext` trait — it also defines the `AggregateId`,
`EntityId`, `TenantId`, `CorrelationId`, `CausationId`, `RequestId`, and
`Metadata` identity types (via the `id_type!` macro) used by
`ExecutionEnvelope` and re-exported from `ego_domain`'s crate root. Those
types are **out of scope** and MUST NOT be deleted. Only the `ExecutionContext`
trait, the `DomainExecutionContext` struct + its impls, and their dedicated
test sections are removed from that file — a partial-file edit, not a file
deletion. `crates/runtime/src/context.rs`, by contrast, contains only
`RuntimeExecutionContext` and can be deleted as a whole file.

---

## Phase 1 — Migrate service-sdk test call sites off deprecated accessors

Per the spec's Accessor Selection Rule: none of these 4 files call
`enforce_tenant()`, so every site is a construction/clone/propagation-stage
read → `tenant_hint()` is the correct replacement everywhere in this phase
(same `Option<&str>` shape as `tenant_id()`, drop-in rename, no behavior
change). These 4 tasks are independent of each other and can run in parallel.

### TASK-001: Migrate `smoke.rs` (4 call sites) [x]
- File: `crates/service-sdk/tests/smoke.rs`
- Lines `189`, `198`, `208`, `209`: `.tenant_id()` → `.tenant_hint()`
- Acceptance: `cargo test -p ego-service-sdk --test smoke` passes; `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk/tests/smoke.rs` returns zero matches.

### TASK-002: Migrate `context_propagation.rs` (2 call sites) [x]
- File: `crates/service-sdk/tests/context_propagation.rs`
- Lines `19`, `30`: `.tenant_id()` → `.tenant_hint()`
- Acceptance: `cargo test -p ego-service-sdk --test context_propagation` passes; `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk/tests/context_propagation.rs` returns zero matches.

### TASK-003: Migrate `context_cross_service.rs` (2 call sites) [x]
- File: `crates/service-sdk/tests/context_cross_service.rs`
- Lines `18`, `32`: `.tenant_id()` → `.tenant_hint()`
- Acceptance: `cargo test -p ego-service-sdk --test context_cross_service` passes; `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk/tests/context_cross_service.rs` returns zero matches.

### TASK-004: Migrate `context_explicit_propagation.rs` (7 call sites — the worst offender) [x]
- File: `crates/service-sdk/tests/context_explicit_propagation.rs`
- Lines `20`, `31`, `34`, `64`, `65`, `69`, `70`: `.tenant_id()` → `.tenant_hint()`
- This file demonstrates propagation of the ingress hint across task/clone boundaries, not enforcement — `tenant_hint()` is correct per the Accessor Selection Rule, not `canonical_tenant()`.
- Acceptance: `cargo test -p ego-service-sdk --test context_explicit_propagation` passes; `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk/tests/context_explicit_propagation.rs` returns zero matches.

---

## Phase 2 — Remove deprecated accessors from `context/mod.rs`

Sequential; depends on Phase 1 (all real callers migrated first). After
Phase 1, the only remaining references to `tenant_id()`/`has_tenant()` are
the 2 legacy-alias tests below plus the method definitions themselves.

### TASK-005: Delete the 2 legacy-alias unit tests [x]
- File: `crates/service-sdk/src/context/mod.rs`
- Delete `tenant_hint_matches_legacy_tenant_id_field` (lines `579-591`) and `tenant_hint_is_none_by_default_matching_legacy` (lines `593-605`) in full, including their preceding explanatory comment block (lines `573-578`) — their subject (the deprecated accessors) disappears in TASK-006.
- Depends on: TASK-001–004 (no real caller may still need these methods when this lands — though these 2 tests were never a real caller, this task must land before or together with TASK-006).
- Acceptance: `cargo test -p ego-service-sdk` still compiles and passes (methods still exist but are now provably unused outside their own definition).

### TASK-006: Delete `ServiceContext::tenant_id()` and `has_tenant()` [x]
- File: `crates/service-sdk/src/context/mod.rs`
- Delete `has_tenant()` (doc comment + `#[deprecated]` attribute + body, lines `305-314`) and `tenant_id()` (doc comment + `#[deprecated]` attribute + body, lines `316-325`) in full.
- Depends on: TASK-005 (must land after, so no test still exercises `#[allow(deprecated)]` code that no longer exists).
- Acceptance: `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/service-sdk` returns zero matches; `cargo build -p ego-service-sdk` and `cargo test -p ego-service-sdk` succeed with **zero** `#[deprecated]` warnings (per spec's "Deprecated accessors do not exist" and "Only the field remains" scenarios).

---

## Phase 3 — Delete orphaned `ExecutionContext` abstractions

TASK-008 and TASK-009 are independent of each other (different crates) and
of Phases 1-2/4; both depend on TASK-007.

### TASK-007: Re-verify zero callers before deleting [x]
- Run `rg "ExecutionContext" crates/ --type rust` (Rust source only, workspace-wide under `crates/`).
- A full-workspace grep already came back clean during proposal review (2026-07-09); this is a re-check immediately before deletion, per the proposal's stated risk mitigation.
- Acceptance: matches are limited to the 4 known files (`crates/domain/src/context.rs`, `crates/domain/src/lib.rs`, `crates/runtime/src/context.rs`, `crates/runtime/src/lib.rs`). Any other match blocks TASK-008/009 until investigated.

### TASK-008: Remove `ExecutionContext`/`DomainExecutionContext` from `crates/domain` [x]
- File: `crates/domain/src/context.rs` — **partial-file edit, not a file deletion** (see boundary note above). Remove:
  - The `ExecutionContext` trait section (comment header + trait, lines `74-91`)
  - The `DomainExecutionContext` struct + its inherent `impl` + `impl ExecutionContext for DomainExecutionContext` + `impl<P> From<ExecutionEnvelope<P>> for DomainExecutionContext` (lines `93-171`)
  - The `#[cfg(test)]` sections `// ExecutionContext trait — concrete test implementation` (`TestContext` struct/impls/tests, lines `350-521`) and `// ExecutionEnvelope → DomainExecutionContext conversion tests` (lines `523-618`)
  - Keep everything else: the `id_type!` macro, `AggregateId`/`EntityId`/`TenantId`/`CorrelationId`/`CausationId`/`RequestId`, `Metadata`, and their identity-type tests (lines `1-349` minus the deleted test section boundaries above — i.e. keep lines `1-349` and the file's closing brace).
- File: `crates/domain/src/lib.rs` — remove `DomainExecutionContext` and `ExecutionContext` from the `pub use context::{...}` list at lines `75-79`; keep every other identifier in that list.
- Depends on: TASK-007.
- Acceptance: `cargo build -p ego-domain` and `cargo test -p ego-domain` succeed; retained identity-type tests (e.g. `test_tenant_id_valid`, `test_aggregate_id_valid`) still pass; `rg "ExecutionContext" crates/domain --type rust` returns zero matches.

### TASK-009: Delete `RuntimeExecutionContext` from `crates/runtime` [x]
- File: `crates/runtime/src/context.rs` — delete the entire file (contains only `RuntimeExecutionContext` and its tests; no shared identity types live here).
- File: `crates/runtime/src/lib.rs` — remove the `pub mod context;` declaration + its doc comment (line `39-40`) and `pub use context::RuntimeExecutionContext;` (line `53`).
- Depends on: TASK-007.
- Acceptance: `cargo build -p ego-runtime` and `cargo test -p ego-runtime` succeed; `rg "ExecutionContext" crates/runtime --type rust` returns zero matches.

---

## Phase 4 — Documentation corrections

Independent of Phases 1-3; can run in parallel with everything above.

### TASK-010: Correct `docs/architecture.md` ambient-propagation claims [x]
- File: `docs/architecture.md`
- Line `89`: `**ego-service-sdk/context/** — ServiceContext (TaskLocal-scoped), tenant isolation` → describe explicit propagation (e.g. "ServiceContext (explicit propagation), tenant isolation") — drop "TaskLocal-scoped".
- Line `118`: `ServiceContext propagates via `tokio::task::TaskLocal` — EntityRef reads context transparently without cross-crate coupling` → rewrite to state the current model: `ServiceContext` is passed explicitly (owned/cloned/parameter-passed) to every call site that needs it — no ambient/TaskLocal read, consistent with the `2026-06-22-remove-ambient-service-context` invariant (INV-001: "There is exactly one mechanism for a component to access a `ServiceContext`: it was given one explicitly").
- Acceptance: `rg "TaskLocal|ambient" docs/architecture.md` returns zero matches describing `ServiceContext` propagation (per spec's "Architecture doc contains no ambient-propagation claim" scenario).

### TASK-011: Reword the living spec's `ExecutionContext` reference [x]
- File: `openspec/specs/service-sdk/spec.md` (the **living** spec — distinct from this change's own delta merge in `specs/service-sdk/spec.md` under this change directory)
- Lines `693-694`, inside FR-008 ("Exactly One Canonical In-Runtime Tenant Representation"): the sentence currently reads `...\`Principal.tenant_id\`, \`ServiceContext.tenant_id\` (the ingress hint), domain \`ExecutionContext\`/\`TenantId\`, and \`ClaimSet::tenant()\` are ingress/legacy carriers only...` — drop "domain `ExecutionContext`/`TenantId`," so the sentence reads `...\`Principal.tenant_id\`, \`ServiceContext.tenant_id\` (the ingress hint), and \`ClaimSet::tenant()\` are ingress/legacy carriers only...` (matches this change's own delta's MODIFIED requirement wording exactly).
- Append a parenthetical note after the requirement paragraph, matching the delta: `(Previously: also listed domain \`ExecutionContext\` among ingress/legacy tenant carriers. That type is deleted by this change and no longer exists.)`
- This is a wording fix only — no FR/requirement semantics change.
- Depends on: TASK-008 (the type must actually be gone before the living spec claims it's gone), but is otherwise a docs-only edit.
- Acceptance: `rg "ExecutionContext" openspec/specs/service-sdk/spec.md` returns zero matches in the requirement body (the historical note mentioning it by name as removed is expected and fine).

---

## Phase 5 — Workspace-wide verification

### TASK-012: Full workspace build, test, and grep sweep [x]
- Run `cargo build --workspace` — zero errors.
- Run `cargo test --workspace` — all tests pass, zero `#[deprecated]` warnings for `tenant_id()`/`has_tenant()`.
- Run `cargo doc --workspace --no-deps` — zero errors/broken doctest references to the deleted APIs (public-API deletions can break doctests or rustdoc intra-doc links even when `cargo build`/`cargo test` stay green).
- Re-run the three acceptance greps workspace-wide:
  - `rg "\.tenant_id\(\)|\.has_tenant\(\)" crates/` → zero matches (only the `pub tenant_id: Option<String>` field and `tenant_hint()` reader survive).
  - `rg "ExecutionContext" crates/ --type rust` → zero Rust-source references (scoped to `crates/`; the historical note TASK-011 adds to `openspec/specs/service-sdk/spec.md` is expected and intentionally outside this sweep).
  - `rg "TaskLocal|ambient" docs/architecture.md` → zero matches describing `ServiceContext` propagation.
- Depends on: all prior tasks (TASK-001–011).
- Acceptance: matches proposal's Success Criteria checklist verbatim.

---

## Review Workload Forecast

Estimated changed lines by phase (deletions dominate — this is a cleanup
change, not new logic):

| Phase | Files touched | Est. lines changed | Nature |
|---|---|---|---|
| 1 (test migration) | 4 files, 15 one-token renames | ~15 | Rename only, zero behavior change |
| 2 (accessor deletion) | 1 file, 2 tests + 2 methods deleted | ~48 | Deletion of dead-after-migration code |
| 3 (orphan type deletion) | 4 files: `domain/context.rs` (partial), `domain/lib.rs`, `runtime/context.rs` (whole file), `runtime/lib.rs` | ~695 | Deletion of zero-caller trait/struct/tests — `runtime/context.rs` alone is ~326 lines, `domain/context.rs`'s removed sections ~366 lines |
| 4 (docs) | 2 files, 2 doc lines + 1 living-spec paragraph reworded | ~10 | Wording only |
| 5 (verification) | 0 merged files | 0 | Read-only checks |
| **Total** | **9 files across 3 crates + 2 docs** | **~770** | ~93% pure deletion of already-orphaned/superseded code |

- **Chained PRs recommended: Yes, with a caveat.** Raw line count (~770) is
  driven almost entirely by Phase 3's dead-code deletion (`runtime/context.rs`
  is a 326-line whole-file removal; `domain/context.rs`'s removed sections
  total ~366 lines) — none of it is new or modified logic, all of it is
  removal of code with zero production callers, verified twice (proposal
  review + TASK-007). Recommend splitting into 2 PRs: **PR1** = Phases 1+2+4
  (~73 lines: test migration, accessor deletion, docs) and **PR2** = Phase 3
  (~695 lines: orphan type deletion). This isolates the small
  behavior-adjacent change (accessor migration) from the large pure-deletion
  change, so a reviewer can skim PR2's diff (delete-only, no logic to trace)
  much faster than the line count implies.
- **400-line budget risk: High by raw line count, Low by review complexity.**
  Total exceeds 400 lines; Phase 3 alone (~695) exceeds it on its own. But
  every line in Phase 3 is a straight deletion of code already proven to have
  zero callers — there is no new logic path for a reviewer to trace, so the
  effective review cost is closer to "confirm the grep sweep is clean" than
  "read 695 lines of diff."
- **Decision needed before apply: Yes** — per the Review Workload Guard,
  surface this to the user before `sdd-apply`: proceed as a single PR with
  `size:exception` (justified by the deletion-only nature), or split as
  recommended above (PR1 small/fast, PR2 large-but-mechanical).
