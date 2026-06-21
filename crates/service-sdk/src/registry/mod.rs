//! Type-keyed service registry.

#[allow(clippy::module_inception)]
mod registry;

pub use registry::{RegistryError, ServiceRegistry};
