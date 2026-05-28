## Why

Constitutional correctness of ego-rs remains theoretical until validated through executable runtime behavior. FOUNDATION-014 through FOUNDATION-020 define interaction, behavior, projection, persistence, placement, lifecycle, and execution semantics, but without an executable slice, these remain unproven. This change creates a minimal deterministic runtime slice to prove constitutional runtime viability.

## What Changes

- **New Capabilities**: Implement a minimal deterministic runtime slice in `core/runtime-slice/` that validates end-to-end constitutional execution flow
- **New Capabilities**: Create in-memory persistence slice and projection for replay validation
- **New Capabilities**: Add semantic observability events to validate observable semantics
- **New Capabilities**: Implement fail-closed execution patterns for invalid states
- **New Capabilities**: Verify lifecycle neutrality preserved

## Capabilities

### New Capabilities
- `deterministic-runtime-slice`: Minimal single-process, memory-only runtime slice validating constitutional execution flow from interaction through behavior, state transition, persistence, projection, and lifecycle
- `replay-validation`: In-memory replay mechanism to verify deterministic equivalence across executions
- `semantic-observability-runtime`: Semantic observation points capturing execution flow without mutating runtime meaning
- `runtime-slice-example`: One minimal executable constitutional example demonstrating full runtime slice validation

### Modified Capabilities
- None - CORE-001 validates foundations without mutation

## Impact

New implementation area: `core/runtime-slice/` containing the executable proof-of-execution slice. No production runtime, distributed execution, or infrastructure integration.
