//! Authentication domain contracts.
//!
//! This module owns the core authentication types for the ego-rs domain layer:
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`AuthenticationError`] | Error variants for failed authentication |
//! | [`Credential`] | Credential material presented by a caller |
//! | [`Identity`] | Resolved principal identity (subject, tenant, roles) |
//! | [`Clock`] | Injectable time source — use mocks in tests |
//! | [`StandardClaims`] | IANA registered JWT claims |
//! | [`Claims`] | Standard + custom claims (all maps use `BTreeMap`) |
//! | [`SecurityContext`] | The fully resolved, authenticated context |
//! | [`AuthenticationProvider`] | Synchronous authentication port |
//!
//! ## Dependency rule
//!
//! This module has NO dependency on any infrastructure crate. Concrete
//! implementations (e.g. `security-jwt`) live in separate crates that depend
//! on `ego-domain`, not the other way around.

/// Authentication error types.
pub mod error;

/// Credential material passed to an [`AuthenticationProvider`].
pub mod credential;

/// Authenticated principal identity.
pub mod identity;

/// Injectable clock abstraction for time-sensitive authentication checks.
pub mod clock;

/// JWT claims — standard registered and custom extension claims.
pub mod claims;

/// Resolved security context produced by a successful authentication.
pub mod security_context;

/// The `AuthenticationProvider` trait — domain port for authentication.
pub mod provider;

pub use claims::{Claims, StandardClaims};
pub use clock::{Clock, SystemClock};
pub use credential::Credential;
pub use error::AuthenticationError;
pub use identity::Identity;
pub use provider::AuthenticationProvider;
pub use security_context::SecurityContext;
