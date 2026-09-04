//! HTTP inbound adapter (design.md AD-2: concrete routes live in
//! `reference-app`, never in the generic `ego-transport` crate).
//!
//! `RegisterInput`/`RegisterOutput` (defined in `crate::application`) serve
//! as both the application's I/O types AND this port's OpenAPI request/
//! response schemas — a single `serde` + `utoipa::ToSchema` struct, not a
//! separate DTO with a mapping layer, since nothing here needs the two
//! representations to diverge.

pub mod handlers;
pub mod router;

pub use handlers::{health_handler, ready_handler, register_handler, users_by_tenant_handler};
pub use router::build_router;

use crate::application::{RegisterInput, RegisterOutput};
use crate::read_side::{TenantUsersView, UserSummary};
use handlers::HealthResponse;

/// OpenAPI document for the reference app: the `/register` write path, the
/// `UsersByTenant` read-side query path, the `/health`/`/ready` operational
/// probes, and the bearer-JWT security scheme every guarded operation
/// requires (see `crate::DEV_SIGNING_KEY`'s doc for the dev-only signing key
/// this scheme verifies against).
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        handlers::register_handler,
        handlers::users_by_tenant_handler,
        handlers::health_handler,
        handlers::ready_handler,
    ),
    components(schemas(
        RegisterInput,
        RegisterOutput,
        TenantUsersView,
        UserSummary,
        HealthResponse
    )),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer_jwt",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}
