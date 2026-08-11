//! Trybuild driver for the guarantee that an `OperationIdentity` cannot exist
//! with only one of its two halves.
//!
//! This is the type-level replacement for two runtime tests in
//! `crates/persistent-entity/tests/receipt_gating.rs` that asserted the receipt
//! gate stayed inactive when a `CommandContext` carried a key without a
//! fingerprint, or the reverse. Those states are no longer constructible, so
//! the assertion moved from "the gate ignores it" to "it cannot be built".
//!
//! **It takes two fixtures, because there are two ways in.** The constructor's
//! arity and the fields' privacy are independent: `new(key)` fails on arity
//! whether or not the fields are public, so an arity fixture cannot notice
//! `key`/`fingerprint` becoming reachable, and a struct literal bypasses `new`
//! altogether. Each fixture covers exactly one, and each says so.
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
    // The constructor requires both halves…
    t.compile_fail("tests/compile_fail/operation_identity_half_constructed.rs");
    // …and it cannot be routed around, because the halves are not fields a
    // caller can write or read.
    t.compile_fail("tests/compile_fail/operation_identity_fields_public.rs");
}
