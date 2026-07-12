# Examples

Runnable examples showing how to build on ego-rs using only its public APIs.

| Example | What it demonstrates |
|---------|----------------------|
| [`reference-app`](./reference-app) | Production reference service (CORE-018) — a real HTTP service (tenant-scoped user registration) wiring together the Runtime/Service SDK, JWT security, tenant enforcement, the CQRS read-side engine, and TestKit end-to-end. |

Smaller, single-concept examples that ship inside a crate's own `examples/` directory (not listed here) include `crates/service-sdk/examples/hello_service.rs` — the minimal `#[service]`/`with_service`/`resolve` walkthrough referenced from the root [README](../README.md).
