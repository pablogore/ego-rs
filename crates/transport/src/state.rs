//! Generic axum application state (AD-1, AD-2).
//!
//! `AppState` carries only what any axum handler needs: the DI runtime to
//! resolve services through, and the authentication provider used to build
//! a `SecurityContext` from an incoming request. It knows nothing about any
//! concrete route or service type — those belong to the application that
//! mounts this crate's router.
//!
//! Per design.md's corrected AD-3: `authn` is carried directly here,
//! constructed by the caller, rather than fished from `Runtime` internals.

use std::sync::Arc;

use ego_security_sdk::AuthenticationProvider;
use ego_service_sdk::runtime::Runtime;

/// Shared axum application state. `Clone` is cheap — both fields are
/// `Arc`-backed.
#[derive(Clone)]
pub struct AppState {
    /// The DI runtime handlers resolve services through.
    pub runtime: Arc<Runtime>,
    /// The authentication provider used to authenticate incoming credentials.
    pub authn: Arc<dyn AuthenticationProvider>,
}

impl AppState {
    /// Builds a new `AppState` from an already-constructed runtime and
    /// authentication provider.
    pub fn new(runtime: Arc<Runtime>, authn: Arc<dyn AuthenticationProvider>) -> Self {
        Self { runtime, authn }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ego_domain::auth::AuthenticationError;
    use ego_security_sdk::{AuthenticationProvider, Credential, SecurityContext};
    use ego_service_sdk::context::ServiceContext;
    use ego_service_sdk::error::ServiceError;
    use ego_service_sdk::runtime::RuntimeBuilder;
    #[allow(unused_imports)]
    use ego_service_sdk_macros::operation;
    use ego_service_sdk_macros::service;

    use super::AppState;

    struct StubAuthn;

    impl AuthenticationProvider for StubAuthn {
        fn authenticate(&self, _credential: &Credential) -> Result<SecurityContext, AuthenticationError> {
            unimplemented!("not exercised by this test")
        }
    }

    #[service(version = "1.0.0")]
    pub trait Echo {
        #[operation]
        async fn echo(&self, ctx: ServiceContext, input: String) -> Result<String, ServiceError>;
    }

    struct EchoImpl;

    #[async_trait]
    impl Echo for EchoImpl {
        async fn echo(&self, _ctx: ServiceContext, input: String) -> Result<String, ServiceError> {
            Ok(input)
        }
    }

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone<T: Clone>() {}

    // TASK-003 (RED): AppState is Clone + Send + Sync.
    #[test]
    fn app_state_is_clone_send_sync() {
        assert_send_sync::<AppState>();
        assert_clone::<AppState>();
    }

    // TASK-003 (RED): a tag registered on the inner Runtime resolves through AppState.
    #[tokio::test]
    async fn registered_tag_resolves_through_app_state() {
        let echo: Arc<dyn Echo> = Arc::new(EchoImpl);
        let rt = RuntimeBuilder::new()
            .with_service::<EchoTag>(echo)
            .expect("registers cleanly")
            .build();

        let state = AppState::new(Arc::new(rt), Arc::new(StubAuthn));

        let proxy = state
            .runtime
            .resolve::<EchoTag>()
            .expect("registered tag resolves");
        let out = proxy.echo(ServiceContext::new(), "hi".into()).await.unwrap();
        assert_eq!(out, "hi");
    }
}
