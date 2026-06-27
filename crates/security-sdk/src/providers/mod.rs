//! Concrete provider implementations — [`basic`], [`rbac`], [`deny_all`], and
//! (behind the `dev-providers` feature) [`allow_all`].

#[cfg(feature = "dev-providers")]
pub mod allow_all;
pub mod basic;
pub mod deny_all;
pub mod rbac;

#[cfg(feature = "dev-providers")]
pub use allow_all::AllowAllAuthorizationProvider;
pub use basic::{BasicAuthenticationProvider, CredentialVerifier};
pub use deny_all::DenyAllAuthorizationProvider;
pub use rbac::RbacProvider;
