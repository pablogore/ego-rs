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

---

## Phase B1d: the HTTP carrier — COMPLETE

Branch: `feat/prod-012-b1d-http-carrier`, off
`feat/prod-012-idempotency-tracker` at `ef55fa5`, targeting the tracker branch
per the hybrid chain strategy.

Scope: exactly two tasks — B1.6 and B1.7. Neither `ServiceContext` nor
`CommandContext` is touched; carriage into either is a later slice. No
reservation store, no replay, no conflict handling.

- [x] B1.6 RED: `crates/transport/tests/idempotency_carrier.rs` runs
      `assert_carrier_conformance` against three `HeaderCarrier` instances
      built from real `axum::http::HeaderMap`s, one with an `Idempotency-Key`
      header and one without. Confirmed genuine RED before the carrier
      existed: `cargo test -p ego-transport --test idempotency_carrier`
      failed with `E0432: unresolved import` — `ego_transport::idempotency`
      did not exist yet, not merely `HeaderCarrier` inside an existing
      module.
- [x] B1.7 GREEN: implemented `HeaderCarrier<'a>(pub &'a HeaderMap)` in
      `crates/transport/src/idempotency.rs`, beside `security.rs` and
      `propagation.rs`. It reads the `Idempotency-Key` header and nothing
      else and reports the stable diagnostic name `"http:Idempotency-Key"`.
      Wired rejection into `crates/transport/src/error.rs` via
      `impl From<OperationKeyRejection> for TransportError`, mapping all three
      reasons — `Missing`, `Invalid` and `Unreadable` — to
      `TransportError::BadRequest`. The same
      category `ServiceError::Validation` and `SecurityError::
      InvalidAccessRequest` already use for caller-supplied input rejected
      before a handler runs. `Conflict` stays reserved for the later
      same-key-different-fingerprint replay mismatch, which this slice does
      not implement. A second RED test,
      `error::tests::operation_key_rejection_status_table`, was written
      before the `From` impl existed and confirmed genuine RED:
      `E0277: the trait bound ... From<OperationKeyRejection> ... is not
      satisfied`.

### Status code choice, stated rather than assumed

`BadRequest` (400) was chosen by matching the existing table in
`crates/transport/src/error.rs` rather than picking a code independently:
`ServiceError::Validation` and `SecurityError::InvalidAccessRequest` both map
to `BadRequest` for the same shape of failure — caller-supplied input that
fails a validation rule before any handler runs. A missing or malformed
`Idempotency-Key` is exactly that shape. `Conflict` (409) was considered and
rejected for this slice: it is reserved by the existing table for a resource
state conflict, and the design's later same-key-different-fingerprint
permanent-conflict response is a distinct, not-yet-implemented case that
should not be conflated with "the header was absent or malformed."

### Out of scope, confirmed by reading the real files first

Before writing any code, the spec file
(`specs/http-transport/spec.md`) and the design's Data Flow section were
read in full. Two things in that spec are explicitly not delivered by this
slice, matching the boundary given: "Valid key is carried into
ServiceContext" (carriage is a later slice; `ServiceContext` is untouched)
and the entire "Replay and Conflict Responses Are Distinguishable"
requirement (needs the reservation store wired end to end). Nothing in this
slice's diff references either.

