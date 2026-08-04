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
