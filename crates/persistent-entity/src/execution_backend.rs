use std::fmt::Debug;

use serde::de::DeserializeOwned;

use crate::command_context::CommandContext;
use crate::error::EntityError;
use crate::persistent_entity::PersistentEntity;

pub trait ExecutionBackend: Debug + Send + Sync {
    fn execute<C, E, S>(
        &self,
        entity: &dyn PersistentEntity<Command = C, Event = E, State = S>,
        state: &S,
        command: &C,
        context: &CommandContext,
    ) -> Result<(Vec<E>, S), EntityError>
    where
        C: Send + Sync + serde::Serialize + 'static,
        E: Send + Sync + Clone + serde::Serialize + 'static,
        S: Clone + Send + Sync + serde::Serialize + DeserializeOwned + 'static;
}
