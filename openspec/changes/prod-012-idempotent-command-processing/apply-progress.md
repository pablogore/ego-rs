# Apply Progress: PROD-012 — End-to-End Idempotent Command Processing

> Scope of the B0 run: **Phase B0 only** (PR 1 of the hybrid chain, D11).
> Scope of the A1 run below: **Phase A1 only** (PR 2). All other phases
> (A2–A4, B1–B7, E1, DOC) are untouched and remain `[ ]` in `tasks.md`.

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

- [ ] A2.1–A2.6, A3.1–A3.5, A4 (Block A remainder — Persistence Foundations)
- [ ] B1.1–B1.10, B2.1–B2.9, B3.1–B3.7, B4.1–B4.8, B5.1–B5.8, B6.1–B6.12, B7.1–B7.11 (Block B remainder)
- [ ] E1.1–E1.2 (Dual-Aggregate Recovery E2E)
- [ ] DOC.1–DOC.3 (Documentation and Rollout)

> A1.1–A1.4 completed below; see "Phase A1: Integration-Test Infrastructure".

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

**Update (A1 run, below): resolved.** The whole planning package, including
this file, was committed separately in `56d2c97 docs(prod-012): SDD planning
package for end-to-end idempotent command processing (#250)`, landing after
B0's code PR merged. `tasks.md` and this file are tracked on `develop` as of
the A1 run and are edited directly rather than staying untracked.

---

## Phase A1: Integration-Test Infrastructure — COMPLETE

Branch: `feat/prod-012-a1-integration-test-infrastructure`, off `develop` at
`d8d853b`, targeting `develop` per D11's hybrid chain strategy (PR 2 of the
chain, after B0).

- [x] A1.1 RED: wrote `crates/integration-tests/tests/event_store_characterization.rs`
      against a package that did not exist yet. Confirmed FAILED before any
      production/infrastructure change: `cargo test -p ego-integration-tests
      --test event_store_characterization` → `error: package ID specification
      'ego-integration-tests' did not match any packages`. That is the genuine
      RED here — no test infrastructure crate existed at all, not merely a
      missing implementation inside an existing one.
- [x] A1.2 GREEN: created `crates/integration-tests/Cargo.toml` (`testcontainers
      = "0.23"`, `testcontainers-modules = "0.11"` with the `postgres` feature,
      `sqlx = "0.8"`, plus `ego-domain`/`ego-persistence` — all as
      dev-dependencies, since this crate ships no library or binary of its
      own) and added `crates/integration-tests` to `[workspace] members` in
      the root `Cargo.toml`. Made A1.1 pass (see TDD Cycle Evidence below).
- [x] A1.3 GREEN: added `"ego-integration-tests" = "tooling"` to `layers.toml`
      (the actual layer map `xtask/src/layers.rs` reads and enforces). Layer
      choice: `tooling`, the same layer as `ego-testkit` and
      `ego-service-sdk-macros` — a sink crate nothing in the workspace depends
      on, exempt from the direction matrix (`allowed_layers("tooling")`
      returns `None`, meaning no restriction is enforced on what it may
      depend on). `cargo run -p xtask -- verify-layers` now reports **17**
      crates (was 16), 0 violations.
- [x] A1.4 Added a "Requirements" section to `README.md` declaring PostgreSQL
      14 as the minimum supported version. The integration-test harness pins
      the exact container tag as a named constant,
      `const POSTGRES_IMAGE_TAG: &str = "14-alpine"` — never a floating
      `latest`, and never left as an unlabeled literal at the call site.

### Unplanned but required fixes (discovered running A1.1 against real Postgres)

Two pre-existing gaps surfaced the first time `ego_persistence::postgres`
code was ever exercised against a real database. Neither is a new capability
from the design (A2/A3/A4/B-anything) — both are bugs in code that already
shipped, uncovered only because no integration-test crate existed until now
to run it for real.

1. **`migrations::run` could not apply its own first migration.**
   `crates/persistence/src/postgres/migrations.rs` executed each migration
   file with `sqlx::query(sql).execute(pool)`, which prepares the string as a
   single statement via the extended query protocol. `001_create_events.sql`
   contains two statements (`CREATE TABLE` then `CREATE INDEX`), and Postgres
   rejected it outright: `cannot insert multiple commands into a prepared
   statement` (SQLSTATE `42601`). Nothing in the workspace had ever called
   `migrations::run` before this test did — confirmed by grep, zero other
   call sites. Fixed by switching to `sqlx::raw_sql(sql).execute(pool)`,
   which uses the simple query protocol and is designed for exactly this:
   multiple semicolon-separated DDL statements in one round trip, with no
   prepared-statement caching needed for a one-shot schema migration. This
   is a one-line change to the execution mechanism inside `run()`, not a new
   migration file — no file under
   `crates/persistence/src/postgres/migrations/` was added or changed.
2. **`PostgreSQLEventStore::append`/`load` panic on a current-thread Tokio
   runtime.** Both bridge into async code internally via
   `tokio::task::block_in_place`, which requires a multi-threaded runtime and
   panics otherwise (`can call blocking only when running on the
   multi-threaded runtime`). `#[tokio::test]` defaults to a current-thread
   runtime. Not a production-code change: the two tests that call
   `append`/`load` now use `#[tokio::test(flavor = "multi_thread")]`, and the
   test file documents why inline. This is itself a characterization fact
   worth keeping — it is part of what "synchronous-looking, actually-blocking"
   means for this store's current API, which design.md AD-2 already names as
   a latent hazard motivating the move to `async`.

