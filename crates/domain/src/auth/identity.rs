//! Identity — the authenticated principal extracted from a credential.
//!
//! An [`Identity`] is a resolved, trusted representation of who the caller
//! is. It is produced by a successful call to
//! [`super::AuthenticationProvider::authenticate`] and embedded inside a
//! [`super::SecurityContext`].

use std::collections::{BTreeMap, BTreeSet};

/// The authenticated principal identity.
///
/// Uses [`BTreeSet`] and [`BTreeMap`] for deterministic ordering — this
/// ensures that serialized representations and equality checks are stable
/// regardless of insertion order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The subject claim (`sub`) — a unique identifier for the principal.
    pub subject: String,

    /// Optional tenant / organization scope extracted from the token.
    pub tenant_id: Option<String>,

    /// The set of roles assigned to this principal.
    pub roles: BTreeSet<String>,

    /// Arbitrary additional attributes attached to this identity.
    pub attributes: BTreeMap<String, String>,
}

impl Identity {
    /// Creates a minimal identity with only a subject.
    ///
    /// `roles` and `attributes` default to empty collections;
    /// `tenant_id` defaults to `None`.
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            tenant_id: None,
            roles: BTreeSet::new(),
            attributes: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_produces_minimal_identity() {
        let id = Identity::new("user-1");
        assert_eq!(id.subject, "user-1");
        assert!(id.tenant_id.is_none());
        assert!(id.roles.is_empty());
        assert!(id.attributes.is_empty());
    }

    #[test]
    fn identity_clone_and_eq() {
        let mut id = Identity::new("alice");
        id.roles.insert("admin".into());
        id.tenant_id = Some("acme".into());
        let id2 = id.clone();
        assert_eq!(id, id2);
    }

    #[test]
    fn roles_are_stored_in_btreeset() {
        let mut id = Identity::new("bob");
        id.roles.insert("writer".into());
        id.roles.insert("reader".into());
        // BTreeSet iterates in lexicographic order
        let mut iter = id.roles.iter();
        assert_eq!(iter.next().unwrap(), "reader");
        assert_eq!(iter.next().unwrap(), "writer");
    }

    #[test]
    fn attributes_are_stored_in_btreemap() {
        let mut id = Identity::new("carol");
        id.attributes.insert("dept".into(), "eng".into());
        id.attributes.insert("region".into(), "eu".into());
        let mut iter = id.attributes.keys();
        assert_eq!(iter.next().unwrap(), "dept");
        assert_eq!(iter.next().unwrap(), "region");
    }
}
