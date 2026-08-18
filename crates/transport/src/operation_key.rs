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
use ego_domain::MetricAttribute;
use ego_service_sdk::idempotency::{resolve_operation_key, OperationKeyRejection};

use crate::error::TransportError;
use crate::idempotency::HeaderCarrier;
use crate::state::AppState;

/// AD-10's `reason` and `carrier` for one rejection.
///
/// Four reasons, one per rejection. `unreadable` is its own value rather than being
/// folded into `invalid`, even though both end as the same status code: the rejection
/// type keeps the two apart on purpose — no `OperationKeyError` describes a value that
/// never became a string — and collapsing them here would discard exactly the
/// distinction it was split to preserve. An operator seeing `unreadable` is looking at
/// a transport or encoding problem; `invalid` is a client sending a malformed key
/// (AD-10b).
///
/// `ambiguous` is separate for the same reason and points somewhere else again: the
/// caller supplied several keys at one location, or one value that already reads as
/// several. Nothing about it need be malformed, so reporting it as `invalid` would
/// send an operator looking at the key's contents when the defect is how many of them
/// arrived — which is far more often a proxy coalescing or replaying a header than a
/// client bug.
///
/// The carrier is **read from the rejection**, never re-derived from the request. It
/// was set from `OperationKeyCarrier::carrier_name` when the rejection was built — a
/// fixed string naming a stable location, `"http:Idempotency-Key"`, and never the
/// value found there. Deriving it again here would create a second place for the two
/// to disagree, and the one that matters is what the rejection actually witnessed.
///
/// Both are `&'static str`, which is what keeps them admissible as dimensions: each
/// is drawn from a set fixed at compile time, where the raw key is caller-supplied
/// and unbounded.
///
/// Exhaustive with no wildcard: a further rejection added upstream breaks the build
/// rather than being reported as whichever arm happened to be last. That gate has
/// already done its job once — `ambiguous` arrived this way — so it stays.
fn key_rejected_attributes(rejection: &OperationKeyRejection) -> (&'static str, &'static str) {
    match rejection {
        OperationKeyRejection::Missing { carrier } => ("missing", carrier),
        OperationKeyRejection::Invalid { carrier, .. } => ("invalid", carrier),
        OperationKeyRejection::Unreadable { carrier } => ("unreadable", carrier),
        OperationKeyRejection::Ambiguous { carrier } => ("ambiguous", carrier),
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
                // Counted here because this is the only place a rejection exists:
                // `resolve_operation_key` returns it and the mapping below discards it,
                // so a counter anywhere downstream would have nothing left to count.
                //
                // One name, two dimensions. `carrier` could never have been folded
                // into the name — it multiplies against the reason and grows with
                // adapters rather than being closed — so before the port carried
                // attributes it was simply not emitted at all. It is now.
                if let Some(obs) = state.runtime.observability() {
                    let (reason, carrier) = key_rejected_attributes(&rejection);
                    obs.counter(
                        "idempotency.key.rejected",
                        1.0,
                        &[
                            MetricAttribute::new("reason", reason),
                            MetricAttribute::new("carrier", carrier),
                        ],
                    );
                }
                TransportError::BadRequest
            })
    }
}
