//! Identity builders — [`PrincipalBuilder`] (CORE-022 Phase 2, design.md AD-7).

use std::collections::BTreeMap;

use ego_domain::context::TenantId;
use ego_security_sdk::principal::{Principal, PrincipalKind, Role, SubjectId};

/// Default subject id used when a test does not override it. Always
/// non-empty, so a no-override [`PrincipalBuilder::build`] never fails
/// `SubjectId`'s validation.
const DEFAULT_SUBJECT: &str = "test:subject";

/// Builds a real [`Principal`] with valid defaults; override only what a
/// test needs. `build()` always produces a `Principal` that satisfies every
/// invariant the production type enforces — there is no `TestPrincipal`.
pub struct PrincipalBuilder {
    kind: PrincipalKind,
    subject: String,
    tenant: Option<String>,
    roles: Vec<Role>,
    attributes: BTreeMap<String, String>,
}

impl PrincipalBuilder {
    /// Starts a builder defaulting to `PrincipalKind::User` and subject
    /// `"test:subject"`.
    pub fn new() -> Self {
        Self {
            kind: PrincipalKind::User,
            subject: DEFAULT_SUBJECT.to_string(),
            tenant: None,
            roles: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    /// Overrides the principal kind.
    pub fn kind(mut self, kind: PrincipalKind) -> Self {
        self.kind = kind;
        self
    }

    /// Overrides the subject id.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = subject.into();
        self
    }

    /// Sets the tenant id.
    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Adds a role.
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(Role(role.into()));
        self
    }

    /// Adds or overwrites an attribute.
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Builds the real [`Principal`].
    ///
    /// # Panics
    /// Only if the subject was explicitly overridden to an empty or
    /// whitespace-only string — the default subject is always valid.
    pub fn build(self) -> Principal {
        let subject_id = SubjectId::new(self.subject)
            .expect("PrincipalBuilder subject must not be empty or whitespace-only");
        let mut principal = Principal::new(self.kind, subject_id);
        if let Some(tenant) = self.tenant {
            let tenant_id = TenantId::new(tenant)
                .expect("PrincipalBuilder tenant must not be empty or whitespace-only");
            principal = principal.with_tenant_id(tenant_id);
        }
        for role in self.roles {
            principal = principal.with_role(role);
        }
        for (key, value) in self.attributes {
            principal = principal.with_attribute(key, value);
        }
        principal
    }
}

impl Default for PrincipalBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: `PrincipalBuilder::new().build()`.
pub fn principal() -> Principal {
    PrincipalBuilder::new().build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ego_security_sdk::principal::PrincipalKind;

    #[test]
    fn default_build_satisfies_production_invariants() {
        // A no-override build must produce a valid, non-empty subject id —
        // PrincipalBuilder never fabricates an invariant-violating Principal.
        let p = PrincipalBuilder::new().build();
        assert_eq!(p.kind, PrincipalKind::User);
        assert_eq!(p.subject_id.as_str(), "test:subject");
        assert!(p.tenant_id.is_none());
        assert!(p.roles.is_empty());
        assert!(p.attributes.is_empty());
    }

    #[test]
    fn overriding_one_field_leaves_others_default() {
        let p = PrincipalBuilder::new().role("admin").build();
        assert!(p.has_role(&ego_security_sdk::principal::Role("admin".into())));
        // Every other field stays at its default.
        assert_eq!(p.subject_id.as_str(), "test:subject");
        assert_eq!(p.kind, PrincipalKind::User);
        assert!(p.tenant_id.is_none());
    }

    #[test]
    fn principal_convenience_matches_default_builder() {
        let a = principal();
        let b = PrincipalBuilder::new().build();
        assert_eq!(a.subject_id.as_str(), b.subject_id.as_str());
        assert_eq!(a.kind, b.kind);
    }

    #[test]
    fn subject_override_is_applied() {
        let p = PrincipalBuilder::new().subject("user:42").build();
        assert_eq!(p.subject_id.as_str(), "user:42");
    }

    #[test]
    fn kind_tenant_and_attribute_overrides_are_applied() {
        let p = PrincipalBuilder::new()
            .kind(PrincipalKind::Service)
            .tenant("acme")
            .attribute("region", "eu-west-1")
            .build();
        assert_eq!(p.kind, PrincipalKind::Service);
        assert_eq!(p.tenant_id.as_ref().map(TenantId::as_str), Some("acme"));
        assert_eq!(
            p.attributes.get("region").map(String::as_str),
            Some("eu-west-1")
        );
    }

    #[test]
    #[should_panic(expected = "PrincipalBuilder subject must not be empty or whitespace-only")]
    fn empty_subject_override_panics() {
        PrincipalBuilder::new().subject("").build();
    }

    #[test]
    #[should_panic(expected = "PrincipalBuilder subject must not be empty or whitespace-only")]
    fn whitespace_only_subject_override_panics() {
        PrincipalBuilder::new().subject("   ").build();
    }

    #[test]
    #[should_panic(expected = "PrincipalBuilder tenant must not be empty or whitespace-only")]
    fn empty_tenant_override_panics() {
        PrincipalBuilder::new().tenant("").build();
    }

    #[test]
    #[should_panic(expected = "PrincipalBuilder tenant must not be empty or whitespace-only")]
    fn whitespace_only_tenant_override_panics() {
        PrincipalBuilder::new().tenant("   ").build();
    }
}
