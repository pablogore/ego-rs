use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A service contract descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: ContractVersion,
    /// The operations available on the service.
    pub operations: Vec<OperationDescriptor>,
    /// The description of the service.
    pub description: Option<String>,
    /// The metadata of the service.
    pub metadata: HashMap<String, String>,
}

/// An operation descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDescriptor {
    /// The name of the operation.
    pub name: String,
    /// The input type of the operation.
    pub input: String,
    /// The output type of the operation.
    pub output: String,
    /// The error types of the operation.
    pub errors: Vec<String>,
    /// The description of the operation.
    pub description: Option<String>,
    /// The metadata of the operation.
    pub metadata: HashMap<String, String>,
}

/// An operation category.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum OperationCategory {
    /// A read operation.
    Read,
    /// A write operation.
    Write,
    /// A query operation.
    Query,
    /// A command operation.
    Command,
    /// A subscription operation.
    Subscription,
}

/// A contract descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDescriptor {
    /// The name of the contract.
    pub name: String,
    /// The version of the contract.
    pub version: ContractVersion,
    /// The fields of the contract.
    pub fields: Vec<FieldDescriptor>,
    /// The description of the contract.
    pub description: Option<String>,
    /// The metadata of the contract.
    pub metadata: HashMap<String, String>,
}

/// A field descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDescriptor {
    /// The name of the field.
    pub name: String,
    /// The type of the field.
    pub field_type: String,
    /// The description of the field.
    pub description: Option<String>,
    /// The metadata of the field.
    pub metadata: HashMap<String, String>,
}