use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A service contract trait.
///
/// This trait should be implemented by all service contracts.
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