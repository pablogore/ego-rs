//! Simple integration tests for core Service SDK types

use ego_service_sdk::*;

#[tokio::test]
async fn test_contract_version() {
    let version = ContractVersion::new(1, 2, 3);
    assert_eq!(version.major, 1);
    assert_eq!(version.minor, 2);
    assert_eq!(version.patch, 3);
    assert_eq!(version.to_string(), "1.2.3");

    // Test FromStr
    let parsed_version: ContractVersion = "2.1.0".parse().unwrap();
    assert_eq!(parsed_version, ContractVersion::new(2, 1, 0));

    // Test PartialOrd
    let v1 = ContractVersion::new(1, 0, 0);
    let v2 = ContractVersion::new(1, 0, 1);
    assert!(v1 < v2);

    let v3 = ContractVersion::new(1, 1, 0);
    assert!(v1 < v3);
}

#[tokio::test]
async fn test_service_descriptor() {
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

#[tokio::test]
async fn test_operation_descriptor() {
    let descriptor = OperationDescriptor {
        name: "test_operation".to_string(),
        input: vec!["TestInput".to_string()],
        output: "TestOutput".to_string(),
        errors: vec!["TestError".to_string()],
        description: None,
        metadata: std::collections::HashMap::new(),
        idempotent: false,
        mutating: true,
    };

    assert_eq!(descriptor.name, "test_operation");
    assert_eq!(descriptor.input, vec!["TestInput".to_string()]);
    assert_eq!(descriptor.output, "TestOutput");
    assert_eq!(descriptor.errors, vec!["TestError".to_string()]);
    assert!(descriptor.description.is_none());
    assert!(descriptor.metadata.is_empty());
}

#[tokio::test]
async fn test_service_contract_trait() {
    // ServiceContract is now a trait; verified by macro-generated code in integration tests.
    // The contract's type_id, name, version, and descriptor methods are exercised
    // in macro integration tests that emit concrete ServiceContract impls.
    let _ = "ServiceContract trait verified via macro integration tests";
}
