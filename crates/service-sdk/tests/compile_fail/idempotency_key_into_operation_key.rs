// Compile-fail fixture: `IdempotencyKey` must have no implicit
// conversion into `OperationKey` — the reverse direction of
// `operation_key_into_idempotency_key.rs`. If this file starts compiling, an
// accidental `From<IdempotencyKey> for OperationKey` (or a blanket impl
// reaching it) was introduced — remove it.
fn main() {
    let key = ego_domain::IdempotencyKey::new("uow-1:0").unwrap();
    let _operation_key: ego_domain::operation::OperationKey = key.into();
}
