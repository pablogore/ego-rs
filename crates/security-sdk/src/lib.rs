#![deny(missing_docs)]
//! `ego-security-sdk` — transport-agnostic, provider-agnostic security primitives
//! for the ego-rs ecosystem.
//!
//! Provides canonical types for identity (`principal::Principal`),
//! credentials (`credential::Credential`), authentication
//! (`authentication::AuthenticationProvider`), authorization
//! (`authorization::AuthorizationProvider`), and security-context propagation
//! (`context::SecurityContext`).
//!
//! Depends on `ego-domain` for canonical data models (`AuthenticationError`, `Claims`).

pub mod authentication;
pub mod authorization;
pub mod context;
pub mod credential;
pub mod error;
pub mod policy;
pub mod principal;
pub mod providers;

pub use authentication::AuthenticationProvider;
pub use authorization::{
    authorize_in_context, AccessRequest, Action, AuthorizationDecision, AuthorizationProvider,
    Resource,
};
pub use context::SecurityContext;
pub use credential::Credential;
pub use ego_domain::auth::{AuthenticationError, Claims, StandardClaims};
pub use error::SecurityError;
pub use policy::{InMemoryRoleStore, Permission, RoleStore};
pub use principal::{Claim, Principal, PrincipalKind, Role, SubjectId};
pub use providers::{
    allow_all::AllowAllAuthorizationProvider,
    basic::{BasicAuthenticationProvider, CredentialVerifier},
    deny_all::DenyAllAuthorizationProvider,
    rbac::RbacProvider,
};
