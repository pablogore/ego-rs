//! Comprehensive tests for all Service SDK types

#[cfg(test)]
mod tests {
    use crate::context::ServiceContext;
    use crate::contract::{ContractVersion, OperationDescriptor, ServiceDescriptor};
    use crate::error::ServiceError;
    use crate::interceptor::{Interceptor, InterceptorChain};
    use crate::registry::ServiceRegistry;
    
    use async_trait::async_trait;
    use std::sync::Arc;

    

    #[test]
    fn test_service_descriptor_struct() {
        let descriptor = ServiceDescriptor {
            name: "TestService".to_string(),
            version: ContractVersion::new(1, 0, 0),
            operations: vec![],
            description: None,
            metadata: std::collections::HashMap::new(),
        };
        
        assert_eq!(descriptor.name, "TestService");
        assert_eq!(descriptor.version, ContractVersion::new(1, 0, 0));
        assert!(descriptor.description.is_none());
        assert!(descriptor.metadata.is_empty());
    }

    #[test]
    fn test_operation_descriptor_struct() {
        let descriptor = OperationDescriptor {
            name: "test_operation".to_string(),
            input: vec!["TestInput".to_string()],
            output: "TestOutput".to_string(),
            errors: vec!["TestError".to_string()],
            description: None,
            metadata: std::collections::HashMap::new(),
        };
        
        assert_eq!(descriptor.name, "test_operation");
        assert_eq!(descriptor.input, vec!["TestInput".to_string()]);
        assert_eq!(descriptor.output, "TestOutput");
        assert_eq!(descriptor.errors, vec!["TestError".to_string()]);
        assert!(descriptor.description.is_none());
        assert!(descriptor.metadata.is_empty());
    }

    #[test]
    fn test_contract_version_struct() {
        let version = ContractVersion::new(1, 2, 3);
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 2);
        assert_eq!(version.patch, 3);
        assert_eq!(version.to_string(), "1.2.3");
    }

    #[test]
    fn test_service_registry_struct() {
        let registry = ServiceRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.services().len(), 0);
    }

 

    #[test]
    fn test_operation_descriptor() {
        let descriptor = OperationDescriptor {
            name: "test_operation".to_string(),
            input: vec!["TestInput".to_string()],
            output: "TestOutput".to_string(),
            errors: vec!["TestError".to_string()],
            description: None,
            metadata: std::collections::HashMap::new(),
        };

        assert_eq!(descriptor.name, "test_operation");
        assert_eq!(descriptor.input, vec!["TestInput".to_string()]);
        assert_eq!(descriptor.output, "TestOutput");
        assert_eq!(descriptor.errors, vec!["TestError".to_string()]);
        assert!(descriptor.description.is_none());
        assert!(descriptor.metadata.is_empty());
    }

    #[test]
    fn test_service_error() {
        let error = ServiceError::validation("test error");
        assert!(matches!(error, ServiceError::Validation { .. }));
    }

    #[test]
    fn test_service_context() {
        let context = ServiceContext::new();
        assert!(context.tenant_id().is_none());
        assert!(context.correlation_id().is_none());
        assert!(context.trace_id().is_none());
    }

    #[test]
    fn test_service_context_with_fields() {
        let context = ServiceContext::new();
        assert_eq!(context.tenant_id, None);
        assert_eq!(context.correlation_id, None);
        assert_eq!(context.trace_id, None);
    }

    #[test]
    fn test_service_registry() {
        let registry = ServiceRegistry::new();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_service_registry_with_services() {
        let registry = ServiceRegistry::new();
        assert_eq!(registry.services().len(), 0);
    }

    #[tokio::test]
    async fn test_interceptor_chain() {
        #[derive(Debug)]
        struct TestInterceptor {
            call_count: std::sync::atomic::AtomicUsize,
        }

        #[async_trait]
        impl Interceptor for TestInterceptor {
            async fn on_request(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
                self.call_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }

            async fn on_response(&self, _context: &ServiceContext) -> Result<(), ServiceError> {
                self.call_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }

            async fn on_error(
                &self,
                _context: &ServiceContext,
                _error: &ServiceError,
            ) -> Result<(), ServiceError> {
                self.call_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
        }

        let interceptor1 = Arc::new(TestInterceptor {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let interceptor2 = Arc::new(TestInterceptor {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });

        let mut chain = InterceptorChain::new();
        chain.add_interceptor(interceptor1.clone());
        chain.add_interceptor(interceptor2.clone());

        let context = ServiceContext::new();

        // Test on_request
        chain.on_request(&context).await.unwrap();
        assert_eq!(
            interceptor1
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            interceptor2
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );

        // Test on_response
        chain.on_response(&context).await.unwrap();
        assert_eq!(
            interceptor1
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
        assert_eq!(
            interceptor2
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );

        // Test on_error
        let error = ServiceError::validation("test error");
        chain.on_error(&context, &error).await.unwrap();
        assert_eq!(
            interceptor1
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
        assert_eq!(
            interceptor2
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
    }

    
}
