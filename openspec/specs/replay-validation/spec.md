## ADDED Requirements

### Requirement: Replay-safe runtime slice
The runtime slice SHALL preserve replay equivalence. Given identical governed inputs, replay SHALL produce state equivalence, projection equivalence, lifecycle equivalence, and observable equivalence.

#### Scenario: Replay preserves semantics
- **WHEN** replay occurs with identical governed inputs
- **THEN** equivalent observable semantics SHALL be preserved

#### Scenario: Replay divergence detected
- **WHEN** replay produces non-equivalent semantics
- **THEN** validation SHALL fail and execution SHALL be rejected

### Requirement: Deterministic equivalence verification
The runtime slice SHALL provide mechanisms to verify that multiple executions with identical governed inputs produce identical outcomes.

#### Scenario: Equivalent execution validated
- **WHEN** two executions are performed with identical governed inputs
- **THEN** the outcomes SHALL be verifiable as equivalent

#### Scenario: Non-equivalent execution rejected
- **WHEN** executions produce different outcomes with identical governed inputs
- **THEN** the runtime SHALL flag this as a constitutional violation

## ADDED Requirements

### Requirement: Runtime slice example
The runtime slice SHALL include one minimal executable constitutional example.

The example SHALL validate:

```text
command
→ behavior
→ state transition
→ persistence
→ replay
→ projection
→ lifecycle
→ observability
```

The example SHALL remain:

```text
single-process
memory-only
deterministic
replay-safe
fail-closed
```

#### Scenario: Example runtime slice executed
- **WHEN** the minimal runtime example executes
- **THEN** constitutional runtime semantics SHALL be demonstrably exercised

#### Scenario: Replay example executed
- **WHEN** the runtime example is replayed
- **THEN** equivalent observable semantics SHALL remain preserved