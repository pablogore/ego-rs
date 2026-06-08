# CORE-007 Gap Analysis

## Gap Sources

| Source | Mechanism | Detection |
|--------|-----------|-----------|
| Event bus overflow | Buffer capacity exceeded, event dropped | Sequence ID gap |
| Channel closure | Sender dropped unexpectedly | Disconnected error |
| Process restart | In-memory state lost | Fresh state, no prior sequence |

## Detection

Gaps are detected by comparing each consumed event's `sequence_id` against `last_sequence_id + 1`:

```
if consumed.sequence_id != last_sequence_id + 1:
    detected_gaps += 1
    log("Gap detected: expected {expected}, got {actual}")
```

## Recovery

CORE-007 does NOT implement gap recovery. Recovery is the responsibility of an external extension that:

1. Observes `detected_gaps > 0`
2. Optionally requests event replay from CORE-006 persistence store
3. Feeds replayed events back through the Scheduler

## Operational Impact

| Gap Severity | Impact | Recommended Action |
|--------------|--------|-------------------|
| Single event | Minor — scheduler has slightly outdated view | No action needed |
| Burst (2-100) | Moderate — activation quality may degrade | Monitor; increase bus capacity if persistent |
| Extended (100+) | Significant — scheduler may make poor suggestions | Increase capacity; review load patterns |
