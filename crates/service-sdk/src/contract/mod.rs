use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A service contract trait.
///
/// Implemented by all service contracts declared via the `#[service]` macro.
pub trait ServiceContract {
    /// Returns the type ID of the service contract.
    fn type_id() -> &'static str;

    /// Returns the name of the service contract.
    fn name() -> &'static str;

    /// Returns the version of the service contract.
    fn version() -> ContractVersion;

    /// Returns the descriptor of the service contract.
    fn descriptor() -> ServiceDescriptor;

    /// Returns the operations available on the service.
    fn operations() -> Vec<OperationDescriptor>;
}

/// An operation on a service contract.
///
/// Describes a single operation available on a service, including its input,
/// output, and possible error types.
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
///
/// A runtime representation of a service contract that includes the service's
/// operations and metadata. This is used for service registration and resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: ContractVersion,
    /// The operations available on the service.
    pub operations: Vec<OperationDescriptor>,
    /// An optional description of the service.
    pub description: Option<String>,
    /// Arbitrary metadata associated with the service.
    pub metadata: std::collections::HashMap<String, String>,
}

/// An operation descriptor.
///
/// A runtime representation of an operation in a service contract.
/// This includes the operation's signature and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDescriptor {
    /// The name of the operation.
    pub name: String,
    /// The input types of the operation.
    pub input: Vec<String>,
    /// The output type of the operation.
    pub output: String,
    /// The error types of the operation.
    pub errors: Vec<String>,
    /// An optional description of the operation.
    pub description: Option<String>,
    /// Arbitrary metadata associated with the operation.
    pub metadata: std::collections::HashMap<String, String>,
}

/// Version of a service contract.
///
/// Represents a semantic version (major.minor.patch) for service contracts.
/// Used to manage service compatibility and evolution.
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
    ///
    /// # Arguments
    /// * `major` - The major version number
    /// * `minor` - The minor version number
    /// * `patch` - The patch version number
    ///
    /// # Returns
    /// A new `ContractVersion` instance
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for ContractVersion {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid version format. Expected major.minor.patch".into());
        }

        let major = parts[0].parse::<u32>()?;
        let minor = parts[1].parse::<u32>()?;
        let patch = parts[2].parse::<u32>()?;

        Ok(ContractVersion {
            major,
            minor,
            patch,
        })
    }
}

impl PartialOrd for ContractVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.major.cmp(&other.major) {
            std::cmp::Ordering::Equal => match self.minor.cmp(&other.minor) {
                std::cmp::Ordering::Equal => self.patch.partial_cmp(&other.patch),
                other => Some(other),
            },
            other => Some(other),
        }
    }
}
