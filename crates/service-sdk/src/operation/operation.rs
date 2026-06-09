use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// An operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// The name of the operation.
    pub name: String,
    /// The input type of the operation.
    pub input: String,
    /// The output type of the operation.
    pub output: String,
    /// The error types of the operation.
    pub errors: Vec<String>,
}

/// An error that can occur with operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationError {
    /// An operation was not found.
    OperationNotFound,
}