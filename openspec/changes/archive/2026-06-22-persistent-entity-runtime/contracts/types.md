# Contract: Shared Value Types

**Files**: `crates/persistent-entity/src/command_context.rs`, `crates/persistent-entity/src/error.rs`

## Purpose

Value types shared across the entity runtime public API. All types are immutable (fields accessible, but structs are not directly constructable outside the crate where invariants apply).

---

## `CommandContext`

```rust
#[derive(Clone, Debug)]
pub struct CommandContext {
    pub tenant_id: TenantId,
    pub correlation_id: CorrelationId,
    pub causation_id: CausationId,
    pub approval_id: Option<ApprovalId>,
    pub metadata: HashMap<String, String>,
}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tenant_id` | `TenantId` | Yes | Multi-tenant scoping. Empty string = single-tenant mode. |
| `correlation_id` | `CorrelationId` | Yes | End-to-end trace id, propagated across commands. |
| `causation_id` | `CausationId` | Yes | Links command to parent event/command. |
| `approval_id` | `Option<ApprovalId>` | No | Functional correlation for approval workflows. |
| `metadata` | `HashMap<String, String>` | Yes | Extensible metadata map. Never null, may be empty. |

### Type Wrappers (from `ego-domain`)

```rust
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct TenantId(pub String);

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct EntityId(pub String);

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct CorrelationId(pub String);

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct CausationId(pub String);

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct ApprovalId(pub String);
```

---

## `CommandResult<E, S>`

```rust
#[derive(Debug)]
pub enum CommandResult<E, S> {
    Events {
        events: Vec<E>,
        new_state: S,
        new_version: u64,
    },
    NoEvents {
        state: S,
    },
}
```

### Variants

| Variant | Meaning | Version change |
|---------|---------|----------------|
| `Events` | Command produced events. Events persisted, version advanced. | `new_version > previous` |
| `NoEvents` | Command was a zero-event query. No persistence. | No change |

---

## `EntityError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    #[error("entity not found: {0}")]
    EntityNotFound(EntityId),

    #[error("version conflict: expected {expected}, current {current}")]
    VersionConflict { expected: u64, current: u64 },

    #[error("entity is passivating, retry later")]
    EntityPassivating,

    #[error("mailbox at capacity ({0})")]
    MailboxFull(usize),

    #[error("reentrancy not allowed")]
    ReentrancyNotAllowed,

    #[error("handler error: {0}")]
    Handler(Box<dyn std::error::Error + Send>),

    #[error("runtime error: {0}")]
    Runtime(String),
}
```

### Error Mapping

| Error | When returned | Caller action |
|-------|---------------|---------------|
| `EntityNotFound` | Non-creation command to non-existent entity | Create entity first, or use creation command |
| `VersionConflict` | `expected_version` doesn't match store version | Refresh state from entity, retry with current version |
| `EntityPassivating` | Entity is draining its mailbox | Retry — entity will be available shortly |
| `MailboxFull` | Entity mailbox at capacity | Retry with backoff |
| `ReentrancyNotAllowed` | Handler sent command to its own entity | Fix handler logic — reentrancy forbidden |
| `Handler` | Business rule violation in command handler | Examine handler logic or input |
| `Runtime` | Internal error (I/O, panic, bug) | Report bug, may require restart |
