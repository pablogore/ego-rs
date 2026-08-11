//! Trybuild driver for two properties of `OperationIdentity`: that it always
//! carries both halves, and that only its constructor may set them.
//!
//! The first is the type-level replacement for two runtime tests in
//! `crates/persistent-entity/tests/receipt_gating.rs` that asserted the receipt
//! gate stayed inactive when a `CommandContext` carried a key without a
//! fingerprint, or the reverse. Those states are no longer constructible, so the
//! assertion moved from "the gate ignores it" to "it cannot be built".
//!
//! # Two fixtures, two different properties — not two halves of one
//!
//! **Completeness** is the constructor's arity, plus Rust's own rule that a
//! struct literal must name every field. Those two together already make "an
//! identity with one half" unrepresentable, and one fixture covers the part that
//! is ours to keep.
//!
//! **Privacy** is a separate property, and losing it would not admit a half
//! identity — a literal still has to supply both. What it would admit is
//! construction that escapes `new`, so any invariant `new` later acquires
//! becomes skippable, and independent mutation afterwards: assigning `key` alone
//! would leave a *different* request's fingerprint attached, which is worse than
//! a missing half because the gate would compare a real fingerprint against the
//! wrong operation.
//!
//! Neither fixture can stand in for the other. `new(key)` fails on arity whether
//! or not the fields are public, so the arity fixture is blind to visibility; and
//! a literal never calls `new`, so it says nothing about arity. Each fixture
//! states which one it covers.
//!
//! Regenerate .stderr snapshots:
//!   TRYBUILD=overwrite cargo test -p ego-service-sdk --test operation_identity_indivisible
//!
//! The pinned toolchain (`rust-toolchain.toml`) is what keeps these error
//! snapshots stable across runs — the exact diagnostic text is
//! rustc-version-dependent.

#[test]
fn an_operation_identity_is_always_complete_and_only_its_constructor_sets_it() {
    let t = trybuild::TestCases::new();
    // Completeness: the constructor requires both halves.
    t.compile_fail("tests/compile_fail/operation_identity_half_constructed.rs");
    // Privacy: neither half can be written directly, so construction cannot
    // bypass `new` and the two cannot drift apart afterwards. Reading a half is
    // allowed, through `key()` and `fingerprint()`.
    t.compile_fail("tests/compile_fail/operation_identity_fields_public.rs");
}
