## ADDED Requirements

### Requirement: Behavior Model authority boundary

Participant interaction semantics SHALL remain governed by Interaction Model while execution behavior semantics SHALL be governed by Behavior Model.

The Interaction Model SHALL remain authoritative for:
- interaction semantics (how participants interact),
- request/reply interaction model,
- fire-and-forget interaction model,
- publish/subscribe interaction model,
- interaction expectations,
- interaction observability,
- interaction governance.

The Behavior Model SHALL remain authoritative for:
- how behavior executes within participant boundaries,
- command handling semantics,
- event handling semantics,
- state transition semantics,
- lifecycle semantics,
- read-only behavior semantics,
- failure behavior semantics.

Authority ownership SHALL remain explicit and non-overlapping.

#### Scenario: Interaction semantics evaluated
- **WHEN** participant interaction semantics are evaluated
- **THEN** Interaction Model SHALL govern HOW participants interact

#### Scenario: Behavior execution evaluated
- **WHEN** behavior execution semantics are evaluated within a participant
- **THEN** Behavior Model SHALL govern HOW behavior executes

#### Scenario: Authority overlap detected
- **WHEN** Interaction Model and Behavior Model authority boundaries overlap
- **THEN** the overlap SHALL be treated as a constitutional violation