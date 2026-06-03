# Feature: PostgreSQL Persistence Backend

**Spec**: 004 | **Date**: 2026-06-03 | **Status**: Planning

## Summary

Provide a production-grade PostgreSQL backend for the persistence SPI traits (`EventStore`, `Repository`, `Snapshot`). This backend replaces the in-memory reference implementation for production deployments while maintaining identical contract behavior.

## Requirements

### FR-1: Event Store
The PostgreSQL event store SHALL persist domain events to a relational table with the following columns: aggregate_id, tenant_id (nullable), version, event_type, payload (JSON), created_at (timestamp). The store SHALL support optimistic concurrency control via version checking on append operations.

### FR-2: Repository
The PostgreSQL repository SHALL persist aggregate roots to a relational table with columns: aggregate_id, tenant_id (nullable), version, payload (JSON), updated_at (timestamp). The repository SHALL support optimistic concurrency control via version checking on save operations.

### FR-3: Snapshot Store
The PostgreSQL snapshot store SHALL persist aggregate snapshots to a relational table with columns: aggregate_id, tenant_id (nullable), version, payload (JSON), created_at (timestamp). The store SHALL skip saves when the incoming version is less than or equal to the existing version.

### FR-4: Multi-Tenancy
All three backends SHALL support optional multi-tenancy via a nullable tenant_id column. Queries SHALL filter by tenant_id when provided.

### FR-5: Error Mapping
Database errors SHALL be mapped to the domain `PersistenceError` types: `NotFound`, `Conflict`, `Internal`.

## Contract Invariants

### CI-1: Event Store Append
Appending N events to a new aggregate SHALL return version N. Appending to an existing aggregate SHALL return previous_version + N. Appending with mismatched expected_version SHALL return `Conflict`.

### CI-2: Event Store Load
Loading events for an existing aggregate SHALL return all events in version order. Loading for a non-existent aggregate SHALL return `NotFound`.

### CI-3: Repository Save
Saving a new aggregate SHALL return version 1. Saving an existing aggregate with correct expected_version SHALL return expected_version + 1. Saving with mismatched expected_version SHALL return `Conflict`.

### CI-4: Repository Load
Loading an existing aggregate SHALL return the stored aggregate. Loading a non-existent aggregate SHALL return `NotFound`.

### CI-5: Snapshot Save
Saving a snapshot with version greater than existing version SHALL succeed. Saving with version less than or equal to existing version SHALL succeed but not update.

### CI-6: Snapshot Load
Loading an existing snapshot SHALL return Some((version, payload)). Loading a non-existent snapshot SHALL return None.

## Acceptance Criteria

- [ ] All contract tests pass against PostgreSQL backend
- [ ] EventStore trait is fully implemented with append, load, list_aggregate_ids
- [ ] Repository trait is fully implemented with save, load, delete
- [ ] Snapshot trait is fully implemented with save_snapshot, load_snapshot
- [ ] Error mapping covers NotFound, Conflict, Internal cases
- [ ] Multi-tenancy filtering works correctly

## Out of Scope

- Connection pooling configuration
- Migration management
- Read replicas / query routing
- Batch operations
- Event deduplication
- Snapshot compaction strategies
