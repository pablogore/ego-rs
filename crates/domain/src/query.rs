use serde::Serialize;

/// Marker trait for query types with an associated Output type.
pub trait Query: Send + Sync {
    type Output: Serialize + Send + Sync;
}
