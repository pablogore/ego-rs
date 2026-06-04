//! # ego-application
//!
//! The application layer of ego-rs. Orchestrates domain logic through
//! hexagonal ports.
//!
//! ## Layer responsibility
//!
//! Application handlers implement [`CommandHandler`] and [`QueryHandler`]
//! traits. They depend **only** on domain traits — never on infrastructure
//! or transport.
//!
//! ## Dependency rule
//!
//! ```text
//! application → domain
//! application → (nothing else internal)
//! ```
//!
//! ## Testing
//!
//! Application handlers are tested with mock ports. No real database,
//! network, or filesystem.

/// Reference hello handler — implements QueryHandler<HelloQuery>.
pub mod hello;

/// Hexagonal port traits — CommandHandler, QueryHandler.
pub mod ports;

pub use hello::HelloHandler;
pub use ports::*;
