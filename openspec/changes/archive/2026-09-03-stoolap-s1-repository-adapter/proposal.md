# Proposal: STOOLAP-S1 — First-Class Stoolap `Repository` Adapter

> Canonical / source of truth. Spanish review companion: `proposal.es.md` (1:1 identifiers:
> D-1..D-12, NG-1..NG-9, IS-1..IS-6, R1..R14, KD-1..KD-3, F-1..F-4, RK-1..RK-7).
>
> Ground truth: the STOOLAP-S1 exploration (Engram `sdd/stoolap-s1/explore`, verdict **GREEN**).
> Its five capability findings against stoolap 0.4.0 are consumed as given, not re-derived.

## Objective

Add **one** Stoolap-backed implementation of `ego_persistence_api::persistence::Repository<A>`, in
its own crate, with **zero** dependency on the PostgreSQL adapter — and a shared cross-backend
conformance harness that judges Memory, PostgreSQL, and Stoolap against the same contract instead of
against each author's reading of it.

## Intent

**The gap is a missing third implementation, not a missing abstraction.** `Repository<A>` has exactly
two implementations today: `InMemoryRepository` (`crates/persistence-memory/src/persistence/repository.rs:11`)
and `PostgreSQLRepository` (`crates/persistence/src/postgres/repository.rs:27`). A deployment that
wants durable aggregate persistence without operating a PostgreSQL server has no option — even though
this workspace already ships a working, disk-backed Stoolap provider for a different port
(`crates/effect-store/src/stoolap/`, PROD-002 Phase 4) and already pins `stoolap 0.4.0` in
`Cargo.lock`.

**The second gap is verification, and it has already cost this workspace once.**
`crates/testkit/src/event_store.rs:1-20` records why `assert_event_store_conformance` exists: two
`EventStore` implementations silently disagreed about the systemwide (tenant-less) partition, both
satisfied the trait signature, only one satisfied its meaning, and *nothing in the workspace compared
them*. `Repository` is in that exact pre-incident state today — two implementations, no shared
harness (CORE-PERSIST-B carried this as KD-4). Adding a third implementation without a harness
triples the surface for the same class of divergence.

**Two findings make this adapter non-obvious, and both are decided here rather than discovered
during apply.** Stoolap's unique-index enforcement is *skipped entirely* when an indexed column is
NULL, and Stoolap has no partial indexes — so PostgreSQL's two-partial-index tenant trick
(migration 015) has no equivalent and a naive port would silently permit duplicate systemwide rows.
And Stoolap's default sync mode does not fsync per commit, so durability is opt-in. Both are D-5 and
D-6 below.

## Active Decisions

