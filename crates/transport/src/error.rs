//! Maps internal service/security errors to HTTP responses (http-transport
//! spec: "Success/Error Response Contract"). Every [`TransportError`] variant
//! carries only a fixed, safe reason string — never the source error's raw
//! message, which may contain internal diagnostic detail.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use ego_security_sdk::SecurityError;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk::idempotency::OperationKeyRejection;
use serde_json::json;

/// A transport-facing error category. Maps 1:1 to an HTTP status code and a
/// fixed reason phrase — no caller-supplied text ever reaches the response
/// body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Malformed or invalid input.
    BadRequest,
    /// Missing or invalid credentials.
    Unauthorized,
    /// Authenticated but not permitted.
    Forbidden,
    /// The requested resource does not exist.
    NotFound,
    /// The request conflicts with the current state.
    Conflict,
    /// The caller exceeded a rate limit.
    TooManyRequests,
    /// The operation exceeded its time budget.
    GatewayTimeout,
    /// The service is temporarily unavailable.
    ServiceUnavailable,
    /// An unexpected internal failure.
    Internal,
}

impl TransportError {
    /// The HTTP status code for this error category.
    pub fn status_code(self) -> StatusCode {
        match self {
            TransportError::BadRequest => StatusCode::BAD_REQUEST,
            TransportError::Unauthorized => StatusCode::UNAUTHORIZED,
            TransportError::Forbidden => StatusCode::FORBIDDEN,
            TransportError::NotFound => StatusCode::NOT_FOUND,
            TransportError::Conflict => StatusCode::CONFLICT,
            TransportError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            TransportError::GatewayTimeout => StatusCode::GATEWAY_TIMEOUT,
            TransportError::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            TransportError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// A fixed, safe-to-expose reason string. Never derived from the source
    /// error's message — that is the whole point of this mapper.
    fn reason(self) -> &'static str {
        match self {
            TransportError::BadRequest => "bad request",
            TransportError::Unauthorized => "unauthorized",
            TransportError::Forbidden => "forbidden",
            TransportError::NotFound => "not found",
            TransportError::Conflict => "conflict",
            TransportError::TooManyRequests => "too many requests",
            TransportError::GatewayTimeout => "gateway timeout",
            TransportError::ServiceUnavailable => "service unavailable",
            TransportError::Internal => "internal error",
        }
    }
}

impl IntoResponse for TransportError {
    fn into_response(self) -> Response {
        (self.status_code(), Json(json!({ "error": self.reason() }))).into_response()
    }
}

impl From<ServiceError> for TransportError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::Validation { .. } => TransportError::BadRequest,
            ServiceError::Authorization { .. } => TransportError::Forbidden,
            ServiceError::Internal { .. } => TransportError::Internal,
            ServiceError::NotFound { .. } => TransportError::NotFound,
            ServiceError::Conflict { .. } => TransportError::Conflict,
            ServiceError::Timeout { .. } => TransportError::GatewayTimeout,
            ServiceError::RateLimit { .. } => TransportError::TooManyRequests,
            ServiceError::ServiceUnavailable { .. } => TransportError::ServiceUnavailable,
            ServiceError::BusinessLogic { .. } => TransportError::Conflict,
            ServiceError::Custom { .. } => TransportError::Internal,
        }
    }
}

impl From<OperationKeyRejection> for TransportError {
    /// A missing, invalid or unreadable `Idempotency-Key` are all
    /// caller-supplied input that failed a rule before any handler ran — the
    /// same category `ServiceError::Validation` and
    /// `SecurityError::InvalidAccessRequest` already map to. None is a resource
    /// conflict (`Conflict` is reserved for a same-key-different-fingerprint
    /// replay mismatch downstream) and none is a credential failure
    /// (`Unauthorized`/`Forbidden`), so `BadRequest` is the closest existing
    /// category rather than a new one.
    ///
    /// They collapse to one status deliberately: the three reasons are kept
    /// distinct upstream so the rejection can say *which* rule failed in a
    /// diagnostic, but a client cannot act differently on them, and splitting
    /// the status would leak how the key was judged.
    fn from(err: OperationKeyRejection) -> Self {
        match err {
            OperationKeyRejection::Missing { .. } => TransportError::BadRequest,
            OperationKeyRejection::Invalid { .. } => TransportError::BadRequest,
            OperationKeyRejection::Unreadable { .. } => TransportError::BadRequest,
        }
    }
}

