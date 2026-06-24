//! # security-jwt
//!
//! JWT-based implementation of [`ego_domain::auth::AuthenticationProvider`].
//!
//! ## Overview
//!
//! This crate provides [`JwtAuthenticator`] — a synchronous, thread-safe
//! authenticator that validates JSON Web Tokens (JWTs) and extracts a
//! [`ego_domain::auth::SecurityContext`] from verified tokens.
//!
//! ## Supported algorithms
//!
//! | Algorithm | Key type |
//! |-----------|----------|
//! | HS256 | Shared HMAC secret (`Vec<u8>`) |
//! | RS256 | RSA public key (PEM) |
//!
//! ## Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use security_jwt::{JwtAuthenticator, JwtConfig, JwtAlgorithm};
//! use ego_domain::auth::{AuthenticationProvider, Credential, SystemClock};
//!
//! let config = JwtConfig {
//!     algorithm: JwtAlgorithm::Hs256 { secret: b"my-secret".to_vec() },
//!     expected_iss: Some("my-service".into()),
//!     expected_aud: None,
//! };
//! let auth = JwtAuthenticator::new(config, Arc::new(SystemClock));
//! // let ctx = auth.authenticate(Credential::BearerToken(raw_token))?;
//! ```
//!
//! ## Layer constraint (NFR-001)
//!
//! `security-jwt` is classified as `infrastructure` in `layers.toml`.
//! It MUST NOT be a dependency of `ego-runtime`.
//!
//! ## No `ego-security-sdk` dependency (CLAR-004)
//!
//! This crate MUST NOT import `ego-security-sdk`. The domain contracts
//! in `ego-domain::auth` are the only external ego-rs dependency.

#![deny(missing_docs)]

/// JWT configuration — algorithm selection and validation parameters.
pub mod config;

/// JWT authenticator — the [`ego_domain::auth::AuthenticationProvider`] implementation.
pub mod authenticator;

pub use authenticator::JwtAuthenticator;
pub use config::{JwtAlgorithm, JwtConfig};
