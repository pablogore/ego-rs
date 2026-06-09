use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    /// The major version.
    pub major: u32,
    /// The minor version.
    pub minor: u32,
    /// The patch version.
    pub patch: u32,
}

/// An error that can occur with versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VersionError {
    /// A version was not found.
    VersionNotFound,
}