### TDD Cycle Evidence

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| B1.6 | `crates/transport/tests/idempotency_carrier.rs` | Integration (in-crate, hermetic — real `HeaderMap`, no network) | N/A (new file) | ✅ `E0432: unresolved import` — `ego_transport::idempotency` did not exist | ✅ 1/1 passed after implementing `HeaderCarrier` | ➖ The shared conformance harness covers all three rows of the policy table against this one adapter, including a real non-UTF-8 header value — no further triangulation needed at this layer | ➖ None needed |
| B1.7 (carrier) | `crates/transport/src/idempotency.rs` | Unit (in-crate) | N/A (new module) | ✅ Module unit tests written alongside the type; the carrier itself did not exist before this task, so the whole module is new | ✅ 5/5 passed | ✅ 5 cases: present, absent, unreadable (real non-UTF-8 bytes), the lowercase header name HTTP/2 actually sends, and the fixed diagnostic name | ➖ None needed — minimal newtype |
| B1.7 (rejection mapping) | `crates/transport/src/error.rs` | Unit (in-crate) | ✅ 20/20 pre-existing `ego-transport` lib tests unaffected | ✅ `E0277: the trait bound ... From<OperationKeyRejection> ... is not satisfied`, then `E0004: non-exhaustive patterns` when the third reason was added | ✅ 1/1 passed after mapping all three reasons to `BadRequest` | ✅ 3 cases: `Missing`, `Invalid` and `Unreadable`, each asserted independently — the exhaustive match proves the third is *mapped*, this proves it collapses to the *same* status | ➖ None needed |

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p ego-transport` → 26 lib + 1 `idempotency_carrier` + 3 `security_extractor` + 1 `server` = 31 passed, 0 failed. Plus the crates the contract change reaches: `ego-service-sdk` 379, `ego-testkit` 86, `ego-domain` 228 — all 0 failed. |
| Runtime harness command/scenario and exact result | N/A — this slice defines a pure header-reading adapter and a pure error-category mapping, both hermetic; no HTTP server, no route, no network socket is exercised. `crates/transport/tests/server.rs`'s existing real-listener test (`serve_handles_a_request_then_shuts_down_gracefully`) is unmodified and still passes, confirming this slice did not disturb the one runtime-adjacent test this crate has. |
| Rollback boundary | Revert the whole commit. The three-state contract and the carrier are one unit: the carrier cannot report `Unreadable` without the contract's third state, and reverting only the transport half would reintroduce the unsound path in which a malformed key is admitted under the compatibility variant. Spans `service-sdk` (contract and policy), `testkit` (conformance harness) and `transport` (carrier and error mapping). No schema, no migration, no persisted state. |

### Review budget — composition, well within the 400-line budget

The slice began as transport-only at 124 lines. Closing the contract gap it exposed
widened it across three crates — `service-sdk` for the third state and its resolution
rule, `testkit` for the conformance harness that now exercises that rule, and
`transport` for the carrier and its error mapping. The figures below are measured
after that widening; the code diff remains inside the 400-line budget, so no
`size:exception` is needed.

| Crate | What changed |
|---|---|
| `service-sdk` | `RawOperationKey`'s three states, the `Unreadable` rejection reason, and the resolution rule that rejects it under every mode |
| `testkit` | The conformance harness's third instance and the two policy rows it asserts |
| `transport` | `HeaderCarrier`, its five unit cases, and the three-reason error mapping |

The widening was accepted rather than deferred because the gap was discovered by
building the first real adapter, which is exactly when a contract's missing case
surfaces. Merging the adapter first would have shipped one that documents its own
unsound path.

### UNIT — the gate for this slice

Hermetic: in-memory `HeaderMap` values only, no clock dependency, no
external resource, no Docker.

- `cargo test -p ego-transport`: **31 passed**, 0 failed
  (26 lib tests — 20 pre-existing plus 6 new: 5 `idempotency` unit tests and
  1 `operation_key_rejection_status_table` — plus the 1 new
  `idempotency_carrier` integration test, plus the 3 pre-existing
  `security_extractor` and 1 pre-existing `server` integration tests,
  unaffected).
- `cargo test -p ego-service-sdk`: **379 passed**, 0 failed — the crate the
  three-state contract and its resolution rule live in.
- `cargo test -p ego-testkit`: **86 passed**, 0 failed — the conformance
  harness, now five cases including the third-state precondition.
- `cargo test -p ego-domain`: **228 passed**, 0 failed — unchanged by this
  slice, run because the contract change is visible from it.

### Static gates — compile and lint only

- `cargo fmt --all -- --check`: clean, exit 0.
- `cargo check --workspace --all-targets`: clean, exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, exit 0,
  zero warnings.
- `cargo run -p xtask -- verify-layers`: `verify-layers: OK (17 crates, 0
  violations)`.
- `cargo run -p xtask -- verify-isolation`: `verify-isolation: OK (17 crates
  checked in isolation)`.
- `cargo run -p xtask -- verify-hygiene`: `verify-hygiene: OK (no un-archived
  duplicates)`.

`cargo test --workspace` was **not** run, per this run's explicit
instructions — Docker/testcontainers/PostgreSQL/network are all out of
bounds for this slice's validation.

### Status

2 tasks complete (B1.6, B1.7). Combined with everything prior: **25 of 92**.
Next in the chain: B1.8–B1.10 — `ServiceContext`/`CommandContext` carriage —
explicitly out of scope for this run.

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
property: every instance of one adapter must agree, which catches a name derived from
per-instance state rather than from the adapter itself — that would make the
diagnostic location depend on which request happened to be rejected.

Two negative tests were added: one adapter reporting two different names fails, and an
empty name fails. The first test's own comment records what it deliberately cannot
express — passing two different implementations is now a compile error, not a runtime
assertion, so there is nothing left to assert about it at runtime.

### Orchestrator diff review — two gaps closed in the carrier

**Case-insensitive lookup was correct but unpinned.** `HeaderMap::get` with a `&str`
name already matches case-insensitively, so the carrier worked — but nothing asserted
it. That matters in practice rather than in theory: HTTP/2 transmits every header name
lowercased, so a real client sends `idempotency-key`, and a refactor to a literal
string comparison would pass every other test in the file while rejecting every HTTP/2
request as though it had sent no key at all. Now pinned by a test that inserts the
lowercase form.

**A non-UTF-8 header value reads as absent, not as present-but-unusable.** `to_str()`
fails and `.ok()` discards the error, so garbage bytes become "no key". Under the
fail-closed default that still rejects and the safe path is unaffected. Under the
compatibility variant it means a malformed key is treated as no key at all and the
request proceeds unguarded, instead of the client being told its key was bad.

**And then it was fixed rather than documented.** The first version of this slice
recorded the gap as out of scope, on the grounds that `raw_operation_key` returned
`Option<&str>` and had no third answer to give. That reasoning identified the cause
correctly and then stopped at the wrong place: a documented hole through which
malformed input silently disables a guarantee is still a hole.

The carrier contract now answers with three states rather than two —
`RawOperationKey::{Absent, Present(&str), Unreadable}` — and
`OperationKeyRejection` gained a matching `Unreadable` reason. Resolution treats a
supplied-but-unusable value the way it treats an invalid one: rejected under **every**
mode, because the compatibility variant loosens only what happens when a caller sent
*no* key, and a caller who sent unreadable bytes did send one.

`Unreadable` is deliberately not folded into `Invalid`: no `OperationKeyError`
describes it, since that type judges a string's validity and this value never became a
string.

The widening was accepted rather than deferred because the gap was *discovered by*
building the first real adapter, which is exactly when a contract's missing case
surfaces. Deferring it would have merged an adapter that documents its own unsound
path. Consequences: the contract and its policy in `service-sdk`, the conformance
harness in `testkit`, and the carrier in `transport` all moved together, and the
exhaustive match in the transport error mapping caught the missing arm at compile
time rather than in review.

### Review correction — the harness promised three states and exercised two

`assert_carrier_conformance` was updated for the three-state contract's *types* but
not its *rules*: it still passed only a with-key and a without-key instance, so
`Unreadable` was never resolved and the rule that it is rejected under **both** modes
— the entire point of adding the state — went unexercised. A harness that advertises
conformance to a contract it does not fully drive is worse than no harness, because it
converts an untested rule into an apparently tested one.

It now takes a third instance and asserts both rows: `Unreadable` under the mandatory
mode and under compatibility, each yielding `OperationKeyRejection::Unreadable`. It
also asserts the third instance reports the state it was passed as, and that all three
instances agree on the carrier name.

The third parameter is mandatory rather than optional, and that is a deliberate trade
recorded in the function's own doc. A carrier whose location physically cannot hold an
unreadable value cannot supply one and therefore cannot use the harness unchanged. The
alternative was an opt-out, which would let any adapter skip the case silently — and a
harness that can be satisfied without exercising a rule is how a contract quietly stops
being enforced. Requiring it makes the gap visible at the call site.

A fifth harness test covers the precondition itself: an instance passed as unreadable
that actually reports absent fails conformance, since collapsing those two is the exact
defect the state was introduced to prevent.

`crates/transport/tests/idempotency_carrier.rs` now builds that third instance from
**real non-UTF-8 bytes** rather than a stand-in, so the hole that motivated the whole
widening is proven end to end through the actual HTTP adapter.

The status table gained its third case. The exhaustive match already guaranteed
`Unreadable` was mapped at all; what was missing was the assertion that it collapses to
the *same* status as the other two, which is the claim the table makes.

---

## Phase B1e: carriage of the operation key from ingress to the actor — COMPLETE

Branch: `feat/prod-012-b1e-key-carriage`, off
`feat/prod-012-idempotency-tracker` at `fe7e89c`, targeting the tracker branch
per the hybrid chain strategy. B1.6/B1.7 (the HTTP carrier) landed in B1d. This
slice does two separate things, and calling them one continuous path is exactly
the overclaim corrected further down: it **prepares** `ServiceContext` to hold the
key, and it **separately proves** that a key already present on a `CommandContext`
travels through `EntityActor` into `handle_command` unchanged.

Nothing joins those two halves here. The bridge — reading
`ServiceContext::operation_key()` to construct the `CommandContext` a service hands
down — is recorded as B6.4a, because the generated slot-3 code is already the point
that reads the resolved key.

Scope: exactly three tasks — B1.8, B1.9, B1.10. No reservation store, no
receipts, no `EventStore`, no schema, no migrations, and no policy change —
the enforcement mode and resolution rules from B1c are untouched; this slice
only carries an already-resolved value.

- [x] B1.8 RED: `crates/service-sdk/src/context/mod.rs` — three tests written
      against methods that did not exist yet: `operation_key()` defaults to
      `None`, round-trips the identical `OperationKey` handed to
      `with_operation_key`, and coexists with `tenant_hint`/`trace_context`
      without disturbing either. Confirmed genuine RED before implementation:
      `cargo test -p ego-service-sdk --lib context::` failed with
      `E0599: no method named 'with_operation_key' found` at both call
      sites — the accessor and builder simply did not exist, not a runtime
      assertion failure.
- [x] B1.9 GREEN: added a private `operation_key: Option<OperationKey>` field
      to `ServiceContext`, plus `with_operation_key` (consuming builder) and
      `operation_key()` (accessor), and wired both into `ServiceContext::new`
      and the hand-rolled `Debug` impl. Deliberately matches the posture
      already established for `tenant_id`/`tenant_hint()` and the private
      `trace_id` — a private field reachable only through the builder/accessor
      pair, so nothing can reach for a raw field and mistake a stale or
      reconstructed value for the one actually carried from ingress. Read
      both existing fields' doc-comments and code before writing this one and
      followed the same shape rather than inventing a third pattern.
      `cargo test -p ego-service-sdk --lib context::` → 26 passed (23
      pre-existing + 3 new).
- [x] B1.10 RED/GREEN: added a `pub operation_key: Option<OperationKey>`
      field to `crates/persistent-entity/src/command_context.rs::CommandContext`,
      matching that struct's own existing convention of plain public fields
      (`tenant_id`, `expected_version`, `causation_id`, `metadata` are all
      `pub`) rather than importing `ServiceContext`'s private+builder posture
      onto a type whose own established shape is different — the design's
      own data-flow sketch shows this field set by direct struct literal
      (`CommandContext{ operation_key: K }`), which only a public field
      permits. Two tests added to `crates/persistent-entity/src/actor.rs`,
      both driving `EntityActor::execute_command` directly (the same
      already-established pattern `build_effect_actor` and its neighbouring
      tests use) rather than only constructing a `CommandContext` and reading
      the field back: a `RecordingContextHandler` captures
      `context.operation_key.clone()` inside its own `handle_command` into an
      `Arc<Mutex<Option<OperationKey>>>`, and each test asserts what the
      handler actually saw, not merely what was passed in. One test sets a
      specific key at construction and asserts the handler observed that
      *exact* value; a second sets no key and asserts the handler observed
      `None` rather than a stale value left over from a prior call. Confirmed
      genuine RED before the field existed: `cargo test -p persistent-entity
      --lib actor::tests::command_context_operation_key` failed with
      `E0609: no field 'operation_key' on type 'CommandContext'` at both the
      read inside `handle_command` and the write in the test — the field
      itself did not exist yet, not a value mismatch.

### The trap named in the apply instructions, and how this slice avoids it

`CommandContext` already carries three fields — `expected_version`,
`causation_id`, `metadata` — that are constructed as `None`/empty at every
real call site (`CommandContext::new(entity_type)`, used verbatim in
`examples/reference-app/src/application.rs` and every reference-app test) and
never read anywhere in `crates/persistent-entity`. `execute_command` passes
its own `self.version` for optimistic concurrency, not
`context.expected_version`. A test that only proves `operation_key` exists on
the struct, or that a `CommandContext` can be constructed carrying one, would
have been the fourth such field — passing trivially regardless of whether the
value ever reaches anything downstream.

Both B1.10 tests instead route the value through the real `execute_command`
call into a handler's `handle_command`, and assert on what the handler
observed rather than on what was constructed. Neither test can pass by
accident: removing the pass-through at `execute_command`'s
`self.entity_handler.handle_command(&command, &current_state, &context)`
call site, or reintroducing a hand-rolled context that drops the field, would
fail the "same value" assertion (or leave the `Mutex` holding the sentinel
value the second test deliberately seeds it with) — not merely fail to
compile.

### Why B1.9's field is private but B1.10's is public

Not an inconsistency — the two structs already disagreed before this slice
touched either of them. `ServiceContext` established private+builder+accessor
for every identity-carrying field it has (`tenant_id`/`tenant_hint()`,
`trace_id`) specifically to prevent a caller reaching for a raw field and
mistaking an ingress hint for a resolved value; the apply instructions named
this posture explicitly for B1.9. `CommandContext` has no such precedent —
every one of its six fields, including the three dead ones, is `pub`, and the
design's own data-flow sketch constructs it as a literal
(`CommandContext{ operation_key: K }`), which a private field would forbid
from outside the crate. Importing `ServiceContext`'s posture onto
`CommandContext` would have been inventing a new convention for one field in
a struct that has never used it, not following an existing one.

### Compile-error RED, not runtime-assertion RED

Both RED steps in this slice are the compiler refusing to build — `E0599` for
the missing `ServiceContext` methods, `E0609` for the missing `CommandContext`
field — rather than a test that compiles and then fails an assertion. This
matches the established pattern for "this doesn't exist yet" scenarios used
throughout B0/A1/A4/B1c in this same file, and was confirmed directly: each
RED was captured by temporarily reverting only the production-code half of
the change (the field/method it depends on) and re-running the exact test
that exercises it, reading the compiler's error text rather than assuming the
test would fail for the intended reason.

### Struct-literal call sites updated, not left broken

Adding a field to a struct whose existing convention is direct struct
literals (no `..Default::default()` anywhere in this crate) breaks every
existing literal construction site until each one is updated. Four such sites
existed before this slice, none of them new: `crates/persistent-entity/src/
testing.rs::create_test_context()`, `crates/persistent-entity/src/
command_envelope.rs`'s two test-module literals, and `crates/persistent-entity/
src/persistent_entity.rs`'s `ctx()` test helper. Each gained a single
`operation_key: None,` line to keep compiling. These four lines are
mechanical compiler-forced upkeep, not new test assertions — the traversal
guarantee itself is carried entirely by the two new tests in `actor.rs`.

### Review budget — well within the 400-line budget

249 inserted lines across six files, zero deletions — no `size:exception`
needed.

| File | Production | Tests | Wiring (mechanical struct-literal upkeep) |
|---|---|---|---|
| `crates/service-sdk/src/context/mod.rs` | 40 | 38 | — |
| `crates/persistent-entity/src/command_context.rs` | 9 | — | — |
| `crates/persistent-entity/src/actor.rs` | — | 158 | — |
| `crates/persistent-entity/src/command_envelope.rs` | — | — | 2 |
| `crates/persistent-entity/src/persistent_entity.rs` | — | — | 1 |
| `crates/persistent-entity/src/testing.rs` | — | — | 1 |
| **Total** | **49** | **196** | **4** |

49 + 196 + 4 = 249, matching `git diff --numstat` exactly. `command_envelope.rs`'s
and `persistent_entity.rs`'s changed lines sit inside pre-existing `#[cfg(test)]`
modules; `testing.rs::create_test_context()` is test-support infrastructure
(the module is `pub mod testing` but named and used exclusively as a fixture
builder), not a change to any code the actor's real dispatch path executes in
production.

