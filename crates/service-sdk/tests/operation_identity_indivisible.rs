//! Trybuild driver for the guarantee that an `OperationIdentity` cannot exist
//! with only one of its two halves.
//!
//! This is the type-level replacement for two runtime tests in
//! `crates/persistent-entity/tests/receipt_gating.rs` that asserted the receipt
//! gate stayed inactive when a `CommandContext` carried a key without a
//! fingerprint, or the reverse. Those states are no longer constructible, so
//! the assertion moved from "the gate ignores it" to "it cannot be built".
//!
//! Regenerate .stderr snapshots:
//!   TRYBUILD=overwrite cargo test -p ego-service-sdk --test operation_identity_indivisible
//!
//! The pinned toolchain (`rust-toolchain.toml`) is what keeps these error
//! snapshots stable across runs — the exact diagnostic text is
//! rustc-version-dependent.

#[test]
fn an_operation_identity_cannot_be_built_from_one_half() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/operation_identity_half_constructed.rs");
}
