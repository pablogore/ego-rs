//! Minimal domain contract for actors.
//!
//! Defines what an actor IS, not how it executes. Runtime mechanics
//! (mailbox, dispatch, sequential processing, supervision execution)
//! are owned by CORE-003. This module owns only the semantic contract.
//!
//! ## Owned by Domain (CORE-002)
//!
//! - [`Actor`] trait — `type Message` only
//! - [`ActorId`] — location-transparent identity
//! - [`actor_id!`] — compile-time deterministic identity
//! - [`ActorLifecycleState`] — semantic states, no execution logic
//! - [`SupervisionStrategy`] — semantic strategy, no execution
//!
//! ## Owned by Runtime (CORE-003)
//!
//! - `ActorSystem` — spawn, stop, state
//! - `ActorRef` — sendable handle
//! - Mailbox — bounded FIFO queue
//! - Sequential processing — one message at a time
//! - Supervision execution — restart/stop/escalate
//!
//! ## Determinism Axiom
//!
//! Given identical actor state, message sequence, logical time, and
//! context, the observable outcome MUST be identical. This contract
//! defines invariance; the runtime enforces it.
//!
//! ## Fail-closed
//!
//! Invalid actor identity (empty name), invalid state transitions,
//! and ambiguous supervision decisions SHALL be rejected, not silently
//! accepted.

/// Minimal contract for an actor.
///
/// An actor is defined solely by the message type it accepts.
/// How the actor processes messages, produces outputs, or manages
/// lifecycle is a runtime concern (CORE-003).
///
/// # Deterministic
///
/// The trait itself specifies no runtime behavior — all execution
/// semantics are delegated to the runtime adapter. This preserves
/// determinism by keeping the domain contract free of execution
/// assumptions.
///
/// # Example
///
/// ```rust
/// use ego_domain::actor::Actor;
///
/// struct MyActor;
///
/// impl Actor for MyActor {
///     type Message = String;
/// }
/// ```
pub trait Actor {
    /// The message type this actor accepts and processes.
    type Message;
}

/// A unique, location-transparent actor identifier.
///
/// Encodes nothing about network location, process, thread, or
/// deployment topology. Resolution to a physical address is a
/// runtime concern (CORE-003).
///
/// # Deterministic
///
/// Construction is fallible — empty names are rejected with
/// [`ActorIdError::Empty`]. Once constructed, the identity is
/// immutable and comparable by value.
///
/// # Fail-closed
///
/// `ActorId::new("")` returns `Err(ActorIdError::Empty)`.
/// An actor with no name is invalid and MUST NOT be created.
///
/// # Example
///
/// ```rust
/// use ego_domain::actor::ActorId;
///
/// let id = ActorId::new("my_actor").unwrap();
/// assert_eq!(id.as_str(), "my_actor");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// A unique, location-transparent actor identifier.
///
/// Encodes nothing about network location, process, thread, or
/// deployment topology. Resolution to a physical address is a
/// runtime concern (CORE-003).
///
/// # Deterministic
///
/// Construction is fallible — empty names are rejected with
/// [`ActorIdError::Empty`]. Once constructed, the identity is
/// immutable and comparable by value.
///
/// # Fail-closed
///
/// `ActorId::new("")` returns `Err(ActorIdError::Empty)`.
/// An actor with no name is invalid and MUST NOT be created.
pub struct ActorId(String);

impl ActorId {
    /// Create a new `ActorId` with the given name.
    ///
    /// Returns `Err(ActorIdError::Empty)` if the name is empty
    /// or contains only whitespace.
    pub fn new(name: impl Into<String>) -> Result<Self, ActorIdError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ActorIdError::Empty);
        }
        Ok(Self(name))
    }

    /// Return the actor name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Errors that can occur when constructing an [`ActorId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorIdError {
    /// The provided actor name was empty or whitespace-only.
    Empty,
}

impl std::fmt::Display for ActorIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "actor id must not be empty"),
        }
    }
}

impl std::error::Error for ActorIdError {}

/// Compile-time deterministic actor identity.
///
/// Produces a `&'static ActorId` at compile time. This ensures
/// actor identities are deterministic — no runtime construction,
/// no dynamic identity, no ambiguity.
///
/// # Deterministic
///
/// The same `actor_id!(name)` expression always produces the same
/// `&'static ActorId` value. This is essential for replay and
/// deterministic testing.
///
/// # Example
///
/// ```rust
/// use ego_domain::actor::ActorId;
/// use ego_domain::actor_id;
///
/// let id: &'static ActorId = actor_id!(my_actor);
/// assert_eq!(id.as_str(), "my_actor");
/// ```
/// Compile-time deterministic actor identity.
///
/// Produces a `&'static ActorId` at compile time. This ensures
/// actor identities are deterministic — no runtime construction,
/// no dynamic identity, no ambiguity.
///
/// # Deterministic
///
/// The same `actor_id!(name)` expression always produces the same
/// `&'static ActorId` value. This is essential for replay and
/// deterministic testing.
#[macro_export]
macro_rules! actor_id {
    ($name:ident) => {
        {
            static ID: ::std::sync::LazyLock<$crate::actor::ActorId> = ::std::sync::LazyLock::new(|| {
                $crate::actor::ActorId::new(::std::stringify!($name))
                    .expect("actor_id! macro: invalid name")
            });
            &*ID
        }
    };
}

