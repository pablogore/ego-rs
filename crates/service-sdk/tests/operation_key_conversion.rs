//! Trybuild driver for the compile-fail guarantee that `OperationKey`
//! and `IdempotencyKey` stay distinct (spec: "OperationKey Is Distinct From
//! IdempotencyKey") have no implicit conversion in either direction.
//!
//! Regenerate .stderr snapshots:
//!   TRYBUILD=overwrite cargo test -p ego-service-sdk --test operation_key_conversion
//!
//! The pinned toolchain (`rust-toolchain.toml`) is what keeps these
//! trait-resolution error snapshots stable across runs — the exact
//! diagnostic text is rustc-version-dependent.

#[test]
fn no_implicit_conversion_between_operation_key_and_idempotency_key() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/operation_key_into_idempotency_key.rs");
    t.compile_fail("tests/compile_fail/idempotency_key_into_operation_key.rs");
}
