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
    mod fencing_window_postgres;
    mod oldest_completed_postgres;
    mod purge_progress_postgres;
    mod replay_from_postgres;
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
}
