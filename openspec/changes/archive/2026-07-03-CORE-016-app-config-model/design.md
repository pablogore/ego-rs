# Design: CORE-016 — Application Configuration Model

## Technical Approach

ego-rs is a **library workspace** — it ships no binary/host crate today. So the
root `AppConfig` type does NOT live in ego-rs; it lives in each downstream
application built on ego-rs. ego-rs's job for CORE-016 is threefold:

1. Publish the **Library Contract** as a single depend-free trait so every infra
   crate's config domain conforms uniformly.
2. Make each existing infra config domain (`RuntimeConfig`, `JwtProviderConfig`,
   `EventBusConfig`, …) satisfy the contract (`Deserialize + Default + validate()`),
   with **no kit-config dependency**.
3. Ship one **reference composition example** (root config + Host pipeline) that
   applications copy, since there is no host to modify here.

This maps directly to proposal.md (Library Contract, layered validation) and
spec.md (infrastructure domains, service construction, config-agnostic runtime).

## Architecture Decisions

### Decision: Home of the `Validate` contract trait

| Option | Tradeoff | Decision |
|--------|----------|----------|
| `ego-domain` (existing foundation) | Every crate already depends on it, zero cycles, no new crate; mild layering smell (config trait in domain) | **Chosen** |
| New `ego-config-core` crate | Clean layering, but a whole crate for one trait — YAGNI | Rejected |
| Duplicate trait per crate | No shared contract, defeats the point | Rejected |

**Rationale**: The contract is a pure, dependency-free trait (`fn validate(&self) -> Result<(), ConfigError>`), not domain logic. `ego-domain` is the only crate every infra crate (security-jwt, ego-scheduler, persistence, transport) already depends on with no kit-config. Placing it there avoids a new crate and any dependency cycle. See Risks for the layering caveat.

### Decision: Root `AppConfig` lives downstream, ego-rs ships a reference example

**Choice**: Provide `examples/reference-app/` (a `[[example]]` or example crate) showing the root config + Host pipeline, rather than an in-workspace `AppConfig`.
**Alternatives considered**: Add a binary/host crate to the workspace (invents an app ego-rs doesn't have; contradicts spec's "name is application-defined"); put `AppConfig` in `service-sdk` (would force one canonical root, spec forbids).
**Rationale**: Spec states the root type and its name belong to the application. ego-rs demonstrates the pattern; it does not own an instance.

### Decision: RuntimeBuilder left unchanged

**Choice**: No change to `service-sdk/src/runtime/builder.rs`.
**Rationale**: It already receives constructed services (`with_security(authn, authz)`) and never raw config; its doc already routes tunables to `EntityRuntimeBuilder::from_value`. It is already spec-compliant.

## Data Flow

```
Host (downstream app / example / test)
  selects sources + secret provider
        │
   kit_config::ConfigLoader (external, invoked ONCE)
        │  materializes + resolves secrets to plain values
        ▼
   AppConfig  ── AppConfig::validate() ──► cross-domain rules
        │              (calls each subtree .validate() first)
        ├─ config.runtime   ─► EntityRuntimeBuilder::from_value / RuntimeConfig
        ├─ config.jwt        ─► Hs256AuthenticationProvider::new(config.jwt)
        ├─ config.scheduler  ─► Scheduler / EventBus::new(config.scheduler)
        └─ config.database   ─► Database::new(config.database)
        │
        ▼
   RuntimeBuilder::new().with_security(authn, authz).build()   (services only)
```

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/domain/src/config.rs` | Create | `pub trait Validate { fn validate(&self) -> Result<(), ConfigError>; }` + `ConfigError` |
| `crates/domain/src/lib.rs` | Modify | Re-export `config::{Validate, ConfigError}` |
| `crates/persistent-entity/src/runtime.rs` | Modify | `impl Validate for RuntimeConfig` (invariant checks; e.g. non-zero capacities) |
| `crates/security-jwt/src/config.rs` | Modify | `impl Validate for JwtProviderConfig` (leeway bound, aud non-empty when Some) |
| `crates/ego-scheduler/src/event_bus.rs` | Modify | `impl Validate for EventBusConfig` |
| `crates/persistence/src/config.rs` | Create | `DatabaseConfig` (Deserialize + Default + Validate) |
| `crates/transport/src/config.rs` | Create | `GrpcServerConfig` (Deserialize + Default + Validate) |
| `examples/reference-app/` | Create | Reference root config + Host wiring + a cross-domain rule |

## Interfaces / Contracts

```rust
// ego-domain — the Library Contract
pub trait Validate {
    fn validate(&self) -> Result<(), ConfigError>;
}

// downstream application (illustrative — not shipped by ego-rs)
#[derive(serde::Deserialize)]
pub struct AppConfig {
    pub runtime:   RuntimeConfig,
    pub jwt:       JwtProviderConfig,
    pub scheduler: EventBusConfig,
    pub database:  DatabaseConfig,
    // app-specific domains here — kit-config stays unaware of them
}

impl Validate for AppConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.runtime.validate()?;
        self.jwt.validate()?;
        self.scheduler.validate()?;
        self.database.validate()?;
        // cross-domain rules (e.g. scheduler enabled requires database configured)
        Ok(())
    }
}
```

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Unit | each `impl Validate` accepts valid / rejects invalid subtree | table tests per crate |
| Unit | `AppConfig::validate` runs subtree checks then cross-domain rule | construct config in-memory, assert error variant |
| Integration | reference example builds services from a config and starts RuntimeBuilder | example test bypassing kit-config (construct `AppConfig` directly) |

## Migration / Rollout

Additive only. Existing config structs stay; each gains a `Validate` impl. No
call sites break — RuntimeBuilder is untouched. No downstream app exists in this
repo to migrate; the example documents the target pattern.

## Open Questions — Resolved

- [x] `Validate` trait lives in `ego-domain`, per the original decision (every infra crate already depends on it, zero new crate, zero cycles).
- [x] `LoggingConfig`/`TelemetryConfig`/`SecurityConfig` have no home crate today — **deferred**, out of scope for this change. CORE-016 covers only domains with an existing or newly-added home crate (`RuntimeConfig`, `JwtProviderConfig`, `EventBusConfig`, `DatabaseConfig`, `GrpcServerConfig`).
