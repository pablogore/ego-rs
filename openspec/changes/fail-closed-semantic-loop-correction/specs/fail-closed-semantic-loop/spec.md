## ADDED Requirements

### Requirement: Semantic loop detection terminates execution
The runtime SHALL terminate execution immediately when semantic_loop_detected is observed. No additional tool calls SHALL be permitted after failure classification. Failure evidence SHALL be deterministically emitted.

#### Scenario: Semantic loop detected
- **WHEN** semantic_loop_detected is emitted
- **THEN** execution SHALL terminate immediately and failure evidence SHALL be deterministically emitted

#### Scenario: Heuristic retries are not allowed
- **WHEN** a heuristic retry is attempted
- **THEN** the retry SHALL be rejected and SHALL NOT execute

#### Scenario: Repeated reasoning loop
- **WHEN** identical reasoning repetition occurs beyond governed threshold
- **THEN** execution SHALL terminate immediately

#### Scenario: Governed threshold prevents infinite loops
- **WHEN** identical reasoning repetition occurs beyond governed threshold
- **THEN** execution SHALL terminate immediately and no additional tool calls SHALL be permitted. The threshold SHALL be deterministic and not based on heuristic behavior.

#### Scenario: Repeated tool recursion
- **WHEN** tool invocations repeat without state progression
- **THEN** execution SHALL terminate immediately as loop detected

#### Scenario: Recursive execution without progression
- **WHEN** recursive execution occurs without state progression
- **THEN** execution SHALL terminate immediately and fail closed

#### Scenario: Recursive execution without progression
- **WHEN** recursive execution occurs without state progression
- **THEN** execution SHALL terminate immediately and fail closed

#### Scenario: Execution budget exhausted
- **WHEN** execution budget is exhausted
- **THEN** execution SHALL fail closed with deterministic evidence

#### Scenario: Post-failure tool invocation rejected
- **WHEN** a tool invocation is attempted after failure classification
- **THEN** the invocation SHALL be rejected and SHALL NOT execute

#### Scenario: No conversational replanning after terminal failure
- **WHEN** a conversational replanning attempt is made after terminal failure
- **THEN** the replanning SHALL be rejected and SHALL NOT execute

#### Scenario: Divergent failure classification detected
- **WHEN** equivalent execution conditions emit different classifications
- **THEN** execution SHALL be treated as a constitutional violation

#### Scenario: Replay visibility after terminal failure
- **WHEN** a replay occurs after terminal failure
- **THEN** equivalent failure evidence SHALL remain preserved

#### Scenario: Replay preserves failure classification equivalence
- **WHEN** the same execution conditions are replayed
- **THEN** the failure classification SHALL remain the same

### Requirement: No progress invariant violation handling
Repeated tool invocation without governed state progression SHALL terminate execution. The runtime SHALL detect absence of state progression and fail closed.

#### Scenario: No progress invariant violated
- **WHEN** tool invocations occur without state progression
- **THEN** the loop SHALL be detected and execution SHALL terminate

#### Scenario: Post-failure tool invocation rejected
- **WHEN** a tool invocation is attempted after failure classification
- **THEN** the invocation SHALL be rejected and SHALL NOT execute

#### Scenario: Threshold evaluation remains replay-equivalent
- **WHEN** the same execution conditions are replayed
- **THEN** the threshold evaluation SHALL produce the same result

### Requirement: Execution budget exhaustion
Execution budget exhaustion SHALL fail closed. When budget is exhausted, execution SHALL terminate with deterministic failure evidence.

#### Scenario: Execution budget exhausted
- **WHEN** execution budget is exhausted
- **THEN** execution SHALL fail closed with deterministic evidence

#### Scenario: Budget exhaustion prevention
- **WHEN** budget threshold approaches
- **THEN** loop detection SHALL trigger if no progress occurs

### Requirement: Deterministic failure classification
Equivalent execution conditions SHALL produce equivalent failure classification. Failure classification SHALL remain deterministic, replay-equivalent, and implementation-neutral.

#### Scenario: Equivalent failure classified
- **WHEN** equivalent execution conditions occur
- **THEN** identical failure classification SHALL be emitted

#### Scenario: Divergent classification detected
- **WHEN** equivalent execution conditions emit different classifications
- **THEN** execution SHALL be treated as a constitutional violation

#### Scenario: Replay preserves failure classification
- **WHEN** replay occurs after failure
- **THEN** equivalent failure classification SHALL remain preserved

#### Scenario: Post-failure tool invocation rejected
- **WHEN** a tool invocation is attempted after failure classification
- **THEN** the invocation SHALL be rejected and SHALL NOT execute

### Requirement: Failure evidence is replay-visible
The failure evidence SHALL be observable through semantic channels for replay validation. Replay SHALL preserve failure evidence equivalence.

#### Scenario: Failure evidence visible
- **WHEN** semantic_loop_detected terminates execution
- **THEN** failure evidence SHALL be observable for replay

#### Scenario: Replay preserves failure evidence equivalence
- **WHEN** a replay occurs after terminal failure
- **THEN** equivalent failure evidence SHALL remain preserved

#### Scenario: Replay visibility after terminal failure
- **WHEN** a replay occurs after terminal failure
- **THEN** equivalent failure evidence SHALL remain preserved

### Requirement: No self-reasoning recovery exists
The runtime SHALL not attempt self-reasoning recovery after a failure classification. Self-reasoning recovery SHALL be rejected and SHALL NOT execute.

#### Scenario: Self-reasoning recovery attempted
- **WHEN** self-reasoning recovery is attempted after failure classification
- **THEN** the recovery SHALL be rejected and SHALL NOT execute
