//! Authentication domain data models.
//!
//! This module owns the core authentication data types for the ego-rs domain layer.
//! Traits live in `ego_security_sdk` (authN + authZ contracts).
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`AuthenticationError`] | Error variants for failed authentication |
//! | [`Credential`] | Credential material presented by a caller |
//! | [`Clock`] | Injectable time source — use mocks in tests |
//! | [`StandardClaims`] | IANA registered JWT claims |
//! | [`Claims`] | Standard + custom claims (all maps use `BTreeMap`) |
//!
//! ## Dependency rule
//!
//! This module has NO dependency on any infrastructure crate.

pub mod error;
pub mod credential;
pub mod clock;
pub mod claims;

pub use claims::{Claims, StandardClaims};
pub use clock::{Clock, SystemClock};
pub use credential::Credential;
pub use error::AuthenticationError;
