## 1. Crate Setup

- [ ] 1.1 Create `crates/runtime/` crate — `ego-runtime`, workspace member
- [ ] 1.2 Add dependency on `ego-domain` (for `Actor`, `ActorId`, `ActorLifecycleState`, `SupervisionStrategy`)
- [ ] 1.3 Add `tokio` dependency (runtime feature, not leaked into domain)
- [ ] 1.4 Wire into workspace `Cargo.toml` members and `layers.toml`

## 2. Core Types

- [ ] 2.1 Implement `ActorSystem` — `spawn`, `stop`, `state` methods
- [ ] 2.2 Implement `ActorRef<M>` — sendable handle, `Clone + Send`, `send(msg) → Result<(), MailboxFull>`
- [ ] 2.3 Implement `Mailbox<M>` — `new(capacity)`, `try_send()`, bounded, FIFO
- [ ] 2.4 Implement `MailboxFull` error type

## 3. Runtime Execution

- [ ] 3.1 Implement sequential processing — dequeue one message, process to completion, dequeue next
- [ ] 3.2 Enforce FIFO ordering within same sender→receiver pair
- [ ] 3.3 At-most-once delivery guarantee — no message duplication
- [ ] 3.4 Message isolation — no shared mutable state between actors

## 4. Runtime Lifecycle

- [ ] 4.1 Implement lifecycle state machine — enforce valid transitions defined by CORE-002
- [ ] 4.2 `spawn` → Created→Starting→Running
- [ ] 4.3 `stop` → Running→Stopping→Stopped
- [ ] 4.4 Terminal states (Stopped, Failed) reject messages and further transitions

## 5. Runtime Supervision

- [ ] 5.1 Implement `RuntimeSupervisor` — parent-child tree
- [ ] 5.2 Spawn child with supervision strategy (Restart, Stop, Escalate)
- [ ] 5.3 Child failure → parent notified → strategy executed
- [ ] 5.4 Escalate unhandled failures to grandparent
- [ ] 5.5 Root supervisor (no parent) terminates the actor on unhandled failure

## 6. Tests

- [ ] 6.1 Test: spawn actor → ActorRef.send → message processed → actor state remains Running
- [ ] 6.2 Test: ActorSystem.stop → actor transitions Stopped → send to stopped actor returns error
- [ ] 6.3 Test: mailbox FIFO — 3 messages sent in order → processed in order
- [ ] 6.4 Test: bounded mailbox rejects 4th message when capacity is 3
- [ ] 6.5 Test: sequential processing — long-running handler blocks next message
- [ ] 6.6 Test: two concurrent actors process independently, no shared state
- [ ] 6.7 Test: supervisor restarts failing child → child back to Running
- [ ] 6.8 Test: supervisor with Escalate propagates failure to grandparent
- [ ] 6.9 Test: mock ActorSystem used — no real network, no persistence, no database

## 7. Verification

- [ ] 7.1 Run `cargo test -p ego-runtime` — all tests pass
- [ ] 7.2 Run `cargo clippy -p ego-runtime -- -D warnings` — no warnings
- [ ] 7.3 Verify `ego-domain` has no runtime dependencies (no tokio, no mailbox)
- [ ] 7.4 Verify `ego-runtime` depends on `ego-domain` (unidirectional — domain→runtime)
- [ ] 7.5 Verify all public items have Rust docs (`///`)