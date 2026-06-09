//! Unit tests for service SDK components with mocks

#[cfg(test)]
mod tests {
    use crate::contract::{ContractVersion, OperationDescriptor, ServiceDescriptor};
    use crate::registry::ServiceRegistry;

    

    

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

 
}