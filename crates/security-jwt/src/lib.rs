//! # security-jwt
//!
//! JWT-based implementation of [`ego_security_sdk::AuthenticationProvider`].
//!
//! ## Overview
//!
//! This crate provides `Hs256AuthenticationProvider`, `Rs256AuthenticationProvider`,
//! and `Es256AuthenticationProvider` — synchronous, thread-safe authenticators that
//! validate JSON Web Tokens (JWTs) and extract a [`ego_security_sdk::SecurityContext`]
//! from verified tokens. Each provider enforces a single algorithm at the type level.
//!
//! ## Supported algorithms
//!
//! | Algorithm | Key type |
//! |-----------|----------|
//! | HS256 | Shared HMAC secret (`Vec<u8>`) via `VerificationKey::Hmac` |
//! | RS256 | RSA public key (PEM) via `VerificationKey::RsaPem` |
//! | ES256 | EC P-256 public key (PEM) via `VerificationKey::EcPem` |
//!
//! ## Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use security_jwt::{
//!     Hs256AuthenticationProvider, JwtProviderConfig, JwtAlgorithm,
//!     LocalKeyResolver, VerificationKey,
//! };
//! use ego_security_sdk::{AuthenticationProvider, Credential};
//! use ego_domain::auth::SystemClock;
//!
//! let resolver = Arc::new(LocalKeyResolver::new(
//!     JwtAlgorithm::Hs256,
//!     VerificationKey::Hmac(b"my-secret".to_vec()),
//! ));
//! let config = JwtProviderConfig {
//!     expected_iss: Some("my-service".into()),
//!     expected_aud: None,
//!     clock_skew_seconds: None,
//! };
//! let auth = Hs256AuthenticationProvider::new(config, resolver, Arc::new(SystemClock));
//! // let ctx = auth.authenticate(&Credential::Bearer(raw_token))?;
//! ```
//!
//! ## Layer constraint (NFR-001)
//!
//! `security-jwt` is classified as `infrastructure` in `layers.toml`.
//! It MUST NOT be a dependency of `ego-runtime`.

#![deny(missing_docs)]

/// JWT configuration — algorithm selection and validation parameters.
pub mod config;

/// OIDC provider configuration.
pub mod oidc_config;

/// JWKS key resolution.
pub mod jwks;

/// OIDC discovery.
pub(crate) mod discovery;

/// DefaultPrincipalMapper + serde_json → ClaimValue conversion.
pub mod principal_mapper;

/// Opaque token introspection.
pub mod introspection;

/// JWT authenticator — the [`ego_security_sdk::AuthenticationProvider`] implementation.
pub mod authenticator;

/// OIDC composite resource server provider.
pub mod oidc_provider;

/// Multi-issuer routing.
pub mod multi_issuer;

/// In-process test fakes (gated behind `feature = "test-kit"`).
#[cfg(feature = "test-kit")]
pub mod test_kit;

mod key_resolver;
mod validation;
#[cfg(test)]
mod test_helpers;

pub use authenticator::{Es256AuthenticationProvider, Hs256AuthenticationProvider, Rs256AuthenticationProvider};
pub use oidc_provider::OidcAuthenticationProvider;
pub use multi_issuer::{IssuerResolver, MultiIssuerAuthenticationProvider, StaticIssuerResolver};
pub use config::{Es256Config, Hs256Config, JwtAlgorithm, JwtProviderConfig, Rs256Config};
pub use key_resolver::{KeyResolver, KeyResolverError, LocalKeyResolver, VerificationKey};
pub use oidc_config::{MultiIssuerConfig, OidcProviderConfig, TokenFormat};
pub use jwks::{HttpJwksProvider, JwksKeyResolver, JwksProvider};
pub use discovery::OidcEndpoints;
pub use principal_mapper::DefaultPrincipalMapper;
pub use introspection::{
    ClientCredentials, HttpIntrospectionProvider, IntrospectionAuthenticationProvider,
    IntrospectionProvider, IntrospectionResult,
};
