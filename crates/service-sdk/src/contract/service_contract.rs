use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::contract::{ContractVersion, OperationDescriptor};

/// A service contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceContract {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: ContractVersion,
    /// The operations available on the service.
    pub operations: Vec<OperationContract>,
    /// The description of the service.
    pub description: Option<String>,
    /// The metadata of the service.
    pub metadata: HashMap<String, String>,
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
    /// The description of the operation.
    pub description: Option<String>,
    /// The metadata of the operation.
    pub metadata: HashMap<String, String>,
}