| ID | Decision | Rationale / evidence |
|----|----------|----------------------|
| D-1 | **New crate `ego-persistence-stoolap` at `crates/persistence-stoolap/`**, sibling to `ego-persistence` (PostgreSQL) and `ego-persistence-memory` (in-memory) | Matches the `ego-*` package / bare-directory convention those two already set (`crates/persistence-memory/Cargo.toml:2`, workspace member list `Cargo.toml:5-8`). Naming is confirmed against the workspace, not inherited from the task description |
| D-2 | **Layer classification is `infrastructure`**, not `foundation`. `layers.toml` gains `"ego-persistence-stoolap" = "infrastructure"` and **`xtask/src/layers.rs` is not modified at all** | An adapter driving an external storage engine is infrastructure by the same reading that puts `ego-persistence` (`layers.toml:27`) and `ego-effect-store` (`:35`) there. `ego-persistence-memory` is `foundation` (`:36`) precisely because it has no backend; this crate does. `infrastructure → domain` is already permitted (`layers.toml:10`) and `ego-persistence-api` is `domain` (`:17`), so no gate relaxation is needed |
| D-3 | **Dependency set is `ego-persistence-api` + `stoolap` + strictly necessary technical crates. `ego-domain` is deliberately *not* a dependency** | `Repository`, `PersistenceError`, and `resolve_tenant` all resolve directly from `ego-persistence-api` (`persistence-api/src/persistence/{repository.rs:12,tenant.rs:29}`), and nothing in a `Repository<A>` implementation needs a domain value type — unlike `ego-persistence-memory`, which needed `Clock` for a different port (CORE-PERSIST-B D-3). `design.md` owns the final import list |
| D-4 | **No async bridging.** Unlike `PostgreSQLRepository`, this adapter does not need `tokio::task::block_in_place` / `Handle::block_on` | `Repository` is a **synchronous** trait (`repository.rs:21-39`) and Stoolap's API is synchronous. `PostgreSQLRepository::block_on` (`postgres/repository.rs:51-53`) exists only to bridge async `sqlx` into a sync trait; that bridge has no reason to exist here. `ego-effect-store` needed `spawn_blocking` for the opposite reason — *its* ports are async. Stated so nobody copies the bridge in by reflex |
| D-5 | **AD — Tenant scope is stored as a NOT-NULL sentinel column, never SQL `NULL`.** The systemwide scope is encoded as the empty string in the adapter's own table; a single plain `UNIQUE (tenant_id, aggregate_id)` index then enforces one row per scope uniformly | Forced by two Stoolap facts: `check_unique_constraints()` returns early — skipping the uniqueness check entirely — when any indexed column is NULL, and `CREATE INDEX` has no `WHERE` predicate support at all. A nullable tenant column would therefore let *duplicate systemwide rows* for one `aggregate_id` exist silently, and PostgreSQL's two-partial-index split (`postgres/repository.rs:114-148`) cannot be replicated. `""` is safe as the sentinel because `resolve_tenant` rejects `Some("")` as `MissingTenant` **before any adapter is reached** (`tenant.rs:32`), so it can never collide with a real tenant. **This is an adapter-internal storage encoding only** — `Repository`'s external `Option<&str>` contract is untouched and no caller ever sees the sentinel (R4). Full mechanics belong to `design.md` |
| D-6 | **AD — Durable commit is an explicit adapter decision, not an inherited default.** The adapter opens its database requesting full sync durability rather than accepting Stoolap's default | Stoolap's default `SyncMode::Normal` does not fsync on every commit; PostgreSQL-equivalent commit durability requires the `sync=full` DSN parameter. This regresses silently and invisibly — evidence in-tree: the existing Stoolap effect-store provider opens `file://{path}` with no sync parameter (`crates/effect-store/src/stoolap/mod.rs:175`). Aggregate state is not a cache; an adapter that loses the last second of commits after a crash is not a `Repository`. Named as a decision so a reviewer approves durability deliberately and a test pins it (R5) |
| D-7 | **Every write conflict maps to `PersistenceError::Conflict`. No new error variant is added** | `PersistenceError` has no retryable variant (unlike `EffectStoreError::TemporarilyUnavailable`). A stale `expected_version` and a Stoolap MVCC write-claim collision are the same event from the caller's seat — *you lost a race, reload and retry* — and callers already retry on `Conflict`. Inventing a variant would reopen the shipped `persistence-api-surface` contract to serve one backend (NG-5) |
| D-8 | **Optimistic concurrency is enforced by a real transaction plus a version-guarded conditional write, not by row locking** | Stoolap has no `SELECT ... FOR UPDATE` anywhere. It does have real ACID transactions with explicit commit/rollback and MVCC per-row write-claim conflict detection, which prevents the same dirty write `FOR UPDATE` guards against in `postgres/repository.rs:89-98`. The pattern mirrors `ego-effect-store`'s already-proven Stoolap CAS approach. Statement-level mechanics belong to `design.md`, not here |
| D-9 | **The cross-backend conformance harness lives in `ego-testkit`**, alongside the harnesses already there — not copied per adapter, and not placed in any one adapter crate | `crates/testkit/src/{event_store.rs,reservation_conformance.rs,observability_conformance.rs}` is this workspace's established home for shared conformance checks, and its own doc comment states the reason (`event_store.rs:1-20`). `ego-testkit` is layer `tooling`, a sink (`layers.toml:13,34`), so a dev-dependency edge from each adapter crate is legal and grants no production reach. `ego-effect-store`'s in-crate `conformance.rs` is the *older* pattern and is not followed here |
| D-10 | **PostgreSQL conformance runs exclusively in the separate `integration-tests/` workspace; Memory and Stoolap run in the root workspace** | `integration-tests/` is a deliberately non-member workspace so the root builds and tests with **no Docker** (`integration-tests/Cargo.toml:1-15`), and it already dev-depends on `ego-testkit` (`:59`). Stoolap is embedded and file-backed, so it needs no container and belongs in the root suite. **No Testcontainers dependency is introduced into the root workspace under any circumstance** (R9) |
| D-11 | **Zero PostgreSQL reuse — stated as a rule, not an aspiration.** The new crate must not name `crate::postgres::*`, `PostgreSQLRepository`, `PgPool`, any `sqlx` helper, any PostgreSQL migration, any PostgreSQL error classification, or any PostgreSQL private test helper | The two backends share a *contract*, not an implementation. Any shortcut through the PostgreSQL crate would make `ego-persistence` a de facto base class and re-couple the backends this change exists to keep independent. Checkable as a dependency and symbol assertion (R7) |
| D-12 | **`Repository` is the only port implemented, and no backend abstraction is created** | Every other port stays unimplemented for Stoolap (NG-1), and no `StorageEngine`, SQL-dialect layer, ORM, generic repository engine, or backend toolkit is introduced (NG-2). One port, one backend, one honest adapter |

