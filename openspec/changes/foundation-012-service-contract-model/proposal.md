## Why

Service interaction boundaries across ego-rs are currently implicit and insufficiently governed. Without constitutional service contract governance, transport semantics leak into service definitions, runtime boundaries become ambiguous, policy behavior becomes implicit, and replay trustworthiness degrades. A dedicated Service Contract Model constitution is needed to define service contract semantics, endpoint contract boundaries, exposure descriptors, service-level policy attachment, deterministic interaction behavior, and governance enforcement — without prescribing transport protocols or runtime implementations.

## What Changes

- Create a constitutional **Service Contract Model** spec (`specs/service-contract-model/spec.md`) that defines service contract semantics, endpoint contract boundaries, exposure descriptor semantics, service policy attachment, deterministic interaction expectations, fail-closed behavior, observability semantics, governance enforcement, and cross-spec governance
- Service interaction boundaries across architectural layers SHALL be jointly governed by Architecture Governance and Service Contract Model
- Amend the **Architecture Governance** spec to cross-reference the Service Contract Model for service boundary governance with explicit, non-overlapping authority ownership

## Capabilities

### New Capabilities
- `service-contract-model`: Constitutional governance for service contract semantics across ego-rs. Defines service contract semantics, endpoint contract boundaries, exposure descriptor model, transport-neutral service definition, service policy attachment semantics, deterministic interaction boundaries, fail-closed service behavior, observability semantics, governance enforcement, and cross-spec authority ownership.

### Modified Capabilities
- `architecture-governance`: Service interaction boundaries SHALL comply with both architectural boundary governance and Service Contract Model governance. Architecture Governance remains authoritative for layer boundaries and dependency direction; Service Contract Model governs service interaction semantics.

## Impact

- `openspec/specs/`: New `service-contract-model/` spec directory with `spec.md`
- `openspec/specs/architecture-governance/spec.md`: Amendment to cross-reference service contract model
- No runtime code, no transport protocol changes, no serialization changes, no networking code
