```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:working-tree-uncommitted
verdict: pass
blockers: 0
critical_findings: 0
warnings: 1
requirements: 1/1 (PersistentEntity Contracts) — implemented per AD-6, spec wording stale (see Issues)
scenarios: 2/2 (Registering a user; Associating a user with a tenant org, per AD-6 shape)
test_command: cargo test --workspace
test_exit_code: 0
focused_test_command: cargo test -p reference-app --test user_entity --test tenant_org_entity
focused_test_result: 4/4 passed
build_command: cargo build --workspace
build_exit_code: 0
clippy_command: cargo clippy -p reference-app --all-targets
clippy_result: zero warnings attributable to reference-app (all emitted warnings are pre-existing, in persistent-entity/security-jwt)
```

## Verification Report

**Change**: core-018-production-reference-service — PR 2 of 3 (`User` + `TenantOrganization` `PersistentEntity` aggregates, TASK-012..015 / Phases 4-5 only)
**Version**: tasks.md (obs #1213), design.md (obs #1212, AD-5/AD-6), specs/reference-service/spec.md, proposal.md (obs #1210)
**Mode**: Strict TDD

### Completeness
| Metric | Value |
|--------|-------|
| Tasks in PR2 scope (TASK-012..015) | 4 |
| Tasks complete (`[x]`) in tasks.md | 4/4 |
| Phases 1-3 (PR1) | Already verified PASS (verify-report-pr1.md) — not re-verified |
| Phases 6-10 (PR3) | Correctly unchecked in tasks.md — out of PR2 scope, not flagged |

### Build & Tests Execution
**Build**: PASSED — `cargo build --workspace` (exit 0), no errors.
**Tests**: PASSED — `cargo test --workspace` exit 0, no failures anywhere in the tree. Focused re-run: `cargo test -p reference-app --test user_entity --test tenant_org_entity` — 4/4 passed (2 `user_entity` + 2 `tenant_org_entity`), independently re-executed, not just re-reading apply-progress's claim.
**Clippy**: `cargo clippy -p reference-app --all-targets` — zero warnings referencing `reference-app`/`examples/reference-app` (confirmed via `rg` filter on the raw clippy output). The 6 warnings shown (too_many_arguments x2, unnecessary_map_or, manual_inspect) are all in `persistent-entity`/`security-jwt`, pre-existing, out of PR2's scope.
**Coverage**: not available — skipped, not a failure.

### Assertion Quality (non-vacuous check)
Both test files exercise the real `PersistentEntity::handle_command`/`apply_event` contract directly, not a smoke test:
- `user_entity.rs`: asserts exactly 1 event from `Register` on `Unregistered`, then asserts `apply_event` transitions state to `Registered{email, tenant_id}` with the exact expected values (not just `is_ok()`).
- `tenant_org_entity.rs`: asserts 1 event + `Present{name}` transition from `Absent`, **and** a second test constructs a `Present` state directly and asserts `handle_command(Ensure, ..)` on it returns zero events. This is a valid idempotency proof: `PersistentEntity::handle_command` is a pure function of `(command, state)` — asserting its behavior when `state = Present` is equivalent to "calling `Ensure` twice" (the second call would receive exactly this `Present` state). Not vacuous.

### Correctness (Static Evidence)
| Requirement | Status | Notes |
|------------|--------|-------|
| `User` implements `PersistentEntity` (Command/Event/State/handle_command/apply_event) | ✅ Implemented | `domain/user.rs`; matches real `PersistentEntity` trait signature in `crates/persistent-entity/src/persistent_entity.rs:33-73` exactly (`async fn handle_command(...) -> Result<Vec<Self::Event>, EntityError>`, not the `CommandResult` wrapper tasks.md's prose implied — apply-progress's own correction, confirmed here against live trait source) |
| `UserRegistered` implements `DomainEvent` (aggregate_id/event_type/payload/occurred_at) | ✅ Implemented | all 4 methods present, matches `crates/domain/src/event.rs:47-61` exactly |
| `TenantOrganization` implements `PersistentEntity`, idempotent `Ensure` | ✅ Implemented | `domain/tenant_org.rs`; `Present` branch returns `Ok(vec![])`, `Absent` branch returns one `OrganizationEnsured` |
| `OrganizationEnsured` implements `DomainEvent` | ✅ Implemented | all 4 methods present |
| `examples/reference-app/src/lib.rs` gained only `pub mod domain;` | ✅ Confirmed | `git diff` shows a single-line addition, no service/runtime wiring |
| `Cargo.toml` gained direct `chrono`/`serde` deps + `[dev-dependencies]` `tokio` | ✅ Confirmed | `git diff` matches apply-progress's claim exactly |
| No `RegisterUser`/service/HTTP code added (PR3 territory) | ✅ Confirmed | `rg -ni "saga|compensat|outbox|RegisterUser|register_user"` in `examples/reference-app/{src,tests}` → zero matches |
| No further `crates/transport` changes (PR1 territory) | ✅ Confirmed | `git status --porcelain crates/transport` shows only PR1's already-verified files (`Cargo.toml`, `lib.rs` modified; `error.rs`/`security.rs`/`server.rs`/`state.rs`/`tests/` untracked, matching verify-report-pr1.md's file list) — nothing new for PR2 |

### Coherence (Design) — AD-5/AD-6
| Decision | Followed? | Notes |
|----------|-----------|-------|
| AD-6 aggregate shapes (`User`: Register/UserRegistered/Unregistered\|Registered; `TenantOrganization`: Ensure/OrganizationEnsured/Absent\|Present{name}) | ✅ Yes | exact match, verified by direct source read |
| AD-5 "benign reusable orphan" (idempotent ensure is the property that makes non-atomic dual-write safe) | ✅ Yes | `tenant_org_entity.rs`'s idempotency test explicitly proves the zero-event-on-Present property this claim depends on |

