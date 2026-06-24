//! # security-jwt
//!
//! JWT-based implementation of [`ego_security_sdk::AuthenticationProvider`].
//!
//! ## Overview
//!
//! This crate provides [`JwtAuthenticator`] — a synchronous, thread-safe
//! authenticator that validates JSON Web Tokens (JWTs) and extracts a
//! [`ego_security_sdk::SecurityContext`] from verified tokens.
//!
//! ## Supported algorithms
//!
//! | Algorithm | Key type |
//! |-----------|----------|
//! | HS256 | Shared HMAC secret (`Vec<u8>`) via `VerificationKey::Hmac` |
//! | RS256 | RSA public key (PEM) via `VerificationKey::RsaPem` |
//!
//! ## Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use security_jwt::{JwtAuthenticator, JwtConfig, JwtAlgorithm, LocalKeyResolver, VerificationKey};
//! use ego_security_sdk::{AuthenticationProvider, Credential};
//! use ego_domain::auth::SystemClock;
//!
//! let resolver = Arc::new(LocalKeyResolver::new(
//!     JwtAlgorithm::Hs256,
//!     VerificationKey::Hmac(b"my-secret".to_vec()),
//! ));
//! let config = JwtConfig {
//!     algorithm: JwtAlgorithm::Hs256,
//!     expected_iss: Some("my-service".into()),
//!     expected_aud: None,
//! };
//! let auth = JwtAuthenticator::new(config, resolver, Arc::new(SystemClock));
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

/// JWT authenticator — the [`ego_security_sdk::AuthenticationProvider`] implementation.
pub mod authenticator;

mod key_resolver;

pub use authenticator::JwtAuthenticator;
pub use config::{JwtAlgorithm, JwtConfig};
pub use key_resolver::{KeyResolver, KeyResolverError, LocalKeyResolver, VerificationKey};
