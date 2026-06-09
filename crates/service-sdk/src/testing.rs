use crate::builder::ServiceBuilderTrait;
use crate::context::ServiceContext;
use crate::contract::{ContractVersion, ServiceDescriptor};
use crate::error::{Result as ServiceResult, ServiceError};
use crate::implementation::{Service, ServiceFactory};
use crate::interceptor::Interceptor;
use async_trait::async_trait;

/// A test service.
pub struct TestService {
    descriptor: ServiceDescriptor,
}

impl Service for TestService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }
}

/// A test service factory.
pub struct TestServiceFactory;

#[async_trait]
impl ServiceFactory for TestServiceFactory {
    async fn create(&self) -> ServiceResult<Box<dyn Service>> {
        let descriptor = ServiceDescriptor {
            name: "TestService".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        };
        Ok(Box::new(TestService { descriptor }))
    }
}

/// A test service builder.
pub struct TestServiceBuilder;

impl ServiceBuilderTrait for TestServiceBuilder {
    fn builder(&self) -> crate::builder::ServiceBuilder {
        let descriptor = ServiceDescriptor {
            name: "TestService".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        };
        crate::builder::ServiceBuilder::new(descriptor)
    }
}

/// A test interceptor.
pub struct TestInterceptor;

#[async_trait]
impl Interceptor for TestInterceptor {
    async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
        Ok(())
    }

    async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
        Ok(())
    }

    async fn on_error(
        &self,
        _context: &ServiceContext,
        _error: &ServiceError,
    ) -> Result<(), ServiceError> {
        Ok(())
    }
}
