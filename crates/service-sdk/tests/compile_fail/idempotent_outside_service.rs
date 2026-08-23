// Fixture: `#[idempotent]` applied outside a `#[service]` trait.
//
// Mirrors `authorize_outside_service.rs`, and for the same reason
// `#[tenant_scoped]` chose to fail loudly rather than pass through like
// `#[operation]`: a marker that silently does nothing when misapplied leaves a
// caller believing an operation is idempotent when nothing enforces it. A
// forgotten guard is a bug; a guard that looks present and is inert is a bug
// nobody goes looking for.
use ego_service_sdk_macros::idempotent;

#[idempotent]
async fn standalone_fn(input: String) -> Result<String, String> {
    Ok(input)
}

fn main() {}
