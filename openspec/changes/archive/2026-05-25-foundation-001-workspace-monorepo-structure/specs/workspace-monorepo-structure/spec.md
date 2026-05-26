## ADDED Requirements

### Requirement: Canonical workspace layout
First-party Rust crates SHALL live under `crates/`. The canonical package groups are `domain`, `application`, `infrastructure`, `transport`, `contracts`, and `observability`.

#### Scenario: Workspace members are declared
- **WHEN** the root `Cargo.toml` is inspected
- **THEN** every first-party crate under `crates/` SHALL be listed as a workspace member

#### Scenario: New crate uses canonical group
- **WHEN** a new first-party crate is added
- **THEN** it SHALL live under a canonical package group

### Requirement: Package groups are not layers
Package groups SHALL NOT introduce architecture layers beyond those defined by `architecture-governance`. Every crate MUST map to exactly one of: domain, application, infrastructure, or transport.

#### Scenario: Support package is added
- **WHEN** a crate is added under `crates/contracts` or `crates/observability`
- **THEN** `layers.toml` SHALL map it to one existing architecture layer

#### Scenario: Unknown layer is declared
- **WHEN** `layers.toml` declares a layer outside domain, application, infrastructure, or transport
- **THEN** layer validation SHALL fail

### Requirement: Crate naming convention
Workspace crate names SHALL use `ego-<group>` for top-level group crates or `ego-<group>-<module>` for split module crates.

#### Scenario: Top-level crate name is checked
- **WHEN** a top-level group crate declares its package name
- **THEN** the name SHALL match `ego-<group>`

#### Scenario: Module crate name is checked
- **WHEN** a split module crate declares its package name
- **THEN** the name SHALL match `ego-<group>-<module>`

### Requirement: Layer ownership source of truth
`layers.toml` SHALL be the source of truth for workspace crate layer ownership. Every workspace crate MUST have exactly one layer mapping.

#### Scenario: Workspace crate is present
- **WHEN** a crate appears in root workspace members
- **THEN** `layers.toml` SHALL contain exactly one mapping for that crate package name

#### Scenario: Stale layer mapping exists
- **WHEN** `layers.toml` references a crate missing from the workspace
- **THEN** validation SHALL fail

### Requirement: Dependency rules mirror architecture governance
Workspace dependency rules SHALL mirror `architecture-governance` and MUST NOT redefine architecture semantics.

#### Scenario: Domain dependencies are checked
- **WHEN** the domain crate dependencies are inspected
- **THEN** they SHALL NOT include application, infrastructure, or transport crates

#### Scenario: Transport dependencies are checked
- **WHEN** the transport crate dependencies are inspected
- **THEN** they SHALL NOT include infrastructure crates

#### Scenario: Infrastructure dependencies are checked
- **WHEN** the infrastructure crate dependencies are inspected
- **THEN** they SHALL NOT include transport crates

### Requirement: Public module boundaries
Each crate SHALL expose intentional public APIs through `src/lib.rs`. Internal modules MUST remain private unless they are part of the crate boundary.

#### Scenario: Cross-crate import is reviewed
- **WHEN** one crate imports another first-party crate
- **THEN** the import SHALL use the dependency crate public API

#### Scenario: Implementation module is private
- **WHEN** a module is an implementation detail
- **THEN** it SHALL remain private to its crate
