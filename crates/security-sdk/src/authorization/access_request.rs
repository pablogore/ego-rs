//! Access request types — `Resource`, `Action`, and `AccessRequest`.
//!
//! # Wildcard semantics
//!
//! Wildcards (`"*"`) are **not valid in access requests**. A request always
//! describes a specific, concrete intent (e.g., `"orders:read"`). Wildcards
//! belong exclusively on the grant/permission side — see
//! [`crate::policy::Permission::action`].
//!
//! `AccessRequest::from_permission` rejects any descriptor that contains `"*"`
//! in either the resource or action segment.

use crate::error::SecurityError;

/// A resource kind with an optional instance identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    /// Resource kind (e.g. `"orders"`, `"invoices"`).
    pub kind: String,
    /// Optional instance identifier (e.g. `"order-42"`).
    pub id: Option<String>,
}

/// A specific action name (e.g. `"read"`, `"write"`, `"delete"`).
///
/// Always represents a **concrete** action. Wildcards are not permitted here;
/// they are valid only in [`crate::policy::Permission::action`] on the grant
/// side of an authorization policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action(pub String);

/// Describes what a principal wants to do: perform `action` on `resource`.
#[derive(Debug, Clone)]
pub struct AccessRequest {
    /// The target resource.
    pub resource: Resource,
    /// The requested action.
    pub action: Action,
}

impl AccessRequest {
    /// Creates an `AccessRequest` from `resource` and `action`.
    pub fn new(resource: Resource, action: Action) -> Self {
        Self { resource, action }
    }

    /// Parses a `"resource:action"` descriptor string.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidAccessRequest`] if:
    /// - The descriptor contains no `':'`.
    /// - Either the resource or action segment is empty.
    /// - Either segment is `"*"` — wildcards are rejected in requests; they
    ///   are valid only on the grant/permission side.
    pub fn from_permission(descriptor: &str) -> Result<Self, SecurityError> {
        let (resource, action) = descriptor.split_once(':').ok_or_else(|| {
            SecurityError::InvalidAccessRequest(format!(
                "expected 'resource:action', got '{descriptor}'"
            ))
        })?;
        if resource.is_empty() {
            return Err(SecurityError::InvalidAccessRequest(
                "resource segment must not be empty".into(),
            ));
        }
        if action.is_empty() {
            return Err(SecurityError::InvalidAccessRequest(
                "action segment must not be empty".into(),
            ));
        }
        if resource == "*" {
            return Err(SecurityError::InvalidAccessRequest(
                "wildcard resource '*' is not valid in an access request; \
                 wildcards belong on the permission/grant side"
                    .into(),
            ));
        }
        if action == "*" {
            return Err(SecurityError::InvalidAccessRequest(
                "wildcard action '*' is not valid in an access request; \
                 wildcards belong on the permission/grant side"
                    .into(),
            ));
        }
        Ok(Self::new(
            Resource {
                kind: resource.into(),
                id: None,
            },
            Action(action.into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_from_resource_and_action() {
        let req = AccessRequest::new(
            Resource {
                kind: "orders".into(),
                id: None,
            },
            Action("read".into()),
        );
        assert_eq!(req.resource.kind, "orders");
        assert_eq!(req.action.0, "read");
    }

    #[test]
    fn resource_with_instance_id() {
        let req = AccessRequest::new(
            Resource {
                kind: "orders".into(),
                id: Some("order-42".into()),
            },
            Action("read".into()),
        );
        assert_eq!(req.resource.id.as_deref(), Some("order-42"));
    }

    #[test]
    fn from_permission_parses_valid_descriptor() {
        let req = AccessRequest::from_permission("orders:read").unwrap();
        assert_eq!(req.resource.kind, "orders");
        assert_eq!(req.action.0, "read");
    }

    #[test]
    fn from_permission_rejects_missing_colon() {
        assert!(matches!(
            AccessRequest::from_permission("bad"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }

    #[test]
    fn from_permission_rejects_empty_resource() {
        assert!(matches!(
            AccessRequest::from_permission(":action"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }

    #[test]
    fn from_permission_rejects_empty_action() {
        assert!(matches!(
            AccessRequest::from_permission("resource:"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }

    #[test]
    fn from_permission_rejects_wildcard_action() {
        assert!(matches!(
            AccessRequest::from_permission("orders:*"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }

    #[test]
    fn from_permission_rejects_wildcard_resource() {
        assert!(matches!(
            AccessRequest::from_permission("*:read"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }

    #[test]
    fn from_permission_rejects_double_wildcard() {
        assert!(matches!(
            AccessRequest::from_permission("*:*"),
            Err(SecurityError::InvalidAccessRequest(_))
        ));
    }
}
