//! CORE-PERSIST-B S1: compile-time proof that every relocated
//! `ego-infrastructure` in-memory implementation resolves to the identical
//! item at both its old (`ego_infrastructure::persistence::in_memory::*`)
//! and new (`ego_persistence_memory::*`) path — not a re-declared copy
//! sharing a name.
//!
//! One identity witness per S1 row of the restated compatibility matrix
//! (design.md AD-10, Integration Points). Concrete structs get an identity
//! function (an identity coercion only type-checks against the exact same
//! type); `paginate`, a free function, gets a function-pointer equality
//! check instead (same reasoning as `resolve_tenant` in
//! `crates/persistence-api/tests/reexport_identity.rs`).
//!
//! `InMemoryEventStoreUnitOfWork` needs no witness here: it is private,
//! reachable only via `Box<dyn EventStoreUnitOfWork<E>>` returned from
//! `EventStore::begin` (design.md Integration Points).
//!
//! Both witness shapes fail to compile before the relocation lands, which is
//! this file's RED state.

fn _identity_in_memory_event_store<E>(
    x: ego_infrastructure::persistence::in_memory::InMemoryEventStore<E>,
) -> ego_persistence_memory::persistence::event_store::InMemoryEventStore<E> {
    x
}

fn _identity_in_memory_repository<A>(
    x: ego_infrastructure::persistence::in_memory::InMemoryRepository<A>,
) -> ego_persistence_memory::persistence::repository::InMemoryRepository<A> {
    x
}

fn _identity_in_memory_snapshot_store(
    x: ego_infrastructure::persistence::in_memory::InMemorySnapshotStore,
) -> ego_persistence_memory::persistence::snapshot::InMemorySnapshotStore {
    x
}

fn _identity_in_memory_read_side_store(
    x: ego_infrastructure::persistence::in_memory::InMemoryReadSideStore,
) -> ego_persistence_memory::read_side::store::InMemoryReadSideStore {
    x
}

// `ego-infrastructure` has no direct dependency on `ego-persistence-api`
// (AD-2 criterion 3 keeps that edge exclusive to `ego-persistence-memory`),
// so this file names `EventStreamElement`/`Offset` through `ego_domain`,
// which it already depends on directly and which resolves to the identical
// item (CORE-PERSIST-A's own identity proof, `persistence-api/tests/reexport_identity.rs`).
type Elem = ego_domain::read_side::event_stream::EventStreamElement<serde_json::Value>;

/// `paginate` is a bare function, not a type — an identity coercion proves
/// nothing about it. A function-pointer equality check only holds if both
/// paths name the exact same compiled function, not two functions that
/// merely share a signature. `paginate` takes `impl Iterator<Item = &'a
/// Elem>`; `std::iter::Empty<&'static Elem>` is a concrete instantiation of
/// that bound, letting both sides monomorphize to the same fn pointer type.
#[test]
fn paginate_old_path_is_the_new_path_function() {
    type EmptyIter = std::iter::Empty<&'static Elem>;
    let old: fn(EmptyIter, &str, Option<&ego_domain::read_side::offset::Offset>, usize) -> Vec<Elem> =
        ego_infrastructure::persistence::in_memory::paginate;
    let new: fn(EmptyIter, &str, Option<&ego_domain::read_side::offset::Offset>, usize) -> Vec<Elem> =
        ego_persistence_memory::read_side::store::paginate;
    assert_eq!(old as usize, new as usize);
}
