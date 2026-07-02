#![deny(missing_docs)]
//! API key authentication provider for the ego-rs security stack.
//!
//! Implements [`ego_security_sdk::AuthenticationProvider`] for opaque API keys
//! using a sync resolver (cache-first, no I/O on the calling thread).
//!
//! # Quick start
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use security_apikey::{ApiKeyAuthenticationProvider, InMemoryApiKeyResolver, ApiKeyRecord, ApiKeyHash};
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
