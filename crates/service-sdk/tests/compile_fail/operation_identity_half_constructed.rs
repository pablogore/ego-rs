// Compile-fail test: an `OperationIdentity` cannot be built from one half.
//
// The receipt gate needs both halves to decide anything. `OperationKey` says
// which operation this is; `OperationFingerprint` says which request it came
// from, and without it a retry cannot be told apart from a different command
// reusing the key. So a key without a fingerprint is not a partial identity —
// it is an identity the gate must ignore entirely.
//
// Together with `operation_identity_fields_public.rs`, this replaces two runtime
// tests that asserted the gate stayed inactive for a half identity
// (`a_command_with_only_an_operation_key_takes_the_previous_path` and
// `a_command_with_only_a_fingerprint_takes_the_previous_path`). Those defended
// the state at runtime; making it unconstructible is strictly stronger.
//
// **What this file covers, precisely: the constructor's arity.** It proves `new`
// cannot be called with one argument. It says nothing about field visibility —
// `new(key)` would keep failing on arity even if `key` and `fingerprint` became
// public, so this fixture alone cannot detect that. Field privacy is the sibling
// fixture's job, and the guarantee needs both.
//
// If this file starts compiling successfully, `new` gained a one-argument form,
// and a service body can once again build an identity carrying the key alone,
// forget the fingerprint, and silently leave idempotency off for an aggregate
// while appearing to switch it on.
fn main() {
    let key = ego_domain::operation::OperationKey::parse("op-1").unwrap();
    let _identity = ego_domain::operation::OperationIdentity::new(key);
}