## Atomicity Gate

**Run.** One port, one backend, one crate, plus the harness that proves the three implementations
agree. The harness is not separable: `crates/testkit/src/event_store.rs:1-20` documents the exact
incident that occurs when a second implementation of a tenant-scoped port ships without one, and
D-5's sentinel encoding makes this adapter's tenant partitioning *structurally different* from both
existing implementations — which is precisely the condition a shared harness exists to police.
Shipping the adapter alone would land the highest-risk decision in this change with no cross-backend
evidence behind it.

Explicitly **OUT**, each because it is an independent decision rather than a missing piece of this
one: any second Stoolap-backed store · any backend abstraction layer · any additional backend ·
CORE-PERSIST-A2 · the `ego-persistence` → `ego-persistence-postgres` rename · any change to
`ego-persistence-api`.

**ATOMICITY: PASS** — matching the exploration's GREEN verdict. No shipped contract requires
modification.

## Scope

**Boundary at a glance**

| | |
|---|---|
| **STOOLAP-S1 includes** | New `ego-persistence-stoolap` crate · `StoolapRepository<A, F>` implementing `Repository<A>` · tenant-sentinel schema (D-5) · durable-commit configuration (D-6) · shared `Repository` conformance harness in `ego-testkit` · that harness run against Memory + Stoolap (root) and PostgreSQL (`integration-tests/`) · `layers.toml` entry + workspace member |
| **STOOLAP-S1 excludes** | Every other Stoolap store · every backend abstraction · every backend beyond Memory/PostgreSQL/Stoolap · CORE-PERSIST-A2 · the persistence-crate rename · every `ego-persistence-api` edit · every PostgreSQL edit |

### In Scope

- **IS-1** — The new crate (D-1), mapped to `infrastructure` in `layers.toml` (D-2), added to the
  root workspace member list, depending only on the set fixed by D-3.
- **IS-2** — `StoolapRepository<A, F>` implementing `ego_persistence_api::persistence::Repository<A>`
  — `save`, `load`, `delete` — with tenant scoping and optimistic concurrency, mirroring
  `PostgreSQLRepository<A, F>`'s public shape (constructor taking a connection target and a
  deserializer, `Debug`, the same generic bounds) with zero dependency on it (D-11).
- **IS-3** — The adapter's own schema and its creation: the tenant-sentinel column and the single
  plain `UNIQUE (tenant_id, aggregate_id)` index (D-5). Owned entirely by this crate; no PostgreSQL
  migration is read, shared, or referenced.
- **IS-4** — Durable-commit configuration on open (D-6), with a test that fails if the adapter falls
  back to Stoolap's default sync mode.
