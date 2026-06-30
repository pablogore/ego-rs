//! Transport-agnostic authentication interceptor.
//!
//! Extracts credentials, authenticates, and populates `ServiceContext`.
//! Never produces transport-specific responses — propagates `AuthenticationError`
//! to the caller (transport adapters handle protocol mapping).
//!
//! Implements the `Interceptor` trait. Adapter crates may call either the
//! trait method or the inherent `intercept` closure form directly.

use std::sync::Arc;

use ego_domain::auth::AuthenticationError;

use crate::authentication::AuthenticationProvider;
use crate::credential_extractor::{CredentialExtractor, RequestContext};

// We avoid importing ServiceContext from service-sdk here — that would
// create a circular dependency. The interceptor populates a security context
// via a callback-style approach. Instead we define the minimal contract:
// callers pass a setter closure. This keeps security-sdk free of service-sdk.
//
// ponytail: ServiceContext lives in service-sdk which depends on security-sdk.
//           To avoid the cycle, AuthenticationInterceptor takes a generic
//           `SecurityContextSetter` rather than `&mut ServiceContext` directly.
//           Adapter crates (which can depend on both) call the concrete form.

/// Authenticates incoming requests and sets the security context on the service context.
///
/// Transport-agnostic: depends on `CredentialExtractor` + `AuthenticationProvider`.
/// Never produces HTTP responses. The caller (transport adapter) handles error mapping.
///
/// ## Two intercept surfaces
///
/// This type exposes two `intercept` methods:
///
/// - **[`Interceptor::intercept`]** (trait method) — returns `Result<Option<SecurityContext>, ...>`.
///   Use this when wiring into a pipeline that receives the context as a return value
///   (e.g. routing middleware that stores context in request extensions after the call).
///
/// - **Inherent `intercept`** (closure form) — accepts `set_security: impl FnOnce(SecurityContext)`.
///   Use this in adapter crates that already hold a mutable reference to a service/request context
///   and want to populate it inline without returning the security context up the stack.
///   This form exists to avoid a circular dependency: `ServiceContext` (in `service-sdk`) depends
///   on `security-sdk`, so `AuthenticationInterceptor` cannot reference it directly.
///
/// When in doubt, prefer the [`Interceptor`] trait method.
pub struct AuthenticationInterceptor {
    extractor: Arc<dyn CredentialExtractor>,
    provider: Arc<dyn AuthenticationProvider>,
}

impl AuthenticationInterceptor {
    /// Create a new interceptor with the given extractor and provider.
    pub fn new(
        extractor: Arc<dyn CredentialExtractor>,
        provider: Arc<dyn AuthenticationProvider>,
    ) -> Self {
        Self { extractor, provider }
    }

    /// Extract a credential from `ctx` and authenticate it.
    ///
    /// On success: calls `set_security` with the resulting `SecurityContext`.
    /// On missing credential: passes through (no call to `set_security`).
    /// On error: returns `Err(AuthenticationError)` — caller handles mapping.
    pub fn intercept(
        &self,
        ctx: &dyn RequestContext,
        set_security: impl FnOnce(crate::context::SecurityContext),
    ) -> Result<(), AuthenticationError> {
        let credential = self.extractor.extract(ctx)?;
        if let Some(cred) = credential {
            let security_context = self.provider.authenticate(&cred)?;
            set_security(security_context);
        }
        Ok(())
    }
}

