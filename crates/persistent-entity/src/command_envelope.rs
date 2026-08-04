//! Command envelope for wrapping commands with context.
//!
//! This module provides two envelope types:
//! - [`CommandEnvelope`]: the serialisable DTO for wire/storage.
//! - [`ActorEnvelope`]: an internal transport that carries a per-command
//!   oneshot reply channel so the spawned actor task can send the result back
//!   to the caller without going through a shared state store.

use tokio::sync::oneshot;

use serde::{Deserialize, Serialize};

use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::mailbox::CommandErasedResult;

/// A command envelope that wraps a command with context information.
///
/// This is the serialisable DTO passed over the wire or stored in the event
/// log. It does NOT contain the reply channel — that lives in [`ActorEnvelope`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope<C> {
    /// The command to be executed.
    pub command: C,
    /// The command context.
    pub context: CommandContext,
}

/// Internal command transport for the spawned actor task.
///
/// Unlike [`CommandEnvelope`], this struct is **not** serialisable: the
/// `oneshot::Sender` cannot implement `Serialize`. It is created per command
/// by [`TokioEntityRef::send_command`] and consumed by
/// [`EntityActor::execute_command`].
///
/// The actor **must** send on `reply` on every exit path (success, no-events,
/// handler error, persist error). Failing to do so will cause the caller to
/// await a future that never resolves.
pub struct ActorEnvelope<C> {
    /// The wrapped command envelope (command + context).
    pub envelope: CommandEnvelope<C>,
    /// One-shot reply channel. The actor sends the erased result back
    /// through this sender.
    pub reply: oneshot::Sender<Result<CommandErasedResult, EntityError>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_context::CommandContext;
    use std::collections::HashMap;

    #[test]
    fn command_envelope_serde_roundtrip() {
        let ctx = CommandContext {
            tenant_id: Some("t1".into()),
            entity_type: "counter".into(),
            entity_id: "e1".into(),
            expected_version: None,
            causation_id: None,
            metadata: HashMap::new(),
            operation_key: None,
        };
        let env = CommandEnvelope {
            command: 42u32,
            context: ctx,
        };
        let json = serde_json::to_string(&env).expect("serialize");
        let back: CommandEnvelope<u32> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.command, 42);
    }

    #[tokio::test]
    async fn actor_envelope_reply_channel_works() {
        let ctx = CommandContext {
            tenant_id: None,
            entity_type: "x".into(),
            entity_id: "y".into(),
            expected_version: None,
            causation_id: None,
            metadata: HashMap::new(),
            operation_key: None,
        };
        let (tx, rx) = oneshot::channel::<Result<CommandErasedResult, EntityError>>();
        let _env = ActorEnvelope {
            envelope: CommandEnvelope {
                command: "hello",
                context: ctx,
            },
            reply: tx,
        };
        // Dropping the sender without sending should make recv return Err.
        drop(_env);
        assert!(rx.await.is_err(), "dropped sender must close the channel");
    }
}
