# Proposal: CORE-008B — Runtime Cleanup & API Consolidation

**Origin:** Residual-debt audit (2026-07-09) following the archive of CORE-008A (Tenant Enforcement). All findings re-verified against the current workspace on this date.

## Intent

CORE-008A migrated tenant context to `tenant_hint()`/`canonical_tenant()` but left the workspace describing and exercising two competing models. This closes that gap before Hardening v1.0:

1. **15 Rust call sites of deprecated `ServiceContext::tenant_id()`/`has_tenant()`**, plus documentation examples — in 4 service-sdk integration test files, 2 call sites inside `context/mod.rs`'s own test module that exist solely to prove the deprecated accessors alias the hint correctly, and one `COOKBOOK.md` example (found during review, not part of the original 15/2 count). Decided during proposal review: the migration window closes now — `tenant_id()`/`has_tenant()` are deleted in this change, not just silenced. Worst offender: `context_explicit_propagation.rs` — the showcase for the NEW explicit-propagation model — has 7 call sites.
2. **Stale architecture doc**: `docs/architecture.md:89` ("ServiceContext (TaskLocal-scoped)") and `:118` ("ServiceContext propagates via `tokio::task::TaskLocal`") describe the ambient model removed by the archived change `2026-06-22-remove-ambient-service-context`.
3. **Orphaned pre-migration abstraction**: `ExecutionContext` trait + `DomainExecutionContext` (`crates/domain/src/context.rs`) and `RuntimeExecutionContext` (`crates/runtime/src/context.rs`, re-exported at `lib.rs:53`) — implemented, tested, publicly exported, zero production callers (verified: referenced only in their own files and `lib.rs` re-exports). The runtime uses `CommandContext` (`crates/persistent-entity/src/command_context.rs`) instead.

## Scope

### In Scope
- Migrate the 4 test files (`smoke.rs`, `context_propagation.rs`, `context_cross_service.rs`, `context_explicit_propagation.rs`) off deprecated accessors, per the Accessor Selection Rule below.
- Delete `ServiceContext::tenant_id()` and `has_tenant()` (the deprecated methods) once no caller references them.
- Remove the two `#[allow(deprecated)]` legacy-alias unit tests in `context/mod.rs` (`tenant_hint_matches_legacy_tenant_id_field`, `tenant_hint_is_none_by_default_matching_legacy`) — their subject no longer exists once the methods are deleted.
- Correct `docs/architecture.md:89,118` to describe explicit propagation.
- Resolve the orphaned `ExecutionContext` types — delete `ExecutionContext`/`DomainExecutionContext` (`crates/domain/src/context.rs`) and `RuntimeExecutionContext` (`crates/runtime/src/context.rs`, plus its `lib.rs` re-exports). Decided during proposal review (no longer an open design-phase question): zero production callers, superseded by `CommandContext`.

