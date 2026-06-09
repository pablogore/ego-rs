use ego_service_sdk::{ContractVersion, ServiceContract, OperationContract, ServiceDescriptor, OperationDescriptor};

#[test]
fn test_contract_version() {
    let version = ContractVersion::new(1, 2, 3);
    assert_eq!(version.major, 1);
    assert_eq!(version.minor, 2);
    assert_eq!(version.patch, 3);
    assert_eq!(version.to_string(), "1.2.3");
}

#[test]
fn test_service_descriptor() {
    let descriptor = ServiceDescriptor {
        name: "TestService".to_string(),
        version: ContractVersion::new(1, 0, 0),
        operations: vec![],
        description: Some("A test service".to_string()),
        metadata: Default::default(),
    };
    
    assert_eq!(descriptor.name, "TestService");
    assert_eq!(descriptor.version, ContractVersion::new(1, 0, 0));
    assert_eq!(descriptor.description, Some("A test service".to_string()));
}

#[test]
fn test_operation_descriptor() {
    let descriptor = OperationDescriptor {
        name: "test_operation".to_string(),
        input: "TestInput".to_string(),
        output: "TestOutput".to_string(),
        errors: vec!["TestError".to_string()],
        description: Some("A test operation".to_string()),
        metadata: Default::default(),
    };
    
    assert_eq!(descriptor.name, "test_operation");
    assert_eq!(descriptor.input, "TestInput");
    assert_eq!(descriptor.output, "TestOutput");
    assert_eq!(descriptor.errors, vec!["TestError".to_string()]);
    assert_eq!(descriptor.description, Some("A test operation".to_string()));
}

#[test]
fn test_service_contract() {
    let contract = ServiceContract {
        name: "TestService".to_string(),
        version: ContractVersion::new(1, 0, 0),
        operations: vec![OperationContract {
            name: "test_operation".to_string(),
            input: "TestInput".to_string(),
            output: "TestOutput".to_string(),
            errors: vec!["TestError".to_string()],
            description: None,
            metadata: Default::default(),
        }],
        description: Some("A test service".to_string()),
        metadata: Default::default(),
    };
    
    assert_eq!(contract.name, "TestService");
    assert_eq!(contract.version, ContractVersion::new(1, 0, 0));
    assert_eq!(contract.operations.len(), 1);
    assert_eq!(contract.description, Some("A test service".to_string()));
}