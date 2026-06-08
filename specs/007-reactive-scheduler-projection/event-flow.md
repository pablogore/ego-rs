# CORE-007 Event Flow

## Actor-to-Scheduler Flow

```
Actor                    Event Bus (bounded)            Scheduler
  │                           │                            │
  │  emit(event)              │                            │
  │ ─────────────────────────▶│                            │
  │                           │   try_send                 │
  │                           │  ┌─────────────────────────┤
  │                           │  │  (non-blocking)         │
  │                           │  │  buffer or drop         │
  │  (continues execution)    │  │                         │
  │ ◀─────────────────────────┤  │                         │
  │                           │  │                         │
  │                           │  │   drain_all()           │
  │                           │  ├─────────────────────▶   │
  │                           │  │                         │
  │                           │  │   update SchedulerState │
  │                           │  │                         │
  │                           │  │   evaluate policy       │
  │                           │  │                         │
  │                           │  │   emit suggestion       │
```

## Flow Guarantees

1. **Non-blocking emission**: Actor never waits for Scheduler
2. **Ordered delivery**: FIFO per channel; sequence_id preserves per-Actor order
3. **Atomic drain**: All pending events are consumed in one operation before state update
4. **Deterministic evaluation**: Policy runs only after state is fully updated
