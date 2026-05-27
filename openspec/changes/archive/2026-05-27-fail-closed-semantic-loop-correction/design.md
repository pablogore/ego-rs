## Context

semantic_loop_detected is emitted as observable but execution continues. This violates fail-closed semantics. The runtime guard must terminate immediately when semantic loops are detected without deterministic progression.

## Goals / Non-Goals

**Goals:**
- Immediate termination on semantic_loop_detected
- Governed threshold for repeated reasoning loops
- Deterministic failure evidence emission
- Replay-visible failure classification

**Non-Goals:**
- Heuristic retries
- Self-reasoning recovery
- Implementation-specific loop detection
- Heuristic retries

## Decisions

### Decision 1: Fail-closed by default
semantic_loop_detected SHALL terminate execution immediately. No continuation after loop detection.

### Decision 2: Governed threshold
Loop detection SHALL use a governed threshold, not heuristics. Threshold SHALL be configurable but deterministic and not based on heuristic behavior.

### Decision 3: State progression invariant
Repeated tool invocations without state progression SHALL trigger immediate loop detection.

### Decision 4: Deterministic failure evidence
Failure evidence SHALL be emitted as observable semantics for replay validation.

### Decision 5: Execution budget enforcement
Execution budget exhaustion SHALL fail closed. When budget is exhausted, termination SHALL occur with deterministic evidence.

## Risks / Trade-offs

- Legitimate repeated valid reasoning MAY be incorrectly terminated - mitigated by governed threshold calibration
- Performance overhead of state tracking - mitigated by memory-only in-memory tracking
- Governed threshold remains governance-owned - ensures that the threshold is controlled and not subject to external influences

- Execution budget exhaustion may terminate legitimate execution - mitigated by configurable budget thresholds

- Conversational replanning after terminal failure - mitigated by preventing any further tool invocations after failure classification

## Verification of Hexagonal Architecture

To verify that the hexagonal architecture is preserved, we will ensure that:
- No external systems or services are introduced.
- All dependencies are internal to the application.
- The architecture remains decoupled and modular.

- Legitimate repeated valid reasoning MAY be incorrectly terminated - mitigated by governed threshold calibration
- Performance overhead of state tracking - mitigated by memory-only in-memory tracking
- Governed threshold remains governance-owned - ensures that the threshold is controlled and not subject to external influences

- Execution budget exhaustion may terminate legitimate execution - mitigated by configurable budget thresholds

- Legitimate repeated valid reasoning MAY be incorrectly terminated - mitigated by governed threshold calibration
- Performance overhead of state tracking - mitigated by memory-only in-memory tracking
- Governed threshold remains governance-owned - ensures that the threshold is controlled and not subject to external influences

- Execution budget exhaustion may terminate legitimate execution - mitigated by configurable budget thresholds
