use serde::{Deserialize, Serialize};

use crate::query::Query;

/// Query for the hello endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelloQuery;

/// Response for the hello endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelloResponse {
    pub message: String,
}

impl Query for HelloQuery {
    type Output = HelloResponse;
}
