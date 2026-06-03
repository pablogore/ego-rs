# Data Model: Persistence SPI

## Entities

### Aggregate

- **Description**: A domain entity with identity, state, and a version tracked by the persistence layer
- **Identity**: By `aggregate_id` (String/ID type) within an optional tenant scope
- **Version**: Monotonically increasing integer tracking mutation count
- **State**: Generic type parameter `A` on `Repository<A>` — domain-specific aggregate state

### Event

- **Description**: A record of something that happened to an aggregate
- **Identity**: By position within an aggregate stream (version number)
- **Contract**: `DomainEvent` trait (defined in `crates/domain/src/event.rs`)
- **Ordering**: Events are ordered by append sequence within a stream

### Tenant

- **Description**: Optional isolation boundary
- **When `Some(TenantId)`**: Enables full isolation — same aggregate IDs in different tenants are separate
- **When `None`**: Single-tenant mode — no isolation boundaries, all data in default scope
- **Validation**: In multi-tenant mode, empty/blank `tenant_id` values are rejected with `PersistenceError::MissingTenant`

### Snapshot

- **Description**: Cached aggregate state at a specific version
- **Version**: Tracks the aggregate version the snapshot represents
- **Payload**: Serialized data (`serde_json::Value` initially)
- **Retrieval**: Always returns the highest-versioned snapshot for an aggregate

### Stream

- **Description**: Ordered sequence of events for a single aggregate within an optional tenant scope
- **Scoping**: Same aggregate ID in different tenants = different streams
- **Version**: The stream version is the total count of events appended

## Key Relationships

- Aggregate has many Events (1:N)
- Aggregate has zero or one Snapshot (per latest version) (1:0..1)
- Stream belongs to Aggregate (1:1)
- Tenant scopes Aggregate, Event, Stream, Snapshot (0..1:N)

## Validation Rules

- `expected_version` must match current stream version or operation fails with `Conflict`
- Empty/missing `tenant_id` in multi-tenant mode fails with `MissingTenant`
- Duplicate aggregate IDs across tenants are valid (isolated by tenant scope)
