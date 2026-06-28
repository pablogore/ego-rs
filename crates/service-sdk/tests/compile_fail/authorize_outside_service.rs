// Fixture: E5 — #[authorize] applied outside a #[service] trait.
// Linked requirement: FR-7, AC-7.1.
use ego_service_sdk_macros::authorize;

#[authorize(context = ctx, permission = "orders:read")]
async fn standalone_fn(ctx: String) -> Result<String, String> {
    Ok(ctx)
}

fn main() {}
