## ADDED Requirements

### Requirement: ActorSystem

`ActorSystem` SHALL be the runtime entry point for actor execution. It SHALL provide: spawn actors by `ActorId`, deliver messages to actor mailboxes, manage actor lifecycle transitions. `ActorSystem` SHALL be a runtime concern (not domain).

```rust
/// The runtime actor system.
///
/// Owns actor lifecycle, message routing, and mailbox management.
pub struct ActorSystem;

impl ActorSystem {
    /// Spawn an actor and return a sendable handle.
    pub fn spawn<A: Actor>(&self, actor: A) -> ActorRef<A::Message>;

    /// Stop an actor by identity.
    pub fn stop(&self, id: &ActorId);

    /// Get the current lifecycle state of an actor.
    pub fn state(&self, id: &ActorId) -> Option<ActorLifecycleState>;
}
```

#### Scenario: Actor spawned
- **WHEN** `ActorSystem::spawn(my_actor)` is called
- **THEN** the actor SHALL be created, transition Starting→Running, and an `ActorRef` SHALL be returned

#### Scenario: Actor stopped
- **WHEN** `ActorSystem::stop(id)` is called for a Running actor
- **THEN** it SHALL transition Running→Stopping→Stopped; no further messages SHALL be processed

### Requirement: ActorRef

`ActorRef<M>` SHALL be a sendable handle that routes messages to the actor's mailbox. It SHALL implement `Clone + Send` where `M: Send`. The sender SHALL NOT need to know the actor's location or runtime internals.

```rust
pub struct ActorRef<M> { /* ... */ }

impl<M> ActorRef<M> {
    /// Send a message to the actor. Returns error if mailbox is full.
    pub fn send(&self, msg: M) -> Result<(), MailboxFull>;
}
```

#### Scenario: Message sent via ActorRef
- **WHEN** `actor_ref.send(msg)` is called
- **THEN** the message SHALL be enqueued in the actor's mailbox and processed sequentially

#### Scenario: Multiple senders
- **WHEN** two different threads send messages to the same `ActorRef`
- **THEN** both messages SHALL arrive in the mailbox; ordering between senders is undefined, ordering from same sender is preserved

### Requirement: Mailbox

`Mailbox<M>` SHALL provide: bounded capacity (configurable at construction), FIFO ordering for messages from the same sender, non-blocking send with explicit rejection on full.

```rust
pub struct Mailbox<M> { /* ... */ }

impl<M> Mailbox<M> {
    pub fn new(capacity: usize) -> Self;
    pub fn try_send(&self, msg: M) -> Result<(), MailboxFull>;
}
```

#### Scenario: FIFO ordering preserved
- **WHEN** actor A sends M1, M2, M3 in sequence to actor B
- **THEN** B's mailbox SHALL present M1, M2, M3 in that order

#### Scenario: Bounded mailbox rejects on full
- **WHEN** a mailbox at capacity receives a send
- **THEN** it SHALL return `Err(MailboxFull)` — no silent drop, no blocking

### Requirement: Sequential message processing

Each actor SHALL process exactly one message at a time. The runtime SHALL NOT begin processing the next message until the current message handler completes. Intra-actor concurrency SHALL NOT occur.

#### Scenario: Sequential processing enforced
- **WHEN** an actor's mailbox has multiple pending messages
- **THEN** they SHALL be processed one at a time; the second message SHALL NOT begin until the first completes

### Requirement: RuntimeSupervisor

`RuntimeSupervisor` SHALL execute supervision strategies defined by CORE-002. When a child actor fails:
- Restart: transition child Starting→Running
- Stop: transition child to Stopped
- Escalate: propagate to grandparent supervisor

#### Scenario: Supervisor restarts child
- **WHEN** a supervised child fails and strategy is Restart
- **THEN** the child SHALL transition to Starting, then Running

#### Scenario: Failure escalates unhandled
- **WHEN** a parent with strategy Escalate receives a child failure
- **THEN** the failure SHALL propagate to the grandparent supervisor

### Requirement: Communication guarantees

The runtime SHALL enforce CORE-002's communication contract:
- **FIFO:** messages from same sender→same receiver = in order
- **At-most-once:** no duplicate delivery
- **Isolation:** no shared mutable state between actors
- **Message immutability:** messages SHALL NOT be mutated after send

### Requirement: Testing contract

Tests SHALL use mock `ActorSystem` or spawn lightweight in-memory instances. No test SHALL require real networking, persistence, or external services. Coverage SHALL be at least 95%.

#### Scenario: Mock ActorSystem used in test
- **WHEN** a test exercises runtime-dependent code
- **THEN** it SHALL use an in-memory `ActorSystem` and SHALL NOT start external infrastructure