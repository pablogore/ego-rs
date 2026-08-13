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
use ego_service_sdk::idempotency::{resolve_operation_key, OperationKeyRejection};

use crate::error::TransportError;
use crate::idempotency::HeaderCarrier;
use crate::state::AppState;

/// The static metric name each rejection folds into.
///
/// Three names, not AD-10's two. `Unreadable` is a variant that table does not list,
/// and it gets its own name rather than being folded into `invalid`: the rejection type
/// keeps the two apart on purpose — no `OperationKeyError` describes a value that never
/// became a string — and collapsing them here would discard exactly the distinction it
/// was split to preserve. An operator seeing `unreadable` is looking at a transport or
/// encoding problem; `invalid` is a client sending a malformed key.
fn key_rejected_metric(rejection: &OperationKeyRejection) -> &'static str {
    match rejection {
        OperationKeyRejection::Missing { .. } => "idempotency.key.rejected.missing",
        OperationKeyRejection::Invalid { .. } => "idempotency.key.rejected.invalid",
        OperationKeyRejection::Unreadable { .. } => "idempotency.key.rejected.unreadable",
    }
}

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
            .map_err(|rejection| {
                // AD-10's `idempotency.key.rejected`, counted here because this is the
                // only place a rejection exists: `resolve_operation_key` returns it and
                // the mapping below discards it, so a counter anywhere downstream would
                // have nothing left to count.
                //
                // The reason is folded into the name — `Observability::metric` takes a
                // name and a value and has no attribute parameter. The variants are a
                // closed enum, and the match is exhaustive with no wildcard so a fourth
                // rejection added upstream breaks the build rather than being counted
                // as whichever arm happened to be last.
                //
                // `carrier` is deliberately *not* folded in. It would multiply against
                // the reason, and it grows with adapters rather than being closed. The
                // value stays available on the rejection for a future dimensional API;
                // what is dropped is only its use as a metric dimension.
                if let Some(obs) = state.runtime.observability() {
                    obs.metric(key_rejected_metric(&rejection), 1.0);
                }
                TransportError::BadRequest
            })
    }
}
