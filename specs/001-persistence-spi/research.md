/# Research: Persistence SPI

## Overview

All NEEDS CLARIFICATION markers resolved during `/speckit.clarify`. The spec is complete and unambiguous. This document consolidates technology decisions, alternatives considered, and rationale.

## Decisions

### Decision 1: Multi-tenancy optionality

- **Decision**: `tenant_id` is `Option<TenantId>`; `None` = single-tenant mode, `Some(id)` = full isolation
- **Rationale**: Single-tenant applications should not be forced to provide a tenant context. The SPI adapts behavior based on the variant.
- **Alternatives considered**:
  - Sentinel/default tenant value: Less type-safe, error-prone
  - Separate trait sets: Duplication, violates DRY
  - Compile-time feature flag: Adds build complexity, limits runtime flexibility

### Decision 2: Runtime-neutral domain traits

- **Decision**: All SPI trait methods are synchronous (no async) with no Tokio/runtime types
- **Rationale**: Domain layer must remain dependency-free. Infrastructure implementations add async behind the SPI boundary.
- **Alternatives considered**: Making traits async from the start — would couple domain to a specific runtime

### Decision 3: Two initial backends (InMemory + PostgreSQL)

- **Decision**: InMemory for testing/reference, PostgreSQL for production
- **Rationale**: Follows hexagonal architecture — contract tests with InMemory validate SPI behavior; PostgreSQL provides production-grade persistence.
- **Alternatives considered**: SQLite as intermediate — deferred; PostgreSQL addresses the immediate production need

### Decision 4: Serialization framework

- **Decision**: `serde_json` for snapshot payloads; `DomainEvent` trait provides the event contract
- **Rationale**: Already present in `ego-domain`, no new external dependencies required
- **Alternatives considered**: Custom serialization trait — would add unnecessary complexity for v1

### Decision 5: Migration infrastructure

- **Decision**: Shared migration infrastructure in `ego-infrastructure` (`MigrationRegistry`, `MigrationContext` trait); each backend owns its migration scripts
- **Rationale**: Versioned, deterministic, idempotent migrations with startup validation — follows established patterns (Flyway, Diesel)
- **Alternatives considered**: Embedding migration logic in domain — violates dependency rules; third-party migration tools — adds coupling