impl crate::interceptor::Interceptor for AuthenticationInterceptor {
    fn intercept(
        &self,
        ctx: &dyn crate::credential_extractor::RequestContext,
    ) -> Result<Option<crate::context::SecurityContext>, ego_domain::auth::AuthenticationError> {
        let credential = self.extractor.extract(ctx)?;
        match credential {
            Some(cred) => Ok(Some(self.provider.authenticate(&cred)?)),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use ego_domain::auth::{AuthenticationError, Claims, Credential};
    use crate::authentication::AuthenticationProvider;
    use crate::context::SecurityContext;
    use crate::credential_extractor::{CredentialExtractor, RequestContext};
    use crate::principal::{Principal, PrincipalKind, SubjectId};

    // --- Test doubles ---

    struct MapCtx(HashMap<String, String>);

    impl MapCtx {
        fn new(headers: &[(&str, &str)]) -> Self {
            let mut map = HashMap::new();
            for (k, v) in headers {
                map.insert(k.to_lowercase(), v.to_string());
            }
            Self(map)
        }
    }

    impl RequestContext for MapCtx {
        fn header(&self, name: &str) -> Option<&str> {
            self.0.get(&name.to_lowercase()).map(String::as_str)
        }
        fn metadata(&self, _: &str) -> Option<&str> { None }
        fn query_param(&self, _: &str) -> Option<&str> { None }
    }

    struct AlwaysBearerExtractor;

    impl CredentialExtractor for AlwaysBearerExtractor {
        fn extract(&self, _: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
            Ok(Some(Credential::Bearer("tok".into())))
        }
    }

    struct NoCredentialExtractor;

    impl CredentialExtractor for NoCredentialExtractor {
        fn extract(&self, _: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
            Ok(None)
        }
    }

    struct FailingExtractor;

    impl CredentialExtractor for FailingExtractor {
        fn extract(&self, _: &dyn RequestContext) -> Result<Option<Credential>, AuthenticationError> {
            Err(AuthenticationError::InvalidToken("malformed".into()))
        }
    }

    fn ok_security_context() -> SecurityContext {
        let principal = Principal::new(
            PrincipalKind::User,
            SubjectId::new("user-1").unwrap(),
        );
        SecurityContext::new(principal, Claims::empty())
    }

    struct OkProvider;

    impl AuthenticationProvider for OkProvider {
        fn authenticate(&self, _: &Credential) -> Result<SecurityContext, AuthenticationError> {
            Ok(ok_security_context())
        }
    }

    struct FailingProvider;

    impl AuthenticationProvider for FailingProvider {
        fn authenticate(&self, _: &Credential) -> Result<SecurityContext, AuthenticationError> {
            Err(AuthenticationError::InvalidSignature)
        }
    }

    // --- Tests ---

    #[test]
    fn intercept_populates_security_context_on_valid_credential() {
        let interceptor = AuthenticationInterceptor::new(
            Arc::new(AlwaysBearerExtractor),
            Arc::new(OkProvider),
        );
        let ctx = MapCtx::new(&[]);
        let mut set_called = false;
        interceptor
            .intercept(&ctx, |_sec| { set_called = true; })
            .unwrap();
        assert!(set_called);
    }

    #[test]
    fn intercept_passes_through_when_no_credential() {
        let interceptor = AuthenticationInterceptor::new(
            Arc::new(NoCredentialExtractor),
            Arc::new(OkProvider),
        );
        let ctx = MapCtx::new(&[]);
        let mut set_called = false;
        interceptor
            .intercept(&ctx, |_| { set_called = true; })
            .unwrap();
        assert!(!set_called, "set_security must not be called with no credential");
    }

    #[test]
    fn intercept_propagates_extractor_error() {
        let interceptor = AuthenticationInterceptor::new(
            Arc::new(FailingExtractor),
            Arc::new(OkProvider),
        );
        let ctx = MapCtx::new(&[]);
        let err = interceptor.intercept(&ctx, |_| {}).unwrap_err();
        assert!(matches!(err, AuthenticationError::InvalidToken(_)));
    }

    #[test]
    fn intercept_propagates_provider_error() {
        let interceptor = AuthenticationInterceptor::new(
            Arc::new(AlwaysBearerExtractor),
            Arc::new(FailingProvider),
        );
        let ctx = MapCtx::new(&[]);
        let err = interceptor.intercept(&ctx, |_| {}).unwrap_err();
        assert_eq!(err, AuthenticationError::InvalidSignature);
    }

    #[test]
    fn interceptor_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AuthenticationInterceptor>();
    }
}
