# Research: CORE-007 Reactive Scheduler & Deterministic Projection Engine

## 1. Event Emission in CORE-006

**Finding**: `ego-runtime` has NO event emission infrastructure. The runtime uses `tokio::sync::mpsc::channel(64)` internally for message passing only. There is no broadcast channel, EventBus, callback registration, or subscriber list.

The `Effect` enum in `ego-domain` has `EventEmission(Vec<E>)` as a variant (a *description of intent*), but actual emission is deferred to the `EffectInterpreter` implementer.

**Decision**: CORE-007 must introduce its own event bus. Follow the established pattern from `persistent-entity` crate:
- `crates/persistent-entity/src/scheduler_event.rs` defines `SchedulerEventBusConfig`, `SchedulerEventSender` (fire-and-forget via `try_send`), `SchedulerEventReceiver` (non-blocking `drain_all`), and `SchedulerTrigger` (tokio::sync::Notify-based wakeup).
- CORE-007 will replicate this pattern with CORE-007-specific event types.

**Reference**: `crates/persistent-entity/src/scheduler_event.rs`

## 2. EntityTriple Type

**Finding**: `EntityTriple` does NOT exist in `ego-domain`. It exists in two places within `persistent-entity`:
- `crates/persistent-entity/src/scheduler.rs`: `{ tenant_id: String, entity_type: String, entity_id: String }`
- `crates/persistent-entity/src/types.rs`: `{ tenant: TenantId, entity_type: &'static str, entity_id: String }` — this module is NOT listed in `lib.rs`, so it's private.

**Decision**: Define `EntityTriple` within the `ego-scheduler` crate rather than modifying `ego-domain` (which would touch shared infrastructure). This keeps CORE-007 self-contained and follows the "patch over rewrite" principle. A future refactor may consolidate all `EntityTriple` definitions into `ego-domain`.

**Alternatives considered**:
- Add to `ego-domain`: Architecturally correct but modifies shared crate beyond CORE-007 scope.
- Import from `persistent-entity`: Creates an undesirable CORE-007 → CORE-006 dependency.

## 3. Existing Event Bus Pattern (persistent-entity)

The `persistent-entity` crate demonstrates a production event bus pattern:

| Component | Purpose |
|-----------|---------|
| `SchedulerEventBusConfig` | Configuration with `capacity: usize` (default 4096) |
| `SchedulerEvent` | Enum with event variants |
| `SchedulerEventSender` | `try_send` via `mpsc::Sender` — non-blocking, returns `false` on full |
| `SchedulerEventReceiver` | `drain_all(&mut self) -> Vec<BusItem>` — non-blocking drain |
| `SchedulerTrigger` | Wraps `tokio::sync::Notify` for async wakeup |
| Factory functions | `event_bus_channel()` and `event_bus_channel_with_config()` |

CORE-007 will replicate this pattern with its own event types.

## 4. Domain Types Available

| Type | Module | Why CORE-007 needs it |
|------|--------|----------------------|
| `ActorId` | `ego-domain::actor` | Source actor identification |
| `EntityId` | `ego-domain::context` | Entity identity |
| `TenantId` | `ego-domain::context` | Multi-tenant scoping |
| `CorrelationId` | `ego-domain::context` | Cross-event correlation |
| `CausationId` | `ego-domain::context` | Causation tracking |
| `DomainEvent` | `ego-domain::event` | Event trait reference |

## 5. Layer Assignment

**Decision**: `foundation` layer (same as `ego-runtime`). CORE-007 is a runtime-adjacent operational concern, not application logic or infrastructure.

**Implications**: Must update:
- `Cargo.toml` workspace members (add `crates/ego-scheduler`)
- `layers.toml` (add `"ego-scheduler" = "foundation"`)
- `scripts/verify-layers.sh` (may need update for new crate)

## 6. Crate Creation

**Decision**: New crate `crates/ego-scheduler/`. Name convention follows existing `ego-*` pattern.

## 7. Testing Strategy

- **Unit tests**: Pure function tests for `SchedulerState::apply()`, `RoundRobin::suggest_activation()`
- **Determinism tests**: Two Scheduler instances fed identical event sequences → identical state
- **Backpressure tests**: Overflow behavior for `Block` and `DropNewest` policies
- **Gap detection tests**: Detectable gaps from event stream with missing sequence_ids
- **Integration tests**: End-to-end event → Scheduler → suggestion flow
