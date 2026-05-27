## Why

The runtime guard currently emits `semantic_loop_detected` as observable but does not terminate execution. This violates the fail-closed principle. Without immediate termination, repeated reasoning loops can continue without state progression, leading to infinite recursion. This change implements fail-closed semantics for semantic loop detection.

## What Changes

- **New Capabilities**: Implement governed threshold for repeated reasoning loops. Ensure the threshold is deterministic and not based on heuristic behavior.
- **New Capabilities**: Add deterministic failure emission on loop detection
- **New Capabilities**: Ensure immediate termination on repeated tool recursion
- **New Capabilities**: Prevent additional tool calls after failure classification
- **New Capabilities**: Ensure failure evidence is replay-visible

## Capabilities

### New Capabilities
- `fail-closed-semantic-loop`: Immediate termination semantics for semantic loop detection with deterministic failure evidence
- `loop-governance`: Governed threshold mechanism preventing repeated tool invocation without state progression

### Modified Capabilities
- None

## Impact

Runtime execution guardrail. Adds fail-closed semantics to prevent infinite reasoning loops. All changes apply to runtime governance layer.
