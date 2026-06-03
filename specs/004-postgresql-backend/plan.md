# Design: PostgreSQL Persistence Backend

**Branch**: `004-postgresql-backend` | **Date**: 2026-06-03 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/004-postgresql-backend/spec.md`

## Summary

Implement PostgreSQL backends for `EventStore`, `Repository`, and `Snapshot` traits using `sqlx` for database operations. The implementation follows the existing `in_memory/` module pattern in `crates/infrastructure/src/persistence/`.

## Technical Context

**Language/Version**: Rust (latest stable, edition 2021)

**Primary Dependencies**: `sqlx` (PostgreSQL runtime features), `chrono`, `serde_json` (existing)

**Storage**: PostgreSQL 14+

**Testing**: `cargo test` — contract tests against PostgreSQL via testcontainers or local database

**Target Platform**: Linux server, macOS (development)

**Performance Goals**: Single-digit millisecond latency for single-event operations. Connection pooling via `sqlx` built-in pool.

**Constraints**: Runtime-neutral domain traits (no async in trait signatures). Infrastructure layer uses async `sqlx`. Error mapping must preserve domain semantics.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Result**: PASS — No violations detected.
- §C (Spec Scope): Single capability — PostgreSQL backend implementations. No speculative abstractions.
- §E (Architecture Freeze): Technology choice (sqlx + PostgreSQL) belongs in plan.md, not spec.md — compliant.
- §A (Anti Over-Engineering): Three backend implementations directly map to three SPI traits. No registries, factories, or plugin systems.
- §D (Minimal Artifacts): spec.md, plan.md, tasks.md — minimal set. No contracts/ or data-model.md needed (inlined in spec.md).
- §H (Modify Before Duplicate): Follows existing `in_memory/` module structure. No new traits or modules beyond what's needed.

## Project Structure

### Documentation (this feature)

```text
specs/004-postgresql-backend/
├── spec.md              # Behavior, requirements, invariants
├── plan.md              # This file (design decisions)
└── tasks.md             # Phase 2 output (executable tasks)
```

### Source Code (repository root)

```text
crates/infrastructure/src/
├── lib.rs                       # module declarations + re-exports
└── persistence/
    ├── mod.rs                   # persistence module (updated: add postgresql)
    ├── in_memory/               # existing in-memory implementations
    │   ├── mod.rs
    │   ├── event_store.rs
    │   ├── repository.rs
    │   └── snapshot.rs
    └── postgresql/              # NEW: PostgreSQL implementations
        ├── mod.rs
        ├── event_store.rs       # PostgreSQLEventStore
        ├── repository.rs        # PostgreSQLRepository
        └── snapshot.rs          # PostgreSQLSnapshotStore
```

**Structure Decision**: Mirror the existing `in_memory/` module structure with a new `postgresql/` module. Each SPI trait gets its own file. No shared PostgreSQL utilities needed — each implementation is self-contained and minimal.

## Database Schema

Three tables, one per SPI trait:

### events table
```sql
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    aggregate_id VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255),
    version BIGINT NOT NULL,
    event_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_events_aggregate ON events(aggregate_id, tenant_id);
```

### aggregates table
```sql
CREATE TABLE aggregates (
    aggregate_id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255),
    version BIGINT NOT NULL,
    payload JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_aggregates_tenant ON aggregates(tenant_id);
```

### snapshots table
```sql
CREATE TABLE snapshots (
    id BIGSERIAL PRIMARY KEY,
    aggregate_id VARCHAR(255) NOT NULL,
    tenant_id VARCHAR(255),
    version BIGINT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX idx_snapshots_aggregate ON snapshots(aggregate_id, tenant_id);
```

## Complexity Tracking

No constitution violations detected. N/A.
