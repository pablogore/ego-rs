# Tasks: CORE-012A — Observability Integration (Security Enforcement Path)

Reads `proposal.md`, `specs/service-sdk/spec.md` (5 ADDED requirements), and
`design.md` (final, twice-revised: fieldless `SecurityDenialKind`, `Option<Arc<dyn
Observability>>` default `None` per AD-2, `RecordedDenial` label-only `Display`,
corrected 3-site call-site table). Strict TDD (`cargo test --workspace`): every
GREEN task is preceded by its RED test task.

**Ground truth reverified during breakdown**: `RuntimeInner` struct at
`runtime_builder.rs:120-140`, `authorization_provider()` accessor at 251-264
(pattern to mirror), `new_with_logger` at 167-187. `RuntimeBuilder` struct at
`builder.rs:40-52`, `build()` calling `new_with_logger` at 158-173. Macro
call sites confirmed exact: `ctx.security()` at lib.rs:285, `authorize_in_context`
`map_err` at 312, `enforce_tenant` `map_err` at 357-358 — design.md's line
numbers match current source. `SemanticEvent::new` returns `Result<Self,
SemanticEventError>` (domain/observability.rs:68-88) — the helper's fixed,
non-empty `"security.denial"` literal makes `Err` unreachable; use `.expect()`
naming that invariant, not a silent `unwrap_or`. No `golden_codegen.rs` snapshot
touches guard-error-path shape (it only snapshots `DepKey` `Debug` output) —
design's "snapshot churn" concern applies to compile+run integration tests
(`authorization_integration.rs`, `tenant_scoped_codegen.rs`), not `insta` goldens;
no golden regen task is needed.

---

## Phase 1 — `SecurityDenialKind` + `RecordedDenial` (AD-1, AD-3)

### TASK-001 [RED]: [x] Test the 3 `Display` labels
- File: `crates/service-sdk/src/runtime/runtime_builder.rs` (new test module additions).
- Assert `RecordedDenial(&SecurityDenialKind::MissingContext).to_string() == "MissingContext"`,
  same for `TenantMismatch`, `AuthorizationDenied` — exactly 3 cases, nothing else.
- Satisfies: spec "Recorded Denial Data Is Redacted", scenario "Recorded event omits raw tenant id and denial reason".

### TASK-002 [GREEN]: [x] Add `SecurityDenialKind` (fieldless) + `RecordedDenial` newtype + `Display`
- Same file. `#[derive(Debug, Clone, Copy)] pub enum SecurityDenialKind { MissingContext, TenantMismatch, AuthorizationDenied }`.
  `RecordedDenial<'a>(&'a SecurityDenialKind)` with hand-written `Display` emitting only the label.
- Depends on: TASK-001.

---

## Phase 2 — `RuntimeInner.observability` + `record_security_denial` (AD-1, AD-2)

### TASK-003 [RED]: [x] Test the helper emits one event with the 3 required fields
- File: `crates/service-sdk/src/runtime/runtime_builder.rs` test module.
- Construct a `RuntimeInner` with a test-double `Observability` (see TASK-006), call
  `record_security_denial("Svc", "op", SecurityDenialKind::AuthorizationDenied)` once,
  assert exactly one captured `SemanticEvent` with `metadata["denial_kind"] ==
  "AuthorizationDenied"`, `metadata["service"] == "Svc"`, `metadata["operation"] == "op"`.
- Satisfies: spec "Reachable Macro-Guard Denials Are Recorded" + "Minimum Recorded Event Contract", both scenarios.

### TASK-004 [RED]: [x] Test the no-op path when `observability` is `None`
- Same file. Construct `RuntimeInner` with `observability: None`, call the helper, assert
  no panic and no observable side effect (nothing to assert on since there's no sink —
  the test proves the call is infallible/silent).
- Satisfies: spec "Runtime Accepts an Observability Implementor, Default Behavior Unchanged", scenario "Omitting with_observability preserves today's behavior exactly" (helper half).

### TASK-005 [GREEN]: [x] Add the `observability` field + `record_security_denial` helper
- File: `crates/service-sdk/src/runtime/runtime_builder.rs`.
- `pub(crate) observability: Option<Arc<dyn ego_domain::Observability>>` field on `RuntimeInner`
  (mirrors `security_providers`); add param to `new_with_logger`.
- `#[doc(hidden)] pub fn record_security_denial(&self, service: &'static str, operation: &'static str, kind: SecurityDenialKind)`:
  builds `SemanticEvent::new("security.denial", "", "", "Denied", "", metadata)`
  with `metadata = {"denial_kind": RecordedDenial(&kind).to_string(), "service": service, "operation": operation}`,
  `.expect("event_name is a fixed non-empty literal")`, then `if let Some(obs) = &self.observability { obs.trace(ev) }`.
- Update the `Debug` impl (line ~142-150) if it needs a new field entry (use `finish_non_exhaustive`, already present — no change needed).
- Depends on: TASK-002, TASK-003, TASK-004.

---

## Phase 3 — `RuntimeBuilder::with_observability` wiring (req 4)

