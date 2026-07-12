//! Axum security-context extractor (AD-3).
//!
//! Reuses `security-sdk`'s transport-neutral `BearerExtractor`/
//! `RequestContext` and the runtime's registered `AuthenticationProvider` —
//! no bearer/JWT parsing is reinvented here. Missing or invalid credentials
//! reject with 401 before any handler runs (http-transport spec: "Security
//! Context Extraction From Requests").

use async_trait::async_trait;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use ego_security_sdk::{BearerExtractor, CredentialExtractor, RequestContext, SecurityContext};

use crate::error::TransportError;
use crate::state::AppState;

/// Wraps axum's `HeaderMap` so `security-sdk`'s extractors can read it
/// without any transport-specific header-parsing logic living in
/// `security-sdk` itself.
pub struct AxumRequestContext<'a>(pub &'a HeaderMap);

impl RequestContext for AxumRequestContext<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.0.get(name)?.to_str().ok()
    }

    fn metadata(&self, _key: &str) -> Option<&str> {
        None
    }

    fn query_param(&self, _name: &str) -> Option<&str> {
        None
    }
}

/// An authenticated request's `SecurityContext`, extracted before the
/// handler runs.
pub struct AuthenticatedContext(pub SecurityContext);

/// Generic over any axum state `S` an `AppState` can be extracted from via
/// `FromRef` — not just `S = AppState` directly. This is what lets a route
/// mounted on a substate (e.g. `(AppState, SomeQueryStore)`, axum's
/// substate pattern) still require authentication without forcing every
/// route in the app onto one combined state struct (AD-2: routes each own
/// only the state they need).
#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedContext
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = TransportError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let ctx = AxumRequestContext(&parts.headers);
        let credential = BearerExtractor
            .extract(&ctx)
            .map_err(|_| TransportError::Unauthorized)?
            .ok_or(TransportError::Unauthorized)?;
        let security_context = state
            .authn
            .authenticate(&credential)
            .map_err(|_| TransportError::Unauthorized)?;
        Ok(AuthenticatedContext(security_context))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    // TASK-005 (RED): header lookup is case-insensitive, mirroring
    // credential_extractor.rs's MockRequestContext tests.
    #[test]
    fn header_lookup_is_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer tok"));
        let ctx = AxumRequestContext(&headers);
        assert_eq!(ctx.header("authorization"), Some("Bearer tok"));
        assert_eq!(ctx.header("AUTHORIZATION"), Some("Bearer tok"));
    }

    #[test]
    fn header_absent_returns_none() {
        let headers = HeaderMap::new();
        let ctx = AxumRequestContext(&headers);
        assert_eq!(ctx.header("authorization"), None);
    }

    #[test]
    fn metadata_and_query_param_are_always_none() {
        let headers = HeaderMap::new();
        let ctx = AxumRequestContext(&headers);
        assert_eq!(ctx.metadata("anything"), None);
        assert_eq!(ctx.query_param("anything"), None);
    }
}
