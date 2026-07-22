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
//! | `command`       | `Command` marker trait for mutating operations |
//! | `context`       | Identity types — `AggregateId`, `EntityId`, `TenantId`, `CorrelationId`, `CausationId`, `RequestId`, `Metadata` |
//! | `effect`        | Effect value types describing execution outcomes |
//! | `event`         | `DomainEvent` trait for event-sourced state |
//! | `idempotency`   | `IdempotencyKey` for safe external effect retry |
//! | `query`         | `Query` trait with typed `Output` for reads |
//! | `read_side`     | Projection engine — processors, sessions, runners, storage SPIs |
//! | `actor`         | `Actor` trait, `ActorId`, `actor_id!` macro (CORE-002) |
//! | `observability` | `Observability` trait, `SemanticEvent`, `Level` (CORE-005) |
//! | `persistence`   | `EventStore`, `Repository`, `Snapshot`, `PersistenceError` traits |
//! | `auth`          | `Claims`, `Credential`, `Clock`, `AuthenticationError` |
//! | `config`        | `Validate`, `ConfigError` (CORE-016) |

/// Actor trait, identity, lifecycle, and supervision.
pub mod actor;

/// Identity types — `AggregateId`, `EntityId`, `TenantId`, `CorrelationId`, `CausationId`, `RequestId`, `Metadata`.
pub mod context;

/// Effect value types describing execution outcomes.
pub mod effect;

/// CQRS command marker trait.
pub mod command;

/// Idempotency key for safe external effect retry.
pub mod idempotency;

/// Domain event trait for event-sourced state.
pub mod event;

/// Observability port — SemanticEvent, Level, Observability trait.
pub mod observability;

/// Persistence SPI — EventStore, Repository, Snapshot, PersistenceError.
pub mod persistence;

/// CQRS query marker trait with typed Output.
pub mod query;

/// Tracer port — TraceContext, TraceId/SpanId, SpanAttributes, Tracer,
/// TracerLifecycle, NoopTracer (PROD-003).
pub mod tracer;

/// Read side projection engine — processors, sessions, runners, and storage SPIs.
pub mod read_side;

/// Authentication domain contracts — Claims, Credential, Clock, AuthenticationError.
pub mod auth;

/// Configuration validation contract — Validate, ConfigError (CORE-016).
pub mod config;

pub use actor::{Actor, ActorId, ActorLifecycleState, SupervisionStrategy};
pub use command::Command;
pub use context::{
    AggregateId, AggregateIdError, CausationId, CausationIdError, CorrelationId,
    CorrelationIdError, EntityId, EntityIdError, Metadata, RequestId, RequestIdError, TenantId,
    TenantIdError,
};
pub use effect::{Effect, ExternalEffectDescription, HandlerResult};
pub use event::DomainEvent;
pub use idempotency::{IdempotencyKey, IdempotencyKeyError};
pub use observability::{Level, Observability, SemanticEvent, SemanticEventError};
pub use query::Query;
pub use tracer::{
    parse_traceparent, NoopTracer, SpanAttributes, SpanId, SpanOutcome, TraceContext,
    TraceId, TraceParseError, Tracer, TracerLifecycle,
};
pub use auth::{
    AuthenticationError, ClaimSet, ClaimValue, Claims, Clock, Credential, StandardClaims,
    SystemClock,
};
pub use config::{ConfigError, Validate};