### TDD Cycle Evidence

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| B1.8/B1.9 | `crates/service-sdk/src/context/mod.rs` | Unit (in-crate), hermetic | ✅ 23/23 pre-existing `context` tests unaffected | ✅ `E0599: no method named 'with_operation_key' found` for both the accessor and the builder | ✅ 3/3 passed after adding the field, builder, accessor, `new()` wiring, and `Debug` field | ✅ 3 cases: default absence, exact round-trip, and non-interference with `tenant_hint`/`trace_context` | ➖ None needed — mirrors an existing, already-reviewed pattern verbatim |
| B1.10 | `crates/persistent-entity/src/actor.rs` | Unit (in-crate), real actor-spawn call path (`EntityActor::execute_command`, not a hand-rolled shortcut) | ✅ 41/41 pre-existing `actor` tests unaffected | ✅ `E0609: no field 'operation_key' on type 'CommandContext'` at both the read inside `handle_command` and the write in the test | ✅ 2/2 passed after adding the field to `CommandContext` and updating the four pre-existing struct-literal call sites | ✅ 2 cases: a specific key set at the boundary reaches the handler unchanged; an absent key reaches the handler as `None`, not a stale prior value — proves both the positive and negative path of "identical value, no reconstruction" | ➖ None needed — minimal field addition, no new abstraction |

### Work Unit Evidence

| Evidence | Value |
|---|---|
| Focused test command and exact result | `cargo test -p ego-service-sdk --lib context::` → 26 passed, 0 failed. `cargo test -p persistent-entity --lib actor::tests::command_context_operation_key` → 2 passed, 0 failed. |
| Runtime harness command/scenario and exact result | `cargo test -p persistent-entity --lib` → 43 passed, 0 failed, including both new tests driving `EntityActor::execute_command` directly — the same real actor-dispatch call `EntityRef`'s production spawn path uses, not a hand-rolled bypass. No Docker, no external resource: the actor is constructed in-process with an in-memory `PersistenceFacade` and `NoopPublisher`, exactly matching the existing `build_effect_actor` pattern this file already uses for its other actor-level tests. |
| Rollback boundary | Revert the one commit. Drop the `operation_key` field, builder and accessor from `ServiceContext` (`crates/service-sdk/src/context/mod.rs`); drop the `operation_key` field from `CommandContext` (`crates/persistent-entity/src/command_context.rs`) and its four struct-literal call sites; delete the two new tests and the `RecordingContextHandler` helper from `crates/persistent-entity/src/actor.rs`. No schema, no migration, no persisted state — every new symbol is unreferenced by any other crate as of this slice — nothing reads `ServiceContext::operation_key()` yet, which is precisely why this slice does not close the phase end to end and why the bridge is recorded as B6.4a. |

### Full Verification

#### UNIT — the gate for this slice

Hermetic: in-memory only, no clock dependency, no external resource, no Docker.

- `cargo test -p ego-service-sdk`: **382 passed** across all targets, 0 failed
  (263 lib — 260 pre-existing + 3 new — plus 119 across integration-test
  binaries and one doc-test, all pre-existing and unaffected).
- `cargo test -p persistent-entity`: **84 passed** across all targets, 0
  failed (43 lib — 41 pre-existing + 2 new — plus 41 across integration-test
  binaries and one doc-test, all pre-existing and unaffected).

#### Static gates — compile and lint only

- `cargo fmt --all -- --check`: clean, exit 0.
- `cargo check --workspace --all-targets`: clean, exit 0.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, exit 0, zero
  warnings.
