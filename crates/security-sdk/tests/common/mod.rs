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
    SecurityContext::new(p.clone())
}

pub fn make_ctx_from_subject(subject: &str) -> SecurityContext {
    let sub = SubjectId::new(subject).unwrap();
    SecurityContext::new(Principal::new(PrincipalKind::User, sub))
}
