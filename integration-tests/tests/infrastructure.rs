//! The infrastructure-backed suite: one binary, one PostgreSQL, one migration run.
//!
//! # Why one target
//!
//! Issue #275 requires *"One shared PostgreSQL per run, isolated per test by
//! schema or by database. Not one container per test."* Separate integration-test
//! files are separate binaries, and separate binaries cannot share an in-process
//! fixture — so a container shared across them needs either an external runner
//! owning its lifecycle, or the files collapsed into one target.
//!
//! One target was chosen after inventorying all eight: none reads an environment
//! variable, none holds a static or `OnceCell`, none installs a global subscriber,
//! none hardcodes a port, and none assumes serial execution. There was no
//! process-level isolation being relied on, so collapsing them was safe.
//!
//! # Both halves of that decision, in the order they were learned
//!
//! The inventory settled the **target**: one binary, because nothing needed a
//! process of its own.
//!
//! It did not settle **ownership**, and the first attempt got that wrong. This
//! file's earlier wording said an external runner "bought nothing" — and then the
//! teardown showed exactly what it buys. A container held in a process-wide cell
//! has its async `Drop` run at process exit with no runtime left to drive it:
//! three consecutive runs left three containers behind, where the old
//! container-per-file shape had leaked none.
//!
//! So the runner is not a second lifecycle bolted onto a settled design; it owns
//! the one responsibility libtest cannot express. The inventory justified the
//! single target, and the teardown finding subsequently justified the runner —
//! two separate conclusions, and recording only the first would make the second
//! look arbitrary.
//!
//! # What is preserved
//!
//! Each file keeps its own module, its own doc header naming its exclusive
//! invariant, and its own fixtures. This is a change of *target*, not a merge of
//! scenarios: nothing here shares state with anything else, because every test
//! still gets a database of its own from `ego_integration_tests::isolated_database`.
//!
//! Run: `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`.
//! Never `cargo test --workspace` at the root — this workspace is not a member.

