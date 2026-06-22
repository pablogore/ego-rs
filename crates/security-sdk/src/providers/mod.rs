//! Concrete provider implementations — [`basic`] and [`rbac`].

pub mod basic;
pub mod rbac;

pub use basic::{BasicAuthenticationProvider, CredentialVerifier};
pub use rbac::RbacProvider;
