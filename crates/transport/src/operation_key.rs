//! Axum operation-key extractor — the HTTP end of the `OperationKey`
//! extraction contract.
//!
//! Three pieces already existed and nothing joined them: [`HeaderCarrier`]
//! knows *where* an HTTP request carries a key, `resolve_operation_key` knows
//! *what* to do about it, and the runtime knows which policy the deployment was
//! built under. This extractor is the join, and it is deliberately the only
//! thing it is.
//!
//! # What lives here, and what does not
//!
//! **Here:** running the extraction once, at the boundary, so handlers declare
//! it rather than repeating it — the same arrangement
//! [`TraceContextExtractor`](crate::propagation::TraceContextExtractor) uses for
//! `traceparent`, and for the same reason.
//!
//! **Not here: the policy.** Whether a missing key is admissible belongs to
//! `resolve_operation_key` and nowhere else. This module reads the configured
//! mode and hands it over; it never matches on it. A `match` on
//! `MandatoryKey`/`Compatibility` in this file would be a second definition of
//! the rule, and the one that decided whether a real request was rejected would
//! not be the one the runtime was validated against.
//!
//! **Not here: the mode's value.** It comes from the runtime, which retained
//! exactly what the builder checked at startup. This extractor invents no
//! default and reads no configuration of its own, so an HTTP deployment cannot
//! end up enforcing something different from what its runtime promised.

use async_trait::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use ego_domain::operation::OperationKey;
use ego_service_sdk::idempotency::resolve_operation_key;

use crate::error::TransportError;
use crate::idempotency::HeaderCarrier;
use crate::state::AppState;

/// The operation key this request carries, as resolved under the runtime's own
/// idempotency policy.
///
/// `None` means the request carried no key **and the deployment permits that** —
/// never that a key was present and unusable, which is rejected before a handler
/// runs. A handler puts this on the `ServiceContext` and does nothing else with
/// it: the value is carried, never regenerated.
pub struct OperationKeyExtractor(pub Option<OperationKey>);

#[async_trait]
impl<S> FromRequestParts<S> for OperationKeyExtractor
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = TransportError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let carrier = HeaderCarrier(&parts.headers);

        // The mode is read, passed on, and never interpreted here.
        let mode = state.runtime.idempotency_enforcement_mode();

        resolve_operation_key(&carrier, mode)
            .map(OperationKeyExtractor)
            // Every rejection is the client's request being unusable, which is a
            // 400 — including a missing key under a runtime that requires one.
            // Deliberately not 401/403: nothing about identity or permission
            // failed, and mapping it there would send a caller looking for the
            // wrong fix.
            .map_err(|_| TransportError::BadRequest)
    }
}
