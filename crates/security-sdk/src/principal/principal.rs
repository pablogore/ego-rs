//! Principal identity type and related types.

use std::collections::{HashMap, HashSet};

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    pub subject: SubjectId,
    /// Roles assigned to this principal.
    pub roles: HashSet<Role>,
    /// Claims asserted about this principal.
    pub claims: Vec<Claim>,
    /// Free-form attributes.
    pub attributes: HashMap<String, String>,
}

impl Principal {
    /// Creates a principal with the given kind and subject; empty roles/claims/attributes.
    pub fn new(kind: PrincipalKind, subject: SubjectId) -> Self {
        Self {
            kind,
            subject,
            roles: HashSet::new(),
            claims: Vec::new(),
            attributes: HashMap::new(),
        }
    }

    /// Builder: adds a role (duplicates are silently deduplicated via [`HashSet`] semantics).
    pub fn with_role(mut self, role: Role) -> Self {
        self.roles.insert(role);
        self
    }

    /// Builder: appends a claim.
    pub fn with_claim(mut self, claim: Claim) -> Self {
        self.claims.push(claim);
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
    use super::{Claim, Principal, PrincipalKind, Role};
    use crate::principal::SubjectId;

    fn make_subject(s: &str) -> SubjectId {
        SubjectId::new(s).unwrap()
    }

    #[test]
    fn constructs_with_required_fields() {
        let subject = make_subject("user:abc");
        let p = Principal::new(PrincipalKind::User, subject.clone());
        assert_eq!(p.kind, PrincipalKind::User);
        assert_eq!(p.subject.as_str(), "user:abc");
        assert!(p.roles.is_empty(), "roles should start empty");
        assert!(p.claims.is_empty(), "claims should start empty");
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
    fn with_role_adds_to_set() {
        // Adding the same role twice should keep only one entry (HashSet semantics).
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_role(Role("admin".into()))
            .with_role(Role("admin".into()));
        assert_eq!(p.roles.len(), 1, "duplicate role should be deduplicated");
    }

    #[test]
    fn with_claim_appends() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_claim(Claim {
                name: "email".into(),
                value: "a@b.com".into(),
            })
            .with_claim(Claim {
                name: "iss".into(),
                value: "https://auth.example.com".into(),
            });
        assert_eq!(p.claims.len(), 2);
    }

    #[test]
    fn with_attribute_sets_key() {
        let p = Principal::new(PrincipalKind::User, make_subject("u:1"))
            .with_attribute("region", "us-east-1");
        assert_eq!(p.attributes.get("region").map(String::as_str), Some("us-east-1"));
    }

    #[test]
    fn has_role_returns_true_when_present() {
        let role = Role("admin".into());
        let p = Principal::new(PrincipalKind::Service, make_subject("svc:1"))
            .with_role(role.clone());
        assert!(p.has_role(&role));
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
        // TS-001: full construction scenario
        let p = Principal::new(PrincipalKind::User, make_subject("user:42"))
            .with_role(Role("admin".into()))
            .with_claim(Claim {
                name: "email".into(),
                value: "alice@example.com".into(),
            })
            .with_attribute("region", "eu-west-1");

        assert_eq!(p.kind, PrincipalKind::User);
        assert_eq!(p.subject.as_str(), "user:42");
        assert!(p.has_role(&Role("admin".into())));
        assert!(p.claims.iter().any(|c| c.name == "email" && c.value == "alice@example.com"));
        assert_eq!(p.attributes.get("region").map(String::as_str), Some("eu-west-1"));
    }
}