- `cargo run -p xtask -- verify-layers`: `verify-layers: OK (17 crates, 0
  violations)` — unchanged crate count; confirms `persistent-entity` did not
  gain a dependency on `service-sdk` (both sides only ever reference
  `ego-domain`'s `OperationKey`).
- `cargo run -p xtask -- verify-isolation`: `verify-isolation: OK (17 crates
  checked in isolation)`.
- `cargo run -p xtask -- verify-hygiene`: `verify-hygiene: OK (no un-archived
  duplicates)`.

`cargo test --workspace` was **not** run, per this run's explicit
instructions — Docker/testcontainers/PostgreSQL/network are all out of
bounds for this slice's validation.

### Status

3 tasks complete (B1.8, B1.9, B1.10). Combined with everything prior:
**28 of 93**.

**Phase B1's ten tasks are all delivered, and B1 is still not end to end.** Those
are not in tension: no task in the phase ever specified the bridge between the two
contexts. What exists after this slice is the `OperationKey` type, one shared
definition of validity and of the missing-key policy, one HTTP adapter conforming
to that contract, and both context types able to hold and hand on the value.
Nothing reads `ServiceContext::operation_key()` to construct the `CommandContext`
a service passes down, so the value cannot yet travel from the transport edge to
the actor.

An earlier version of this record claimed otherwise, and the claim was wrong in a
way worth naming: the rollback note in this same section already admitted "nothing
wires `ServiceContext::operation_key()` into `CommandContext` yet" while the status
line above it said the phase closed end to end. The evidence contradicted itself
within one section.

The gap is recorded as **B6.4a** in `tasks.md`, which raised the task total from 92
to 93. It belongs in the generated slot-3 code because that is already the point
which reads the resolved key — not in a bridge invented here to make a sentence
true.

Next in the chain: B2 is already complete; the next open work is B4 (async
`EventStore` unit of work, needs A1 and A4 — both landed) and the F1 follow-ups.
B3 onward waits on schema.

### Correction applied before commit — identifier-free comments

The first draft of this slice's two new test-section comments (in
`context/mod.rs` and `actor.rs`) named the ticket directly
("carriage of the caller-supplied operation key" prefixed with the
project code). Caught by scanning the diff against the project's own
convention before committing — the same convention the B1c record above
already documents nineteen violations of — and rewritten to describe only
the behaviour, with no ticket, phase, or decision identifier anywhere in
either comment. Verified by re-scanning the final diff for the forbidden
patterns after the edit, with zero matches.

---

## Phase A2, first slice: the type accessor — COMPLETE

Branch: `feat/prod-012-a2i-aggregate-type-column`, off the tracker.

- [x] A2.2 RED: `EntityTriple` exposes the entity type through an `aggregate_type()`
      accessor, with tests recording that the joined form cannot be reversed in
      general — two different type/id pairs produce the identical joined string when a
      type name contains the separator.

**37 lines of Rust, purely additive.** No schema change, no SQL, no `EventStore`
change, and `aggregate_id()` keeps its exact current meaning — the new test only
*calls* it, to demonstrate the collision.

### The blocker that reshaped this slice

A first version of this slice also carried the `EventStore` signature change, the
nullable column and the write-path switch, on the reasoning that new writes would then
record the type immediately and the debt would stop growing. **Review rejected it, and
was right.**

`load` and `append`'s version check would have queried
`aggregate_type = $1 AND aggregate_id = $2` while historical rows still held `NULL` and
a joined `"user-7"`. Neither condition matches. One of them fails for the same
three-valued-logic reason this change already documents twice elsewhere: a comparison
against `NULL` is never true.

The consequence is not degraded reads. Every historical stream reads as absent,
`append` computes version 0 from `COALESCE(MAX(version), 0)`, the actor concludes the
aggregate is new, and it writes a **second, forked stream** under the split identity
while the original rows sit orphaned and unreachable. Once traffic has passed through
that window there is no clean revert — two partial histories exist for one aggregate.

The PR body had claimed the opposite: "nothing has been rewritten, so there is no data
to restore — which is precisely the property that makes this half safe to land ahead of
the other." The migration rewrites nothing; the *running code* writes divergent
history. That distinction is the whole of it.

### The signal that was missed

In the unsafe version, the inherited characterization tests had to be **edited** to
keep compiling, and that was recorded as mechanical fallout. It was not. A
characterization test exists to pin current behaviour; needing to adapt one is the test
reporting that the behaviour changed. That was the alarm and it was read as noise.

In this slice those tests pass **untouched**, and the file is not in the diff. That is
now the stated evidence that the slice is additive — a property to check rather than
assert.

### What moved to the second slice

The column, the preflight and its four aborts, the backfill, the coordinated
switch-over of read and write identity, the post-verification, `SET NOT NULL`, the
reverse operation, the report, the runbook, and the real-PostgreSQL suite. One
transition, one moment of change, no window between two states.

### UNIT — hermetic

- `cargo test -p persistent-entity --lib`: **45 passed**, 0 failed.
- `cargo test -p ego-persistence --lib`: **6 passed**, 0 failed.

### INTEGRATION — inherited, and unmodified

- `cargo test -p ego-integration-tests`: **3 passed**, 0 failed, against real Postgres.
  The test file is **not** in this slice's diff. That is the point.

### Static gates

`fmt`, `check --workspace --all-targets`, `clippy -D warnings` and the three
`xtask verify-*`: all pass. `cargo test --workspace` was not run.

---

## Phase A2, second slice: the coordinated transition — COMPLETE

Branch: `feat/prod-012-a2ii-coordinated-transition`, off the tracker.

Everything that touches schema or data, landing as one transition with no window
between two states: the column, the preflight and its four aborts, the transactional
backfill, the switch-over of read and write identity, the post-verification,
`SET NOT NULL`, the reverse operation, the report, the runbook, and a fail-closed
open-time check.

- [x] A2.1, A2.7 — preflight, before a single row is written. The scan runs, every row
      is classified in memory, and the function **returns before the first `UPDATE`**.
      Four aborts, each naming the offending rows: no registered type matches, more than
      one matches, the bare identifier is empty or whitespace, and the post-split
      identity would collide with another row's.
- [x] A2.3 — migration `007` adds the nullable column; the operator tool performs the
      forward step, which is not derivable from data alone.
- [x] A2.4 — the switch-over: `EventStore` carries the type alongside the id
      **synchronously**, the write path uses the structural identity, and the column
      becomes mandatory. One step, because activating the identity before the data is
      transformed forks history and making the column mandatory before the switch-over
      would reject writes from the old path.
- [x] A2.5 — the reverse operation rejoins exactly what was split and drops the column,
      inside one transaction.
- [x] A2.8 — post-verification, after the `UPDATE`s and before `SET NOT NULL`, inside the
      same transaction: row count unchanged, post-split identity unique **as written**, and
      per stream the versions the consecutive run `1..=n`. Any failure rolls the whole
      transaction back. Referential integrity is deliberately not checked: no migration
      declares a foreign key, so a check would imply one exists.
- [x] A2.9 — machine-readable report and a runbook binary that exits non-zero on abort,
      so a pipeline can branch on the exit code without parsing the report.
- [x] A2.10 — the fail-closed open-time check.

### The guard, and why documentation was not enough

Review found that nothing sequenced the backfill against the new code serving traffic.
`migrations::run` adds the column at startup; the new queries then filter on the type.
Deploying the new binary before running the backfill reproduces the exact hazard that
got the first slice rejected — every historical stream reads as absent, the version check
returns zero, and an append forks history while the original rows sit orphaned.

"It is in the runbook" is the same class of guarantee already rejected once in this
change. So the store now **refuses to open** while any row has `aggregate_type IS NULL`.

Three properties, all required and all met:

- It runs **after** the migration and **before** any store operation is possible. That
  is enforced by the constructor's shape rather than by discipline: `open` returns a
  result, so on the unmigrated path no store value exists and there is nothing to call a
  read or an append on.
- It runs on **every** open, with **no cached flag**. A cached answer goes stale exactly
  when an old writer inserts one more untyped row mid-transition — the case worth
  catching.
- It fails with an explicit message that says *why*, not merely that it refused.

`open` is now the only constructor. Leaving an unchecked one available would have made
the guard advisory.

**Cost, stated:** one existence query per open. That buys refusing to operate in a state
where correctness is not achievable.

### The runbook order, recorded in the binary itself

```
1. quiesce the old writers   — stop every process still writing untyped rows
2. run the tool              — applies the migration, then the backfill
3. read the report           — a non-zero exit means nothing was written
4. the tool has already set the column mandatory on success
5. start the new binary      — only now
```

Step 1 is the easiest to skip and the most damaging: while an old instance still inserts
untyped rows, the tool can commit a table that was complete when checked and incomplete a
moment later. The open-time check then refuses to start the new binary, which is the
intended outcome but turns a clean transition into an outage to diagnose.

The check does not replace the order. It makes getting the order wrong visible and
recoverable instead of silent.

### UNIT — hermetic

- `cargo test -p ego-domain --lib`: **219 passed**, 0 failed.
- `cargo test -p ego-persistence --lib`: **11 passed**, 0 failed.
- `cargo test -p persistent-entity --lib`: **45 passed**, 0 failed.

### INTEGRATION — real PostgreSQL

- `cargo test -p ego-integration-tests`: **8 + 3 passed**, 0 failed. The eight cover the
  four aborts, the clean path with row-count and stream-integrity preservation, the
  zero-row case, the exact revert, and the open-time guard refusing and then admitting.
  The three are the inherited characterization tests, adapted here because this is the
  slice where the identity genuinely changes — which is exactly what that adaptation is
  supposed to signal.

