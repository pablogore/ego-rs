## Why

Transport exposure of service contracts across ego-rs is currently implicit and insufficiently governed. The Service Contract Model (FOUNDATION-012) defines WHAT service interactions mean, but no constitutional spec governs HOW those interactions become transport-exposed. Without constitutional transport binding governance, service semantics leak into transport implementations, retry semantics become implicit, observability becomes transport-dependent, and replay trustworthiness degrades. A dedicated Transport Binding Model constitution is needed to define how service contracts are exposed — without prescribing transport protocols or runtime implementations.

## What Changes

- Create a constitutional **Transport Binding Model** spec (`specs/transport-binding-model/spec.md`) that defines transport binding semantics, endpoint exposure binding, exposure descriptor binding, transport policy attachment, deterministic transport behavior, fail-closed transport behavior, transport observability semantics, governance enforcement, and cross-spec governance
- Transport binding SHALL govern HOW service contracts are exposed while Service Contract Model governs WHAT interactions mean
- Amend the **Architecture Governance** spec to cross-reference the Transport Binding Model for transport exposure governance across architectural boundaries
- Amend the **Runtime Abstraction** spec to cross-reference the Transport Binding Model for runtime-mediated transport exposure governance

## Capabilities

### New Capabilities
- `transport-binding-model`: Constitutional governance for transport binding across ego-rs. Defines transport binding semantics, endpoint exposure binding, exposure descriptor binding, transport policy attachment, deterministic transport exposure, fail-closed transport behavior, transport observability semantics, governance enforcement, and cross-spec authority ownership.

### Modified Capabilities
- `architecture-governance`: Transport exposure of service contracts across architectural boundaries SHALL be governed by the Transport Binding Model. Architecture Governance remains authoritative for layer boundaries and dependency direction; Transport Binding Model governs exposure binding semantics.
- `runtime-abstraction`: Runtime-mediated transport exposure SHALL be governed by the Transport Binding Model while runtime execution semantics remain governed by Runtime Abstraction.

## Impact

- `openspec/specs/`: New `transport-binding-model/` spec directory with `spec.md`
- `openspec/specs/architecture-governance/spec.md`: Amendment to cross-reference transport binding model
- `openspec/specs/runtime-abstraction/spec.md`: Amendment to cross-reference transport binding governance for runtime-mediated transport exposure
- No runtime code, no transport protocol changes, no serialization changes, no networking code