### TASK-006 [RED]: [x] Add `RecordingObservability` test double + Noop-default regression test
- File: `crates/service-sdk/src/runtime/builder.rs` test module (dev-only, mirrors existing
  `StubAdapter`/`StubConfig` test fixtures in the same file).
- `struct RecordingObservability { events: Mutex<Vec<SemanticEvent>> }` implementing
  `ego_domain::Observability::trace` by pushing into `events`.
- Test: build a `Runtime` via `.with_observability(Arc::new(RecordingObservability::new()))`,
  invoke a denied guarded operation, assert the double captured the event (build succeeds, wiring reaches the field).
- Test: build a `Runtime` **without** calling `.with_observability(...)`, invoke both an
  allowed and a denied operation, assert identical return values/errors to pre-change behavior, no panic.
- Satisfies: spec "Runtime Accepts an Observability Implementor, Default Behavior Unchanged", both scenarios.

### TASK-007 [GREEN]: [x] `RuntimeBuilder::with_observability(...)` + thread through `build()`
- File: `crates/service-sdk/src/runtime/builder.rs`.
- Add `observability: Option<Arc<dyn ego_domain::Observability>>` field to `RuntimeBuilder`
  (default `None` in `new()`), `pub fn with_observability(mut self, obs: Arc<dyn ego_domain::Observability>) -> Self`
  setting `Some(obs)`, and pass it into the `RuntimeInner::new_with_logger(...)` call in `build()` (~line 168-173).
- Depends on: TASK-005, TASK-006.

---

## Phase 4 — Macro call-site edits (3 sites, design's corrected table)

### TASK-008 [RED]: [x] Guard-denial recording tests in the macro's own integration suites
- Files: `crates/service-sdk/tests/authorization_integration.rs` (extend existing `#[authorize]`-only
  fixture) and `crates/service-sdk/tests/tenant_scoped_codegen.rs` (extend existing `#[tenant_scoped]` fixture).
- Add/extend fixtures to register `.with_observability(Arc::new(RecordingObservability::new()))`
  and assert, per guard:
  - `#[authorize]`, missing `SecurityContext` → exactly one event, `MissingContext` (site: `ctx.security()`, lib.rs:285).
  - `#[authorize]`, provider denies → exactly one event, `AuthorizationDenied` (site: `authorize_in_context` `map_err`, lib.rs:312) — reuse the existing `t19_deny_path_body_does_not_execute`-style fixture, extend its assertions rather than duplicating the fixture.
  - `#[authorize]`, `ProviderError`/`CapabilityNotEnabled` paths (lib.rs:292-300) → assert **no** event recorded (infra failure, excluded per design).
  - `#[tenant_scoped]` alone, tenant mismatch → exactly one event, `TenantMismatch` (site: `enforce_tenant` `map_err`, lib.rs:357-358).
  - `#[tenant_scoped]` alone, unresolvable context → exactly one event, `MissingContext` (same `enforce_tenant` site, different `SecurityError` arm).
- Satisfies: spec "Reachable Macro-Guard Denials Are Recorded", scenario "A single-guard denial records one event".

### TASK-009 [GREEN]: [x] Wire the 3 recording calls into `service-sdk-macros/src/lib.rs`
- File: `crates/service-sdk-macros/src/lib.rs`.
- Site 1 (~285): in the `ctx.security().ok_or_else(...)` closure, before constructing the
  error, call `__rt.record_security_denial(stringify!(#trait_name), stringify!(#method_name), SecurityDenialKind::MissingContext)`
  — requires the `__rt` binding to exist before this closure runs; reorder so `self.runtime.upgrade()`
  (currently at 290, after the security-context check) is available at 285, or reuse a
  pre-upgraded runtime handle. Confirm the reorder does not change the existing error precedence
  (missing-context still fails before provider-unavailable).
- Site 2 (~312): in the `authorize_in_context(...).await.map_err(...)` closure, match on
  `&e`: on `SecurityError::AuthorizationDenied { .. }` call `__rt.record_security_denial(TRAIT, METHOD, SecurityDenialKind::AuthorizationDenied)`, all other arms (`ProviderError`, `CapabilityNotEnabled`) record nothing, then apply the existing `<#err_ty>::from(e)` conversion unchanged.
