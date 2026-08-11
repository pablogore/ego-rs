// Compile-fail test: an `OperationIdentity` cannot be built from one half.
//
// The receipt gate needs both halves to decide anything. `OperationKey` says
// which operation this is; `OperationFingerprint` says which request it came
// from, and without it a retry cannot be told apart from a different command
// reusing the key. So a key without a fingerprint is not a partial identity —
// it is an identity the gate must ignore entirely.
//
// This replaces two runtime tests that asserted the gate stayed inactive for a
// half identity (`a_command_with_only_an_operation_key_takes_the_previous_path`
// and `a_command_with_only_a_fingerprint_takes_the_previous_path`). Those
// defended the state at runtime; making the state unconstructible is strictly
// stronger, and this file is what keeps that claim honest — the fields are
// private and `new` takes both, so there is no way to express half an identity.
//
// If this file starts compiling successfully, a one-argument constructor or a
// public field was added, and a service body can once again transfer the key,
// forget the fingerprint, and silently leave idempotency off for an aggregate
// while appearing to switch it on.
fn main() {
    let key = ego_domain::operation::OperationKey::parse("op-1").unwrap();
    let _identity = ego_domain::operation::OperationIdentity::new(key);
}
