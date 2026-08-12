//! Concrete HTTP route table (design.md AD-2: concrete routes live in
//! `reference-app`, never in the generic `ego-transport` crate).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::ReservationRejection;
use ego_transport::{
    AppState, AuthenticatedContext, OperationKeyExtractor, TraceContextExtractor, TransportError,
};
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
        // The three refusals a client can reason about all say "this key is
        // taken, by this request or another" — 409, which is the one status a
        // caller can act on without reading prose.
        RegisterUserError::Refused(
            ReservationRejection::SelfInProgress
            | ReservationRejection::OtherInProgress
            | ReservationRejection::FingerprintConflict,
        ) => TransportError::Conflict,
        // The store could not answer. 503 rather than 500: the request is
        // well-formed and may well succeed later, and a caller that stops
        // retrying on 500 would give up on something transient.
        RegisterUserError::Refused(ReservationRejection::StoreUnavailable) => {
            TransportError::ServiceUnavailable
        }
        // The three remaining cases, all needing someone to look rather than
        // something to be retried: the operation completed but its answer cannot
        // be read back; the request could not be fingerprinted at all; or no
        // tenant scope was resolved before the reservation. No amount of waiting
        // is a justified recovery for any of them, so 500 is the honest answer.
        //
        // **Named, not `_`.** This arm used to be a wildcard, and a wildcard here
        // decides the status for every variant that does not exist yet: an eighth
        // refusal would silently arrive as 500 without anyone choosing that. The
        // three arms above and this one now cover the enum exhaustively, so adding
        // a variant fails to compile until someone maps it — the same criterion
        // the reservation outcome match already holds itself to. That is worth a
        // compile error precisely because the wrong default is invisible: a 500 is
        // never obviously incorrect from the outside.
        //
        // Measured rather than assumed: adding an eighth variant to
        // `ReservationRejection` fails this build with E0004, naming the unmapped
        // variant. Checked and reverted when this arm was written.
        //
        // `TenantUnresolved` is deliberately here and not a 4xx. It means an
        // operation is marked idempotent while nothing on its path resolves a
        // scope — a wiring fault in this service, not something the caller did or
        // can fix. Reporting it as a client error would send the caller looking
        // for a mistake in a request that is fine.
        //
        // Deliberately not "retrying reproduces it exactly". That holds for the
        // stored bytes, which do not change. It does not follow for a
        // fingerprint failure: `Serialize` is satisfied at compile time, so what
        // failed is *this value*, and a hand-written impl may fail on one value
        // and succeed on the next, or depend on state outside the value
        // entirely. Same overstatement that was withdrawn from the epilogue's
        // docs — the status is chosen for the action it implies, not for a
        // prediction about recurrence.
        RegisterUserError::Refused(
            ReservationRejection::StoredResponseIncompatible
            | ReservationRejection::RequestNotFingerprintable
            | ReservationRejection::TenantUnresolved,
        ) => TransportError::Internal,
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
    // PROD-012: the `Idempotency-Key` header is read, validated and admitted or
    // refused at the boundary, once, under the runtime's own policy — the same
    // arrangement as the trace context above. A rejection happens here, before
    // `register` is invoked at all. This handler only carries the result
    // forward; it re-decides nothing and regenerates nothing.
    OperationKeyExtractor(operation_key): OperationKeyExtractor,
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

    let mut ctx = ServiceContext::new()
        .with_security(Arc::new(security))
        .with_tenant_id(input.tenant_id.clone())
        .with_trace_context(trace_context);
    // Set only when the boundary resolved one. `None` means this deployment
    // permits a keyless request, and inventing a key here would manufacture an
    // identity the caller never supplied — which is the one thing the extraction
    // contract exists to prevent.
    if let Some(key) = operation_key {
        ctx = ctx.with_operation_key(key);
    }

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

    /// Every `ReservationRejection`, pinned to the status it becomes.
    ///
    /// The HTTP-level suite (`http_replay_and_conflict.rs`) drives five of these
    /// through the real router by scripting the reservation store. The sixth,
    /// `RequestNotFingerprintable`, **cannot be provoked that way**: it is
    /// raised before the store is reached, when an operation's arguments fail to
    /// serialise, and `RegisterInput` always serialises. Its translation is
    /// therefore proven here, directly against the mapper — which is also why
    /// this test enumerates all six rather than only the one that is otherwise
    /// unreachable. A table with one entry proven somewhere else and five
    /// assumed is how a mapping drifts.
    ///
    /// The split is by what the caller can do about it, never by which enum the
    /// value came from.
    #[test]
    fn every_reservation_rejection_maps_to_the_status_its_caller_can_act_on() {
        use ego_service_sdk::runtime::ReservationRejection;

        let cases = [
            // This key is taken — by this request or another. Something a caller
            // can act on: wait, or stop reusing the key.
            (ReservationRejection::SelfInProgress, StatusCode::CONFLICT),
            (ReservationRejection::OtherInProgress, StatusCode::CONFLICT),
            (
                ReservationRejection::FingerprintConflict,
                StatusCode::CONFLICT,
            ),
            // The machinery could not answer. Well-formed, and may well succeed
            // later — a caller that gave up on a 500 would abandon something
            // transient.
            (
                ReservationRejection::StoreUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            // Neither of the above: nothing a caller can do, and waiting is not
            // a justified strategy. Someone has to look.
            (
                ReservationRejection::StoredResponseIncompatible,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                ReservationRejection::RequestNotFingerprintable,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];

        for (rejection, expected) in cases {
            assert_eq!(
                map_register_error(RegisterUserError::Refused(rejection.clone())).status_code(),
                expected,
                "{rejection:?} must translate to the status its caller can act on"
            );
        }
    }

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