- Site 3 (~357-358): in the `enforce_tenant(...).map_err(...)` closure, match on `&e`:
  `SecurityError::TenantMismatch { .. } => TenantMismatch`, `SecurityError::MissingContext => MissingContext`,
  `_ => {}` (no other arm reachable here per design's verification), then apply the existing conversion.
- Explicitly do NOT touch the second `MissingContext`-shaped site (`self.runtime.upgrade()`
  dropped-runtime failure at lib.rs:290/351) — leave it uninstrumented per design's exclusion.
- Depends on: TASK-002, TASK-005, TASK-008.

---

## Phase 5 — Double-attribute short-circuit + CrossTenantDenied non-regression (reqs 1, 5)

### TASK-010 [RED]: [x] Test exactly-one-event when both attributes are present
- File: `crates/service-sdk/tests/authorization_integration.rs` or a new
  `crates/service-sdk/tests/security_denial_observability.rs` (new file, if the existing
  suites' fixtures don't cleanly support a dual-attribute method).
- Fixture: one operation guarded by both `#[authorize]` and `#[tenant_scoped]`.
  Case A: authorize denies → assert exactly one event (`AuthorizationDenied`), no tenant event.
  Case B: authorize passes, tenant enforcement denies → assert exactly one event (`TenantMismatch` or `MissingContext`).
  Case C: both pass → assert zero denial events recorded.
- Satisfies: spec "Reachable Macro-Guard Denials Are Recorded", scenarios "A denial with both attributes present still records exactly one event" and "Allowed invocations record no denial event".

### TASK-011 [RED]: [x] Test `CrossTenantDenied` stays uninstrumented
- Same new/extended file.
- Assert that invoking every macro-guarded fixture used above, allowed or denied, never
  produces an event with `denial_kind == "CrossTenantDenied"` (this variant doesn't exist
  in `SecurityDenialKind` at all — the test is really "the enum has exactly 3 variants and
  no call site can ever construct a 4th").
- Satisfies: spec "CrossTenantDenied Remains Uninstrumented By Design", scenario "No reachable path emits a CrossTenantDenied event today".

### TASK-012 [GREEN]: [x] Make TASK-010/011 pass
- No new production code expected — Phase 4's call-site wiring plus the guard
  short-circuit (existing codegen order, lib.rs:384-385: `#authorize_guard` then
  `#enforce_tenant_block`, each `?`-returning) already guarantees this. If a case fails,
  the fix belongs in Phase 4's call sites, not here — this task is the regression gate,
  not new logic.
- Depends on: TASK-009, TASK-010, TASK-011.

---

## Phase 6 — Redaction cross-check (req 3, diagnostic-side — no new code)

### TASK-013 [Verify only]: [x] Confirm `SecurityError::Debug` still carries raw detail
- No file change. Run the existing AD-010 `Debug` test coverage in
  `crates/security-sdk/src/error/mod.rs` and confirm it already asserts raw `tenant_id`/
  `reason` appear in `Debug` for `TenantMismatch`/`AuthorizationDenied` — cited as
  already-satisfied per design, not re-implemented.
- Satisfies: spec "Recorded Denial Data Is Redacted", scenario "Full diagnostic detail remains available via the original error's Debug only".

---

## Phase 7 — Traceability (spec requirement → task mapping for sdd-verify)

| Spec requirement | Scenario | Task(s) |
|---|---|---|
| Reachable Macro-Guard Denials Are Recorded | single-guard denial | TASK-008, TASK-009 |
| Reachable Macro-Guard Denials Are Recorded | both attrs, exactly one event | TASK-010, TASK-012 |
| Reachable Macro-Guard Denials Are Recorded | allowed = no event | TASK-010, TASK-012 |
| Minimum Recorded Event Contract | both scenarios | TASK-003, TASK-005 |
| Recorded Denial Data Is Redacted | recorded form omits raw data | TASK-001, TASK-002 |
| Recorded Denial Data Is Redacted | Debug retains raw detail | TASK-013 |
| Runtime Accepts Observability, Default Unchanged | both scenarios | TASK-004, TASK-006, TASK-007 |
| CrossTenantDenied Remains Uninstrumented | both scenarios | TASK-011, TASK-012 |

---

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | ~260-320 |
| 400-line budget risk | Low |
| Chained PRs recommended | No |
| Suggested split | Single PR |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: pending
400-line budget risk: Low

### Suggested Work Units

| Unit | Goal | Likely PR | Focused test command | Runtime harness | Rollback boundary |
|------|------|-----------|----------------------|-----------------|-------------------|
| 1 | Phases 1-3: types + helper + builder wiring, Noop-default proven | PR 1 (single PR) | `cargo test -p ego-service-sdk runtime_builder` | `RecordingObservability` unit tests (no live process needed) | Revert `runtime_builder.rs`/`builder.rs` hunks — additive, no caller depends on the field yet |
| 2 | Phase 4-6: 3 macro call sites + integration/regression tests | PR 1 (single PR) | `cargo test -p ego-service-sdk --test authorization_integration --test tenant_scoped_codegen` | `authorization_integration.rs`/`tenant_scoped_codegen.rs` real macro-expanded fixtures | Revert `service-sdk-macros/src/lib.rs` 3-site hunk independently of Phase 1-3 (types remain unused but harmless) |

Rationale for Low risk / single PR: ~5 files (`runtime_builder.rs`, `builder.rs`,
`lib.rs`, 2 test files), no new crate dependency, no golden-snapshot regen (verified —
`golden_codegen.rs` only snapshots `DepKey`, unaffected), additive-only per design's
Rollback section. Total estimate stays comfortably under the 400-line budget even
counting new tests.

---

Total: 13 tasks across 7 phases. Sequential except within Phase 1/2/3 (types → helper →
builder, each strictly gates the next); Phase 4 depends on Phases 1-3 combined; Phase 5
depends on Phase 4; Phase 6 is independent (verification-only, no dependency); Phase 7
is a documentation-only mapping task, no code.
