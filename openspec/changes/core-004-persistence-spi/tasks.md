## [ ] 1. Persistence Port Traits

- [ ] 1.1 Define `EventStore` trait in `crates/domain/` — `append(aggregate_id, events) → Result<(), PersistenceError>`, `read(aggregate_id, from_version) → Result<Vec<Event>, PersistenceError>`
- [ ] 1.2 Define `SnapshotStore` trait in `crates/domain/` — `save(aggregate_id, version, state) → Result<(), PersistenceError>`, `load(aggregate_id) → Result<Option<(u64, State)>, PersistenceError>`
- [ ] 1.3 Define `PersistenceError` enum — NotFound, VersionConflict, StorageFailure, AmbiguousState
- [ ] 1.4 Define `PersistableEvent` trait — `event_type() → &str`, `event_version() → u64`

## [ ] 2. Replay Semantics

- [ ] 2.1 Event replay: given identical event stream, replay SHALL produce identical reconstructed state
- [ ] 2.2 Snapshot + event catch-up: load latest snapshot, apply subsequent events to rebuild state
- [ ] 2.3 Fail-closed on version conflict: if expected version != actual version, reject the append

## [ ] 3. In-Memory Adapter

- [ ] 3.1 Implement `InMemoryEventStore` in `crates/infrastructure/` — implements `EventStore` trait
- [ ] 3.2 Implement `InMemorySnapshotStore` in `crates/infrastructure/` — implements `SnapshotStore` trait
- [ ] 3.3 Version conflict detection — `append` fails if aggregate version has changed since last read

## [ ] 4. Tests

- [ ] 4.1 Test: append events → read events → events match in order and content
- [ ] 4.2 Test: append with wrong version → returns `VersionConflict` error
- [ ] 4.3 Test: save snapshot → load snapshot → state matches
- [ ] 4.4 Test: snapshot + event catch-up → load snapshot at v3, apply events v4-v7 → state matches state after v7
- [ ] 4.5 Test: event replay determinism — replay same events twice → identical reconstructed state
- [ ] 4.6 Test: read non-existent aggregate → returns empty
- [ ] 4.7 Test: fail-closed on ambiguous storage state

## [ ] 5. Verification

- [ ] 5.1 Run `cargo test --workspace` — all tests pass
- [ ] 5.2 Run `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] 5.3 Verify traits live in `crates/domain/` (hexagonal: domain defines port)
- [ ] 5.4 Verify in-memory adapter lives in `crates/infrastructure/` (hexagonal: infrastructure implements port)
- [ ] 5.5 Verify tests use in-memory adapter only — no real database, no filesystem, no network