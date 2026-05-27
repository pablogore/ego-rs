## ADDED Requirements

### Requirement: Governed threshold authority
The governed threshold SHALL remain explicitly governance-owned. The threshold SHALL remain deterministic, implementation-neutral, externally governed, and replay-equivalent.

#### Scenario: Threshold governance evaluated
- **WHEN** governed threshold authority is evaluated
- **THEN** the threshold SHALL remain explicit and governance-owned

#### Scenario: Heuristic threshold rejected
- **WHEN** threshold depends on probabilistic or heuristic runtime behavior
- **THEN** execution SHALL be treated as non-conformant

#### Scenario: Replay threshold equivalence
- **WHEN** equivalent replay occurs
- **THEN** equivalent threshold evaluation SHALL be preserved

## ADDED Requirements

### Requirement: Recursive tool cycle prevention
The runtime SHALL detect recursive tool invocation cycles without governed progression.

Recursive tool cycles SHALL fail closed. Recursive execution SHALL terminate immediately when cyclic tool dependency occurs without governed progression.

#### Scenario: Recursive tool cycle detected
- **WHEN** tool invocation enters a repeated cyclic dependency graph
- **THEN** execution SHALL terminate immediately

#### Scenario: Recursive cycle without progression
- **WHEN** cyclic tool execution occurs without governed progression
- **THEN** execution SHALL fail closed

#### Scenario: Replay preserves recursive failure
- **WHEN** replay occurs after recursive tool termination
- **THEN** equivalent recursive failure evidence SHALL remain preserved