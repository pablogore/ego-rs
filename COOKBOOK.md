# ego-rs Cookbook

Practical recipes for using the ego-rs framework.

---

## Getting Started

### Project structure

```text
my-service/
├── Cargo.toml          # depends on ego-domain, ego-application
├── src/
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── commands.rs   # Command types
│   │   ├── events.rs     # DomainEvent types
│   │   └── queries.rs    # Query types
│   ├── application/
│   │   ├── mod.rs
│   │   ├── handlers.rs   # CommandHandler + QueryHandler impls
│   │   └── ports.rs      # External port traits
│   └── main.rs
```

### Cargo.toml

```toml
[dependencies]
ego-domain = { path = "../../crates/domain" }
ego-application = { path = "../../crates/application" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

---

## Recipe 1: Define a Command

Commands carry the data needed to mutate state. They implement the `Command` marker trait.

```rust
use ego_domain::Command;

#[derive(Debug)]
pub struct CreateUser {
    pub user_id: String,
    pub name: String,
    pub email: String,
}

impl Command for CreateUser {}
```

---

## Recipe 2: Define a Domain Event

Events record facts that happened. They implement `DomainEvent`.

```rust
use chrono::Utc;
use ego_domain::DomainEvent;
use serde_json::json;

pub struct UserCreated {
    pub user_id: String,
    pub name: String,
    pub email: String,
    pub occurred_at: chrono::DateTime<Utc>,
}

impl DomainEvent for UserCreated {
    fn aggregate_id(&self) -> &str {
        &self.user_id
    }

    fn event_type(&self) -> &str {
        "UserCreated"
    }

    fn payload(&self) -> &serde_json::Value {
        // For simplicity, construct JSON inline.
        // In production, derive Serialize and use serde_json::to_value.
        unimplemented!("store payload as serde_json::Value")
    }

    fn occurred_at(&self) -> &chrono::DateTime<Utc> {
        &self.occurred_at
    }
}
```

---

## Recipe 3: Define a Query

Queries request data without mutation.

```rust
use ego_domain::Query;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct UserProfile {
    pub name: String,
    pub email: String,
}

pub struct GetUser {
    pub user_id: String,
}

impl Query for GetUser {
    type Output = UserProfile;
}
```

---

## Recipe 4: Implement a Command Handler

Handlers implement `CommandHandler<C>` from the application layer.

```rust
use ego_application::ports::CommandHandler;

pub struct CreateUserHandler;

impl CommandHandler<CreateUser> for CreateUserHandler {
    type Error = String;

    fn handle(&self, cmd: &CreateUser) -> Result<(), Self::Error> {
        // 1. Validate
        if cmd.name.is_empty() {
            return Err("name is required".into());
        }

        // 2. Build event
        let event = UserCreated {
            user_id: cmd.user_id.clone(),
            name: cmd.name.clone(),
            email: cmd.email.clone(),
            occurred_at: Utc::now(),
        };

        // 3. Persist (via port — injected dependency)
        // 4. Publish event

        Ok(())
    }
}
```

---

## Recipe 5: Implement a Query Handler

```rust
use ego_application::ports::QueryHandler;

pub struct GetUserHandler;

impl QueryHandler<GetUser> for GetUserHandler {
    type Error = String;

    fn handle(&self, query: &GetUser) -> Result<UserProfile, Self::Error> {
        // Fetch from read model / projection
        Ok(UserProfile {
            name: "Alice".into(),
            email: "alice@example.com".into(),
        })
    }
}
```

---

## Recipe 6: Define an Actor (CORE-002)

```rust
use ego_domain::actor::{Actor, ActorId, actor_id};

// Actor identity — compile-time, deterministic
let user_actor: &'static ActorId = actor_id!(user_manager);

enum UserMessage {
    CreateUser { name: String, email: String },
    DeleteUser { user_id: String },
}

pub struct UserManager {
    users: Vec<User>,
}

impl Actor for UserManager {
    type Message = UserMessage;
}
```

The `actor_id!` macro produces a deterministic `&'static ActorId` at compile time. No runtime construction, no dynamic identity.

---

## Recipe 7: Spawn and Send Messages (CORE-003 — coming)

> Note: CORE-003 (Runtime Actor Execution) is **Pending**. The `ego-runtime` crate
> and its types shown below are aspirational — the final API may differ.

```rust
use ego_runtime::{ActorSystem, ActorRef};

let system = ActorSystem::new();

// Spawn the actor
let actor_ref: ActorRef<UserMessage> = system.spawn(UserManager::default());

// Send a message
actor_ref.send(UserMessage::CreateUser {
    name: "Bob".into(),
    email: "bob@example.com".into(),
}).expect("mailbox full");

// Stop the actor
system.stop(actor_id!(user_manager));
```

---

## Recipe 8: Supervision (CORE-003 — coming)

> Note: CORE-003 (Runtime Actor Execution) is **Pending**. The API below is
> aspirational and may change when implemented.

```rust
use ego_runtime::supervisor::{RuntimeSupervisor, SupervisionStrategy};

// Spawn child with restart-on-failure
let supervisor = RuntimeSupervisor::new();
let child_ref = supervisor.spawn_child(
    fragile_actor,
    SupervisionStrategy::Restart,
);

// If fragile_actor fails, it auto-restarts
// If it fails too many times, supervisor escalates
```

---

## Recipe 9: Test with Mocks

The framework mandates mock-only testing. No real infrastructure in tests.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_user_rejects_empty_name() {
        let handler = CreateUserHandler;
        let cmd = CreateUser {
            user_id: "1".into(),
            name: "".into(),
            email: "a@b.com".into(),
        };
        let result = handler.handle(&cmd);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "name is required");
    }
}
```

---

## Recipe 10: Hexagonal Testing Pattern

When your handler depends on an external port (e.g., `EventStore`), inject a mock:

```rust
pub trait UserRepository: Send + Sync {
    fn save(&self, user: &User) -> Result<(), RepositoryError>;
}

pub struct CreateUserHandler<R: UserRepository> {
    repo: R,
}

impl<R: UserRepository> CommandHandler<CreateUser> for CreateUserHandler<R> {
    type Error = String;
    fn handle(&self, cmd: &CreateUser) -> Result<(), Self::Error> {
        let user = User { id: cmd.user_id.clone(), name: cmd.name.clone() };
        self.repo.save(&user).map_err(|e| e.to_string())
    }
}

// In tests:
#[cfg(test)]
mod tests {
    struct MockRepo { saved: std::sync::Mutex<Vec<User>> }
    // ...implement UserRepository...
}
```

---

## Conventions

| Convention | Rule |
|-----------|------|
| Error types | Always explicit — no `anyhow`, no `Box<dyn Error>` |
| Determinism | No `rand`, no `SystemTime::now()`, no `HashMap` iteration |
| Serialization | `serde` + `serde_json` for contracts |
| Coverage | >= 95%, enforced in CI |
| Tests | Mock ports only — no database, no network, no filesystem |
| Docs | `///` on every public item — `#![deny(missing_docs)]` recommended |

---

## Next Steps

1. Read [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the full design
2. Explore [`openspec/specs/`](./openspec/specs/) for constitutional rules
3. See [`openspec/changes/`](./openspec/changes/) for active development specs
4. Run `cargo test --workspace` to verify everything works