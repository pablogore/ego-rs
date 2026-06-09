use crate::context::ServiceContext;
use crate::contract::{ContractVersion, ServiceDescriptor};
use crate::error::{Result as ServiceResult, ServiceError};
use crate::implementation::Service;
use crate::interceptor::{Interceptor, InterceptorChain};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Service reference for invoking services.
///
/// This is a proxy type that allows calling services without direct dependencies.
#[async_trait]
pub trait ServiceReference: Send + Sync {
    /// Returns the service descriptor for this reference.
    fn descriptor(&self) -> &ServiceDescriptor;

    /// Returns the service name.
    fn name(&self) -> &str {
        &self.descriptor().name
    }

    /// Returns the service version.
    fn version(&self) -> &ContractVersion {
        &self.descriptor().version
    }

    /// Returns the service metadata.
    fn metadata(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Invokes an operation on the service.
    ///
    /// This is a generic invocation method that can be used to call any operation
    /// on the service.
    async fn invoke(&self, operation: &str, input: Option<&[u8]>) -> ServiceResult<Vec<u8>>;
}

/// A service reference that can be used to invoke services.
pub struct ServiceRef<T: Service> {
    service: Arc<T>,
    interceptor_chain: InterceptorChain,
}

impl<T: Service> ServiceRef<T> {
    /// Creates a new service reference.
    pub fn new(service: Arc<T>) -> Self {
        Self {
            service,
            interceptor_chain: InterceptorChain::default(),
        }
    }

    /// Adds an interceptor to the chain.
    pub fn add_interceptor(&mut self, interceptor: Arc<dyn Interceptor>) {
        self.interceptor_chain.add_interceptor(interceptor);
    }
}

#[async_trait]
impl<T: Service> ServiceReference for ServiceRef<T> {
    fn descriptor(&self) -> &ServiceDescriptor {
        self.service.descriptor()
    }

    async fn invoke(&self, _operation: &str, _input: Option<&[u8]>) -> ServiceResult<Vec<u8>> {
        // Create a new context for now to avoid compilation issues
        let context = ServiceContext::new();

        // Run the interceptor chain on request
        self.interceptor_chain.on_request(&context).await?;

        // For now, just return an error to avoid compilation issues
        // In a real implementation, this would call the actual service operation
        Err(ServiceError::internal(
            "Service invocation not implemented in this test",
        ))
    }
}