- **IS-5** — A shared `Repository` conformance harness in `ego-testkit` (D-9), covering at minimum:
  version advance from a new aggregate, rejection of a stale `expected_version`, round-trip
  load fidelity, `NotFound` on absent load and delete, `MissingTenant` on `Some("")`, and — with the
  rigor of `integration-tests/tests/infrastructure/repository_tenant_scoping_postgres.rs` — that the
  systemwide scope is isolated from every concrete tenant and equal to itself across calls.
- **IS-6** — That harness executed against all three implementations per D-10, plus spec deltas per
  the Capabilities section.

### Out of Scope — Non-Goals

Every item is a **non-goal with a stated reason**, not an omission.

- **NG-1 — No second Stoolap-backed store.** `EventStore`, `Snapshot`, `OperationReservationStore`,
  `OffsetStore`, and `DedupStore` gain no Stoolap implementation. **Reason**: each is a distinct
  contract with its own tenant, ordering, and durability semantics; bundling them would make the
  diff unreviewable and would settle five schema questions behind one approval. Carried as **F-1**.
- **NG-2 — No `StorageEngine`, SQL-dialect abstraction, ORM, generic repository engine, or backend
  toolkit is created.** **Reason**: a shared abstraction extracted from two backends is a guess.
  The prior persistence-extensibility audit concluded that duplication across backends here is
  cheap and that the abstraction is not yet earned; three concrete implementations of one port is
  the evidence a future extraction would need, not a reason to pre-build it (**F-2**).
- **NG-3 — No backend beyond Memory, PostgreSQL, and Stoolap.** No Oracle, MySQL, SQLite, or any
  other engine, and no extension point that anticipates one. **Reason**: the supported backend
  matrix is exactly these three, permanently. Anticipating a fourth is the abstraction NG-2 rejects,
  wearing a different name.
- **NG-4 — CORE-PERSIST-A2 is not performed.** `EffectStateStore`, `EffectDedupStore`, and
  `RetentionMaintenance` stay in `crates/runtime/src/effects/store.rs`. **Reason**: a port
  relocation is an ownership decision with its own blast radius, already identified as its own
  change (**F-3**).
- **NG-5 — `crates/persistence-api/` is not edited at all.** No port method, bound, supertrait,
  default body, or error variant changes (D-7). **Reason**: a backend that requires its contract to
  move is a backend that does not fit the contract — and the exploration confirmed this one does.
- **NG-6 — `crates/persistence/` is not renamed and not modified.** No `ego-persistence-postgres`
  rename, no SQL change, no migration, no index change. **Reason**: confirmed low-cost but optional
  by the prior audit, and entirely independent of whether a Stoolap adapter exists (**F-4**).
- **NG-7 — No PostgreSQL implementation detail is reused, shared, extracted, or imported** (D-11).
  **Reason**: the two adapters share a contract, not a lineage.
- **NG-8 — No Testcontainers, Docker, or container dependency enters the root workspace.**
  **Reason**: `integration-tests/Cargo.toml:1-12` makes the no-Docker root a structural guarantee,
  not a convention; Stoolap is embedded and needs nothing (D-10).
- **NG-9 — No existing implementation changes behavior to match the new one.** If the shared harness
  exposes a genuine divergence in `InMemoryRepository` or `PostgreSQLRepository`, it is recorded as
  named debt with a follow-up, not fixed inside this diff. **Reason**: a correctness fix to a
  shipped adapter deserves its own tests, its own blast-radius review, and its own reviewer — the
  same rule CORE-PERSIST-B applied to KD-5.

## Capabilities

### New Capabilities

- `persistence-stoolap-adapter`: the observable contract that a Stoolap-backed `Repository<A>`
  exists; that it scopes aggregates by tenant with the systemwide scope isolated from every concrete
  tenant and equal to itself; that it enforces optimistic concurrency, reporting every lost race as
  a conflict; that a committed save survives process restart; and that its externally observable
  behavior is indistinguishable from the in-memory and PostgreSQL implementations for every scenario
  the shared harness covers.

### Modified Capabilities

