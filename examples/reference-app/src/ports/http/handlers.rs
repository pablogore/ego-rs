//! Concrete HTTP route table (design.md AD-2: concrete routes live in
//! `reference-app`, never in the generic `ego-transport` crate).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use ego_service_sdk::context::ServiceContext;
use ego_transport::{AppState, AuthenticatedContext, TraceContextExtractor, TransportError};
use kitlogger_log_domain::Severity;

use crate::application::{
    RegisterInput, RegisterOutput, RegisterUser, RegisterUserError, RegisterUserTag,
};
use crate::read_side::{TenantUsersView, UsersByTenantStore};

/// Best-effort text log through the kit-config-materialized logger
/// (`build_runtime`'s `.with_logger(...)`, absent in tests). A logging
/// failure must never fail the request it's describing.
fn log(state: &AppState, severity: Severity, message: &str) {
    if let Some(logger) = state.runtime.logger() {
        let _ = logger.log(severity, message);
    }
}

/// Maps `RegisterUser`'s own error type to the transport-facing error
/// category. `ego-transport` only knows about `ServiceError`/`SecurityError`
/// (AD-2: mechanism only) — this mapping stays in the application that owns
/// the concrete error type. `Security(_)` delegates to `ego-transport`'s own
/// granular `From<SecurityError> for TransportError` (crates/transport/src/error.rs)
/// instead of collapsing every denial to 403 — that table already
/// distinguishes 401 (e.g. `MissingContext`) from 403 (e.g.
/// `CrossTenantDenied`) from 500 (e.g. `ProviderError`).
fn map_register_error(err: RegisterUserError) -> TransportError {
    match err {
        RegisterUserError::Security(e) => e.into(),
        RegisterUserError::EntityWrite(_) => TransportError::Internal,
    }
}

/// `POST /register` — resolves `RegisterUser` via `Runtime::resolve` and
/// invokes it (http-transport spec: "Request reaches the guarded operation").
#[utoipa::path(
    post,
    path = "/register",
    request_body = RegisterInput,
    responses(
        (status = 201, description = "User registered (reference-service spec: \"Successful registration\")", body = RegisterOutput),
        (status = 401, description = "Missing or invalid credentials — RegisterUser never invoked (http-transport spec: \"Missing or invalid credentials rejected pre-invocation\")"),
        (status = 403, description = "Authorization denied or cross-tenant request denied (reference-service spec: \"Unauthorized principal denied\", \"Cross-tenant request denied\")"),
        (status = 500, description = "User write failed after the TenantOrganization write already succeeded (reference-service spec: \"TenantOrganization succeeds, User write fails\")"),
    ),
    security(("bearer_jwt" = [])),
)]
pub async fn register_handler(
    State(state): State<AppState>,
    AuthenticatedContext(security): AuthenticatedContext,
    // PROD-003 (service-sdk spec: "Trace-Context Originates At HTTP
    // Ingress"): origination happens exactly once, at the transport
    // boundary (`ego_transport::propagation::TraceContextExtractor`) —
    // continues an inbound `traceparent` when present and well-formed, else
    // starts a fresh root trace. Handlers just declare the extractor; they
    // never hand-repeat the header-reading/origination logic.
    TraceContextExtractor(trace_context): TraceContextExtractor,
    Json(input): Json<RegisterInput>,
) -> Result<(StatusCode, Json<RegisterOutput>), TransportError> {
    log(
        &state,
        Severity::Info,
        &format!("POST /register: user_id={}", input.user_id),
    );

    let proxy = state
        .runtime
        .resolve::<RegisterUserTag>()
        .map_err(|_| TransportError::Internal)?;

    let ctx = ServiceContext::new()
        .with_security(Arc::new(security))
        .with_tenant_id(input.tenant_id.clone())
        .with_trace_context(trace_context);

    let user_id = input.user_id.clone();
    match proxy.register(ctx, input).await {
        Ok(output) => {
            log(
                &state,
                Severity::Info,
                &format!("register_user.success: user_id={user_id}"),
            );
            Ok((StatusCode::CREATED, Json(output)))
        }
        Err(err) => {
            log(
                &state,
                Severity::Error,
                &format!("register_user.failure: user_id={user_id} error={err}"),
            );
            Err(map_register_error(err))
        }
    }
}

/// `GET /tenants/{tenant_id}/users` — queries the `UsersByTenant` read-side
/// projection (CORE-005's real read-side engine, new capability layered on
/// top of CORE-018's `RegisterUser` write path; see `crate::read_side`).
///
/// Finding 1 (security fix): this route used to have no authentication at
/// all — any unauthenticated caller could read any tenant's users/org name.
/// Now requires a valid bearer JWT (`AuthenticatedContext`, 401 pre-handler
/// on missing/invalid credentials) AND the authenticated principal's own
/// tenant must match the requested path `tenant_id` (403 otherwise).
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/users",
    params(("tenant_id" = String, Path, description = "Tenant organization to query")),
    responses(
        (status = 200, description = "Users registered under this tenant's organization, as seen by the read-side projection", body = TenantUsersView),
        (status = 401, description = "Missing or invalid credentials"),
        (status = 403, description = "Authenticated principal's tenant does not match the requested tenant_id"),
    ),
    security(("bearer_jwt" = [])),
)]
pub async fn users_by_tenant_handler(
    State(store): State<UsersByTenantStore>,
    AuthenticatedContext(security): AuthenticatedContext,
    Path(tenant_id): Path<String>,
) -> Result<Json<TenantUsersView>, TransportError> {
    match security.principal().tenant_id.as_ref() {
        Some(principal_tenant) if principal_tenant.as_str() == tenant_id => {
            Ok(Json(store.view(&tenant_id)))
        }
        _ => Err(TransportError::Forbidden),
    }
}

#[cfg(test)]
mod tests {
    use ego_security_sdk::SecurityError;

    use super::*;

    // Finding 4 (RED first): map_register_error must delegate every
    // Security(_) denial to ego-transport's own granular
    // SecurityError -> TransportError mapping (crates/transport/src/error.rs),
    // not unconditionally collapse it to 403. Before the fix, every case
    // below returns Forbidden regardless of the underlying SecurityError.
    #[test]
    fn security_denials_map_through_the_granular_transport_table() {
        let cases = [
            (
                SecurityError::AuthenticationFailed("x".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (SecurityError::MissingContext, StatusCode::UNAUTHORIZED),
            (
                SecurityError::CapabilityNotEnabled,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                SecurityError::ProviderError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                SecurityError::AuthorizationDenied { reason: "x".into() },
                StatusCode::FORBIDDEN,
            ),
            (
                SecurityError::TenantMismatch {
                    expected: "a".into(),
                    actual: "b".into(),
                },
                StatusCode::FORBIDDEN,
            ),
            (
                SecurityError::CrossTenantDenied { reason: "x".into() },
                StatusCode::FORBIDDEN,
            ),
        ];
        for (security_err, expected) in cases {
            let err = RegisterUserError::Security(security_err);
            assert_eq!(map_register_error(err).status_code(), expected);
        }
    }

    #[test]
    fn entity_write_failure_maps_to_internal() {
        let err = RegisterUserError::EntityWrite("boom".into());
        assert_eq!(
            map_register_error(err).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
