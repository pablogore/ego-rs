//! CQRS query marker trait.
//!
//! Defines `Query` — the contract for read-only operations. Each query
//! declares its `Output` type for type-safe handler resolution.

use serde::Serialize;

/// Trait for query types in a CQRS system.
///
/// Queries request data without mutating state. Each query declares
/// its output type via the associated `Output`. Handlers in the
/// application layer process queries and return the typed result.
///
/// # Example
///
/// ```rust
/// use ego_domain::Query;
/// use serde::Serialize;
///
/// #[derive(Debug, Serialize)]
/// struct UserProfile { name: String, email: String }
///
/// struct GetUser { user_id: String }
///
/// impl Query for GetUser {
///     type Output = UserProfile;
/// }
/// ```
pub trait Query: Send + Sync {
    /// The type returned when this query is processed.
    type Output: Serialize + Send + Sync;
}