- **None expected.** `persistence-api-surface` is untouched (NG-5, R6). `foundation-integrity` needs
  no matrix change — `infrastructure → domain` is already permitted (`layers.toml:10`), so the new
  `layers.toml` entry *satisfies* the existing completeness requirement rather than modifying it.
  `testkit`'s spec does not enumerate individual conformance harnesses, so adding one requires no
  delta. All three are listed so the spec phase **confirms** rather than assumes; if it finds an
  existing requirement that must change, that becomes a blocking question, not a silent edit.

## Approach

Create the crate with the D-3 dependency set; define its own schema with the sentinel tenant column
and the single unique index; implement `save` as one real transaction — read current version,
compare in Rust, conditional version-guarded write, any conflict folded to
`PersistenceError::Conflict` — with `load` and `delete` as ordinary scoped statements; open the
database with durable commit explicitly requested.

Order matters for reviewability, and the harness comes first: write the shared `Repository`
conformance harness in `ego-testkit` and run it green against the two *existing* implementations
before a single line of Stoolap code exists. That sequencing is what turns the harness from
documentation into a judge — it is calibrated against known-good implementations, so a later
Stoolap failure is unambiguously the adapter's, and any divergence it exposes between Memory and
PostgreSQL surfaces as named debt (NG-9) rather than as a confusing new-adapter failure. Then the
adapter, then the third harness run.

## Acceptance Requirements

Each is independently checkable and doubles as this change's success criteria.

- [x] **R1 — The adapter exists and satisfies the contract.** `StoolapRepository<A, F>` implements
      `ego_persistence_api::persistence::Repository<A>` and passes the IS-5 harness in full.
- [x] **R2 — Cross-backend agreement is proven, not asserted.** The identical harness runs green
      against `InMemoryRepository`, `PostgreSQLRepository`, and `StoolapRepository` — one harness,
      three subjects, no per-backend variant or skipped scenario.
- [x] **R3 — Systemwide uniqueness actually holds.** Two saves of the same `aggregate_id` under the
      systemwide scope produce one row and correct version arithmetic — the failure mode a nullable
      tenant column would have permitted (D-5) is proven absent, not argued absent.
- [x] **R4 — The sentinel never escapes the adapter.** No caller-visible value, error message, or
      returned type exposes the encoding; `Repository`'s `Option<&str>` contract behaves identically
      across all three implementations, including `MissingTenant` on `Some("")`.
- [x] **R5 — Durability is pinned by a test.** A committed save survives a close/reopen cycle, and
      the adapter's configured sync mode is asserted rather than assumed (D-6).
- [x] **R6 — No contract change.** `crates/persistence-api/**` is unmodified; no port's method set,
      bounds, supertraits, default bodies, or error variants change.
- [x] **R7 — Backend independence.** `crates/persistence-stoolap/` names no `sqlx`, `PgPool`,
      `ego-persistence`, PostgreSQL migration, or PostgreSQL symbol anywhere in its manifest or
      sources; `crates/persistence/**` appears nowhere in the diff.
- [x] **R8 — Dependency and layer integrity.** The crate's `Cargo.toml` names exactly the D-3 set;
      `cargo run -p xtask -- verify-layers` passes with no new violation and no matrix edit.
- [x] **R9 — Root workspace stays Docker-free.** `cargo test --workspace` passes with no container
      runtime available, and no Testcontainers dependency appears in the root workspace.
- [x] **R10 — Scope containment.** No Stoolap implementation of any port other than `Repository`
      exists, and no `StorageEngine`/dialect/ORM/generic-engine abstraction is introduced.
- [x] **R11 — Existing behavior is unchanged.** `InMemoryRepository` and `PostgreSQLRepository` are
      byte-identical in behavior; any divergence the harness exposes is recorded as debt (NG-9), not
      fixed here.
- [x] **R12 — Conflict fidelity.** A stale `expected_version` and a concurrent-writer race both
      surface as `PersistenceError::Conflict`, and no error variant is added.
- [x] **R13 — Harness ownership.** The harness is declared once, in `ego-testkit`, and is consumed
      by every backend rather than copied.
