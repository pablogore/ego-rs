# ego-rs

A hexagonal, actor-oriented, deterministic backend framework for Rust.

## Principles

- **Deterministic-first** — same inputs, same outputs, every time
- **Fail-closed** — ambiguous states produce rejection, not silent continuation
- **Hexagonal architecture** — domain owns contracts, runtime owns execution
- **CQRS + Event Sourcing** — commands mutate, events record, queries read
- **Framework-first** — build the framework before modeling runtime governance
- **Minimal primitives** — one concept, one trait, one responsibility

## Requirements

- Rust — see `rust-toolchain.toml` for the pinned version.
- **PostgreSQL 14 or later** to *run* an application on the `ego-persistence`
  backend. This is the declared minimum this workspace supports; it tracks
  PostgreSQL's own support lifecycle rather than any feature this framework uses
  today.

**No Docker, and no database, is required to build or test this workspace.**
`cargo test --workspace` runs entirely in-process. That is a constitutional
requirement, not a convenience: CC-R11 (No Infrastructure Dependency) and UT-R4
(No Testcontainers) forbid infrastructure in these tests, and
`scripts/detect-integration-tests.sh` enforces it in CI.

Coverage that genuinely needs a real database or socket will live in `integration-tests/`
— outside the root Cargo workspace, inside this repository, as an independent Cargo
workspace that the root neither compiles nor runs. It is invoked explicitly with
`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`. That workspace does not
exist yet: until issue #275 closes, those invariants remain unverified.
`docs/integration-test-backlog.md` records every such property and what it must
assert, so that work is deferred rather than lost.

## Crates

| Crate | Layer | Description |
|-------|-------|-------------|
| `ego-domain` | Domain | Core contracts: commands, queries, events, actors, persistence — zero dependency on infrastructure/transport/runtime |
| `ego-application` | Application | Command/query handlers orchestrating domain logic through hexagonal ports |
| `ego-infrastructure` | Infrastructure | Adapters (in-memory, read-side store) implementing domain ports |
| `ego-persistence` | Infrastructure | PostgreSQL persistence backend |
| `ego-transport` | Transport | HTTP mechanism layer (app state, security extractor, error mapper, server bootstrap) — concrete routes live in the application, not here |
| `ego-runtime` | Runtime | Platform abstraction for executing actors; the `Runtime` trait plus the read-side (CQRS) tag scheduler |
| `ego-runtime-tokio` | Runtime | Tokio-backed `Runtime` implementation (`TokioRuntime`, `TokioRuntimeBuilder`) |
| `ego-event-adapter` | Runtime | Converts between protobuf events, CloudEvents, and EventStore format |
| `persistent-entity` | Runtime | Event-sourced entity runtime and SDK (Command/Event/State, `EntityRuntimeBuilder`) |
| `ego-scheduler` | Runtime | Actor mailbox scheduling and event bus (dispatch, backpressure, gap detection) |
| `ego-service-sdk` | Service SDK | `RuntimeBuilder`/`Runtime`, service registry, DI, interceptors — the public surface most applications build on |
| `ego-service-sdk-macros` | Service SDK | Proc-macros: `#[service]`, `#[operation]`, `#[authorize]`, `#[tenant_scoped]` |
| `ego-security-sdk` | Security | Transport- and provider-agnostic security primitives (`SecurityContext`, `AuthenticationProvider`, `AuthorizationProvider`) |
| `security-jwt` | Security | JWT authentication providers (HS256/RS256/ES256) |
| `security-apikey` | Security | API key authentication provider |
| `ego-testkit` | Testing | Reusable test fixtures for building on ego-rs services |

## Quick Start

Define a service contract with the real macros, register it, resolve it, invoke it — no hidden state, no manual proxy wiring:

```rust
use std::sync::Arc;
use ego_service_sdk::app::App;
use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::error::ServiceError;
use ego_service_sdk_macros::{operation, service};

#[service(version = "1.0.0")]
pub trait HelloService {
    #[operation]
    async fn greet(&self, ctx: ServiceContext, name: String) -> Result<String, ServiceError>;
}

pub struct HelloServiceImpl;

#[async_trait::async_trait]
impl HelloService for HelloServiceImpl {
    async fn greet(&self, _ctx: ServiceContext, name: String) -> Result<String, ServiceError> {
        Ok(format!("hello, {name}"))
    }
}

#[tokio::main]
async fn main() {
    let instance: Arc<dyn HelloService> = Arc::new(HelloServiceImpl);
    let app = App::builder()
        .service_instance::<HelloServiceTag>(instance)
        .build()
        .expect("composition succeeds");

    let hello = app.resolve::<HelloServiceTag>().expect("registered tag resolves");
    let out = hello.greet(ServiceContext::new(), "world".into()).await.unwrap();
    println!("{out}"); // hello, world
}
```

See `crates/service-sdk/examples/hello_service.rs` for the full runnable version.

## Reference Service

[`examples/reference-app`](./examples/reference-app) is the production reference service (CORE-018) — a dogfooding milestone that builds a real capability (tenant-scoped user registration) using only ego-rs's public APIs: Runtime/Service SDK, config, logging, JWT security, tenant enforcement, the CQRS read-side engine, and TestKit, wired together end-to-end behind a real HTTP server with Swagger docs. See [its README](./examples/reference-app/README.md) to run it.

## Documentation

See [COOKBOOK.md](./COOKBOOK.md) for usage recipes and [ARCHITECTURE.md](./ARCHITECTURE.md) for design.

## License

MIT
