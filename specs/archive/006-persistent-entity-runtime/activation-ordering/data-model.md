# Data Model: Activation Ordering

**Date**: 2026-06-07  
**Source**: [spec.md](spec.md), [registry-visibility-semantics.md](registry-visibility-semantics.md)

## Entity: `EntityTriple`

| Field | Type | Description |
|-------|------|-------------|
| `tenant_id` | `String` | Multi-tenant isolation scope |
| `entity_type` | `String` | Entity type discriminator (e.g., "order", "account") |
| `entity_id` | `String` | Unique identifier within (tenant, type) scope |

**Uniqueness**: `(tenant_id, entity_type, entity_id)` — used as key in all registry maps.

**Identity rule**: No two entities share the same triple. `aggregate_id()` = `"{entity_type}:{entity_id}"`.

## Entity: `LifecycleStateMachine`

| State | In Registry? | Ready? | Transitions To |
|-------|-------------|--------|----------------|
| `Recovering` | Yes | No | `Active`, `Failed` |
| `Active` | Yes | Yes | `Passivating`, `Failed` |
| `Passivating` | Yes | Yes (draining) | `Passivated`, `Failed` |
| `Passivated` | No | N/A | (none — re-activation creates new actor) |
| `Failed` | No (removed) | N/A | (none — retry creates new actor) |

**Valid transitions**:
- `Recovering → Active` (recovery success)
- `Recovering → Failed` (recovery error)
- `Active → Passivating` (idle timeout)
- `Active → Failed` (runtime error)
- `Passivating → Passivated` (drain complete)
- `Passivating → Failed` (drain error)

## Entity: `SharedActivation`

| Field | Type | Purpose |
|-------|------|---------|
| `lock` | `Mutex<()>` | Per-entity serialization for spawn decision |
| `result_tx` | `watch::Sender<Option<EntityError>>` | Recovery outcome notification |
| `result_rx` | `watch::Receiver<Option<EntityError>>` | Recovery outcome observation |

**Lifetime**: Created on first activation attempt. Released after `insert_active()`. Removed from `pending_activations` map after guard release.

## Entity: `ActorHandle`

| Field | Type | Purpose |
|-------|------|---------|
| `sender` | `Box<dyn Any + Send + Sync>` | Type-erased `mpsc::Sender<CommandEnvelope<C>>` |
| `join` | `JoinHandle<()>` | Actor task handle |

**Ownership**: Registry holds the Sender. Actor task holds the Receiver. Both must exist for the channel to be open.

## Entity: `PassivationEntry`

| Field | Type | Purpose |
|-------|------|---------|
| `last_known_version` | `u64` | Entity version at passivation time |
| `passivated_at` | `Instant` | Timestamp for expiry/cleanup |

## Registry Maps

| Map | Key | Value | Purpose |
|-----|-----|-------|---------|
| `active` | `EntityTriple` | `ActorHandle` | Active entity lookup for command routing |
| `passivated` | `EntityTriple` | `PassivationEntry` | Passivated entity tracking |
| `pending_activations` | `EntityTriple` | `Arc<SharedActivation>` | Single-flight activation coordination |
