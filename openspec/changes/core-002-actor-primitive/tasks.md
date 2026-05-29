## 1. Domain Types (crates/domain/)

- [x] 1.1 Define `Actor` trait — minimal: `type Message;` — no `fn receive`, no output semantics
- [x] 1.2 Define `ActorId` struct — newtype over String, constructor with non-empty validation
- [x] 1.3 Implement `actor_id!` macro — compile-time deterministic identity, returns `&'static ActorId`
- [x] 1.4 Define semantic `ActorLifecycleState` enum — Created, Starting, Running, Stopping, Stopped, Failed
- [x] 1.5 Define `SupervisionStrategy` enum — Restart, Stop, Escalate
- [x] 1.6 Wire into `crates/domain/src/lib.rs` — `pub mod actor;`
- [x] 1.7 Add Rust docs (`///`) to all public items: `Actor`, `ActorId`, `actor_id!`, `ActorLifecycleState`, `SupervisionStrategy`

## 2. Domain Tests

- [x] 2.1 Test: `ActorId::new("valid")` succeeds, `ActorId::new("")` fails
- [x] 2.2 Test: `actor_id!(my_actor)` produces `&'static ActorId` with value `"my_actor"`
- [ ] 2.3 Test: `ActorId` equality — same name = same id, different name = different id
- [ ] 2.4 Test: `ActorLifecycleState` terminal states (Stopped, Failed) are distinct from Running
- [ ] 2.5 Test: `SupervisionStrategy` enum values are independent

## 3. Verification

- [ ] 3.1 Run `cargo test -p ego-domain` — all domain tests pass
- [ ] 3.2 Run `cargo clippy -p ego-domain -- -D warnings` — no warnings
- [ ] 3.3 Verify `Actor` trait has exactly `type Message;` — no `fn receive`, no output, no effects
- [ ] 3.4 Verify domain crate has no runtime dependencies (no tokio, no scheduler, no dispatch)
- [ ] 3.5 Verify all public items have Rust docs
