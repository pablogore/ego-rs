use std::sync::Arc;

use ego_security_sdk::authentication::AuthenticationProvider;
use ego_security_sdk::authorization::AuthorizationProvider;

use crate::interceptor::InterceptorChain;
use crate::registry::ServiceRegistry;
use crate::runtime::runtime_builder::RuntimeInner;

/// Builder for constructing a [`Runtime`] with optional security providers.
pub struct RuntimeBuilder {
    registry: ServiceRegistry,
    interceptor_chain: Arc<InterceptorChain>,
    authn: Option<Arc<dyn AuthenticationProvider>>,
    authz: Option<Arc<dyn AuthorizationProvider>>,
}

impl RuntimeBuilder {
    /// Creates a new `RuntimeBuilder` with default (empty) configuration.
    pub fn new() -> Self {
        Self {
            registry: ServiceRegistry::new(),
            interceptor_chain: Arc::new(InterceptorChain::new()),
            authn: None,
            authz: None,
        }
    }

    /// Registers authentication and authorization providers for this runtime.
    pub fn with_security(
        self,
        authn: Arc<dyn AuthenticationProvider>,
        authz: Arc<dyn AuthorizationProvider>,
    ) -> Self {
        Self {
            authn: Some(authn),
            authz: Some(authz),
            ..self
        }
    }

    /// Consumes the builder and produces a [`Runtime`].
    ///
    /// Always succeeds — security is optional.
    pub fn build(self) -> Runtime {
        let security_providers = match (self.authn, self.authz) {
            (Some(authn), Some(authz)) => Some((authn, authz)),
            _ => None,
        };
        Runtime {
            inner: Arc::new(RuntimeInner::new(
                self.registry,
                self.interceptor_chain,
                security_providers,
            )),
        }
    }
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A configured runtime handle wrapping shared [`RuntimeInner`] state.
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

impl Runtime {
    /// Returns a reference to the inner [`RuntimeInner`].
    pub fn inner(&self) -> &Arc<RuntimeInner> {
        &self.inner
    }

    /// Returns the registered security providers, if any.
    pub fn security_providers(
        &self,
    ) -> Option<&(Arc<dyn AuthenticationProvider>, Arc<dyn AuthorizationProvider>)> {
        self.inner.security_providers.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ego_security_sdk::authentication::AuthenticationProvider;
    use ego_security_sdk::authorization::{
        AuthorizationDecision, AuthorizationProvider,
    };
    use ego_security_sdk::context::SecurityContext;
    use ego_security_sdk::credential::Credential;
    use ego_security_sdk::error::SecurityError;
    use ego_security_sdk::principal::{Principal, PrincipalKind, SubjectId};
    use ego_security_sdk::AuthenticationError;

    use super::{Runtime, RuntimeBuilder};

    struct StubAuthn;

    impl AuthenticationProvider for StubAuthn {
        fn authenticate(&self, _credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
            let subject = SubjectId::new("user:stub").unwrap();
            let principal = Principal::new(PrincipalKind::User, subject);
            Ok(SecurityContext::new(principal))
        }
    }

    struct StubAuthz;

    #[async_trait]
    impl AuthorizationProvider for StubAuthz {
        async fn authorize(
            &self,
            _principal: &Principal,
            _request: &ego_security_sdk::authorization::AccessRequest,
            _ctx: &ego_security_sdk::context::SecurityContext,
        ) -> Result<AuthorizationDecision, SecurityError> {
            Ok(AuthorizationDecision::Allow)
        }
    }

    #[test]
    fn build_without_security_succeeds() {
        let rt = RuntimeBuilder::new().build();
        assert!(rt.security_providers().is_none());
    }

    #[test]
    fn build_with_security_succeeds() {
        let rt = RuntimeBuilder::new()
            .with_security(Arc::new(StubAuthn), Arc::new(StubAuthz))
            .build();
        assert!(rt.security_providers().is_some());
    }

    #[test]
    fn runtime_inner_is_accessible() {
        let rt = RuntimeBuilder::new().build();
        let _inner = rt.inner();
    }

    #[test]
    fn runtime_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Runtime>();
    }
}