mod infrastructure {
    /// PROD-015 T-04.1–T-04.4 (IS-4, full weight per D-9): the offline
    /// `aggregate_type` backfill's four transactional-behavior cases —
    /// preflight abort, post-verification rollback, zero-row commit, and the
    /// revert round trip.
    mod aggregate_type_backfill_postgres;
    mod concurrent_replicas_postgres;
    mod conflict_from_postgres;
    /// Unix only, structurally rather than by accident.
    ///
    /// The scenario's whole point is that the child dies by **SIGABRT**, and
    /// reading a signal from an exit status is `std::os::unix`. Degrading it
    /// elsewhere to "any non-zero exit" would keep the test compiling and throw
    /// away the guarantee: a child that merely panicked, failed an assertion or
    /// could not find its database would satisfy it, and none of those is a
    /// crash. A scenario that cannot be expressed on a platform is better absent
    /// there than present and hollow.
    #[cfg(unix)]
    mod dual_aggregate_crash_recovery_postgres;
    mod durable_entity_progress_postgres;
    /// PROD-015 T-01.1/T-01.2 (IS-1, foundational): `PostgreSQLEventStore` and
    /// `PostgresOperationReservationStore` judged against the identical shared
    /// conformance harnesses the in-memory adapters satisfy. IS-6 (uncommitted
    /// staged writes are invisible; a dropped unit of work persists nothing) is
    /// retired into this same run per D-4/AD-4 — demonstrated here, no separate
    /// test or ledger row.
    mod durable_store_conformance_postgres;
    /// PROD-002 PR5 Phase 7.5: composition-only validation that a real
    /// `PostgresEffectStore` composes with `RuntimeBuilder::with_effect_store`
    /// end to end — not a re-test of Tier 1/2/3 conformance below.
    mod effect_store_composition_postgres;
    /// PROD-002 G11: Tier 2/3 PostgreSQL effect-store conformance, relocated
    /// here from the old per-crate `crates/integration-tests`.
    mod effect_store_postgres_conformance;
    /// PROD-002 G11: `PostgresEffectStore`-specific unit tests, relocated
    /// alongside `effect_store_postgres_conformance` above.
    mod effect_store_postgres_unit;
    mod entity_event_stores_wiring_postgres;
    /// PROD-015 T-03.1/T-03.2 (IS-2 post-`23505` scope + IS-5, both P0/P1): an
    /// N-way concurrent append race on one stream, and NULL-tenant identity
    /// verified behaviorally under SQL's three-valued comparison.
    mod events_identity_race_postgres;
    mod fencing_window_postgres;
    /// PROD-015 T-02.1 (IS-3, P0): six real contenders racing one expired
    /// lease under a real row lock leave exactly one `TakenOver` winner and
    /// the fencing token advances by exactly one, never by the contender
    /// count.
    mod lease_contention_postgres;
    mod oldest_completed_postgres;
    /// PROD-015 T-05.1–T-05.4 (IS-9, P1): the narrow PostgreSQL 14
    /// compatibility slice — anti-vacuity guard, migration-set schema
    /// completeness, the systemwide-duplicate `23505` refusal, and the
    /// aggregate-type backfill/revert round trip, all against the run's
    /// separate PG14 container. Deliberately not a second full run of the
    /// main suite; see the file's own doc comment for what is excluded.
    mod pg14_compatibility;
    /// PROD-P0.2: `build_runtime_with`'s fail-closed JWT verification-key
    /// gate under `Profile::Production`, over a real, migrated PostgreSQL
    /// pool (the only way this profile is reachable — see
    /// `durable_entity_progress_postgres`'s own note on this).
    mod production_jwt_key_postgres;
    /// PROD-P0.3: `build_runtime_with`'s fail-closed tenancy gate under
    /// `Profile::Production` — `single_tenant_mode = false` is refused
    /// (persistence tenant is fixed per runtime process, never per request;
    /// see the gate's own comment in `reference_app::build_runtime_with`),
    /// over a real, migrated PostgreSQL pool.
    mod production_tenancy_postgres;
    mod purge_progress_postgres;
    /// PROD-014C (PR2): `PostgreSQLReadSideClaimStore` against real
    /// PostgreSQL — the execution-exclusion gap `read_side_progress_postgres`
    /// (PROD-014B) names but does not close: concurrent-second-claimant
    /// exclusion, expiry-driven takeover, stale-owner fencing (with a
    /// token-isolation probe), renewal extending a live lease, no ordering
    /// interference with a stream's events, and immediate reclaim on
    /// release.
    mod read_side_claiming_postgres;
    /// PROD-014B (PR2): the durable read-side `OffsetStore`/`DedupStore` pair
    /// against real PostgreSQL — restart survival, tenant isolation,
    /// last-write-wins offsets, dedup convergence (sequential and
    /// concurrent), tenant-independent dedup identity, durability
    /// reporting, and unapplied-migration `Fatal` classification.
    mod read_side_progress_postgres;
    mod receipt_identity_isolation_postgres;
    mod replay_from_postgres;
    /// STOOLAP-S1 (design AD-9, D-10): the shared `Repository` conformance
    /// harness's PostgreSQL run — one of the harness's three subjects,
    /// judged against the identical scenarios `InMemoryRepository` and
    /// `StoolapRepository` satisfy.
    mod repository_conformance_postgres;
    /// KD-2: `PostgreSQLRepository`'s tenant-scoped predicates against the
    /// `aggregates` table, including the systemwide (`tenant_id IS NULL`)
    /// scope no prior test here exercised.
    mod repository_tenant_scoping_postgres;
    /// Not named `*_postgres` like its neighbours, because it is not a scenario
    /// traversing the framework at all: it reads PostgreSQL's own catalog and
    /// asserts the shape of the indexes the other modules depend on.
    mod schema_index_assertion;
    /// Unix only, structurally rather than by accident — same reasoning as
    /// `dual_aggregate_crash_recovery_postgres`: the scenario's whole point is
    /// that the child dies by **SIGABRT**, and reading a signal from an exit
    /// status is `std::os::unix`.
    #[cfg(unix)]
    mod single_aggregate_crash_recovery_postgres;
    mod takeover_fencing_postgres;
    /// PROD-P1.1 Required Test 2: real transport boundary acceptance for
    /// `GET /health`/`GET /ready` over a real, migrated PostgreSQL-backed
    /// Production-style composition. See the file's own doc comment for
    /// what it does and does not prove (readiness does not yet depend on
    /// Postgres connectivity — zero `HealthContributor`s are registered in
    /// this composition today).
    mod wire_health_readiness_postgres;
    /// Real transport boundary acceptance: a real TCP socket, a real HTTP
    /// client, and `reference_app::ports::http::build_router` — the exact
    /// router production serves — carrying `POST /register` through the
    /// real JWT auth path to a durable PostgreSQL write. See the file's own
    /// doc comment for what it does and does not prove.
    mod wire_register_postgres;
}