### Spec-vs-Design Discrepancy — Independently Adjudicated
**Finding**: `specs/reference-service/spec.md`'s "Associating a user with a tenant org" scenario (lines 21-24) and its contract table (line 12) describe an accumulating membership-set shape: event `UserAssociatedWithTenant`, state "current membership set" / "includes the new member". The actual implementation (and design.md AD-6, tasks.md's own "ground truth reverified" note) uses an idempotent ensure-only shape: `Command::Ensure{org_id,name}` → `Event::OrganizationEnsured` → `State::Absent | Present{name}` — **no membership tracking at all** (confirmed: `tenant_org.rs` never references `user_id`).

**Independent check performed** (not just re-stating the apply agent's flag): searched every artifact that could plausibly depend on `TenantOrganization` enumerating members:
- `proposal.md` (obs #1210) success criteria — only requires guard-chain proof and dual-entity persistence; no membership enumeration requirement anywhere.
- `specs/reference-service/spec.md`'s own "Successful registration" scenario (Happy Path, line 47) — "a `User` entity exists and the `TenantOrganization` reflects the new membership" — satisfied by `TenantOrganization` being `Present`; nothing requires reading back *which* user is a member from `TenantOrganization`'s own state (that relationship is carried by `User.tenant_id`, not duplicated in `TenantOrganization`).
- `specs/http-transport/spec.md` — no route, response field, or scenario references member enumeration; the "Success/Error Response Contract" only maps outcome category to status code.
- `RegisterUser Observability` requirement (spec.md lines 58-65) — only requires a success/failure event per invocation, no membership data.
- design.md's Interfaces/Contracts section — `RegisterOutput`'s shape isn't specified with a member list.

**Conclusion**: this is genuinely stale prose, not a silently dropped requirement. AD-6's rationale is also independently sound, not just decision-backed-by-fiat: a membership-set on `TenantOrganization` would duplicate the User↔Org relationship that `User.tenant_id` already owns, and — more importantly — would weaken, not strengthen, AD-5's non-atomicity story. If `TenantOrganization`'s state were "the set of member user_ids," `Ensure`-style idempotency would have to become "add member X," and the org-first / user-fails-after residue would leave behind a phantom member entry with no corresponding `User` — a worse, harder-to-reconcile leftover than today's "empty, safely-reusable org" claim. The chosen shape is the one that makes the "benign orphan" property actually benign.

**Recommended exact spec.md fix** (for the sdd-verify pass immediately following PR3, or applied now — verify does not edit specs itself):
```diff
- | `TenantOrganization` | associate a user with the org | `UserAssociatedWithTenant` | current membership set |
+ | `TenantOrganization` | ensure the org exists (idempotent) | `OrganizationEnsured` | `Absent` \| `Present{name}` |
```
```diff
- #### Scenario: Associating a user with a tenant org
- - GIVEN an existing `TenantOrganization` entity
- - WHEN the associate command is handled
- - THEN `UserAssociatedWithTenant` is produced and applied, and state includes the new member
+ #### Scenario: Ensuring a tenant org exists
+ - GIVEN a `TenantOrganization` entity in `Absent` state
+ - WHEN the `Ensure` command is handled
+ - THEN `OrganizationEnsured` is produced and applied, and state transitions to `Present{name}`
+ - AND WHEN `Ensure` is handled again on an already-`Present` org, no event is produced (idempotent)
```
Also reword the Happy Path scenario (line 47) from "the `TenantOrganization` reflects the new membership" to "the `TenantOrganization` is `Present`" to stop implying a per-member state.

### TDD Compliance
| Check | Result | Details |
|-------|--------|---------|
| RED confirmed | ✅ | apply-progress documents real `E0433: cannot find domain in reference_app` before `src/domain/*.rs` existed |
| GREEN confirmed | ✅ | 4/4 pass on independent re-run here |
| Triangulation adequate | ✅ | 2 tests per entity: creation-path + idempotency/transition |
| Tasks marked complete match code state | ✅ | TASK-012..015 all `[x]`, code present and correct |

### Issues Found
**CRITICAL**: None.
**WARNING**: `specs/reference-service/spec.md` still describes `TenantOrganization` with a stale `UserAssociatedWithTenant`/"membership set" shape that conflicts with the actually-implemented (and independently-verified-sound) AD-6 idempotent-ensure shape. No dependent requirement anywhere in the spec/proposal/http-transport artifacts relies on membership enumeration — confirmed by direct search, not assumption. Fix recommended above; non-blocking for PR3 apply, but should be reconciled at or before final archive.
**SUGGESTION**: None beyond the spec wording fix above.

### Non-Goal / Scope-Boundary Confirmation
- No `RegisterUser`, service, or HTTP route code added in this PR — confirmed via `rg`.
- No further `crates/transport` changes beyond PR1's already-verified set — confirmed via `git status --porcelain`.
- No saga/compensation/outbox code — confirmed via `rg`.

### Verdict
**PASS** — TASK-012..015 are genuinely implemented, independently rebuilt (`cargo build --workspace`), independently retested (`cargo test --workspace` clean; 4/4 focused entity tests re-run), and clippy-clean for `reference-app`. The spec-vs-design divergence on `TenantOrganization`'s shape was independently investigated (not just accepted from the apply agent's flag) and confirmed to be stale documentation with no dependent requirement — a WARNING, not a CRITICAL. PR2 stayed strictly within its scope (no PR1/PR3 territory touched). **Ready to proceed to PR3 apply** (Phases 6-10: `RegisterUser` guard chain, partial-failure proof, observability, HTTP wiring, e2e).
