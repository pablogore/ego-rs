//! Canonical route-table assembly (design.md AD-2/AD-7). Every caller that
//! needs a runnable `Router` over an `AppState` — `main.rs` and this
//! crate's own integration tests — goes through `build_router` instead of
//! hand-rolling `Router::new().route(...)`, so route wiring lives in
//! exactly one place.

use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use ego_transport::AppState;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use super::handlers::{register_handler, users_by_tenant_handler};
use super::ApiDoc;
use crate::read_side::UsersByTenantStore;

/// Combined state for `/tenants/:tenant_id/users` (Finding 1: this route now
/// requires authentication, so it needs both `AppState`, for the
/// `AuthenticatedContext` extractor, and `UsersByTenantStore`, the read
/// model). axum 0.7's `FromRef` has no blanket impl for raw tuples (only the
/// identity `impl<T: Clone> FromRef<T> for T`) — the substate pattern
/// requires a named struct with its own `FromRef` impl per component, which
/// is what this provides. `/register` still only needs plain `AppState`.
#[derive(Clone)]
struct ReadSideState {
    app: AppState,
    query: UsersByTenantStore,
}

impl FromRef<ReadSideState> for AppState {
    fn from_ref(input: &ReadSideState) -> AppState {
        input.app.clone()
    }
}

impl FromRef<ReadSideState> for UsersByTenantStore {
    fn from_ref(input: &ReadSideState) -> UsersByTenantStore {
        input.query.clone()
    }
}

/// Assembles the full route table for the reference app: the `/register`
/// write route, the `UsersByTenant` read-side query route, plus
/// interactive OpenAPI docs (Swagger UI at `/swagger-ui`, the raw spec at
/// `/api-docs/openapi.json`).
///
/// `/register` and `/tenants/:tenant_id/users` are built as two
/// substate-scoped sub-routers (axum's `Router<S>::with_state` erases each
/// to `Router<()>`) and merged — `AppState` (write side) and
/// `UsersByTenantStore` (read side) stay independent state types instead of
/// forcing one combined app-state struct on every route; `ReadSideState`
/// above exists only to satisfy axum's substate `FromRef` requirement for
/// the one route that needs both.
pub fn build_router(state: AppState, users_by_tenant: UsersByTenantStore) -> Router {
    let write_routes = Router::new().route("/register", post(register_handler)).with_state(state.clone());

    let read_side_routes = Router::new()
        .route("/tenants/:tenant_id/users", get(users_by_tenant_handler))
        .with_state(ReadSideState { app: state, query: users_by_tenant });

    write_routes
        .merge(read_side_routes)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
}
