// Compile-fail fixture: `OperationKey` must have no implicit
// conversion into `IdempotencyKey`. If this file starts compiling, an
// accidental `From<OperationKey> for IdempotencyKey` (or a blanket impl
// reaching it) was introduced — remove it. A deliberate bridge, if ever
// needed, must be a deliberately named function, never a generic conversion
// trait implementation.
fn main() {
    let key = ego_domain::operation::OperationKey::parse("op-1").unwrap();
    let _idempotency_key: ego_domain::IdempotencyKey = key.into();
}
