// Trybuild tests for PROD-012 B6.1 — the `#[idempotent]` marker's misuse
// diagnostics.
//
// The marker is checked at compile time rather than ignored, following
// `#[authorize]` and `#[tenant_scoped]` and deliberately not `#[operation]`.
// The reasoning is the one already recorded on `#[tenant_scoped]`: a marker that
// silently does nothing when misapplied is a false sense of enforcement. Here
// the failure mode is worse than a missing marker — an operation everyone
// believes is idempotent, with nothing reserving, replaying, or refusing its
// retries.
//
// Regenerate .stderr snapshots:
//   TRYBUILD=overwrite cargo test -p ego-service-sdk --test idempotent_marker

/// The control for the two negative fixtures: both obligations satisfied.
#[test]
fn idempotent_bounds_compile_pass() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/idempotent_bounds_ok.rs");
}

#[test]
fn idempotent_marker_compile_fail() {
    let t = trybuild::TestCases::new();
    // Outside a `#[service]` trait: the attribute is never consumed by the
    // generator, so it would expand to nothing at all.
    t.compile_fail("tests/compile_fail/idempotent_outside_service.rs");
    // Inside a `#[service]` trait but not on an `#[operation]`: there is no
    // dispatched path for the reservation slot to run in.
    t.compile_fail("tests/compile_fail/idempotent_without_operation.rs");
    // Output that cannot round-trip. Its error type satisfies `From` so the
    // failure can only be the serde obligation.
    t.compile_fail("tests/compile_fail/idempotent_output_not_serialisable.rs");
    // Error that cannot absorb a refused reservation. Its output satisfies both
    // serde bounds so the failure can only be the `From` obligation.
    t.compile_fail("tests/compile_fail/idempotent_error_without_from.rs");
}
