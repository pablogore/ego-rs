use tokio::sync::{mpsc, oneshot};

use crate::command_context::CommandContext;
use crate::error::EntityError;

pub struct CommandEnvelope<C> {
    pub command: C,
    pub ctx: CommandContext,
    pub response_tx: oneshot::Sender<Result<CommandErasedResult, EntityError>>,
    pub expected_version: Option<u64>,
}

/// Type-erased return value from command execution.
pub type CommandErasedResult = Box<dyn std::any::Any + Send>;

pub struct Mailbox<C> {
    sender: mpsc::Sender<CommandEnvelope<C>>,
    capacity: usize,
}

impl<C> Mailbox<C> {
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<CommandEnvelope<C>>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Mailbox { sender: tx, capacity }, rx)
    }

    pub fn try_send(
        &self,
        envelope: CommandEnvelope<C>,
    ) -> Result<(), TrySendError<C>> {
        self.sender.try_send(envelope).map_err(|e| match e {
            mpsc::error::TrySendError::Full(envelope) => TrySendError::Full(envelope),
            mpsc::error::TrySendError::Closed(envelope) => TrySendError::Closed(envelope),
        })
    }

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    pub fn sender(&self) -> mpsc::Sender<CommandEnvelope<C>> {
        self.sender.clone()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

pub enum TrySendError<C> {
    Full(CommandEnvelope<C>),
    Closed(CommandEnvelope<C>),
}
