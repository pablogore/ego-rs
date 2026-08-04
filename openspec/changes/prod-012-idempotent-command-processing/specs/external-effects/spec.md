# Delta for External Effects

## ADDED Requirements

### Requirement: EffectDedupStore Uses the Injected Clock

`EffectDedupStore` MUST source its notion of current time exclusively from
the runtime-injected `Clock` (shared with `OperationReservationStore`). It
MUST NOT call `Utc::now()` directly anywhere in its dedup/expiry logic.

#### Scenario: Deduplication expiry follows the injected Clock, not wall-clock time
- GIVEN an `EffectDedupStore` wired with a deterministic test `Clock`
- WHEN the test `Clock` is advanced past a dedup entry's expiry
- THEN the store treats that entry as expired based on the injected `Clock`
  value, with no direct `Utc::now()` call observed in the code path