/// Semantic lifecycle states for an actor.
///
/// These are **semantic only** — they define what states exist,
/// not how transitions happen. Transition execution is owned by
/// the runtime (CORE-003).
///
/// # Deterministic
///
/// Terminal states (Stopped, Failed) are immutable by definition.
/// The runtime MUST NOT transition from a terminal state.
///
/// # Fail-closed
///
/// An actor in Stopped or Failed state MUST NOT accept or process
/// messages. The runtime SHALL reject any attempt to transition
/// from a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorLifecycleState {
    /// Actor identity registered but not yet starting.
    Created,
    /// Actor initializing (transition to Running expected).
    Starting,
    /// Actor is active and processing messages.
    Running,
    /// Actor is shutting down gracefully.
    Stopping,
    /// Actor has stopped. Terminal state — no further transitions.
    Stopped,
    /// Actor has failed. Terminal state — no further transitions.
    Failed,
}

impl ActorLifecycleState {
    /// Returns `true` if this is a terminal state (Stopped or Failed).
    ///
    /// # Example
    ///
    /// ```rust
    /// use ego_domain::actor::ActorLifecycleState;
    ///
    /// assert!(ActorLifecycleState::Stopped.is_terminal());
    /// assert!(ActorLifecycleState::Failed.is_terminal());
    /// assert!(!ActorLifecycleState::Running.is_terminal());
    /// ```
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

/// Supervision strategy for handling child actor failures.
///
/// Semantic only — execution (failure detection, strategy application)
/// is owned by the runtime (CORE-003).
///
/// # Deterministic
///
/// The strategy choice is deterministic and made by the parent actor.
/// The runtime executes the chosen strategy without ambiguity.
///
/// # Example
///
/// ```rust
/// use ego_domain::actor::SupervisionStrategy;
///
/// match SupervisionStrategy::Restart {
///     SupervisionStrategy::Restart => println!("restarting child"),
///     SupervisionStrategy::Stop => println!("stopping child"),
///     SupervisionStrategy::Escalate => println!("escalating failure"),
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionStrategy {
    /// Restart the failed child actor.
    Restart,
    /// Stop the failed child actor permanently.
    Stop,
    /// Escalate the failure to the parent supervisor.
    Escalate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_id_valid() {
        let id = ActorId::new("worker").unwrap();
        assert_eq!(id.as_str(), "worker");
    }

    #[test]
    fn actor_id_empty_rejected() {
        assert_eq!(ActorId::new(""), Err(ActorIdError::Empty));
    }

    #[test]
    fn actor_id_whitespace_rejected() {
        assert_eq!(ActorId::new("   "), Err(ActorIdError::Empty));
    }

    #[test]
    fn actor_id_equality() {
        let a = ActorId::new("alpha").unwrap();
        let b = ActorId::new("alpha").unwrap();
        let c = ActorId::new("beta").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn actor_id_macro_produces_static() {
        let id: &'static ActorId = actor_id!(my_test_actor);
        assert_eq!(id.as_str(), "my_test_actor");
    }

    #[test]
    fn actor_id_macro_repeatable() {
        let a: &'static ActorId = actor_id!(repeat_actor);
        let b: &'static ActorId = actor_id!(repeat_actor);
        assert_eq!(a, b);
    }

    #[test]
    fn actor_id_macro_value() {
        let id: &'static ActorId = actor_id!(my_actor);
        assert_eq!(id.as_str(), "my_actor");
    }

    #[test]
    fn lifecycle_created_not_terminal() {
        assert!(!ActorLifecycleState::Created.is_terminal());
    }

    #[test]
    fn lifecycle_running_not_terminal() {
        assert!(!ActorLifecycleState::Running.is_terminal());
    }

    #[test]
    fn lifecycle_stopped_is_terminal() {
        assert!(ActorLifecycleState::Stopped.is_terminal());
    }

    #[test]
    fn lifecycle_failed_is_terminal() {
        assert!(ActorLifecycleState::Failed.is_terminal());
    }

    #[test]
    fn supervision_strategies_independent() {
        let strategies = [
            SupervisionStrategy::Restart,
            SupervisionStrategy::Stop,
            SupervisionStrategy::Escalate,
        ];
        for i in 0..strategies.len() {
            for j in (i + 1)..strategies.len() {
                assert_ne!(strategies[i], strategies[j]);
            }
        }
    }

    #[test]
    fn debug_format() {
        let id = ActorId::new("debug_test").unwrap();
        let debug = format!("{:?}", id);
        assert!(debug.contains("debug_test"));
    }

    #[test]
    fn clone_equality() {
        let original = ActorId::new("clone_me").unwrap();
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn actor_id_equality_same_name() {
        let id1 = ActorId::new("same_name").unwrap();
        let id2 = ActorId::new("same_name").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn actor_id_equality_different_name() {
        let id1 = ActorId::new("name1").unwrap();
        let id2 = ActorId::new("name2").unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn lifecycle_terminal_states_distinct_from_running() {
        assert_ne!(ActorLifecycleState::Stopped, ActorLifecycleState::Running);
        assert_ne!(ActorLifecycleState::Failed, ActorLifecycleState::Running);
    }

}