### Static gates

`fmt`, `check --workspace --all-targets`, `clippy -D warnings` and the three
`xtask verify-*`: all pass. `cargo test --workspace` was not run.

### Review budget — accepted exception

Roughly 1,100 lines. The exception was accepted before the work started, on the grounds
that this slice concentrates the dangerous operation and its entire proof, and that the
additive half had already been split out to keep it reviewable on its own. The guard
added to that total and is minimal logic directly required for the migration to be safe.


### Review round two — A2.8 was claimed and not implemented

Review of #262 found the blocker: the flow was `SELECT` + preflight → `UPDATE` → `SET NOT
NULL` → `commit`, with **nothing in between**. The post-verification A2.8 committed to did
not exist. The count was never re-read, uniqueness was checked only over the values
computed in memory before the writes, and per-stream version continuity was checked
**nowhere at all**. A historical stream holding versions 1 and 3 would have been
consolidated. The clean-path test passed because its fixture is well-formed, which is
exactly why it could not catch this.

Corrected: three checks now run after the writes and before the column becomes mandatory,
against the rows as they were actually written, and any failure rolls the transaction back.

Two consequences worth naming, because neither is cosmetic:

- **`RolledBack` is a new outcome, distinct from `Aborted`.** In a preflight abort no row
  was ever written and the guarantee comes from the ordering; in a rollback rows *were*
  written and the guarantee comes from discarding them. Both leave the table unchanged,
  but they are different events for an operator, and effects a rollback does not undo — an
  advanced sequence — belong only to the second. Collapsing them into one variant would
  have repeated the imprecision already corrected once in this slice, where a comment
  called an unwritten transaction a "rollback".
- **Both post-verification queries compare stream identity with `IS NOT DISTINCT FROM`,
  not `=`.** `tenant_id` is nullable and `NULL = NULL` is `NULL`, so an untenanted stream
  would never match itself and its violations would have gone unreported — a check that
  silently exempts exactly the rows it was meant to examine. `GROUP BY` already treats
  nulls as equal; only the join back to the offending rows needed it.

Why per-stream continuity is the right check and not an arbitrary strictness: the split
changes which rows belong to which stream, so a mis-partition shows up as a stream missing
version 1 or carrying a hole. It is the property that proves the transformation preserved
history's shape. The tool cannot distinguish a hole it created from one already in the
data, so it refuses either way — consolidating on the assumption that the gap was
pre-existing is precisely the guess this migration must not make.

New test: `a_stream_with_a_version_gap_rolls_the_whole_transformation_back`. Every row in it
splits cleanly, so preflight has no objection and the rewrite genuinely runs; the refusal
comes from reading the written rows back. It asserts three separate things, because "it
refused" is not the same claim as "nothing changed": the outcome names both rows of the
discontinuous stream and only those, every row is still in its joined pre-split form
including the well-formed stream alongside it, and the column is **still nullable**. That
last assertion is the load-bearing one — a column left mandatory would be a consolidated
fragment of a transition that is supposed to be all-or-nothing.

Also corrected: `tasks.md` claimed 29 complete and 74 pending while this slice marks nine
more. Now 38 and 65, matching both the checkbox count and the PR body.

Re-verified after the correction: integration **9 passed** (the eight above plus the new
one), plus the 3 characterization tests. Unit 219 / 11 / 45. `fmt`, `check`, `clippy -D
warnings` and the three `xtask verify-*` clean.
---

## Phase A3, first slice: null-safe tenant comparison — COMPLETE

Branch: `feat/prod-012-a3-event-uniqueness`, off the tracker at `3f0f5aa`.

A3 turned out to have a prerequisite that was not in the task list, so it ships as its own
slice ahead of the unique indexes.

### The defect

`resolve_tenant(None)` resolves the systemwide mode to SQL NULL, and all three of the
store's queries compared `tenant_id` to that bound parameter with plain `=`. In SQL's
three-valued logic `tenant_id = NULL` is unknown, never true — for every row, including the
rows whose tenant genuinely is NULL. A systemwide stream was therefore invisible to its own
reads:

- the version check inside `append` always read an empty history,
- `load` always reported the aggregate absent,
- `list_aggregate_ids` never listed it.

The consequence is worse than invisibility. Because the version check always returned 0,
every systemwide append at `expected_version = 0` succeeded and wrote version 1 **again**.
History duplicated silently, with no error anywhere. The RED run demonstrated exactly that:
the duplicate append returned `Ok(1)` instead of a conflict.

Pre-existing, not introduced by A2-ii: `git show e87018a` has the same `tenant_id = $2`.
A2-ii preserved it while adding the type column.

### Why it went unnoticed, which matters more than the fix

The in-memory implementation of the same port keys streams by
`(String, String, Option<String>)` in a `HashMap`, and in Rust `None == None`. So the
in-memory store has always handled the systemwide partition correctly while the Postgres one
did not. **Two implementations of one port disagreed, and the hermetic suite agreed with the
correct one.** Any test written against the in-memory store would have reported systemwide
mode as working.

That is an argument for a shared conformance suite over the `EventStore` port, the way A2
built one for the carrier contract. Registered as debt below rather than done here.

### The fix

All three queries now use `tenant_id IS NOT DISTINCT FROM $n`, which compares two NULLs as
equal while keeping NULL distinct from any concrete tenant. That second half is not
incidental — a comparison that matched NULL against everything would have made the first two
tests pass for entirely the wrong reason, and a systemwide read returning another tenant's
events is an isolation breach strictly worse than the invisibility being fixed. There is a
test for it.

This is the same defect class corrected in A2-ii's post-verification queries. It is now in
every `tenant_id` comparison the store makes.

### Why this must precede A3.2

`CREATE UNIQUE INDEX` fails on a table that already holds duplicates. The duplicates in the
NULL partition are produced by exactly this defect, so adding the indexes first would turn a
silent data problem into an opaque boot failure. Fixing the comparison stops new duplicates;
A3.2 still needs a story for ones already there, which is the second slice's job.

### Verification

- INTEGRATION, real PostgreSQL: **5 new + 9 + 3 = 17 passed**, 0 failed. RED first: all five
  new tests failed before the fix, for the predicted reasons.
- UNIT: ego-domain **219**, ego-persistence **11**, ego-infrastructure **24**,
  persistent-entity **45**. All pass.
- STATIC: `fmt`, `check --workspace --all-targets`, `clippy -D warnings`, and the three
  `xtask verify-*` all clean. `cargo test --workspace` not run.

The characterization suite's module note, which had recorded this gap and deferred it to
"the uniqueness work", is updated to say it is closed and where it is now pinned. Its own
tests are untouched — they use a concrete tenant, which was never the broken path.

### Debt found, deliberately not fixed here

1. **Migrations 004, 005 and 006 are orphans.** The SQL files exist but no runner executes
   them — `migrations()` registers only 001, 002, 003 and 007 — and no code queries
   `read_side_offsets`, `processed_events` or `projection_state`. Dead schema, the same
   pattern as the dead `.gitlab-ci.yml` found earlier. Out of A3's scope.
2. **No conformance suite over the `EventStore` port**, which is why the divergence above
   survived.
3. **`list_aggregate_ids` still filters `aggregate_type IS NOT NULL`** with a comment saying
   the column may still be NULL. After A2-ii's guard and `SET NOT NULL` it cannot be. The
   filter is harmless; the comment is now false. One-line follow-up, not mixed into this
   diff.

### Debt from this slice, closed rather than carried

All three items registered above are fixed in the same slice.

**1. The `EventStore` port has a conformance harness.**
`crates/testkit/src/event_store.rs` states the identity half of the contract once — version
advance, stale-version rejection, ordered readback, absent-stream reporting, the systemwide
and tenant partitions staying separate under a shared type and id, and the per-partition
listing. Both adapters are judged against it: the in-memory store hermetically in
`crates/infrastructure/tests/`, the PostgreSQL store against a real database in
`crates/integration-tests/`.

The harness was verified to have teeth rather than assumed to. With the null-safe comparison
temporarily reverted, the PostgreSQL run fails on exactly the divergence:

```
a systemwide stream must see the history it just wrote: appending at expected version 1
must succeed, not be rejected as though the stream were empty:
Conflict { aggregate_id: "conformance-shared-identity", expected: 1, actual: 0 }
```

while the in-memory store passes the identical assertions. That is the divergence
reproduced, not described. The fix was then restored and the diff confirmed to hold only the
intended change.

Deliberately not asserted: durability, concurrency, snapshotting. A harness that demands
more than the contract turns every adapter into a copy of whichever one it was written
against.

`ego-testkit` is a **dev-dependency** of `ego-infrastructure`, never a build-time one —
testkit is tooling, and no production crate may depend on tooling. `verify-layers` excludes
dev edges for that reason and still reports 17 crates, 0 violations.

**2. The migration registry is checked against the filesystem.**
Migrations 004, 005 and 006 are removed, and `migrations.rs` gains a bidirectional test:
every `.sql` file is registered, and every registration has a file. Plus one asserting the
registry ascends by numeric prefix, since registration order *is* execution order.

Removing rather than registering, and why: the three files had no consumer — no Postgres
read-side adapter exists, only the domain SPI traits — so registering them would create
three unused tables in every deployment for a feature that does not exist. The pattern this
repository now follows is the one 007 used: the migration ships with the code that needs it.
Git keeps them at `e5b4074` for whoever writes that adapter. The reverse call is one review
comment away; the test is the part that matters either way, because it converts "someone
forgot" into a failing test.

**3. `list_aggregate_ids` no longer filters on a condition that cannot occur.**
The `AND aggregate_type IS NOT NULL` filter and its comment claiming rows may still predate
the backfill are both gone. `open` refuses to return a store while any row lacks its type,
and the backfill makes the column mandatory in the database, so by the time the method is
callable the column is non-null for every row. A guard against an impossible state implies
the state is possible.

### Verification after the debt fixes

- UNIT: ego-domain **219**, ego-persistence **13** (up 2 — the registry tests),
  ego-infrastructure **24**, persistent-entity **45**, ego-testkit **86**. All pass.
- HERMETIC conformance: in-memory store **1 passed**.
- INTEGRATION, real PostgreSQL: **9 + 3 + 1 + 5 = 18 passed**, 0 failed.
- STATIC: `fmt`, `clippy -D warnings`, and the three `xtask verify-*` all clean.

Clippy again caught something the passing test run did not — an unused import in the new
conformance test. Fixed before commit, not deferred.

---

## Phase A3, second slice: database-enforced stream identity — COMPLETE

Same branch as A3-i, third commit. A3 is now fully complete.

### The strategy, chosen by measurement rather than assumption

A single conventional `UNIQUE (tenant_id, aggregate_type, aggregate_id, version)` would have
been wrong, and wrong in the exact place this change already found damage: PostgreSQL treats
every NULL as distinct from every other NULL, so such an index permits unlimited duplicate
identities in the tenant-less partition.

`NULLS NOT DISTINCT` expresses the intent in one index, but it arrived in PostgreSQL 15 and
this workspace declares 14 as its floor (README.md). That was verified against the pinned
`14-alpine` image rather than taken from documentation:

```
ERROR:  syntax error at or near "NULLS"
LINE 1: ...CREATE UNIQUE INDEX ux ON probe (a, b) NULLS NOT DISTINCT;
                                                  ^
