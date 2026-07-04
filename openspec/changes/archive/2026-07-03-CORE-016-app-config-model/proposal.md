# Proposal: CORE-016 — Application Configuration Model

## Context

The original roadmap planned a "Configuration Framework".

That is no longer the correct architectural direction.

The ecosystem already has a canonical configuration framework:

- kit-config

CORE-016 MUST NOT design another configuration framework.

Instead, CORE-016 defines the **configuration architecture for ego.rs applications**.

kit-config is simply the implementation chosen for that architecture.

---

# Primary Goal

Define how applications built on ego.rs own, compose, validate and consume configuration.

This proposal is about the **application configuration model**, not about parsing TOML or implementing loaders.

---

# Architectural Principle

Configuration belongs to the application.

Libraries own configuration **types**.

Applications own the complete configuration tree.

The configuration framework builds the application configuration tree.

---

# Desired Architecture

```
                  Host
        (CLI / HTTP / Worker / Lambda / tests)
                   │
               config.toml
                   │
             environment
                   │
             secret providers
                   │
              CLI overrides
                   │
             ----------------
             |  kit-config  |
             ----------------
                   │
                   ▼
               AppConfig
                   │
               Application
        ┌──────────┼──────────┐
        │          │          │
     JwtConfig Telemetry Database
        │          │          │
        ▼          ▼          ▼
 security-jwt telemetry persistence
```

The Host selects sources and invokes kit-config exactly once.

The Application receives the already-built AppConfig and owns composition.

Each library contributes its own configuration subtree.

---

# Ownership Model

Clarify ownership of every layer.

Host:

- chooses configuration sources
- decides precedence
- selects the secret provider
- invokes kit-config exactly once
- hands the resulting AppConfig to the Application

Application:

- owns AppConfig
- performs cross-module validation (business composition rules)
- composes services from configured subtrees
- never selects sources, never invokes kit-config directly

Library:

- exposes configuration types
- validates its own configuration
- never loads configuration
- never depends on kit-config
- never depends on a secret provider

Framework (kit-config):

- builds configuration
- deserializes
- merges
- resolves secrets into plain values
- performs structural validation

---

# Validation Ownership

Validation is layered, matching the Ownership Model above:

1. **kit-config** — parsing, deserialization, structural validation.
2. **Library** — `validate()` on its own configuration subtree (self-contained invariants).
3. **Application** — cross-module business rules that span subtrees.

Example of an Application-level rule: JWT enabled while Security is disabled, or
Scheduler enabled without Persistence configured.

---

# Secrets Ownership

Secrets are infrastructure, not library configuration.

- The Host selects which secret provider to use (Vault, AWS Secrets Manager, Azure Key
  Vault, GCP Secret Manager).
- kit-config resolves secret values before AppConfig is deserialized.
- Application and Library code only ever see resolved plain values — never a provider
  type or a secret reference.

This keeps every library and the Application decoupled from which secret backend is in use.

---

# Library Contract

The minimum contract every library must satisfy:

Must provide:

- a configuration type (`#[derive(Deserialize)]`)
- a `Default` implementation
- a `validate()` method for its own invariants

Must NOT provide:

- configuration loading
- parsing
- source selection
- merge logic
- a dependency on kit-config
- a dependency on any secret provider

---

# Questions to Answer — Resolved

## 1. Who owns configuration?

The Host owns *loading* (sources, precedence, secret provider). The Application owns
the resulting AppConfig and business composition. See Ownership Model.

## 2. Who composes AppConfig?

kit-config builds it structurally; the Host invokes kit-config and hands the result
to the Application. The Application never invokes kit-config itself.

## 3. How does a library expose configuration?

A plain `#[derive(Deserialize)]` struct with `Default`, unaware of TOML/YAML/Env. See
Library Contract.

## 4. How is configuration validation performed?

Three layers — kit-config (structural), Library (`validate()` on its own subtree),
Application (cross-module rules). See Validation Ownership.

## 5. How do nested configurations work?

Unchanged from the original draft — AppConfig composes each library's config type as
a field; kit-config deserializes the whole tree in one pass.

## 6. How are defaults handled?

Each library provides `impl Default` for its own config type. No central defaults
mechanism is needed or introduced.

## 7. How should testing override configuration?

Construct `AppConfig` (or a subtree) directly in test code and pass it straight to
the Application/service constructors — bypassing the Host and kit-config entirely.
This is possible because libraries never depend on kit-config.

## 8. How will secrets integrate later?

Secrets are infrastructure, resolved by the Host via kit-config before AppConfig is
built. Libraries never see a provider type. See Secrets Ownership.

---

# Runtime

Clarify explicitly:

RuntimeBuilder does NOT own business configuration.

Configuration terminates before runtime construction.

RuntimeBuilder MUST NOT accept raw configuration objects. It only ever receives
fully-constructed services. This is a canonical rule, not just an example below.

Example:

```rust
let config = loader.load::<AppConfig>()?;

let jwt = JwtService::new(config.jwt);

let scheduler = Scheduler::new(config.scheduler);

RuntimeBuilder::new()
    .with_security(...)
    .build();
```

The runtime consumes configured services.

It does not construct configuration.

---

# Non Goals

Do NOT design:

- ConfigLoader
- ConfigurationProvider
- Builder
- Parser
- Source
- TOML
- YAML
- JSON
- Env loading
- CLI loading

Those already belong to kit-config.

---

# Future Compatibility

The model must naturally support:

- Vault
- AWS Secrets Manager
- Azure Key Vault
- GCP Secret Manager
- Hot Reload
- Profiles
- Multi-environment deployments

without changing library APIs.

---

# Success Criteria

A developer should only need to:

```rust
#[derive(Deserialize)]
pub struct JwtConfig { ... }

#[derive(Deserialize)]
pub struct AppConfig {
    jwt: JwtConfig,
    telemetry: TelemetryConfig,
    scheduler: SchedulerConfig,
}
```

and then:

```rust
let app = ConfigLoader::builder()
    .toml("config.toml")
    .env()
    .build()
    .load::<AppConfig>()?;
```

Every library remains completely unaware of:

- TOML
- YAML
- Env
- Vault
- CLI
- Merge algorithms

The application is the only composition root.

---

# Constraints

Architecture first.

No wrappers.

No duplicated abstractions.

No new configuration framework.

kit-config is the canonical implementation.

The proposal should define the model, not the loader.

---

# Scope Boundaries

Confirmed layering for this change:

- **Proposal** — architecture, ownership, responsibilities (this document).
- **Spec** — observable behavior, exact contracts, exact signatures.
- **Design** — implementation strategy, concrete APIs, module layout.

---

# Deliverables

Produce only:

- proposal.md

Do NOT produce:

- spec.md
- design.md
- tasks.md

Focus on architecture and ownership.

---

# ✅ Proposal Frozen

All six clarification questions converge on a single, consistent architecture with no
contradictions. Ready for:

- Spec
- Design
- Tasks
