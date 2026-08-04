# Apply Progress: PROD-012 — End-to-End Idempotent Command Processing

> Scope of this run: **Phase B0 only** (PR 1 of the hybrid chain, D11). All
> other phases (A1–A4, B1–B7, E1, DOC) are untouched and remain `[ ]` in
> `tasks.md`.

## Phase B0: Defensive `UserEntity` Fix — COMPLETE

- [x] B0.1 RED: `examples/reference-app/tests/user_entity.rs` — added
      `register_when_already_registered_is_a_noop`, asserting that given
      `UserState::Registered` already, `handle_command(UserCommand::Register)`
      returns `Ok(vec![])` instead of a second `UserRegistered`. Confirmed
      FAILED before implementation (`left: 1, right: 0`).
- [x] B0.2 GREEN: `examples/reference-app/src/domain/user.rs::UserEntity::handle_command`
      now takes `state: &Self::State` (was `_state`, discarded) and returns
      `Ok(vec![])` immediately when `matches!(state, UserState::Registered { .. })`,
      before the existing non-empty-email validation and `UserRegistered` emission.
- [x] B0.3 Doc-comment added directly above `handle_command` stating this is
      **defence in depth, not a durable idempotency guarantee**. Corrected
      during PR review — see "PR review correction" below; the first version
      of this comment made a factually false claim about process restarts.

### Unplanned but required fix (regression discovered during `cargo test --workspace`)

`examples/reference-app/tests/effects_e2e.rs`'s
`describe_deliver_retry_then_dedup_through_the_real_actor_spawn_path` asserted
that a second `send_command(Register)` against the SAME already-rehydrated
`UserEntity` handle returned `CommandResult::Events` (i.e. relied on the old
double-registration behavior, with dedup only enforced at the effect-delivery
layer). B0.2's fix makes that second call a `CommandResult::NoEvents` no-op at
the entity level. Updated the assertion and surrounding comment to reflect the
new (correct) behavior; the rest of the test (dedup/executor-count assertions)
is unaffected and still passes. This is an in-scope, minimal consequence of
B0.2 — not a new PROD-012 piece.

## TDD Cycle Evidence

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| B0.1/B0.2 | `examples/reference-app/tests/user_entity.rs` | Unit (async, in-crate integration test dir per `skills/testing`) | ✅ 0/0 relevant pre-existing (filter matched none; full `user_entity.rs` file: 3/3 passing baseline) | ✅ Written first, failed with `left: 1, right: 0` | ✅ Passed after `handle_command` state-match added | ➖ Single new scenario (spec names exactly one no-op case); existing 3 tests (unregistered-register, apply_event, empty-email-rejected) act as the triangulating/regression cases and still pass | ➖ None needed — change is a minimal guard clause |

## Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p reference-app --test user_entity` → 4 passed; 0 failed. (Note: the literal command in `tasks.md`'s work-unit table, `cargo test -p reference-app user_entity`, is a substring filter over test *function* names, not file names, and matches 0 of these tests since none contain the literal substring `user_entity`; `--test user_entity` is the correct way to target this file and was used instead.) |
| Runtime harness command/scenario and exact result | `examples/reference-app/tests/effects_e2e.rs` (real actor-spawn E2E path) — `cargo test -p reference-app --test effects_e2e` → 1 passed after updating its second-registration assertion to `CommandResult::NoEvents` to match B0's new correct behavior. |
| Rollback boundary | Revert the `handle_command` guard clause + doc-comment in `examples/reference-app/src/domain/user.rs`, the new test in `tests/user_entity.rs`, and the one assertion/comment change in `tests/effects_e2e.rs`. No schema, no migration, no persisted state — a pure in-process behavior change. |

## Full Verification

- `cargo test -p reference-app --test user_entity`: 4 passed, 0 failed.
- `cargo test --workspace`: all crates `test result: ok` (16 `test result: ok` blocks across unit/integration/doc-tests), 0 failures.
- `cargo fmt --all -- --check`: clean, exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean, exit 0, zero warnings.

## Remaining Tasks (out of scope for this run — untouched)

- [ ] A1.1–A1.4, A2.1–A2.6, A3.1–A3.5, A4.1–A4.5 (Block A — Persistence Foundations)
- [ ] B1.1–B1.10, B2.1–B2.9, B3.1–B3.7, B4.1–B4.8, B5.1–B5.8, B6.1–B6.12, B7.1–B7.11 (Block B remainder)
- [ ] E1.1–E1.2 (Dual-Aggregate Recovery E2E)
- [ ] DOC.1–DOC.3 (Documentation and Rollout)

## Status