### A gap discovered, deliberately not fixed (out of A1's scope)

While writing the happy-path test with `tenant_id: None` (the NULL-tenant
"systemwide" mode), both the version-check `SELECT` inside `append` and the
`SELECT` inside `load` silently behaved as if the aggregate had no prior
history at all. Cause: the queries filter with `tenant_id = $2`, and SQL's
three-valued logic makes `column = NULL` evaluate to unknown (never true) for
every row, regardless of how many rows actually exist with a `NULL`
`tenant_id`. This means the version-conflict check is a no-op today whenever
a caller passes `tenant_id: None` — a second append at a stale
`expected_version` silently succeeds instead of being rejected.

This is real and currently unpinned. The null-safe form is
`tenant_id IS NOT DISTINCT FROM $2`, which compares equal when both sides are
NULL — note that `tenant_id IS $2` is not valid SQL, so the fix is a different
operator rather than a variant of `IS`. Applying it is outside A1.1–A1.4 and
touches exactly the query logic A3 is
already scoped to harden around the NULL-tenant case (AD-1). Rather than
either silently working around it inside a "characterization" test or
quietly fixing production code outside this slice's four closed tasks, the
two behavioral tests use a concrete tenant (`Some("tenant-1")`) and the test
file documents the gap in a module-level comment, in plain language, so a
future reader — likely working on A3 — finds it named rather than
rediscovers it.

## TDD Cycle Evidence (A1)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| A1.1/A1.2 | `crates/integration-tests/tests/event_store_characterization.rs` | Integration (real Postgres via testcontainers) | N/A (new crate; no pre-existing tests to protect) | ✅ Written first, failed because the package did not exist: `error: package ID specification 'ego-integration-tests' did not match any packages` | ✅ 3/3 passing after creating `Cargo.toml`, registering the workspace member, adding the `layers.toml` entry, and fixing the two unplanned gaps above | ✅ 3 cases: happy-path append+load, stale-version conflict rejection, live-schema uniqueness-gap assertion — three genuinely different code paths, not restatements of one scenario | ➖ None needed — no duplication or complexity introduced worth extracting at this size |

## Work Unit Evidence (A1)

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p ego-integration-tests --test event_store_characterization` → 3 passed; 0 failed. Requires Docker; ran against colima (`DOCKER_HOST=unix:///Users/pablogore/.colima/default/docker.sock` on this machine — no Docker-host assumption is baked into the test: it asks the runtime via `get_host()` and the container's mapped port, normalising only the `localhost` name to its literal address so a host lacking that hosts entry still connects, while a genuinely remote host is honoured as reported). |
| Runtime harness command/scenario and exact result | Real Postgres 14-alpine container via `testcontainers`/`testcontainers-modules`, started and torn down per test — this crate exists specifically because no other layer in the workspace may touch a real external resource (`skills/testing` Rule 3). Verified the container genuinely starts each run (not a cached/skipped fixture) and that removing Docker access reproduces a loud panic, not a silent pass: confirmed earlier in this session, invoking the same start path with no reachable Docker socket panicked with `failed to initialize a docker client: Socket not found: /var/run/docker.sock` rather than skipping. |
| Rollback boundary | Delete `crates/integration-tests/`; drop its `"crates/integration-tests"` entry from the root `Cargo.toml` `[workspace] members`; drop its `"ego-integration-tests" = "tooling"` entry from `layers.toml`; revert the `README.md` "Requirements" section; revert the one-line `sqlx::query` → `sqlx::raw_sql` change in `crates/persistence/src/postgres/migrations.rs`. No schema change, no migration file added or removed, no data. |

## Full Verification (A1)

All eight declared gates, run against this branch:

- `cargo fmt --all -- --check`: clean, exit 0.
- `cargo check --workspace --all-targets`: clean, exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, exit 0, zero warnings.
- `cargo test --workspace`: all blocks `test result: ok`, 0 failures anywhere
  in the workspace, including `event_store_characterization` (3 passed).
- `cargo test --workspace --doc`: all blocks `test result: ok`, 0 failures.
- `cargo run -p xtask -- verify-layers`: `verify-layers: OK (17 crates, 0 violations)`.
- `cargo run -p xtask -- verify-isolation`: `verify-isolation: OK (17 crates checked in isolation)`.
- `cargo run -p xtask -- verify-hygiene`: `verify-hygiene: OK (no un-archived duplicates)`.

## Status (A1)

4/4 assigned A1 tasks complete (A1.1–A1.4). Combined with B0: 7 tasks
complete workspace-wide. Branch: `feat/prod-012-a1-integration-test-infrastructure`,
targeting `develop` per D11's hybrid chain strategy. Committed and opened as
**PR #253**, the second unit of the chain after B0, following an orchestrator
diff review that corrected the hard-coded container address and documented the
Docker requirement. Next unit in the chain per the dependency graph: A2
(`aggregate_type` real column), which stays blocked pending an operational
pinned pipeline or an explicit decision to accept the manual risk of rewriting
persisted identifiers.

### Included in this PR, unlike at B0's time

