# Proposal: CORE-018b — Restrict RuntimeInner Construction to RuntimeBuilder

Tracks GitHub issue #118.

## Intent

`RuntimeInner::new()` (`crates/service-sdk/src/runtime/runtime_builder.rs`, line 138)
and `impl Default for RuntimeInner` (~line 251) are both `pub`. Either constructs a
fully-formed `RuntimeInner` directly — `logger: None`, an empty
`Mutex::new(TeardownStack::new())`, arbitrary `security_providers` — bypassing
`RuntimeBuilder` entirely.

Consequences today:

- A caller can hold a "runtime" with no logger even though the host configured one,
  and no registered teardown — silently diverging from real bootstrap behavior.
- The TASK-014 comment (lines 131–137) already flags that a rogue instance with
  custom `security_providers` could bypass authorization once cross-tenant
  enforcement is active.
- CORE-017's design.md documented this exact gap under "Future Considerations" as
  explicitly out of scope. This change closes it.

## What Changes

- Restrict `RuntimeInner::new()` to `pub(crate)`.
- Restrict or remove `impl Default for RuntimeInner` (removal preferred — an empty
  runtime is never a valid value; keep `pub(crate)` only if internal code needs it).
- Migrate all call sites that construct `RuntimeInner` directly to go through
  `RuntimeBuilder::build()`. Known sites (a call-site survey during tasks MUST
  re-verify before implementation):
  - test module in `crates/service-sdk/src/runtime/builder.rs`
  - `crates/service-sdk/tests/authorization_integration.rs`

After this change, `RuntimeBuilder::build()` is the ONLY construction path for
`RuntimeInner`, making the CORE-017 lifecycle guarantees (logger wiring, ordered
teardown) structurally unavoidable rather than conventional.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `service-sdk`: add a requirement that `RuntimeInner` is not publicly constructible;
  the only construction path is `RuntimeBuilder::build()`.

## Non-Goals

- Do NOT implement `.with_adapter()` / `.with_config()` on `RuntimeBuilder` — that is
  issue #120, which depends on this landing first but is a separate change.
- Do NOT touch kit-config wiring or host examples — issue #119 is independent.
- Do NOT add new authorization or tenant-enforcement logic beyond what exists.
- Pure visibility restriction + call-site migration. No behavioral changes to
  correctly-built runtimes.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/service-sdk/src/runtime/runtime_builder.rs` | Modified | `new()` → `pub(crate)`; `Default` removed or `pub(crate)` |
| `crates/service-sdk/src/runtime/builder.rs` (tests) | Modified | Construct via `RuntimeBuilder` |
| `crates/service-sdk/tests/authorization_integration.rs` | Modified | Construct via `RuntimeBuilder` |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Unknown external/direct constructors beyond surveyed sites | Low | Compiler enforces: any missed site fails to build; fix by migrating to `RuntimeBuilder` |
| Tests relied on hand-crafted `RuntimeInner` states unreachable via builder | Med | If a test needs such a state, add a `#[cfg(test)]`/`pub(crate)` helper — never re-widen public visibility |

## Rollback Plan

Revert the commit. Visibility-only change plus test migrations; no data, config, or
API-consumer migration to unwind.

## Dependencies

- None. Blocks issue #120 (builder DI write-side), which should start only after this
  merges.

## Success Criteria

- [ ] `RuntimeInner::new()` and any remaining `Default` impl are not `pub`.
- [ ] Grep finds no `RuntimeInner` construction outside `RuntimeBuilder::build()` and
      crate-internal test helpers.
- [ ] Workspace builds and full test suite passes after call-site migration.
- [ ] Issue #120 can rely on `RuntimeBuilder` as the single construction path.
