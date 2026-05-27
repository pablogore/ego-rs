# ego-rs

A hexagonal, actor-oriented, deterministic backend framework for Rust.

## Principles

- **Deterministic-first** — same inputs, same outputs, every time
- **Fail-closed** — ambiguous states produce rejection, not silent continuation
- **Hexagonal architecture** — domain owns contracts, runtime owns execution
- **CQRS + Event Sourcing** — commands mutate, events record, queries read
- **Framework-first** — build the framework before modeling runtime governance
- **Minimal primitives** — one concept, one trait, one responsibility

## Crates

| Crate | Layer | Description |
|-------|-------|-------------|
| `ego-domain` | Domain | Core contracts: Actor, Command, Event, Query |
| `ego-application` | Application | Command/Query handlers, ports |
| `ego-infrastructure` | Infrastructure | Adapters (in-memory, persistence) |
| `ego-transport` | Transport | HTTP/gRPC endpoint wiring |
| `runtime-slice` | Runtime (core) | Deterministic execution types |

## Quick Start

```rust
use ego_domain::actor::Actor;

struct MyActor;
impl Actor for MyActor {
    type Message = String;
}
```

See [COOKBOOK.md](./COOKBOOK.md) for usage recipes and [ARCHITECTURE.md](./ARCHITECTURE.md) for design.

## License

MIT
