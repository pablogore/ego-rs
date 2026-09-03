//! S1 (design AD-9): the shared `Repository` conformance harness, run against
//! `InMemoryRepository`.
//!
//! `ego-testkit` already depends on `ego-persistence-memory` as a normal
//! dependency (`Cargo.toml`), so this run adds no new dependency edge (EC-2).
//! `crates/persistence-memory/` itself is not touched at all.

use ego_persistence_memory::persistence::repository::InMemoryRepository;
use ego_testkit::{assert_repository_conformance, ConformanceAggregate};

#[test]
fn in_memory_repository_satisfies_the_shared_conformance_suite() {
    let mut repository: InMemoryRepository<ConformanceAggregate> = InMemoryRepository::new();
    assert_repository_conformance(&mut repository);
}