`tasks.md`'s checkbox flips for A1.1–A1.4 and this file's A1 section are
committed on the same branch as the code. At B0's time the whole planning
package was untracked, so bundling any of it risked a ~2,700-line diff; both
files are now tracked on `develop` (via `56d2c97`, #250), so this update is
four checkbox flips plus this section — small enough to travel with the code
it describes. Nothing else from the wider planning package is touched.

---

## Phase A4: Common `Clock` — COMPLETE

Branch: `feat/prod-012-a4-common-clock`, off `develop` at `10b221d`, targeting
`develop`. This unit runs out of its originally planned order: the dependency
graph makes A4 independent of Postgres, so it moves ahead of A2/A3 (both still
blocked); the chain for this run is B0 → A1 → A4. Final scope is A4.1–A4.2 only:
the run began with A4.1–A4.5, and A4.3–A4.5 were reverted during diff review once
their premise proved wrong — see the note below. Nothing from A2, A3 or B2 is
touched.

- [x] A4.1 RED: wrote the test module for `crates/domain/src/time/clock.rs`
      before the `Clock` trait and `SystemClock` struct existed at this path.
      Four tests were written initially, carried over from `auth/clock.rs`'s
      suite. The check that compared `SystemClock`'s reading against
      `Utc::now()` was then **deliberately removed** during review: it asserted
      that the operating system's clock behaves like a clock, non-deterministically,
      rather than asserting anything about this module. It is replaced by a
      structural assertion that `SystemClock` satisfies the trait, and every
      remaining time-dependent case runs against a fixed clock. No unit test in
      this slice reads the wall clock.
      Confirmed FAILED before implementation — `cargo test -p ego-domain --lib
      time::clock` produced six `E0405`/`E0432`/`E0599` compile errors (`cannot
      find trait Clock in this scope`, etc.), not a runtime assertion failure —
      the module genuinely didn't exist yet.
- [x] A4.2 GREEN: added the `Clock` trait and `SystemClock` struct to
      `crates/domain/src/time/clock.rs` (byte-identical implementation to
      the one that lived in `auth/clock.rs`), created `crates/domain/src/
      time/mod.rs` declaring the module, registered `pub mod time;` in
      `crates/domain/src/lib.rs`, and rewrote `crates/domain/src/auth/
      clock.rs` down to a single compatibility re-export: `pub use
      crate::time::clock::{Clock, SystemClock};`. No `#[deprecated]`
      attribute — this workspace treats warnings as errors, so a deprecation
      notice would fail the build at every existing JWT/`auth` call site
      rather than merely warn; the re-export is the documented permanent
      compatibility path per the design decision, not a temporary shim.
      `cargo test -p ego-domain --lib time::clock` → 4 passed.
### A4.3–A4.5 removed after inspecting the real call sites

These three tasks were implemented, then reverted during orchestrator diff
review, because the premise they rested on was wrong. Recorded rather than
quietly dropped, since the same premise appears in `design.md` AD-8.

The tasks asked for a `Clock` to be injected into `EffectDedupStore`, citing
`crates/runtime/src/effects/store.rs:58` as a direct `Utc::now()` call in that
store. Reading the code:

- That `Utc::now()` is inside `Timestamp::now()`, a free constructor on
  `Timestamp`, not inside any `EffectDedupStore` method.
- `EffectDedupStore`'s three methods — `reserve`, `commit_success`, `release` —
  neither take nor read time.
- `EffectStateStore`'s time-aware methods already receive time as a parameter:
  `claim_due(now, limit)`, `recover_in_flight(now)` and
  `mark_retryable(.., next_at)`. Time is injected per call already.
- Every `Timestamp::now()` inside `store.rs` sits below the `#[cfg(test)]`
  boundary at line 677, so none of them is a production read.

What had been built was therefore a `clock` field read by nothing except a
`now()` accessor added alongside it, plus a test asserting that accessor returns
whatever the constructor was handed. That test cannot fail for an interesting
reason, and a field with no reader is not a seam — it is dead weight that a later
slice would have had to reconcile.

`crates/runtime/src/effects/store.rs` and
`crates/service-sdk/src/runtime/builder.rs` were restored to their state on
`develop`. Neither appears in this slice's diff.

The genuine gap this exposed is elsewhere and is recorded as a follow-up in
`tasks.md`: `EffectRunner` reads the wall clock in production at
`crates/runtime/src/effects/runner.rs:546` and `:1017`. That is the retry
subsystem, it is not needed for B2, and it belongs in its own unit rather than
inside an unrelated slice.

A4 is complete at A4.1–A4.2, which is precisely what B2 consumes: one common
`Clock` available to inject into the reservation store when that store exists.

## TDD Cycle Evidence (A4)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| A4.1/A4.2 | `crates/domain/src/time/clock.rs` | Unit (in-crate), hermetic — no external resource, no wall-clock read | N/A (new module; coverage derives from the pre-existing `auth/clock.rs` suite, with one case deliberately replaced) | ✅ Written first; failed to compile (`E0405`/`E0432`/`E0599`, `Clock`/`SystemClock` not found) — a genuine "doesn't exist yet" RED, not an assertion failure | ✅ 4/4 passing after moving the trait/struct into place | ✅ 4 cases: `SystemClock` satisfies the trait contract structurally (generic bound and trait object), `FixedClock` returns its exact value twice over proving determinism, object safety behind `Box` asserted with `FixedClock`, and the trait works behind `Arc`. The original wall-clock plausibility check was removed rather than carried over. | ➖ None needed — a pure move, no new complexity |

## Work Unit Evidence (A4)

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p ego-domain --lib time::clock` → 4 passed, 0 failed. `cargo test -p ego-runtime --lib effects::store` → 29 passed, 0 failed. `cargo test -p ego-service-sdk --lib` → 250 passed, 0 failed. |
| Runtime harness command/scenario and exact result | N/A — per the tasks artifact's own row for this unit: "N/A — pure unit test, no external service." No Postgres, no testcontainers, no real actor spawn is exercised by this slice; the full `cargo test --workspace` run below is the closest thing to a runtime harness and is green including the Docker-backed `ego-integration-tests` crate, which this slice did not touch. |
| Rollback boundary | Delete `crates/domain/src/time/`, drop the `pub mod time;` line from `crates/domain/src/lib.rs`, and restore `crates/domain/src/auth/clock.rs` to its pre-move content. Zero behaviour change in either direction, since the move preserved the trait and its implementation byte for byte and the original import path still resolves. No other crate is touched, and nothing touches schema, migrations or persisted state. |

## Full Verification (A4)

### UNIT — the gate for A4

This slice touches only `crates/domain`, so this is what A4 stands or falls on.
Hermetic: no Docker, no external resource, no wall-clock dependency.

- `cargo test -p ego-domain --lib time::clock`: **4/4 passed**, 0 failed
  (`system_clock_satisfies_the_clock_contract`, `fixed_clock_returns_exact_time`,
  `clock_is_object_safe`, `clock_works_behind_arc`).

### INTEGRATION — inherited regression, run additionally

Green, and **not a requirement of A4**. Recorded because it was executed.

- `cargo test --workspace` (with `DOCKER_HOST` pointed at the colima socket):
  103 `test result: ok` blocks, 0 failures anywhere, including the Docker-backed
  `event_store_characterization` suite inherited from A1 (3 passed). This gate
  became Docker-dependent when A1 landed; it is repository-wide regression
  coverage, not a check A4 introduces or depends on.

### Repository validation — general gates, also green

- `cargo fmt --all -- --check`: clean, exit 0.
- `cargo check --workspace --all-targets`: clean, exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, exit 0, zero warnings.
- `cargo test --workspace --doc`: all blocks `test result: ok`, 0 failures.
- `cargo run -p xtask -- verify-layers`: `verify-layers: OK (17 crates, 0 violations)`.
- `cargo run -p xtask -- verify-isolation`: `verify-isolation: OK (17 crates checked in isolation)`.
- `cargo run -p xtask -- verify-hygiene`: `verify-hygiene: OK (no un-archived duplicates)`.

## Status (A4)

A4 complete at A4.1–A4.2. A4.3–A4.5 were built, then reverted on review and
removed from the task list, replaced by two follow-up tasks for the effect retry
subsystem. Combined with B0 and A1: 9 of 92 tasks complete workspace-wide.
Branch: `feat/prod-012-a4-common-clock`,
targeting `develop`. Per the consciously reordered chain for this run (B0 →
A1 → A4), A2 and A3 remain deliberately untouched and blocked; B2 is
unblocked by this slice (needs A4's `Clock` for deterministic lease/expiry
tests) but is not started here.

---

## Phase B1 (partial): `OperationKey` and `OperationFingerprint` — COMPLETE

Branch: `feat/prod-012-b1-operation-key`, off `develop` at `cbc0187`.

- [x] B1.1 RED: unit tests in `crates/domain/src/operation/key.rs` — `parse`
      rejects empty and whitespace-only input, rejects over-length input, and
      accepts a bounded-length valid string; fingerprint equality is by value.
- [x] B1.2 GREEN: `OperationKey`, `OperationKeyError` and
      `OperationFingerprint` implemented in that module, a sibling of
      `idempotency.rs` rather than part of its type family, so the compiler
      keeps the two identities distinct.

**Why these two tasks land alone.** The reservation contract cannot define its
request type without them, so they were pulled out of B1 ahead of the rest.
B1.3–B1.10 — the no-conversion compile-fail assertion, the extraction contract
and its policy table, the HTTP carrier, and carriage through the service and
command contexts — remain open and follow separately.

### Provenance, stated rather than implied

The code in this slice and the two that follow it was authored during an earlier
apply run that was interrupted, and was found already present in the working tree
by the run that committed it. It was verified against the design's Interfaces
section and the specs rather than rewritten, and it is being split into reviewable
slices without reworking its content. Recorded so the authorship of the diff is
not misread.

### UNIT — the gate for this slice

Hermetic: in-memory only, no clock dependency, no external resource.

- `cargo test -p ego-domain --lib operation`: **7/7 passed**, 0 failed.

### Static gates — compile and lint only

No test suite is executed by these, and none of them requires Docker.

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo run -p xtask -- verify-layers`
- `cargo run -p xtask -- verify-isolation`
- `cargo run -p xtask -- verify-hygiene`

`cargo test --workspace` was deliberately **not** run: it pulls in the inherited
Docker-backed integration suite, which is not part of this slice's validation.

### Status

2 tasks complete (B1.1, B1.2). Combined with B0, A1 and A4: 11 of 92 complete.
Next in the chain: the reservation contract, then its in-memory implementation.

---

## Phase B2, contract slice: the reservation port — COMPLETE

Branch: `feat/prod-012-b2a-reservation-contract`, stacked on the
`OperationKey` slice, which it genuinely depends on: `reservation.rs` imports
`OperationKey` and `OperationFingerprint` for its request type.

- [x] B2.2 GREEN: `OperationReservationStore` and its supporting types —
      `OperationId`, `OwnerId`, `FencingToken`, `Lease`, `OwnerFence`,
      `ReserveRequest`, `ReservationOutcome`, `StoredResponse` and
      `ReservationError` — defined in `crates/domain/src/operation/reservation.rs`
      per the design's Interfaces section. Port only: no implementation lives in
      the domain crate, which its hexagonal boundary forbids.

### Why the contract lands separately

The seven tests in this module are type-level — that a taken-over fencing token
is strictly greater than the original, that `OperationId` is scoped by tenant and
key, that `OwnerFence` carries the full verification triple, that the outcome
variants are constructible and comparable, that `StoredResponse` compares by
content, that `StaleOwner` is distinguishable from a backend error, and that
`ReserveRequest` carries both a fingerprint and a lease bound. None of them
exercises `reserve`, so none needs an implementation. That is what makes this a
coherent slice rather than an arbitrary cut, and it keeps the contract reviewable
on its own.

### UNIT — the gate for this slice

Hermetic: no clock, no external resource, no implementation under test.

- `cargo test -p ego-domain --lib operation`: **14/14 passed**, 0 failed
  (7 from the identity types, 7 from the port's supporting types).

### Static gates — compile and lint only

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo run -p xtask -- verify-layers`
- `cargo run -p xtask -- verify-isolation`
- `cargo run -p xtask -- verify-hygiene`

`cargo test --workspace` was deliberately **not** run: it pulls in the inherited
Docker-backed integration suite, which is not part of this slice's validation.

### Status

1 task complete (B2.2). Combined with everything prior: 12 of 92 complete.
Next: the in-memory implementation and the behavioural tests.

### Review correction — fencing overflow and the expired-owner hole

Two real defects, both found by review of the split, both fixed.

**`FencingToken::next` was an unchecked `+ 1`.** At `u64::MAX` that panics in a
debug build and wraps in a release build. Wrapping is the dangerous outcome: a
wrapped token can compare equal to a fence a prior owner still holds, un-fencing
the very owner a takeover exists to exclude. The whole point of the type is
exclusion, so neither a panic nor a silent wrap is acceptable. `next` now returns
`Option<Self>` from a checked add, and the store surfaces exhaustion as an explicit
`ReservationError::FencingExhausted` rather than minting a token that no longer
fences. Two hermetic tests cover it: the boundary reports exhaustion instead of
wrapping, and a thousand consecutive advances stay strictly monotonic.

**The contract now states that an expired lease is not an owned lease.** `renew`'s
doc already said "still-valid lease", but nothing in the contract said what happens
when the triple matches and the lease has lapsed — and the implementation let it
through. Each mutating method now documents that an expired lease is rejected as a
stale owner, with the reason it matters: a lapsed holder renewing would resurrect a
dead lease and defeat a legitimate takeover; completing would publish a result for
an operation it no longer owns, which a later replay would serve as authoritative;
abandoning would discard a reservation another caller was entitled to seize.

A traceability note: the topology decision this chain follows is **D11 in
`decisions.md`**, not "AD-11" — `design.md` carries AD-1 through AD-10 and has no
eleventh architecture decision.

---

## Phase B2, implementation slice: in-memory reservation store — COMPLETE

Branch: `feat/prod-012-b2b-in-memory-store`, stacked on the contract slice.

- [x] B2.1 RED: `reserve` returns a fresh claim on first call, the same owner
      mid-lease sees its own claim in progress, a different owner mid-lease is
      told the operation is already in progress elsewhere.
- [x] B2.3 RED / B2.4 GREEN: advancing the clock past the lease bound makes a
      stale reservation eligible for takeover, and takeover mints a strictly
      greater fencing token in the same critical section as the read.
- [x] B2.5 RED / B2.6 GREEN: a stale fence is rejected on `renew`, `complete`
      and `abandon`, and the reservation is left untouched. The check compares
      the full triple, not merely whether some token is stored.
- [x] B2.7: renewal is caller-driven only. A long-running operation either
      finishes inside its configured lease or is taken over; nothing renews in
      the background. `renew` exists for a caller that needs it.
- [x] B2.8 RED / B2.9 GREEN: the same key arriving with a different fingerprint
      yields a conflict, never a silent dedupe.

### Ordering that carries the guarantee

The fingerprint guard is the first arm after the not-found case, ahead of every
lease and ownership branch. That ordering is load-bearing: if a lease check ran
first, a conflicting payload arriving after expiry would be reinterpreted as a
legitimate takeover and admitted.

### Review correction — a lapsed holder could still mutate the reservation

Review found a real hole. `renew`, `complete` and `abandon` verified the fence
triple but never compared the clock against the lease bound, so in the window
between expiry and somebody else's takeover the lapsed holder could still act. It
could renew and resurrect a dead lease, defeating a takeover that had already
become legitimate; complete, publishing a result for an operation it no longer
owned, which a later replay would serve as authoritative; or abandon, discarding a
reservation another caller was entitled to seize. The port's own documentation said
"still-valid lease" while the implementation accepted an invalid one.

All three now require `now < lease_until` alongside the triple, and reject with
`StaleOwner` without mutating.

**Second correction, on the same code:** the first fix read the clock *before*
acquiring the lock, and the comment justifying it was wrong — it argued for "one
instant for the whole decision", when the property that matters is an instant read
*inside* the critical section. Reading before the lock leaves a time-of-check to
time-of-use race: a caller can read the clock while the lease is live, block on the
mutex, have the lease lapse during that wait, then enter carrying a stale instant
and mutate a reservation another caller is already entitled to seize. Validation and
mutation must be linearised together. All three now lock first and read the clock
inside, which is also what `reserve` already did — it takes the lock and only then
compares the clock against the lease bound.

A hermetic concurrency test covers it: the test itself holds the store's lock,
starts the mutation, advances the clock to exactly the lease bound while still
holding it, and only then releases. Because the advance strictly precedes the
release, any clock read taken inside the critical section necessarily observes the
expired lease. Its limitation is written into the test rather than left implied — as
a regression detector for the ordering it is not airtight, since a spawned task that
has not yet reached its clock read would also observe the expired instant and reject.
Closing that gap needs a sleep or a synchronisation seam inside the store, and this
suite forbids the first and does not warrant the second.

Three tests pin it at **exactly** `lease_until` — the same instant `reserve`
already treats as expired and seizable. Testing the boundary rather than a
comfortable margin is what keeps the two decisions on one definition of expiry
instead of leaving a one-instant window where a lease is simultaneously seizable
and renewable. Each asserts the reservation is unmodified, by observing that
another owner still takes it over afterwards, rather than only that the call
errored — "rejected but mutated" would corrupt state just as badly.

The takeover path also adopts the checked fencing advance from the contract slice
and surfaces `FencingExhausted` instead of wrapping.

### Determinism

`TestClock` holds a mutex-guarded instant and advances only when a test calls
`advance`. No test reads the wall clock, sleeps, or depends on machine speed.

### Review budget — size exception, deliberately concentrated here

One file, of which the clear majority is behavioural tests. This exceeds the
400-line budget and the exception is accepted here rather than spread across the
chain: the identity types and the port contract were split out precisely so it
applies only to the slice whose bulk is coverage of executable behaviour.
Splitting further would separate tests from the code they exercise.

### UNIT — the gate for this slice

Hermetic: in-memory state, deterministic clock, no external resource.

- `cargo test -p ego-testkit`: **81/81 passed**, 0 failed.
- `cargo test -p ego-domain --lib operation`: **16/16 passed**, 0 failed.

### Static gates — compile and lint only

`cargo fmt --all -- --check`, `cargo check --workspace --all-targets`,
`cargo clippy --workspace --all-targets -- -D warnings`, and the three
`xtask verify-*` commands: all pass.

`cargo test --workspace` was **not** run. Docker availability is not part of this
slice's validation.

### Status

8 tasks complete (B2.1, B2.3–B2.9). Combined with everything prior: **20 of 92**.
The reservation contract now has an executable model; the durable Postgres
implementation waits on the schema work.

---

## Phase B1c: the extraction contract and its policy — COMPLETE

Branch: `feat/prod-012-b1c-extraction-contract`, off
`feat/prod-012-idempotency-tracker` at `d3b3845`, targeting the tracker branch
per D11's hybrid chain strategy (this branch does not merge to `develop`
directly — only the consolidated tracker does).

Scope: exactly four pieces, per the apply instructions — B1.3, B1.4, B1.5, and
`assert_carrier_conformance` in `crates/testkit`. B1.6–B1.10 (the HTTP carrier,
`ServiceContext`/`CommandContext` carriage) remain open and follow separately;
nothing under `crates/transport` was touched.

- [x] B1.3 RED/GREEN: `crates/service-sdk/tests/operation_key_conversion.rs`
      drives `trybuild` against two new `tests/compile_fail/` fixtures — one
      per direction (`OperationKey` → `IdempotencyKey` and the reverse).
      Confirmed genuine RED before the driver existed:
      `cargo test -p ego-service-sdk --test operation_key_conversion` failed
      because no such test target existed. The `.stderr` snapshots were
      generated deliberately with `TRYBUILD=overwrite` and read rather than
      accepted blindly — both are `E0277: the trait bound
      ... From<...> is not satisfied`, i.e. the compiler itself confirms
      neither `From<OperationKey> for IdempotencyKey` nor its reverse exists
      anywhere reachable from this workspace today (D7, spec scenario "No
      implicit conversion compiles").
- [x] B1.4 RED / B1.5 GREEN: `crates/service-sdk/src/idempotency/extraction.rs`
      defines `OperationKeyCarrier` (reads one string and nothing else — no
      request, no headers, no protocol knowledge) and `resolve_operation_key`,
      the single validation and missing-key policy entry point (AD-4).
      `crates/service-sdk/src/runtime/idempotency.rs` defines
      `IdempotencyEnforcementMode` (`MandatoryKey` default, `Compatibility`
      bounded escape hatch), mirroring `TenantEnforcementMode`
      (`runtime/tenant.rs:143`)'s posture: a fixed-invariant enum, not
      `dyn`-dispatched, with its doc-comment stating why. Confirmed genuine RED
      for each file before its production code existed: the test module for
      `runtime/idempotency.rs` compiled to "0 tests" (module not yet wired into
      `runtime/mod.rs`), and `idempotency/extraction.rs`'s test module produced
      `E0432: unresolved imports` for all three not-yet-defined symbols.
      `OperationKeyRejection::Invalid` is rejected under *every* mode,
      including `Compatibility` — that variant loosens only the missing-key
      policy, never what counts as a valid key; a malformed key is not
      "absent."
- [x] Conformance harness: `assert_carrier_conformance` in
      `crates/testkit/src/idempotency.rs`. **Deviation from design.md's literal
      AD-4 sketch, flagged per apply instructions** (the same posture
      `runtime/tenant.rs`'s `CanonicalTenant` doc-comment already takes for its
      own deviation): the sketch shows a single-argument
      `assert_carrier_conformance(&carrier)`, but one fixed carrier instance
      can only ever report one `raw_operation_key()` value, so it cannot
      exercise both halves of `resolve_operation_key`'s policy table (key
      present vs. key absent) against the identical adapter implementation.
      Implemented instead as
      `assert_carrier_conformance<C: OperationKeyCarrier>(with_key: &C, without_key: &C)`
      — two instances of the same adapter type, one carrying a key and one not.
      The generic bound is what enforces "same type": an earlier version took
      `&dyn` twice and therefore accepted two unrelated implementations, which
      review caught and which the note further down records.
      Confirmed genuine RED before the function existed:
      `cargo test -p ego-testkit --lib idempotency::` failed with
      `E0432: unresolved import` for `assert_carrier_conformance` in both the
      test module and `lib.rs`'s re-export. Proven against a test-local
      `FakeCarrier` (not the future HTTP `HeaderCarrier`, which is out of
      scope here): one test proves a correctly-implemented pair passes
      silently, one `#[should_panic]` test proves a mislabeled pair (a
      "without_key" instance that still reports a key) is caught rather than
      silently accepted.

### Why these four pieces land as one work unit

All four exist to satisfy one guarantee — D7's "no implicit conversion" plus
AD-4's "one shared extraction contract" — and the conformance harness is
meaningless without the policy function it exercises. Splitting the
compile-fail assertion from the extraction contract, or the contract from its
conformance harness, would fragment one coherent review into pieces that
cannot be evaluated independently of each other.

### Review budget — stated plainly, not absorbed

This slice's diff, after the review corrections, is **534 inserted lines of code
across 12 files** plus **224 inserted and 5 deleted lines across the two planning
artefacts**, for a whole-PR total of **+758 / −5 across 14 files**. Of the code, 14
lines are generated `.stderr` trybuild snapshots — goldens, excluded from the
authored-risk count per the review-workload convention but still part of snapshot
identity. **Authored risk count: 520 lines — this exceeds the 400-line review
budget.** The figure grew from 475 when the conformance harness was corrected; the
correction added the generic bound, the missing name assertion and two negative
tests. No `size:exception` was pre-negotiated for this
specific B1c slice in `tasks.md`'s Review Workload Forecast (that table
groups all of B1 into one PR-6-sized unit rather than pre-splitting B1.3–B1.5
from B1.6–B1.10). Recorded honestly rather than silently absorbed, following
the precedent set by the B2 implementation slice's own stated size exception
above: the four pieces are tightly coupled to one guarantee (see above), and
tests are kept with the behavior they verify per `skills/work-unit-commits`,
which is most of what drives the line count — `extraction.rs` and
`idempotency.rs` (testkit) are each roughly half test code.

