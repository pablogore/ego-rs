//! Policy model — `Permission`, `RoleStore` trait, and `InMemoryRoleStore`.

pub mod in_memory_role_store;

pub use in_memory_role_store::InMemoryRoleStore;

use async_trait::async_trait;

use crate::{error::SecurityError, principal::Role};

/// A grant of `action` on a resource kind, mapped to roles by the [`RoleStore`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Permission {
    /// Resource kind this permission applies to (e.g. `"orders"`).
    pub resource: String,
    /// Action granted (e.g. `"read"`). `"*"` is the wildcard that grants any
    /// action on this resource. Wildcards are valid here (grant side) but are
    /// rejected on the request side — see [`crate::authorization::AccessRequest`].
    pub action: String,
}

/// Maps roles to their granted permissions.
///
/// Object-safe; stored as `Arc<dyn RoleStore>`. An unknown role returns
/// `Ok(Vec::new())`, never an error.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RoleStore: Send + Sync {
    /// Returns the permissions granted by `role`.
    ///
    /// An unknown role returns `Ok(Vec::new())`, not an error.
    async fn permissions_for_role(&self, role: &Role) -> Result<Vec<Permission>, SecurityError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::principal::Role;

    #[test]
    fn permission_equality() {
        let a = Permission {
            resource: "orders".into(),
            action: "read".into(),
        };
        let b = Permission {
            resource: "orders".into(),
            action: "read".into(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn permission_hashes_consistently() {
        use std::collections::HashSet;
        let p = Permission {
            resource: "orders".into(),
            action: "read".into(),
        };
        let mut set = HashSet::new();
        set.insert(p.clone());
        set.insert(p.clone());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn role_store_is_object_safe() {
        struct StubStore;

        #[async_trait::async_trait]
        impl RoleStore for StubStore {
            async fn permissions_for_role(
                &self,
                _: &Role,
            ) -> Result<Vec<Permission>, SecurityError> {
                Ok(vec![])
            }
        }

        let _: Arc<dyn RoleStore> = Arc::new(StubStore);
    }

    #[test]
    fn role_store_dyn_is_send_and_sync() {
        fn assert_send_sync<T: ?Sized + Send + Sync>() {}
        assert_send_sync::<dyn RoleStore>();
    }

    #[tokio::test]
    async fn unknown_role_returns_empty_vec() {
        let store = InMemoryRoleStore::new();
        let result = store
            .permissions_for_role(&Role("unknown".into()))
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn known_role_returns_permissions() {
        let store = InMemoryRoleStore::new().with_role(
            Role("admin".into()),
            vec![Permission {
                resource: "orders".into(),
                action: "read".into(),
            }],
        );
        let perms = store
            .permissions_for_role(&Role("admin".into()))
            .await
            .unwrap();
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0].resource, "orders");
        assert_eq!(perms[0].action, "read");
    }
}
