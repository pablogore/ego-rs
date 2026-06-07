use async_trait::async_trait;

use crate::command_context::CommandContext;

#[derive(Debug, Clone)]
pub enum CommandResult<E, S> {
    Events {
        events: Vec<E>,
        new_state: S,
        new_version: u64,
    },
    NoEvents {
        state: S,
    },
}

#[async_trait]
pub trait PersistentEntity<C: Send + 'static, E: Send + 'static, S: Send + 'static>:
    Send + Sync
{
    async fn handle_command(
        &self,
        state: &S,
        command: C,
        ctx: CommandContext,
    ) -> Result<Vec<E>, String>;

    async fn apply_event(&self, state: &S, event: E) -> S;

    fn initial_state(&self) -> S;
}
