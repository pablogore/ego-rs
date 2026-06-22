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
//! Depends on no ego crate — only on third-party libraries — so any layer may
//! import it without risking a dependency cycle.

pub mod authentication;
pub mod authorization;
pub mod context;
pub mod credential;
pub mod error;
pub mod policy;
pub mod principal;
pub mod providers;

pub use error::SecurityError;
pub use principal::{Claim, Principal, PrincipalKind, Role, SubjectId};
pub use credential::Credential;
pub use authentication::AuthenticationProvider;
pub use authorization::{
    authorize_in_context, AccessRequest, Action, AuthorizationDecision, AuthorizationProvider,
    Resource,
};
pub use policy::{InMemoryRoleStore, Permission, RoleStore};
pub use context::SecurityContext;
pub use providers::{
    basic::{BasicAuthenticationProvider, CredentialVerifier},
    rbac::RbacProvider,
};