3/3 assigned B0 tasks complete (B0.1, B0.2, B0.3). Branch:
`feat/prod-012-b0-user-entity-defensive-check`, targeting `develop` per D11's
hybrid chain strategy (Block A units, including B0, land individually on
`develop`). Next work unit in the chain per the dependency graph: A1
(integration-test infrastructure).

### Orchestrator diff review — one correction applied

The `effects_e2e.rs` adaptation was correct but incomplete. That test is named
`describe_deliver_retry_then_dedup_through_the_real_actor_spawn_path`, and its
original comment stated the second `accept()` **must dedupe at the delivery
runner** — it used the double-registration bug as the trigger that exercised
delivery-runner dedup end to end. After B0.2 the second command no-ops, no second
effect is described, and that dedup path is no longer exercised: the assertion
still passes, but trivially.

The guarantee itself is not lost — delivery-runner dedup is covered by five unit
tests in `crates/runtime/src/effects/runner.rs`
(`happy_path_success_marks_succeeded_and_commits_dedup`,
`dedup_conflict_marks_invalid_effect_terminal`,
`dedup_reserve_transient_failure_retries_then_succeeds`,
`dedup_reserve_permanent_error_is_immediately_terminal_without_retry`,
`dedup_other_succeeded_on_a_fresh_submission_is_marked_succeeded_not_terminal_failed`)
plus `cross_tenant_dedup_never_collides_even_with_identical_type_and_key` in
`crates/runtime/src/effects/store.rs`. What remained was a test name asserting
coverage it no longer had.

Applied: renamed to
`describe_deliver_retry_through_the_real_actor_spawn_path_and_repeat_register_is_a_noop`
and documented in-file where delivery-runner dedup coverage actually lives. Within
B0's blast radius — B0 invalidated the test's meaning, so B0 fixes it.

### PR review correction — a false claim about process restarts

The first version of B0.3's doc-comment (and the commit message and PR body that
repeated it) claimed the state check is "never reached at all" after a process
restart. **That is false**, and review caught it.

`rebuild_state_from_persistence` (`crates/persistent-entity/src/actor.rs:101`)
loads the snapshot plus stored events and folds them through `apply_events`
before `recover_state` (`actor.rs:148`) marks the actor `Active`. A rehydrated
`UserEntity` therefore sees `UserState::Registered` and the check **does** run and
**does** prevent a second event.

The real limit is narrower and conditional: the check can only prevent another
event **once the prior append has actually committed**. What B0 genuinely does not
provide:

- replay of the original response to the caller,
- detection of the same idempotency key arriving with a different payload
  fingerprint,
- coordination across the several aggregates one operation may touch,
- protection against concurrent actors before either has committed its append,
- continuation of an operation that was only partially executed.

Corrected in three places, since all three repeated the same error: the
doc-comment, the commit message, and the PR body. The test doc-comment in
`tests/user_entity.rs` was also reworded, because "in-process state drift"
implied a scope limit that does not exist.

Lesson worth keeping: the claim was plausible, internally consistent, and wrong.
It survived authoring, self-review, and a full green test run, because no test
asserts what a comment says. Only a reader who knew the recovery path caught it.

### Merged

PR #249 merged into `develop` on 2026-08-03 as squash commit **`378a639`**.
Verified after the merge: the guard is present at
`examples/reference-app/src/domain/user.rs:135`, the focused tests pass on
`develop` (5 passed), and `cargo fmt --all -- --check` is clean. Feature branch
deleted locally and on the remote.

Merged with `gh pr merge 249 --squash --admin`, at the user's explicit
instruction. That bypassed `develop`'s `required_approving_review_count: 1`
(possible because `enforce_admins` is `false`). Recorded here because the PR
carries **zero submitted reviews** — the review that produced the doc-comment
correction happened in conversation and was never posted to GitHub, so the
repository history shows an unreviewed merge even though a real review occurred.

Next unit in the chain: A1 (integration-test infrastructure).

### Delivered

- Commit `0293088` (amended to include the rename), 3 files, +75 / -7.
- **PR [#249](https://github.com/pablogore/ego-rs/pull/249)** → `develop`,
  `MERGEABLE`, `REVIEW_REQUIRED`.
- Verified by the orchestrator, not only reported: `cargo fmt --all -- --check`
  clean; `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  exit 0; `cargo test -p reference-app --test user_entity --test effects_e2e` 5
  passed; `cargo test --workspace` all green, zero failures.

### Not in the PR

The `openspec/changes/prod-012-idempotent-command-processing/` folder is entirely
untracked — the whole planning package (proposal, specs, design, decisions, tasks,
this file) has never been committed. Bundling it into a code PR meant to stay tiny
would have added ~2,700 lines of documents. The repo's own convention is a separate
planning-docs PR, per `21082dc docs(prod-005): SDD planning package for Runtime
Health Model`. Still pending.
