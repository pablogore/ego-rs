# Contracts: Persistent Entity Runtime

**Feature**: `006-persistent-entity-runtime`

Public API contracts and trait definitions for the `ego-persistent-entity` crate. These contracts define the boundary between user code and the framework. Implementation details are intentionally absent.

## Contents

| File | Description |
|------|-------------|
| [persistent-entity.md](persistent-entity.md) | `PersistentEntity` trait — user implements this for domain entities |
| [entity-ref.md](entity-ref.md) | `EntityRef` API — command sending handle |
| [runtime.md](runtime.md) | `EntityRuntime` and `EntityRuntimeBuilder` — lifecycle manager |
| [spi.md](spi.md) | SPI contracts: `EventPublisher`, `SnapshotStrategy` |
| [types.md](types.md) | Shared value types: `CommandContext`, `EntityError`, `CommandResult` |

## Principles

1. **No implementation type leakage**: All public types are domain-owned. No Tokio, Postgres, or framework types appear in public signatures.
2. **Determinism**: All handler inputs/outputs are pure values. No side effects in public contracts.
3. **Immutability**: All data structures are immutable. State transitions produce new instances.
4. **Testability**: All contracts are testable with in-memory backends. No infrastructure required.
