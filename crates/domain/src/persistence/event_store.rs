use async_trait::async_trait;

use crate::event::DomainEvent;
use crate::persistence::{PersistenceError, StoredEvent};

/// Trait for appending and loading domain events.
///
/// Implementations provide event-sourced persistence backed by any storage system.
///
/// The stream identity is `(aggregate_type, aggregate_id)`, not a single
/// joined string. Keeping the type a distinct component rather than
/// concatenating it into the identifier means a caller that needs one
/// component back is never left trying to parse it out of a joined value —
/// which, for at least one known caller, is not even possible in general
/// (a type name that itself contains the join separator can make two
/// different type/id pairs produce the identical joined string).
///
/// # Why the storage methods are asynchronous
///
/// Every implementation that talks to a real database is asynchronous
/// underneath. Presenting a synchronous surface over it does not remove the
/// wait; it only hides where the wait happens. The PostgreSQL implementation
/// used to bridge the gap with `block_in_place` plus `block_on`, which pinned a
/// runtime worker for the duration of each round trip and made the store
/// unusable on a current-thread runtime — a constraint that leaked all the way
/// into test attributes.
///
/// # Why `#[async_trait]` and not `async fn` in trait
///
/// Native `async fn` in traits is stable, and it is not usable here: this trait
/// is consumed as `dyn EventStore<E> + Send` behind a shared lock, and a native
/// `async fn` makes a trait non-dyn-compatible. `#[async_trait]` boxes the
/// returned futures, which costs one allocation per call and keeps the trait
/// object that every caller depends on.
///
/// `stream_version_offset` stays synchronous deliberately. It reports a static
/// property of how a store was configured, has no fallible path and no I/O, and
/// no implementation consults storage to answer it — making it asynchronous
/// would add a boxed future per call to describe a constant.
#[async_trait]
pub trait EventStore<E: DomainEvent> {
    /// Append events to the event stream for the given aggregate.
    ///
    /// - `aggregate_type`: The registered type this stream belongs to.
    /// - `aggregate_id`: The bare identifier of the aggregate, distinct from its type.
    /// - `tenant_id`: Optional tenant scope. `Some("")` (empty string) is treated as missing tenant.
    /// - `expected_version`: Optimistic concurrency check. Use `0` for new aggregates.
    /// - `events`: The events to append, wrapped with optional metadata.
    ///
    /// Returns the new stream version on success, or a `PersistenceError`.
    #[allow(clippy::too_many_arguments)]
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError>;

    /// Load all events for the given aggregate in the given tenant.
    ///
    /// Returns `PersistenceError::NotFound` if the aggregate stream does not exist.
    async fn load(
        &self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<Vec<StoredEvent<E>>, PersistenceError>;

    /// List all `(aggregate_type, aggregate_id)` pairs known to this store,
    /// optionally scoped to a tenant.
    async fn list_aggregate_ids(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, PersistenceError>;

    /// Returns the logical position of the first event that `load` would return.
    ///
    /// Stores that hold all events from the beginning return `0`.
    /// Stores that have a pre-seeded version offset (e.g. for test setup) override
    /// this to return the number of events that precede the physical stream.
    ///
    /// This is used by [`PersistenceFacade`] to correctly filter post-snapshot events
    /// when recovering entity state.
    ///
    /// [`PersistenceFacade`]: persistent_entity::persistence::PersistenceFacade
    fn stream_version_offset(
        &self,
        _aggregate_type: &str,
        _aggregate_id: &str,
        _tenant_id: Option<&str>,
    ) -> u64 {
        0
    }

    /// Opens a [unit of work](EventStoreUnitOfWork): a span in which appends
    /// either all become durable together or none of them do.
    ///
    /// Takes `&self`, not `&mut self`. A store handing out a transaction is not
    /// mutating itself — the mutable state belongs to the returned unit of work —
    /// so requiring exclusive access here would force every caller behind a lock
    /// it does not need.
    ///
    /// There is no default implementation. A default would have to either
    /// pretend, by returning something that commits each append as it arrives,
    /// or fail, and both would let an implementation claim transactional
    /// semantics it does not provide. Every store answers this explicitly, and a
    /// store that genuinely cannot offer one says so with an error.
    async fn begin(&self) -> Result<Box<dyn EventStoreUnitOfWork<E>>, PersistenceError>;
}

/// A unit of work over the event store: a span in which appends either all
/// become durable together or none of them do.
///
/// # Why this exists alongside [`EventStore::append`]
///
/// `append` owns its own transaction and commits before returning, which makes
/// it complete on its own and useless as a building block. Nothing can be made
/// to land atomically *with* an append — a second write to another table, a
/// second stream, a durable marker — because by the time `append` hands back,
/// the decision is already made. A caller that needs two writes to share a fate
/// needs to hold the transaction open, and only the store can hand that out.
///
/// # Committing consumes the unit of work
///
/// [`commit`](Self::commit) takes `self: Box<Self>`, so a committed unit of work
/// cannot be used again — the compiler rejects it rather than an implementation
/// discovering a committed transaction at runtime.
///
/// # Dropping is the rollback
///
/// There is deliberately no `rollback` method. A unit of work that is dropped
/// without being committed must leave the store exactly as it was, which makes
/// the safe outcome the one that happens when a caller returns early, is
/// cancelled, or panics — precisely the paths where an explicit call would be
/// missed. An explicit `rollback` would add a second way to express what
/// dropping already means, and the failure mode it invites is forgetting it.
#[async_trait]
pub trait EventStoreUnitOfWork<E: DomainEvent>: Send {
    /// Appends events to a stream inside this unit of work.
    ///
    /// The arguments and the optimistic-concurrency contract are the same as
    /// [`EventStore::append`]'s. The difference is durability: nothing here is
    /// visible to any other reader until [`commit`](Self::commit) succeeds.
    ///
    /// Returns the stream version this unit of work has advanced to. That
    /// version is provisional until the commit succeeds, and callers must not
    /// treat it as durable before then.
    async fn append(
        &mut self,
        aggregate_type: &str,
        aggregate_id: &str,
        tenant_id: Option<&str>,
        expected_version: i64,
        events: Vec<StoredEvent<E>>,
    ) -> Result<i64, PersistenceError>;

    /// Makes everything appended in this unit of work durable, as one step.
    ///
    /// On failure nothing is durable: a failed commit is not a partial commit.
    async fn commit(self: Box<Self>) -> Result<(), PersistenceError>;
}
