use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    /// The name of the service.
    pub name: String,
    /// The version of the service.
    pub version: String,
}

/// An error that can occur with services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceError {
    /// A service was not found.
    ServiceNotFound,
}