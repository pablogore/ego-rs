use std::sync::Arc;

use persistent_entity::command_context::CommandContext;
use persistent_entity::error::EntityError;
use persistent_entity::persistent_entity::{CommandResult, PersistentEntity};
use persistent_entity::runtime::EntityRuntime;
use persistent_entity::testing::{TestCommand, TestEvent, TestState};

/// Send `count` concurrent Increment commands to an entity.
/// Spawns one tokio task per command, each creating its own EntityRef.
pub async fn spawn_concurrent_commands(
    count: usize,
    runtime: Arc<EntityRuntime<TestEvent>>,
    entity_type: &'static str,
    entity_id: &'static str,
    handler: Arc<dyn PersistentEntity<TestCommand, TestEvent, TestState>>,
) -> Vec<Result<CommandResult<TestEvent, TestState>, EntityError>> {
    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let ctx = CommandContext::new();
        let command = TestCommand::Increment((i + 1) as u64);
        let h = handler.clone();
        let rt = runtime.clone();

        handles.push(tokio::spawn(async move {
            let entity_ref = rt.entity_ref::<TestCommand, TestState>(entity_type, entity_id, h);
            entity_ref.send(command, ctx, None).await
        }));
    }

    let mut results = Vec::with_capacity(count);
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results
}