impl From<SecurityError> for TransportError {
    fn from(err: SecurityError) -> Self {
        match err {
            SecurityError::AuthenticationFailed(_) => TransportError::Unauthorized,
            SecurityError::InvalidCredential(_) => TransportError::Unauthorized,
            SecurityError::InvalidSubjectId(_) => TransportError::Unauthorized,
            SecurityError::AuthorizationDenied { .. } => TransportError::Forbidden,
            SecurityError::MissingContext => TransportError::Unauthorized,
            SecurityError::CapabilityNotEnabled => TransportError::Internal,
            SecurityError::ProviderError(_) => TransportError::Internal,
            SecurityError::InvalidAccessRequest(_) => TransportError::BadRequest,
            SecurityError::TenantMismatch { .. } => TransportError::Forbidden,
            SecurityError::CrossTenantDenied { .. } => TransportError::Forbidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_text(resp: Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // TASK-001 (RED): ServiceError -> StatusCode table.
    #[test]
    fn service_error_status_table() {
        let cases = [
            (ServiceError::validation("x"), StatusCode::BAD_REQUEST),
            (ServiceError::authorization("x"), StatusCode::FORBIDDEN),
            (
                ServiceError::internal("x"),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (ServiceError::not_found("x"), StatusCode::NOT_FOUND),
            (ServiceError::conflict("x"), StatusCode::CONFLICT),
            (ServiceError::timeout("x"), StatusCode::GATEWAY_TIMEOUT),
            (ServiceError::rate_limit("x"), StatusCode::TOO_MANY_REQUESTS),
            (
                ServiceError::service_unavailable("x"),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (ServiceError::business_logic("x"), StatusCode::CONFLICT),
            (ServiceError::custom("x"), StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (err, expected) in cases {
            let mapped: TransportError = err.into();
            assert_eq!(mapped.status_code(), expected);
        }
    }

    // TASK-001 (RED): SecurityError -> StatusCode table.
    #[test]
    fn security_error_status_table() {
        let cases = [
            (
                SecurityError::AuthenticationFailed("x".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                SecurityError::InvalidCredential("x".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                SecurityError::InvalidSubjectId("x".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                SecurityError::AuthorizationDenied { reason: "x".into() },
                StatusCode::FORBIDDEN,
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
                SecurityError::InvalidAccessRequest("x".into()),
                StatusCode::BAD_REQUEST,
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
        for (err, expected) in cases {
            let mapped: TransportError = err.into();
            assert_eq!(mapped.status_code(), expected);
        }
    }

    // RED: a rejected OperationKey extraction (missing or invalid) maps to the
    // same status category as other caller-supplied-input failures rejected
    // before a handler runs (ServiceError::Validation, SecurityError::
    // InvalidAccessRequest) — a bad request, never a server-side category.
    // All three reasons collapse to one status: distinct upstream so a rejection
    // can name which rule failed, identical here because a client cannot act
    // differently on them and splitting would leak how the key was judged.
    #[test]
    fn operation_key_rejection_status_table() {
        let cases = [
            (
                OperationKeyRejection::Missing {
                    carrier: "http:Idempotency-Key",
                },
                StatusCode::BAD_REQUEST,
            ),
            (
                OperationKeyRejection::Invalid {
                    carrier: "http:Idempotency-Key",
                    source: ego_domain::operation::OperationKeyError::Empty,
                },
                StatusCode::BAD_REQUEST,
            ),
            // The third reason. The exhaustive match already guarantees it is
            // mapped at all; this asserts it collapses to the *same* status as
            // the other two, which is the claim the table actually makes.
            (
                OperationKeyRejection::Unreadable {
                    carrier: "http:Idempotency-Key",
                },
                StatusCode::BAD_REQUEST,
            ),
        ];
        for (err, expected) in cases {
            let mapped: TransportError = err.into();
            assert_eq!(mapped.status_code(), expected);
        }
    }

    // TASK-001 (RED): no raw error Debug/message text leaks into the response body.
    #[tokio::test]
    async fn response_body_never_leaks_raw_error_message() {
        let secret = "super-secret-internal-diagnostic-detail";
        let errors: Vec<TransportError> = vec![
            ServiceError::internal(secret).into(),
            ServiceError::validation(secret).into(),
            SecurityError::AuthenticationFailed(secret.into()).into(),
            SecurityError::ProviderError(secret.into()).into(),
        ];
        for err in errors {
            let resp = err.into_response();
            let text = body_text(resp).await;
            assert!(
                !text.contains(secret),
                "response body leaked internal diagnostic detail: {text}"
            );
        }
    }
}
