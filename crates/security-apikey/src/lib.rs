#![deny(missing_docs)]
//! API key authentication provider for the ego-rs security stack.
//!
//! Implements [`ego_security_sdk::AuthenticationProvider`] for opaque API keys
//! using a sync resolver (cache-first, no I/O on the calling thread).
//!
//! # Quick start
//!
//! ```rust
//! use std::collections::HashMap;
//! use std::sync::Arc;
//!
//! use ego_domain::auth::SystemClock;
//! use ego_security_sdk::{AuthenticationProvider, Credential, Principal, PrincipalKind, SubjectId};
//! use security_apikey::{
//!     ApiKeyAuthenticationProvider, ApiKeyHash, ApiKeyId, ApiKeyRecord, InMemoryApiKeyResolver,
//! };
//!
//! let secret = b"correct-horse-battery-staple";
//! let key_id = ApiKeyId::new("svc-key-1").unwrap();
//!
//! let mut resolver = InMemoryApiKeyResolver::new();
//! resolver.insert(
//!     key_id,
//!     ApiKeyRecord {
//!         principal: Principal::new(PrincipalKind::Service, SubjectId::new("billing-service").unwrap()),
//!         scopes: vec!["invoices:read".to_string()],
//!         expires_at: None,
//!         metadata: Arc::new(HashMap::new()),
//!         key_hash: ApiKeyHash::of(secret),
//!     },
//! );
//!
//! let provider = ApiKeyAuthenticationProvider::new(Arc::new(resolver), Arc::new(SystemClock));
//!
//! let credential = Credential::Bearer(format!("svc-key-1.{}", std::str::from_utf8(secret).unwrap()));
//! let ctx = provider.authenticate(&credential).expect("valid key authenticates");
//! assert_eq!(ctx.principal.subject_id.as_str(), "billing-service");
//!
//! // A wrong secret is rejected — indistinguishable from an unknown key id
//! // (see ApiKeyAuthenticationProvider's docs on the constant-time invariant).
//! let wrong = Credential::Bearer("svc-key-1.wrong-secret".to_string());
//! assert!(provider.authenticate(&wrong).is_err());
//! ```

pub(crate) mod authenticator;
pub(crate) mod key_hash;
pub(crate) mod key_id;
pub(crate) mod parser;
pub(crate) mod resolver;
pub(crate) mod secret;

pub use authenticator::{ApiKeyAuthenticationProvider, MAX_KEY_BYTES};
pub use key_hash::ApiKeyHash;
pub use key_id::ApiKeyId;
pub use parser::{ApiKeyParser, DefaultApiKeyParser};
pub use resolver::{
    ApiKeyRecord, ApiKeyResolver, ApiKeyResolverError, InMemoryApiKeyResolver, LocalApiKeyResolver,
};
pub use secret::Secret;
