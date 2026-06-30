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
pub mod credential_extractor;
pub mod error;
pub mod interceptor;
pub mod policy;
pub mod principal;
pub mod principal_mapper;
pub mod providers;

pub use authentication::{AuthenticationInterceptor, AuthenticationProvider};
pub use interceptor::Interceptor;
pub use credential_extractor::{ApiKeyExtractor, BasicExtractor, BearerExtractor, CredentialExtractor, RequestContext};
pub use principal_mapper::PrincipalMapper;
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
#[cfg(feature = "dev-providers")]
pub use providers::allow_all::AllowAllAuthorizationProvider;
pub use providers::{
    basic::{BasicAuthenticationProvider, CredentialVerifier},
    deny_all::DenyAllAuthorizationProvider,
    rbac::RbacProvider,
};