### UNIT — the gate for this slice

Hermetic: in-memory only, no clock dependency, no external resource, no
Docker.

- `cargo test -p ego-service-sdk`: **377 passed** across all targets, 0 failed (including both compile-fail cases; was 250 pre-existing +
  8 new: 2 in `runtime::idempotency`, 6 in `idempotency::extraction`), plus the
  `operation_key_conversion` trybuild driver (1 passed, exercising both
  compile-fail fixtures) and pre-existing integration/doc-test binaries,
  unaffected.
- `cargo test -p ego-testkit`: **85 passed**, 0 failed (81 pre-existing + 4 new
  in `idempotency::tests`).

### Static gates — compile and lint only

- `cargo fmt --all -- --check`: clean, exit 0 (after one `cargo fmt --all` pass
  to apply the project's import-ordering and line-wrap conventions to the new
  files).
- `cargo check --workspace --all-targets`: clean, exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, exit 0, zero
  warnings.
- `cargo run -p xtask -- verify-layers`: `verify-layers: OK (17 crates, 0
  violations)`.
- `cargo run -p xtask -- verify-isolation`: `verify-isolation: OK (17 crates
  checked in isolation)`.
- `cargo run -p xtask -- verify-hygiene`: `verify-hygiene: OK (no un-archived
  duplicates)`.

