//! Service contract module.
//!
//! This module provides the canonical descriptor types and contract version utilities.

pub mod descriptor;
pub mod version;

pub use descriptor::{ContractDescriptor, FieldDescriptor, OperationDescriptor, ServiceDescriptor};
pub use version::{ContractVersion, VersionConstraint};

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
