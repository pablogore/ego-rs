use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A service contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceContract {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: ContractVersion,
}

/// An operation on a service contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationContract {
    /// The name of the operation.
    pub name: String,
    /// The input type of the operation.
    pub input: String,
    /// The output type of the operation.
    pub output: String,
    /// The error types of the operation.
    pub errors: Vec<String>,
}

/// A service descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: ContractVersion,
    /// The operations available on the service.
    pub operations: Vec<OperationDescriptor>,
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
}

/// Version of a service contract.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContractVersion {
    /// Major version number.
    pub major: u32,
    /// Minor version number.
    pub minor: u32,
    /// Patch version number.
    pub patch: u32,
}

impl ContractVersion {
    /// Creates a new contract version.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }
}

impl std::fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}