pg_index.indnullsnotdistinct present on 14: 0 columns
```

So the equivalent strategy is stated explicitly: two partial unique indexes over
complementary predicates. Every row satisfies exactly one of `tenant_id IS NOT NULL` and
`tenant_id IS NULL`, so together they cover the table with no gap and no overlap — the same
semantics the store's queries express with `IS NOT DISTINCT FROM`.

The systemwide half omits `tenant_id` from its column list, because its predicate already
fixes that column to NULL for every row it contains; including it would index a constant. The
asymmetry is deliberate and pinned rather than left to be rediscovered.

### The `23505` translation, and the lie it used to tell

The mapping to `PersistenceError::Conflict` already existed and was unreachable. Making it
reachable exposed that it reported `actual: current` — and the in-process check immediately
above has *already proven* `current == expected_version`. So the first real conflict this
schema could produce would have claimed the expected and actual versions were the same
number: self-contradictory, and useless to whoever has to act on it.

The aborted transaction cannot be queried, so the stream is re-read on another connection.
That value is a reading taken after the failure rather than at the instant of it, which is
the only thing "actual" can mean once a competing writer exists. Documented as such at the
call site.

### Reaching the branch deterministically

Inside one transaction the violation is unreachable by construction: the version check reads
`MAX(version)`, so no existing row sits where the insert is about to write. It needs a
competing commit between the read and the insert.

The test creates that window without depending on timing. Another connection inserts the
row and **does not commit** — invisible to the version check, but the unique index already
holds its slot. `append` then reads 0, agrees, issues its `INSERT`, and **blocks**. The test
polls `pg_locks` until it observes an ungranted lock, then commits the competing
transaction, at which point the blocked insert fails with `23505`.

The blocking observation is also what identifies *which* guard fired: had the in-process
check caught this, `append` would have returned immediately and nothing would have waited.

### Both new guarantees verified to have teeth

Neither was assumed to work. Each was broken on purpose and observed to fail:

1. **Restoring the old `actual: current`** — the deterministic test fails with `left: 0,
   right: 1`. That `0` is positive proof the `23505` branch executed, since the version check
   had read the stream as empty and passed.
2. **Unregistering migration 008** — four of the five uniqueness tests fail, and the
   migration-registry guard added in A3-i objects by name:
   `these migration files exist but no code runs them: ["008_events_stream_identity_unique"]`.
   The fifth, "two tenants may hold the same identity", correctly keeps passing: it does not
   depend on the index.

Both reversions were then restored and the diff confirmed to hold only intended changes.

### The characterization test did what it was written to do

`events_table_provides_no_uniqueness_guarantee_for_the_stream_identity_today` failed on this
slice with `left: 3, right: 1` — exactly as its own failure message instructed: *"update this
test to assert the new guarantee instead of the gap"*. Rewritten, not deleted, so the file
still records the transition. It now asserts only that a unique guarantee exists; the precise
shape lives in `schema_index_assertion.rs`, because two places to update is one place to
forget.

### A defect found in already-merged code

The event store's open-time refusal message had lost its line continuations and shipped with
runs of eighteen spaces inside an operator-facing string. It is in the merged tracker — it
went in with A2-ii and was not caught in review either. Restored, and verified by
reconstructing the runtime value rather than eyeballing the source. A scan of every string
literal in `crates/` for interior space runs found no others. I do not have a confirmed cause
and am not inventing one.

### Held out of scope, as instructed

No receipts, no reservations, no fencing, no async surface, no unit of work. The store stays
synchronous and gains no handle.

### Verification

- INTEGRATION, real PostgreSQL: **9 + 3 + 1 + 3 + 5 + 5 = 26 passed**, 0 failed.
- HERMETIC conformance: in-memory store **1 passed**.
- UNIT: ego-domain **219**, ego-persistence **13**, ego-infrastructure **24**,
  persistent-entity **45**, ego-testkit **86**.
- STATIC: `fmt`, `clippy -D warnings`, `verify-layers` (17 crates, 0 violations),
  `verify-isolation`, `verify-hygiene` — all clean. `cargo test --workspace` not run.

Clippy caught a `type_complexity` violation the passing suite did not — the third slice in a
row where a green test run coexisted with a failing static gate. Fixed with a named type
alias before commit.

**A3 complete. 106 tasks, 46 complete, 60 pending.**

---

## Phase B4, first slice: the asynchronous `EventStore` — COMPLETE

Branch: `feat/prod-012-b4i-async-event-store`, off the tracker at `c3e1dd4`.

B4 ships in three slices. This one changes the contract and every caller, and adds no new
capability: no unit of work, no receipts, no metadata column. Behaviour is meant to be
identical, and the pre-existing suite is the evidence.

### The trait

`append`, `load` and `list_aggregate_ids` are now `async`. `stream_version_offset` stays
synchronous: it reports a static property of how a store was configured, has no fallible
path and no I/O, and no implementation consults storage to answer it — making it async would
add a boxed future per call to describe a constant.

`#[async_trait]`, not native `async fn` in trait. Native `async fn` is stable and unusable
here: the trait is consumed as `dyn EventStore<E> + Send` behind a shared lock, and a native
`async fn` makes a trait non-dyn-compatible. The cost is one allocation per call; the
alternative is losing the trait object every caller depends on.

### What the bridge was costing

`PostgreSQLEventStore` presented a synchronous surface over an asynchronous driver by
wrapping each method body in `block_in_place` + `block_on`. That never removed the wait — it
only hid where the wait happened, pinned a runtime worker for the duration of every round
trip, and made the store panic outright on a current-thread runtime.

