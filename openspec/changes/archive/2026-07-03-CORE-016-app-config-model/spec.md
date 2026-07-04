# CORE-016 Spec — ego.rs Configuration Integration

## Context

KIT-001 has frozen the architecture of `kit-config`.

`kit-config` is now the canonical application configuration framework for the ecosystem.

This specification does **not** redesign configuration loading.

Instead, it specifies how ego.rs applications integrate with `kit-config`.

---

# Scope

Define the canonical configuration model for ego.rs applications.

This specification describes:

- the root configuration model
- composition of infrastructure domains
- ownership boundaries
- runtime integration

It does **not** redesign:

- ConfigLoader
- ConfigurationSource
- ConfigModule
- Validation
- feature flags
- parsing
- loaders

Those belong to KIT-001.

---

# Goals

Every ego.rs application should expose a single root configuration model.

Example:

```rust
pub struct AppConfig {
    pub runtime: RuntimeConfig,
    pub logging: LoggingConfig,
    pub telemetry: TelemetryConfig,
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub security: SecurityConfig,

    // application-specific
    pub billing: BillingConfig,
}
```

The application composes reusable library domains together with its own domains.

---

# Requirements

## Root Configuration

An ego.rs application MUST expose a single root configuration type.

The name is application-defined.

Examples:

- AppConfig
- GatewayConfiguration
- BillingConfiguration
- AtlasConfiguration

The name has no semantic meaning.

---

## Infrastructure Domains

Reusable infrastructure crates publish their own configuration domains.

Examples:

- RuntimeConfig
- LoggingConfig
- TelemetryConfig
- DatabaseConfig
- SchedulerConfig
- SecurityConfig
- JwtConfig
- GrpcServerConfig

Applications compose these into the root model.

---

## Application Domains

Applications may define arbitrary configuration domains.

Example:

```rust
pub struct BillingConfig

pub struct InvoiceConfig

pub struct RecommendationConfig
```

kit-config must remain unaware of these domains.

---

## Validation

Validation ownership follows KIT-001.

Host

↓

structural validation

Library

↓

domain invariants

Application

↓

cross-domain rules

The root configuration SHOULD implement Validation when cross-domain validation is required.

---

## Runtime Integration

RuntimeBuilder MUST NOT receive raw configuration values.

Example:

NOT allowed

```rust
RuntimeBuilder::new()
    .with_logging_config(...)
```

Canonical model

```rust
let logger = Logger::new(config.logging);

RuntimeBuilder::new()
    .with_logger(logger)
```

Configuration materialization completes before runtime construction begins.

---

## Service Construction

Services receive typed configuration.

Example:

```rust
Database::new(config.database)

JwtProvider::new(config.jwt)

Scheduler::new(config.scheduler)
```

Services never load configuration themselves.

---

## Secrets

Libraries receive resolved values only.

No library may interact directly with:

- Vault
- AWS Secrets Manager
- Azure Key Vault
- GCP Secret Manager

---

## Configuration Ownership

Host owns:

- configuration loading
- source selection
- precedence
- secrets
- profiles

Application owns:

- root model
- composition
- cross-domain validation

Libraries own:

- reusable configuration domains

kit-config owns:

- materialization

---

## Observable Behavior

A correctly configured ego.rs application:

1. selects configuration sources
2. materializes the root configuration using kit-config
3. performs validation according to KIT-001
4. constructs infrastructure services
5. constructs application services
6. starts RuntimeBuilder

---

# Non Goals

Do not redesign:

- kit-config
- loaders
- ConfigModule
- Validation
- ConfigurationSource
- feature flags

Do not introduce:

- ego.rs-specific configuration framework
- wrapper around kit-config
- duplicate loading APIs

---

# Success Criteria

After this specification:

- every ego.rs application follows the same configuration architecture
- infrastructure crates expose reusable configuration domains
- applications compose those domains into a root configuration
- RuntimeBuilder remains configuration-agnostic
- services never load configuration
- configuration loading is entirely delegated to kit-config
- ownership boundaries remain identical to KIT-001