- [x] **R14 — Deferred work is named, not silently handled.** F-1 through F-4 are recorded with
      owners and prerequisites, and nothing in those boundaries is touched.

## Known Debt (carried, not fixed)

- **KD-1** — `Repository` has had no shared conformance harness since CORE-PERSIST-B named it
  (KD-4 there). This change closes it for `Repository` only; `Snapshot`, `OffsetStore`, and
  `DedupStore` remain uncovered.
- **KD-2** — The existing Stoolap effect-store provider opens `file://{path}` with no sync parameter
  (`crates/effect-store/src/stoolap/mod.rs:175`), so it runs at Stoolap's non-fsync default.
  **Observed, not judged and not changed here** — that is a different port, a different durability
  requirement, and a different reviewer. Recorded so the question is asked, not assumed answered.
- **KD-3** — Any Memory/PostgreSQL divergence the new harness exposes (NG-9), if one appears.

## Named Follow-Ups

- **F-1** — Additional Stoolap-backed stores (`EventStore`, `Snapshot`, `OperationReservationStore`,
  `OffsetStore`, `DedupStore`), each as its own change with its own schema decisions (NG-1).
- **F-2** — Revisit backend abstraction *only after* three concrete implementations of a second port
  exist and the duplication is measured rather than predicted (NG-2).
- **F-3** — **CORE-PERSIST-A2**: relocate `EffectStateStore`, `EffectDedupStore`, and
  `RetentionMaintenance` out of `crates/runtime/src/effects/store.rs` into `ego-persistence-api`
  (NG-4).