That last part had leaked out of the storage layer and into test attributes. Three test files
carried `flavor = "multi_thread"` with comments explaining it was "load-bearing" because of
`block_in_place`. With the bridge gone those comments were false, so they are corrected and
the files now run on the default current-thread runtime — which is the demonstration, not
just the claim. Two tests keep `multi_thread` and their comments now give the real reason:
one drives a competing transaction while an append is blocked, the other needs a race to
actually be contested.

### The lock had to change, and only one of them

`PersistenceFacade` holds its stores behind `Arc<Mutex<dyn ...>>`. `parking_lot`'s guard is
not `Send`, so holding it across an `.await` makes every future that touches the store
non-`Send` and therefore unspawnable. The event-store lock is now `tokio::sync::Mutex`, which
yields a `Send` guard and, like `parking_lot`, does not poison — so the original
non-poisoning rationale is preserved, not discarded.

The snapshot lock stays on `parking_lot`. `Snapshot` is still synchronous, so its guard has
nothing to hold across; converting it now would add an `.await` to acquire a lock nothing
waits on. The asymmetry is deliberate and documented at the field, so it reads as a decision
rather than an oversight.

This is a **public API change**: `with_event_store` and `PersistenceFacade::with_stores` now
take `Arc<tokio::sync::Mutex<dyn EventStore<E> + Send>>`.

### A test double that would have quietly parked a worker

Two doubles in `guaranteed_completion_tests.rs` gate on a `std::sync::mpsc::Receiver`. Two
problems surfaced at once: a receiver is `Send` but not `Sync`, so `&self` could not satisfy
the trait's `Send` future bound; and a blocking `recv()` inside an async method parks a
runtime worker — which `block_in_place` used to compensate for by handing the runtime a
replacement thread, and nothing does now.

Wrapping the receiver in a lock would have fixed the type error and left the second problem
in place. They now use `tokio::sync::mpsc` and await, which is what an async double should
do.

### Scope boundary, stated

`Repository` and `Snapshot` still bridge with `block_in_place` + `block_on` in
`crates/persistence/src/postgres/`. They are separate traits and are not part of B4. The
remaining references to `block_in_place` are those two implementations plus three historical
mentions in comments explaining what the event store used to do.

### Verification

Behaviour-preservation is the claim, so the evidence is the pre-existing suite passing:

- UNIT: ego-domain **219**, ego-persistence **13**, ego-infrastructure **24**,
  persistent-entity **45**, ego-testkit **86**.
- `persistent-entity` integration tests — the actor recovery, activation-ordering,
  persistence-failure and guaranteed-completion paths: **45 + 13 + 5 + 7 + 6 + 6 + 3 = 85
  passed**.
- HERMETIC conformance, now on a current-thread runtime: **1 passed**.
- INTEGRATION, real PostgreSQL: **9 + 3 + 1 + 3 + 5 + 5 = 26 passed**, 0 failed.
- STATIC: `fmt`, `clippy -D warnings`, `verify-layers` (17 crates, 0 violations),
  `verify-isolation`, `verify-hygiene` — all clean. `cargo test --workspace` not run.

Clippy again caught what the suite did not: three now-unused `parking_lot::Mutex` imports.
Fourth slice in a row.

**107 tasks, 48 complete, 59 pending.**

### Review round one on B4-i — two corrections

Both findings were correct.

**1. B4.8 was marked complete while the command its own text requires had not been run.**
The task reads "run full `cargo test --workspace` to catch ripple", and the PR body said in
plain words that the command was not run. A contradiction inside one record, of the same class
already corrected twice in this change.

The available resolutions were to weaken the task or to satisfy it. Weakening it would have
removed exactly the check that matters here: a trait signature change reaches crates nobody
thinks to name, and choosing which crates to run is the author deciding which ripple counts.

So it was run: **112 suites, 1 540 passed, 0 failed, 0 ignored, exit 0.** That is now the
evidence attached to the task, and B4-i is the first slice in this change where the
workspace-wide suite is the stated gate rather than a per-crate selection.

**2. `GatedPanicOnceEventStore::load` still described a mechanism it no longer uses.**
The comment read "Synchronous, explicit wait … Blocks this one Tokio worker thread; the other
7 (`worker_threads = 8`) keep servicing the 100 caller tasks", above a line that is now
`recv().await`.

The arithmetic was right — that test really does use `worker_threads = 8` — and the mechanism
was wrong, which is the worse half. Awaiting *yields* the worker back to the runtime; the 100
callers keep progressing because of that, not because seven spare workers absorb the loss of
one. A reader would have taken the wrong model of why the test works from a comment that
still added up.

Corrected to describe what the code does, and to name what changed: a blocking receive would
park a worker for as long as the gate is held, which is what this did while the store bridged
async to sync, and what `block_in_place` was compensating for.

A scan for the same class of stale claim elsewhere in the tree found none.

The sibling double, `PanicOnLoadEventStore`, was already correct — its comment was rewritten
when the channel was converted. Only this one was missed.

---

## Phase B4, second slice: the unit of work — COMPLETE

Branch: `feat/prod-012-b4ii-unit-of-work`, off the tracker at `e1e8a8f`.

### Why the trait exists at all

`EventStore::append` owns its own transaction and commits before returning. That makes it
complete on its own and useless as a building block: nothing can be made to land atomically
*with* an append, because by the time it hands back the decision is already made. A caller
that needs two writes to share a fate has to hold the transaction open, and only the store can
hand that out.

### Two shape decisions, and what they buy

**`commit` takes `self: Box<Self>`.** A committed unit of work cannot be used again, and the
compiler is what refuses — rather than an implementation discovering a spent transaction at
runtime and having to invent an error for it.

**There is no `rollback`.** Dropping is the rollback, so the safe outcome is the one that
happens on an early return, a cancellation, or a panic — exactly the paths where an explicit
call is what gets missed. An explicit `rollback` would add a second way to say what dropping
already means, and the failure mode it invites is forgetting it.

`begin` takes `&self`, not `&mut self`: handing out a transaction does not mutate the store, so
requiring exclusive access would force every caller behind a lock it does not need.

### `begin` has no default implementation, deliberately

A default would have to either pretend — returning something that commits each append as it
arrives — or fail. Both let an implementation claim transactional semantics it does not
provide. So all nine implementors answer explicitly:

- **Postgres**: one real transaction. Rollback-on-drop is `sqlx`'s, not reimplemented; tracking
  commit state a second time in order to disagree with it would be the only thing to gain.
- **Both in-memory stores**: stage appends and publish on commit, so dropping discards them —
  the same observable outcome as abandoning a transaction, reached by staging rather than by
  rolling back.
- **The no-op store**: a unit of work that discards. That is the store's whole contract, and
  erroring instead would make the no-op facade unusable by any caller that opens one — which is
  the exact situation the no-op facade exists for.
- **The five test doubles**: an explicit refusal. They inject failures into the direct append
  path and no test using them opens a unit of work; if one ever does, the message says what is
  missing instead of the test failing somewhere further away.

### The in-memory version check was the trap

A staging implementation's version check has to count committed events **and** what the unit of
work has already staged. Consulting only committed state makes a second append to the same
stream inside one unit of work fail — and the conformance harness now covers exactly that,
because it is the mistake this shape invites.

### Both guarantees verified to have teeth

Broken on purpose, observed to fail, restored:

1. **Postgres UoW committing eagerly** (pool instead of transaction) — 3 of the 4 unit-of-work
   tests fail: drop-leaves-nothing, isolation, and shared-fate. The fourth, "commit makes
   durable", correctly keeps passing: it does not discriminate between the two behaviours, and
   a test that passes either way is not evidence.
2. **In-memory version check ignoring its own staged appends** — conformance fails with
   `Conflict { aggregate_id: "conformance-committed-uow", expected: 2, actual: 0 }`, which is
   the predicted mistake, reproduced.

### The conformance harness now covers the unit of work

Staged appends invisible until commit, durable after it, discarded on drop, and a second append
in one unit of work seeing the first. Put to **both** implementations, because a staging
implementation and a transactional one can only be trusted to agree if the same assertions are
put to both — and these two already disagreed once about the tenant-less partition while both
satisfying the trait's signature.

### `confirm_receipt` deferred to B5, with reason

