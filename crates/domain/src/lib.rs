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
//! | `observability` | `Observability` trait, `SemanticEvent`, `Level` (CORE-005) |
//! | `persistence`   | `EventStore`, `Repository`, `Snapshot`, `PersistenceError` traits |

/// Actor trait, identity, lifecycle, and supervision.
pub mod actor;

/// Execution context — identity, correlation, and metadata.
pub mod context;

/// Effect value types describing execution outcomes.
pub mod effect;

/// CQRS command marker trait.
/// Idempotency key for safe external effect retry.
pub mod idempotency;

pub mod command;

/// Domain event trait for event-sourced state.
pub mod event;

/// Example: HelloQuery / HelloResponse.
pub mod hello;

/// Observability port — SemanticEvent, Level, Observability trait.
pub mod observability;

/// Persistence SPI — EventStore, Repository, Snapshot, PersistenceError.
pub mod persistence;

/// CQRS query marker trait with typed Output.
pub mod query;

/// Execution envelope — transport-neutral payload, identity, correlation, and metadata carrier.
pub mod envelope;

pub use actor::{Actor, ActorId, ActorLifecycleState, SupervisionStrategy};
pub use command::Command;
pub use context::{
    AggregateId, AggregateIdError, CausationId, CausationIdError, CorrelationId,
   CorrelationIdError, DomainExecutionContext, EntityId, EntityIdError, ExecutionContext,
    Metadata, RequestId, RequestIdError, TenantId, TenantIdError,
};
pub use envelope::ExecutionEnvelope;
pub use effect::{Effect, ExternalEffectDescription, HandlerResult};
pub use idempotency::{IdempotencyKey, IdempotencyKeyError};
pub use event::DomainEvent;
pub use observability::{Level, Observability, SemanticEvent, SemanticEventError};
pub use query::Query;
