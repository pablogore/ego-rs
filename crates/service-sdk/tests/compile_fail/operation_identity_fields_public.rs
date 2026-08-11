// Compile-fail test: `OperationIdentity`'s halves cannot be written directly.
//
// # What this proves, and what it does not
//
// It proves **field privacy** — that construction is mediated by
// [`OperationIdentity::new`] and that neither half can be assigned afterwards.
// It does *not* prove indivisibility, and it is worth being exact about why:
// making `key` and `fingerprint` public would not let anyone build half an
// identity. Rust requires every field in a struct literal, so the value is still
// constructed atomically with both halves present.
//
// Two different properties are lost instead:
//
// 1. **Construction escapes `new`.** Any invariant `new` ever acquires — a
//    length check, a normalisation, a rejection — becomes skippable by writing a
//    literal, and nothing at the call site would show it was skipped.
// 2. **The halves become independently mutable.** `identity.key = other_key;`
//    would leave the fingerprint of a *different* request attached to this one.
//    That is how a mismatched pair actually arises, and it is a worse failure
//    than a missing half: the gate would compare a real fingerprint against the
//    wrong operation and answer with confidence.
//
// Completeness — that an identity always has both halves — is guaranteed by the
// sibling fixture's constructor arity plus Rust's own struct-literal rule, and
// needs no second test.
//
// Deliberately *not* tested here: reading a half. That is legitimate and
// publicly supported through `key()` and `fingerprint()`. A fixture asserting a
// field read fails would be defending something the design does not claim, and
// its error would mask this one in the snapshot.
//
// If this file starts compiling successfully, `key`/`fingerprint` became
// writable: construction can bypass `new`, and either half can be mutated
// independently of the other.
fn main() {
    let key = ego_domain::operation::OperationKey::parse("op-1").unwrap();
    let fingerprint = ego_domain::operation::OperationFingerprint::new("f".repeat(64));

    let _literal = ego_domain::operation::OperationIdentity { key, fingerprint };
}
