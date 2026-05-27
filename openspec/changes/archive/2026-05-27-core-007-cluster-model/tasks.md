## 1. Cluster Model Spec

- [ ] 1.1 Define node identity — unique, location-independent identifier
- [ ] 1.2 Define membership states — Joining, Active, Leaving, Removed, Failed
- [ ] 1.3 Define actor placement semantics — which node owns which actor
- [ ] 1.4 Define partition semantics — how network partitions affect membership and placement
- [ ] 1.5 Define fail-closed behavior on partition — ambiguous membership = failed, not assumed active

## 2. Tests

- [ ] 2.1 Test: node joins cluster → transitions to Active
- [ ] 2.2 Test: node leaves cluster → transitions to Removed
- [ ] 2.3 Test: actor placement is deterministic given identical topology
- [ ] 2.4 Test: partition causes fail-closed — ambiguous membership SHALL NOT proceed
- [ ] 2.5 Test: mock runtime used — no real network, no real distribution

## 3. Verification

- [ ] 3.1 Run `cargo test --workspace` — all tests pass
- [ ] 3.2 Run `cargo clippy --workspace -- -D warnings` — no warnings
- [ ] 3.3 Verify cluster model depends on actor model, not vice versa

**Note:** CORE-007 is DEFERRED to post-MVP. Implement after CORE-002 (Actor Primitive), CORE-004 (Persistence SPI), and transport are stable. The cluster model requires actors that can be distributed across nodes.