`cargo test --workspace` was **not** run, per this run's explicit instructions
— only the two listed UNIT gates and the six static gates were permitted.

### TDD Cycle Evidence

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| B1.3 | `crates/service-sdk/tests/operation_key_conversion.rs` + 2 `compile_fail/` fixtures | Integration (trybuild) | N/A (new files) | ✅ Confirmed: no such test target existed before the driver was written | ✅ 1/1 passed after generating both `.stderr` snapshots and reading them | ✅ 2 cases — both conversion directions, each independently confirmed absent by the compiler | ➖ None needed — minimal fixtures |
| B1.4/B1.5 (extraction) | `crates/service-sdk/src/idempotency/extraction.rs` | Unit | N/A (new module) | ✅ `E0432` unresolved imports for `OperationKeyCarrier`/`OperationKeyRejection`/`resolve_operation_key` | ✅ 6/6 passed after implementing the trait, enum, and function | ✅ 6 cases: present+valid under both modes, missing under both modes, invalid under both modes — the full policy table | ➖ None needed |
| B1.5 (mode) | `crates/service-sdk/src/runtime/idempotency.rs` | Unit | N/A (new module) | ✅ Module not yet wired into `runtime/mod.rs` — filtered test run matched 0 of 250 existing tests, and neither new test | ✅ 2/2 passed after wiring the module and implementing `IdempotencyEnforcementMode` + its `Default` impl | ➖ Structural: exactly two variants, `Default` has one possible correct answer per D1 | ➖ None needed |
| Conformance harness | `crates/testkit/src/idempotency.rs` | Unit | N/A (new module) | ✅ `E0432` unresolved import for `assert_carrier_conformance` in both the test module and `lib.rs` | ✅ 4/4 passed after implementing the function and applying the review corrections | ✅ 4 cases: a conforming pair passes silently; a mislabeled pair, one adapter reporting two different names, and an empty name each panic with their own expected message | ➖ None needed |

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p ego-service-sdk` → 377 passed across all targets, 0 failed. `cargo test -p ego-testkit` → 85 passed, 0 failed. |
| Runtime harness command/scenario and exact result | N/A — this slice defines a pure validation function and a policy enum, both hermetic; no request, no transport, no runtime boundary exists yet for this contract (that arrives with the HTTP carrier in B1.6–B1.7, explicitly out of scope here). |
| Rollback boundary | Delete `crates/service-sdk/src/idempotency/`, `crates/service-sdk/src/runtime/idempotency.rs`, `crates/service-sdk/tests/operation_key_conversion.rs`, `crates/service-sdk/tests/compile_fail/{operation_key_into_idempotency_key,idempotency_key_into_operation_key}.{rs,stderr}`, and `crates/testkit/src/idempotency.rs`; revert the four `mod`/`pub use` one-line additions in `crates/service-sdk/src/lib.rs`, `crates/service-sdk/src/runtime/mod.rs`, and `crates/testkit/src/lib.rs`. No schema, no migration, no persisted state — every new symbol is unreferenced by any other crate as of this slice. |

### Status

3 tasks complete (B1.3, B1.4, B1.5) plus the `testkit` conformance harness
(not independently numbered in `tasks.md` — it is part of B1.5's deliverable
per the design's AD-4 consequence and the `testkit` spec). Combined with
everything prior: **23 of 92**. Next in the chain: B1.6–B1.7 (the HTTP
carrier under `crates/transport`), then B1.8–B1.10 (`ServiceContext`/
`CommandContext` carriage) — both explicitly out of scope for this run.

### Orchestrator diff review — identifier convention violated in the new code

Nineteen decision and task identifiers had been written into the doc-comments and
test fixtures this slice introduced — `D1`, `D7`, `D9`, `AD-4`, `AD-10`, `B1.3`, and
`design.md` references — across all five new files. That breaks the project
convention that a comment must explain behaviour and reasons rather than cite the
planning artefact that produced it: the identifier is archived within months and the
comment then explains nothing.

Every one was rewritten to carry the substance instead of the citation. For example,
"no server-side generation (D1)" became a statement of *why*: a server-minted key is
a function of the request as received, so a retry produces a different one and
deduplicates nothing. The pre-existing identifiers in the older `compile_fail/`
fixtures were left alone — cleaning those is separate, opportunistic work, not this
slice's business.

The `trybuild` goldens are line-sensitive, so the rewrite was verified by re-running
the suite rather than by inspection: both compile-fail expectations still match.

### Review budget — composition of the overrun

534 inserted lines of code across twelve files, of which 14 are generated `.stderr`
goldens. Composition:

| Part | Production | Tests |
|---|---|---|
| `idempotency/extraction.rs` | 91 | 108 |
| `runtime/idempotency.rs` | 46 | 20 |
| `testkit/idempotency.rs` | 103 | 97 |
| compile-fail driver, fixtures and goldens | — | 50 |
| module wiring | 19 | — |

Which rolls up to the four figures reported on the pull request — 137 lines of
production decision logic, 103 of reusable testkit infrastructure, 275 of tests,
fixtures and goldens, and 19 of wiring. Those sum to 534 exactly.

That exceeds the 400-line budget. Stated rather than absorbed, with the composition
that makes it judgeable: the **actual decision logic under review is 137 lines** —
the extraction policy and the enforcement mode. The testkit harness's 103 lines are
production only in the sense that they compile into a library crate; their purpose is
testing. Splitting further would separate the conformance harness from the contract
it defines, and that harness exists precisely so the next slice's adapter can be
judged against the contract rather than against its author's reading of it.

### Review correction — the conformance harness did not verify what it promised

Three defects in `assert_carrier_conformance`, two found by review and one alongside
them.

**It took `&dyn OperationKeyCarrier` twice**, so nothing stopped a caller passing two
*different* implementations — which is precisely what a harness named "conformance of
one adapter" must not permit. It is now generic over a single `C:
OperationKeyCarrier`, so the compiler enforces that both instances are the same
adapter. That is the stronger place for the constraint than any runtime assertion.

**`without_key.carrier_name()` was never asserted at all.** A `without_key` instance
could report a different name, or an empty one, and still pass. The harness documents
that it guarantees a stable diagnostic location, and it did not.

**And the "stable across calls" assertion compared `with_key.carrier_name()` to
itself.** For a method returning `&'static str` that is tautologically true — it could
not fail for any reason, interesting or otherwise. It claimed a property and proved
nothing.

The check is now `!with_key.carrier_name().is_empty()` plus
`with_key.carrier_name() == without_key.carrier_name()`. The second is the real
property: two instances of one adapter must agree, which catches a name derived from
per-instance state rather than from the adapter itself — that would make the
diagnostic location depend on which request happened to be rejected.

Two negative tests were added: one adapter reporting two different names fails, and an
empty name fails. The first test's own comment records what it deliberately cannot
express — passing two different implementations is now a compile error, not a runtime
assertion, so there is nothing left to assert about it at runtime.