- **F-4** — Optional `ego-persistence` → `ego-persistence-postgres` rename, confirmed low-cost and
  non-blocking by the prior audit (NG-6).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/persistence-stoolap/` | New | The whole crate: `Cargo.toml`, schema, `StoolapRepository` (IS-1..IS-4) |
| `crates/testkit/src/` | Modified | New shared `Repository` conformance harness + `lib.rs` export (IS-5, D-9) |
| `crates/persistence-memory/`, `crates/persistence/` | Test-only | Dev-dependency + a test target invoking the shared harness; **no source change** (IS-6, R11) |
| `integration-tests/` | Modified | PostgreSQL run of the shared harness (D-10, IS-6) |
| `layers.toml`, root `Cargo.toml` | Modified | One layer entry, one workspace member (IS-1) |
| `xtask/src/layers.rs` | Untouched | No matrix change required (D-2) |
| `crates/persistence-api/` | Untouched | No contract change (NG-5, R6) |
| `crates/runtime/`, `crates/effect-store/` | Untouched | Deferred (NG-4, KD-2) |
| `openspec/specs/persistence-stoolap-adapter/spec.md` | New | Delta per IS-6 |

## Risks

| ID | Risk | Likelihood | Mitigation |
|----|------|------------|------------|
| RK-1 | **The sentinel encoding leaks into caller-visible behavior**, so the Stoolap adapter is subtly not the same `Repository` the other two are | Med | D-5 confines it to storage; R4 makes non-leakage a checkable assertion; R2 makes the *same* harness judge all three, so a leak fails a test rather than surviving as a comment |
| RK-2 | **Durability silently regresses to Stoolap's default** — the exact thing that is easy to inherit, with in-tree precedent (KD-2) | Med | D-6 makes it a decision with a named reviewer; R5 pins it with a test that fails on the default rather than a code comment that asks nicely |
| RK-3 | **Stoolap's MVCC write-claim conflict surfaces as an opaque generic error**, misclassified as `Internal` and never retried by a caller that would have succeeded | Med | D-7 folds every write conflict into `Conflict`; R12 requires both a stale version and a real concurrent race to be shown producing it. `design.md` owns the exact classification predicate |
| RK-4 | **The harness is written to whatever the Stoolap adapter happens to do**, and passes trivially | Med | The Approach fixes the order: the harness is written and proven green against the two existing implementations *before* any Stoolap code exists |
| RK-5 | **A Memory/PostgreSQL divergence surfaces mid-change**, tempting an in-diff fix to a shipped adapter | Med | NG-9 and R11 make that a follow-up by rule (KD-3). If the divergence blocks the harness from being written at all, that is a blocking question for the user, not an improvised fix |
| RK-6 | **Scope creep toward a backend abstraction** — three implementations of one port is exactly the moment the pattern looks extractable | Med | NG-2, NG-3, R10, and the workspace's own architecture-maturation rule: a principle needs 2–3 independent recurrences before promotion. One port is one data point (F-2) |
| RK-7 | **Review budget.** A new crate, a new harness, and three harness call sites will exceed the 400-line budget | High | Forecast, not hidden. `sdd-tasks` should slice as: (1) shared harness + Memory/PostgreSQL runs, (2) the crate with schema and `save`/`load`/`delete`, (3) the Stoolap harness run plus the durability test. Each slice leaves the workspace compiling and every prior slice green |

## Rollback Plan

**One revert commit, at any point, with zero external breakage.**

Nothing outside the new crate depends on it: `ego-persistence-stoolap` is additive, no existing crate
gains a non-dev dependency on it, and no existing source file changes behavior. Reverting is: drop
`crates/persistence-stoolap/`, remove the workspace member and the `layers.toml` entry, drop the
harness from `ego-testkit` and its three call sites. `xtask/src/layers.rs` never changed, so there is
no gate state to unwind, and `crates/persistence-api/` and `crates/persistence/` were never touched.

This holds **mid-flight**, which is what makes RK-7's slicing safe: a partially-landed STOOLAP-S1 is a
workspace that has gained a conformance harness and possibly a new unused crate. Neither is reachable
from any production wiring path, because nothing wires the adapter — a deployment opts in by adding
the dependency, and none does yet.

No data, schema, or migration in any existing store is involved in either direction. The adapter
creates only its own tables, in its own database file.

## Dependencies

- `persistence-api-surface` (shipped, CORE-PERSIST-A) — the `Repository<A>` contract this crate
  implements, consumed entirely unchanged.
- `persistence-memory-adapter` (shipped, CORE-PERSIST-B) — supplies the second harness subject.
- `foundation-integrity` (archived) — FR-001 (completeness), FR-002 (direction), FR-003 (no cycles),
  FR-005 (isolated compilation), consumed unchanged.
- `openspec/config.yaml` design rule "No circular dependencies between crates" — upheld by
  construction (D-2, D-3).
- `stoolap 0.4.0` — already pinned in `Cargo.lock` (checksum `420d8bd6…`) via `ego-effect-store`'s
  optional `stoolap` feature. **No new external crate, service, or infrastructure is introduced.**
- The STOOLAP-S1 exploration's five capability findings (Engram `sdd/stoolap-s1/explore`).

## Proposal question round

This proposal was produced without an interactive round. Five product questions would sharpen it;
until they are answered, the stated assumption applies.

1. **Who is the customer of a Stoolap `Repository`?** Single-node or edge deployments that want
   durable aggregates without operating PostgreSQL, or primarily faster/more realistic testing than
   in-memory? *Assumption: durable single-node deployments — which is why D-6 treats commit
   durability as non-negotiable rather than as a tunable.*
2. **Is Stoolap a supported production backend or a supported-but-not-recommended one?** The answer
   changes what the spec must say about operational expectations, backup, and concurrency limits.
   *Assumption: supported, with single-node concurrency characteristics stated honestly in the spec.*
3. **RK-5 / NG-9** — if the shared harness exposes a real divergence between the two *existing*
   implementations, should this change stop and escalate, or record it and continue? *Assumption:
   record and continue, unless the divergence makes a harness scenario unwritable.*
4. **KD-2** — is the effect-store's non-fsync Stoolap default intentional for that port, or an
   unnoticed inheritance worth its own follow-up? *Assumption: out of scope either way; recorded as
   an observation, not scheduled.*
5. **F-1 sequencing** — after `Repository`, which store earns the next Stoolap adapter, and is there
   a deployment that needs the full set before any of it is useful? *Assumption: none is scheduled
   here; `Repository` stands alone as a complete, independently useful slice.*
