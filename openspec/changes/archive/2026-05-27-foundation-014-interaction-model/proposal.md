## Why

Participant interaction semantics across ego-rs are currently implicit and insufficiently governed. The Service Contract Model (FOUNDATION-012) defines WHAT an interaction means, and the Transport Binding Model (FOUNDATION-013) defines HOW an interaction becomes transport-exposed, but no constitutional spec governs HOW participants interact. Without constitutional interaction governance, response expectations become ambiguous, workflow coordination becomes inconsistent, retry expectations leak into behavior, observability loses semantic meaning, and replay trustworthiness degrades. A dedicated Interaction Model constitution is needed to define participant interaction semantics without prescribing transport protocols, runtime implementations, or orchestration engines.

## What Changes

- Create a constitutional **Interaction Model** spec (`specs/interaction-model/spec.md`) that defines interaction semantics, request/reply interactions, fire-and-forget interactions, publish/subscribe interactions, stream interactions, approval interactions, workflow interactions, deterministic interaction behavior, fail-closed interaction behavior, interaction observability semantics, governance enforcement, and cross-spec governance
- Interaction Model SHALL govern HOW participants interact while Service Contract Model governs WHAT interactions mean and Transport Binding Model governs HOW interactions become transport-exposed
- Amend the **Architecture Governance** spec to cross-reference the Interaction Model for participant interaction governance across architectural boundaries
- Amend the **Runtime Abstraction** spec to cross-reference the Interaction Model for runtime-mediated interaction governance

## Capabilities

### New Capabilities
- `interaction-model`: Constitutional governance for participant interaction semantics across ego-rs. Defines interaction semantics, request/reply interactions, fire-and-forget interactions, publish/subscribe interactions, stream interactions, approval interactions, workflow interactions, deterministic interaction behavior, fail-closed interaction behavior, interaction observability semantics, governance enforcement, and cross-spec authority ownership.

### Modified Capabilities
- `architecture-governance`: Participant interaction behavior across architectural boundaries SHALL be governed by the Interaction Model. Architecture Governance remains authoritative for layer boundaries and dependency direction; Interaction Model governs participant interaction semantics.
- `runtime-abstraction`: Runtime-mediated participant interaction SHALL be governed by the Interaction Model while runtime execution semantics remain governed by Runtime Abstraction.

## Impact

- `openspec/specs/`: New `interaction-model/` spec directory with `spec.md`
- `openspec/specs/architecture-governance/spec.md`: Amendment to cross-reference interaction model
- `openspec/specs/runtime-abstraction/spec.md`: Amendment to cross-reference interaction governance for runtime-mediated interactions
- No runtime code, no transport protocol changes, no serialization changes, no networking code, no actor model changes