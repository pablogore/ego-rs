//! Shared fixtures for this crate's integration tests.
//!
//! Constructors for principals and security contexts, so each test states the
//! identity it needs rather than rebuilding one. `dead_code` is allowed because
//! every integration-test file compiles this module in full while using only
//! the constructors it needs.

#![allow(dead_code)]

use ego_security_sdk::{
    context::SecurityContext,
    principal::{Principal, PrincipalKind, Role, SubjectId},
};

pub fn principal_with_role(role: &str) -> Principal {
    let subject = SubjectId::new(format!("user:{role}")).unwrap();
    Principal::new(PrincipalKind::User, subject).with_role(Role(role.into()))
}

pub fn make_ctx(p: &Principal) -> SecurityContext {
    SecurityContext::empty(p.clone())
}

pub fn make_ctx_from_subject(subject: &str) -> SecurityContext {
    let sub = SubjectId::new(subject).unwrap();
    SecurityContext::empty(Principal::new(PrincipalKind::User, sub))
}