### Out of Scope
- New runtime features.
- `ego-scheduler` `DropOldest`→`DropNewest` fallback (issue #79, unrelated to this migration).
- Clustering, streaming, OAuth2.
- The underlying `pub tenant_id: Option<String>` field on `ServiceContext` — kept public as a non-authoritative hint per AD-011 (CORE-008A design.md); this change removes the deprecated *accessor methods* only, not the field itself.

## Accessor Selection Rule (normative)

To prevent this question from recurring, tests must select the tenant accessor by pipeline stage, not by habit:

| What the test exercises | Correct accessor |
|---|---|
| Context construction | `tenant_hint()` |
| Clone before runtime enforcement | `tenant_hint()` |
| Explicit propagation (task spawn, parameter passing) | `tenant_hint()` |
| Runtime / `TenantResolver` | `canonical_tenant()` |
| Authorization | `canonical_tenant()` |
| Enforcement (`enforce_tenant`, `#[tenant_scoped]`) | `canonical_tenant()` |

Reason: `canonical_tenant()` reads `resolved_tenant`, which is `pub(crate)` and set only by `enforce_tenant()` via `set_resolved_tenant()`. A `ServiceContext` built directly via `with_tenant_id()` never runs `enforce_tenant()`, so `canonical_tenant()` is `None` there — not a bug, just an earlier pipeline stage with no Established Fact yet. `context_explicit_propagation.rs` demonstrates propagation of the ingress hint across task boundaries, not enforcement, so it uses `tenant_hint()`.

## Capabilities

**Behavior**: No runtime behavior changes. This proposal only removes obsolete APIs, documentation drift, and unused abstractions.

One living-spec correction is required as a direct consequence: `openspec/specs/service-sdk/spec.md:694` names domain `ExecutionContext` by identifier in the tenant-authority-precedence explanation. Once the type is deleted, that line must be reworded to drop the now-nonexistent reference — a wording fix, not a requirement or FR change.

## Decisions (resolved during proposal review — not deferred to design)

- **Deprecated accessors**: `tenant_id()`/`has_tenant()` are deleted in this change, per the Accessor Selection Rule above governing what replaces each call site.
- **Orphan types**: `ExecutionContext`/`DomainExecutionContext`/`RuntimeExecutionContext` are deleted, not documented as an extension point. Zero production callers (grep-verified), superseded by `CommandContext`; git history preserves them if ever needed.

Design phase is not required to re-litigate either decision; it may proceed straight to spec/tasks unless it finds new evidence.

## Approach

Mechanical: migrate each `ctx.tenant_id()`/`has_tenant()` call site per the Accessor Selection Rule, delete the two now-defined methods and their two dedicated legacy-alias tests, rewrite the two doc lines, delete the three orphaned context files' content and their `lib.rs` re-exports.

## Blast Radius

| Area | Impact |
|---|---|
| `crates/service-sdk/tests/{smoke,context_propagation,context_cross_service,context_explicit_propagation}.rs` | Modified — accessor migration to `tenant_hint()` |
| `crates/service-sdk/src/context/mod.rs` | Modified — delete `tenant_id()`/`has_tenant()` and their 2 legacy-alias tests |
| `docs/architecture.md` | Modified — 2 stale lines |
| `crates/domain/src/context.rs`, `crates/runtime/src/context.rs`, `crates/{domain,runtime}/src/lib.rs` | Modified — delete `ExecutionContext`/`DomainExecutionContext`/`RuntimeExecutionContext` and their re-exports |
| `openspec/specs/service-sdk/spec.md:694` | Modified — reword the tenant-authority-precedence line to drop the `ExecutionContext` reference (wording only, no FR/requirement change) |

Verified clean, no changes needed: `examples/`, `benches/`, `docs/` (none reference these types), and no remaining code/comment references anywhere in the workspace outside the files listed above (full-workspace grep, zero hits).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Accessor choice deviates from pipeline stage, defeating a test's intent | Low | Accessor Selection Rule above is now explicit and normative; verified against the 4 files' actual constructors (none call `enforce_tenant`) |
| Orphan types have an undiscovered external consumer | Low | Workspace-only project; grep-verified zero callers; git-revertible |
| A caller of `tenant_id()`/`has_tenant()` outside the files enumerated here surfaces during implementation | Low | Full-workspace grep already run (`rg "\.tenant_id\(\)|\.has_tenant\(\)"`); apply phase re-runs it before deleting the methods |

## Rollback Plan

Trivial: all changes are docs/test/dead-code edits, revert cleanly via `git revert`. No data, wire, or persistence impact.

## Success Criteria

- [ ] `cargo build --workspace` and `cargo test --workspace` pass with `ServiceContext::tenant_id()`/`has_tenant()` fully removed (no remaining callers, no `#[allow(deprecated)]` left to silence).
- [ ] `docs/architecture.md` contains no TaskLocal/ambient-propagation claims.
- [ ] `ExecutionContext`/`DomainExecutionContext`/`RuntimeExecutionContext` and their re-exports are deleted; workspace still builds green.
