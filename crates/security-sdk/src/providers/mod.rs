//! Concrete provider implementations — [`basic`], [`rbac`], [`allow_all`], and [`deny_all`].

pub mod allow_all;
pub mod basic;
pub mod deny_all;
pub mod rbac;

pub use allow_all::AllowAllAuthorizationProvider;
pub use basic::{BasicAuthenticationProvider, CredentialVerifier};
pub use deny_all::DenyAllAuthorizationProvider;
pub use rbac::RbacProvider;
