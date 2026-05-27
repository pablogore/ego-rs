//! # ego-domain
//!
//! The domain layer of ego-rs — a hexagonal, actor-oriented, deterministic
//! backend framework.
//!
//! ## Layer responsibility
//!
//! The domain layer owns the core contracts: traits for commands, queries,
//! events, actors, and persistence. It has **zero** dependencies on
//! infrastructure, transport, or runtime crates.
//!
//! ## Architecture
//!
//! ```text
//! transport → application → domain
//! infrastructure → application → domain
//! domain → (nothing internal)
//! ```
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `command` | `Command` marker trait for mutating operations |
//! | `event` | `DomainEvent` trait for event-sourced state |
//! | `query` | `Query` trait with typed `Output` for reads |
//! | `actor` | `Actor` trait, `ActorId`, `actor_id!` macro (CORE-002) |
//! | `hello` | Example: `HelloQuery` / `HelloResponse` |

pub mod actor;
pub mod command;
pub mod event;
pub mod hello;
pub mod query;

pub use actor::{Actor, ActorId, ActorLifecycleState, SupervisionStrategy};
pub use command::Command;
pub use event::DomainEvent;
pub use query::Query;