B4.2b's text names `append`, `confirm_receipt` and `commit`. Only two shipped. Nothing backs the
third today — no `operation_receipts` table, no receipt type, no caller — verified rather than
assumed. A trait method whose every implementation answers "not yet" is the same
premise-without-backing already trimmed from A4 in this change. Recorded as B5.3a, where the
migration and semantics it needs arrive.

### Found, reported, not fixed here

**`persistent_entity::persistence::InMemoryEventStore` does not partition by tenant at all.**
Its `StreamKey` is `(String, String)` — type and id, no tenant — and every method takes
`_tenant_id` unused. Two streams sharing a type and id in different tenants collide into one.

It matters more than a test-support store normally would, because it is the **default** the
runtime builder installs when no store is supplied. The durable store and the other in-memory
store both partition by tenant; this one silently does not, which is precisely the divergence
class the conformance harness exists to catch — and running the harness against it would fail
immediately, for a reason unrelated to this slice.

Not fixed here: changing the key of the default store alters behaviour for every test that
relies on it, and needs its own verification. B4-ii's claim is transactional semantics.

### Verification

- INTEGRATION, real PostgreSQL: `event_store_uow` **4 passed**; full integration suite
  unchanged and green.
- HERMETIC conformance, extended: **1 passed**. Postgres conformance, extended: **1 passed**.
- WORKSPACE: `cargo test --workspace` — **113 suites, 1 544 passed, 0 failed**, exit 0. Run for
  the same reason as in B4-i: a required trait method reaches every implementor, including ones
  nobody would think to name.
- STATIC: `fmt`, `clippy -D warnings` (clean first time), `verify-layers` (17 crates, 0
  violations), `verify-isolation`, `verify-hygiene`.

**110 tasks, 54 complete, 56 pending.**

### Review round one on B4-ii — a functional blocker I introduced

**`StagingUnitOfWork::append` ignored `version_offsets` while the direct append path includes
them.** Verified at `persistence.rs`: `EventStore::append` computes `stream.len() + offset`; the
unit of work computed `committed + staged`. With offset 5 on an empty stream the direct path
accepts `expected_version: 5` and the unit of work rejects it reporting `actual: 0`.

The worse half is not the arithmetic. **My comment asserted the opposite of what the code it
compared against does** — it claimed a unit of work adding offsets "would answer a different
question than the direct append path does", when the direct path is precisely where the offset is
added. A wrong comment that reads as a justification is harder to catch than a wrong line,
because it tells the next reader to stop looking.

Corrected to `offset + committed + staged`. The offsets are cloned into the unit of work at
`begin`, which is exact rather than approximate: `with_version_offset` is a builder that consumes
`self`, so offsets are fixed before the store can be used and cannot change while a unit of work
is open.

#### The test the divergence needed

`crates/persistent-entity/tests/in_memory_version_offset_parity.rs`, four cases:

1. The **direct** path treats the offset as part of the version — characterizing the behaviour
   the other path has to match, rather than assuming it.
2. The **unit-of-work** path agrees, on the same stream with the same argument.
3. The two paths agree at **every** expected version around the offset, compared **to each
   other** rather than against restated literals. The first two tests each pin one path to
   numbers, which would let both drift together if someone changed the semantics in both places;
   this one fails whenever they disagree, whatever either decides the version is.
4. A stream with **no** declared offset starts at zero through both paths — guarding against a
   fix that reads the wrong key or defaults to something other than zero, which would satisfy the
   first three while breaking every ordinary stream.

Verified to have teeth by restoring the reviewed arithmetic: 2 of the 4 fail, and the parity test
names the divergence directly —

```
the two paths disagreed at expected version 0: direct accepted = false, unit of work
accepted = true
```

The two that kept passing are the direct-path characterization, which the defect does not touch,
and the no-offset case, where there is nothing to diverge about. Both correct.

#### B4.5 split, per the review

It could not stay closed as one item while the default in-memory store ignores tenants:

- **B4.5a** — infrastructure store's unit of work plus conformance: complete. That store
  partitions by tenant, so it is judged against the same tenant-scoped assertions as the durable
  one.
- **B4.5b** — persistent-entity store's unit of work with matching version arithmetic: complete.
- **B4.5c** — tenant-partitioned `StreamKey` for that store and running the harness against it:
  **explicitly pending**. Until it lands, "In-Memory Store Does Not Silently Diverge" holds for
  the infrastructure store only, and the default store is outside the harness. Stated that way in
  `tasks.md` rather than implied by a closed checkbox.

#### Verification after the corrections

- `in_memory_version_offset_parity`: **4 passed**.
- WORKSPACE: `cargo test --workspace` — **114 suites, 1 548 passed, 0 failed**, exit 0.
- STATIC: `fmt`, `clippy -D warnings`, `verify-layers` (17 crates, 0 violations),
  `verify-isolation`, `verify-hygiene` — all clean.

**112 tasks, 55 complete, 57 pending.**

---

## Unplanned slice: recovery of a fresh aggregate — a live defect

Branch: `fix/prod-012-fresh-aggregate-recovery`, off the tracker at `827f507`.

B4.5c's premise — the two in-memory stores and the durable one disagree, and the default store is
outside the harness — turned out to be hiding something worse than a tenant-key mismatch.

### The defect

`PersistenceFacade::load_for_recovery` propagated whatever error `EventStore::load` returned. The
durable store reports an aggregate with no events as `PersistenceError::NotFound`. So recovery of
a never-persisted aggregate failed, which means **no entity could be activated for the first time
against PostgreSQL**.

Reproduced against a real database before any fix was written:

```
an aggregate with no events yet is the ordinary first state of every entity, not a
recovery failure: "aggregate 'counter-never-written' not found"
```

It predates PROD-012 and is live on `develop`.

### Why nothing caught it

Every recovery test wires the in-memory store, and that one returns `Ok(vec![])` for a stream it
has never seen instead of reporting absence. The forgiving implementation was the one under test.

That is the **third** time in this change that two implementations of one port disagreed and the
suite happened to exercise the one that was right — after the systemwide tenant comparison and
the unit-of-work version offsets. The pattern is not that these particular stores are careless;
it is that a port with two implementations and no shared conformance obligation will drift, and
the drift surfaces wherever the durable path is the one nobody tested.

### The fix, and the boundary it must not cross

`NotFound` is absorbed as "no history". Every other error still fails recovery.

That second half carries as much weight as the first. An unreadable stream — dropped connection,
malformed row, rejected payload — is not an empty one. Recovering it as a fresh entity would
start appending from version zero over history the entity never saw, forking a stream that
already exists. The absence case is a normal state; the unreadable case is a failure, and
collapsing them would trade a startup error for silent history divergence.

### A gap in my own tests, found by breaking the fix on purpose

The first teeth check replaced the `NotFound` arm with `Err(_) => Vec::new()` — absorbing
*everything* — and the integration suite **stayed green**. So the tests proved the absence case
and said nothing about the boundary the comment claimed to hold.

`crates/persistent-entity/tests/recovery_absorbs_only_absence.rs` closes it, hermetically, with a
store whose `load` fails for a reason that is not absence. With that test present, absorbing
every error fails:

```
a store that could not be read must fail recovery, not report an empty history; got 0 event(s)
```

And reverting the fix entirely fails the integration test. Both directions pinned.

### Held back deliberately

The `resolve_tenant` consolidation prepared while investigating this is **not** in this slice.
Four adapters carry a private copy of the tenant-scope rule — the PostgreSQL module and three
in-memory ones — and B4.5c needs a fifth call site. The four were compared and are semantically
identical (two textual variants differing only in `match` arm order), so consolidating into the
domain is behaviour-preserving. It belongs with B4.5c, not bundled into a defect fix. The patch
is kept for that slice.

### Verification

- INTEGRATION, real PostgreSQL: `recovery_of_a_fresh_aggregate` **3 passed** — the store's own
  answer characterized first, then recovery against the durable store, then against the in-memory
  one, reaching the identical outcome.
- HERMETIC: `recovery_absorbs_only_absence` **1 passed**.
- WORKSPACE: `cargo test --workspace` — **116 suites, 1 552 passed, 0 failed**.
- STATIC: `fmt`, `clippy -D warnings`, `verify-layers` (17 crates, 0 violations),
  `verify-isolation`, `verify-hygiene`.

Clippy caught an unused import that the entire green workspace suite did not — the fifth
consecutive slice where that happened. `cargo test` does not apply `-D warnings`, so a green suite
is not evidence about the static gates, and by now that is a rule rather than an observation.

**113 tasks, 56 complete, 57 pending. B4.5c still open.**
