// Compile-fail test: an `OperationIdentity` cannot be assembled around its
// constructor.
//
// The sibling fixture `operation_identity_half_constructed.rs` covers the
// constructor's arity — that `new` cannot be called with one argument. That is
// only half the guarantee, because a struct literal never calls `new` at all: if
// `key` and `fingerprint` became public, an identity could be assembled field by
// field, and the arity fixture would keep failing exactly as designed while the
// property it is supposed to protect had already gone.
//
// So this file targets the other entry point: field privacy, checked through the
// one operation that would let a caller supply the halves independently.
//
// Deliberately *not* tested here: reading a half. That is legitimate and
// publicly supported through `key()` and `fingerprint()` — the guarantee is
// about what can be **built**, not about what can be looked at. A fixture that
// also asserted a field read fails would be asserting something the design does
// not claim, and its snapshot would mask this error with that one.
//
// If this file starts compiling successfully, `key`/`fingerprint` became
// writable and half an identity can be assembled again.
fn main() {
    let key = ego_domain::operation::OperationKey::parse("op-1").unwrap();
    let fingerprint = ego_domain::operation::OperationFingerprint::new("f".repeat(64));

    let _literal = ego_domain::operation::OperationIdentity { key, fingerprint };
}
