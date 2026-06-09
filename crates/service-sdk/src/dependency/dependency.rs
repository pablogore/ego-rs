use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// The name of the dependency.
    pub name: String,
    /// The version of the dependency.
    pub version: String,
}

/// An error that can occur with dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DependencyError {
    /// A dependency was not found.
    DependencyNotFound,
    /// A dependency cycle was detected.
    DependencyCycle,
}