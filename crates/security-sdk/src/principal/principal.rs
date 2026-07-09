//! Principal identity type and related types.

use std::collections::{BTreeMap, BTreeSet};

use ego_domain::context::TenantId;

use crate::principal::SubjectId;

/// What kind of actor a [`Principal`] represents.
///
/// Marked `#[non_exhaustive]` so future actor categories can be added
/// without a breaking change to existing match arms.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalKind {
    /// A human end user.
    User,
    /// A service / workload identity.
    Service,
    /// An OS-level or runtime process.
    Process,
    /// An autonomous agent.
    Agent,
}

/// A named role assigned to a principal.
///
/// # Ordering
///
/// `Role` implements `Ord` solely so it can be used as a [`BTreeSet`] key
/// in [`Principal::roles`]. The order is **lexicographic** (alphabetical on
/// the inner string) and carries **no privilege semantics**:
/// `"admin" < "viewer"` is true — the opposite of a typical privilege ladder.
///
/// Do **not** use `<`/`>`/`cmp` on roles for access-control decisions.
/// For permission checks, call [`Principal::has_role`] or query a [`RoleStore`].
///
/// [`RoleStore`]: crate::policy::RoleStore
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Role(pub String);

/// A typed assertion about the principal (name + value), provider-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// Claim name (e.g. `"email"`, `"iss"`).
    pub name: String,
    /// Claim value as a string (JSON-encoded for structured claims).
    pub value: String,
}

/// An arbitrary key/value attribute attached to a principal.
pub type Attribute = (String, String);

/// The authenticated actor flowing through the system.
///
/// A `Principal` never stores credentials — credentials are inputs to
/// authentication only.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Kind of actor.
    pub kind: PrincipalKind,
    /// Canonical, validated subject id.
    pub subject_id: SubjectId,
    /// Tenant/workspace this principal belongs to, if any.
    pub tenant_id: Option<TenantId>,
    /// Roles assigned to this principal (sorted for deterministic iteration).
    pub roles: BTreeSet<Role>,
    /// Free-form attributes.
    pub attributes: BTreeMap<String, String>,
}

impl Principal {
    /// Creates a principal with the given kind and subject; empty roles/attributes.
    pub fn new(kind: PrincipalKind, subject_id: SubjectId) -> Self {
        Self {
            kind,
            subject_id,
            tenant_id: None,
            roles: BTreeSet::new(),
            attributes: BTreeMap::new(),
        }
    }

    /// Builder: sets the tenant id. Takes a pre-validated `TenantId`;
    /// validation is the caller's responsibility (the type is the proof).
    ///
    /// Not to be confused with `ServiceContext::with_tenant_id` (service-sdk),
    /// which takes any raw string as a non-authoritative caller-supplied hint —
    /// a different type, a different trust boundary.
    pub fn with_tenant_id(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Builder: adds a role (duplicates are silently deduplicated via [`BTreeSet`] semantics — roles are sorted lexicographically, not by privilege level).
    pub fn with_role(mut self, role: Role) -> Self {
        self.roles.insert(role);
        self
    }

    /// Builder: adds or overwrites an attribute.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Returns `true` if this principal holds `role`.
    pub fn has_role(&self, role: &Role) -> bool {
        self.roles.contains(role)
    }
}

#[cfg(test)]
mod tests {
    use super::{Principal, PrincipalKind, Role};
    use crate::principal::SubjectId;
    use ego_domain::context::TenantId;

    fn make_subject(s: &str) -> SubjectId {
        SubjectId::new(s).unwrap()
    }

    #[test]
    fn constructs_with_required_fields() {
        let subject = make_subject("user:abc");
        let p = Principal::new(PrincipalKind::User, subject.clone());
        assert_eq!(p.kind, PrincipalKind::User);
        assert_eq!(p.subject_id.as_str(), "user:abc");
        assert!(p.tenant_id.is_none());
        assert!(p.roles.is_empty(), "roles should start empty");
        assert!(p.attributes.is_empty(), "attributes should start empty");
    }

    #[test]
    fn all_principal_kinds_roundtrip() {
        for kind in [
            PrincipalKind::User,
            PrincipalKind::Service,
            PrincipalKind::Process,
            PrincipalKind::Agent,
        ] {
            let p = Principal::new(kind, make_subject("s:1"));
            assert_eq!(p.kind, kind, "kind roundtrip failed for {:?}", kind);
        }
    }

    #[test]
    fn with_tenant_id_sets_field() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_tenant_id(TenantId::new("acme").unwrap());
        assert_eq!(p.tenant_id, Some(TenantId::new("acme").unwrap()));
    }

    #[test]
    fn with_tenant_id_overwrites() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_tenant_id(TenantId::new("acme").unwrap())
            .with_tenant_id(TenantId::new("contoso").unwrap());
        assert_eq!(p.tenant_id, Some(TenantId::new("contoso").unwrap()));
    }

    #[test]
    fn with_role_adds_to_set() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_role(Role("admin".into()))
            .with_role(Role("admin".into()));
        assert_eq!(p.roles.len(), 1, "duplicate role should be deduplicated");
    }

    #[test]
    fn with_attribute_sets_key() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_attribute("region", "us-east-1");
        assert_eq!(
            p.attributes.get("region").map(String::as_str),
            Some("us-east-1")
        );
    }

    #[test]
    fn has_role_returns_true_when_present() {
        let role = Role("admin".into());
        let p =
            Principal::new(PrincipalKind::Service, make_subject("svc:1")).with_role(role.clone());
        assert!(p.has_role(&role));
    }

    #[test]
    fn role_ord_is_lexicographic_not_privilege() {
        // "admin" < "viewer" alphabetically — the opposite of a privilege ladder.
        // This test exists to make that counter-intuitive fact executable and
        // visible: never use Ord on Role for access-control comparisons.
        assert!(Role("admin".into()) < Role("viewer".into()));
        assert!(Role("superadmin".into()) > Role("admin".into()));
    }

    #[test]
    fn has_role_returns_false_when_absent() {
        let p = Principal::new(PrincipalKind::Service, make_subject("svc:1"))
            .with_role(Role("viewer".into()));
        assert!(!p.has_role(&Role("superuser".into())));
    }

    #[test]
    fn with_attribute_overwrites_existing_key() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_attribute("region", "us-east-1")
            .with_attribute("region", "eu-west-1");
        assert_eq!(
            p.attributes.get("region").map(String::as_str),
            Some("eu-west-1"),
            "second call should overwrite the first"
        );
    }

    #[test]
    fn subject_id_and_attributes() {
        let p = Principal::new(PrincipalKind::User, make_subject("user:42"))
            .with_role(Role("admin".into()))
            .with_tenant_id(TenantId::new("acme").unwrap())
            .with_attribute("region", "eu-west-1");

        assert_eq!(p.kind, PrincipalKind::User);
        assert_eq!(p.subject_id.as_str(), "user:42");
        assert!(p.has_role(&Role("admin".into())));
        assert_eq!(p.tenant_id, Some(TenantId::new("acme").unwrap()));
        assert_eq!(
            p.attributes.get("region").map(String::as_str),
            Some("eu-west-1")
        );
    }
}
