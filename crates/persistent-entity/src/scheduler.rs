use std::collections::VecDeque;
use tokio::sync::{Semaphore, Mutex};

pub struct Scheduler {
    semaphore: Semaphore,
    pending_queue: Mutex<VecDeque<EntityTriple>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct EntityTriple {
    pub tenant_id: String,
    pub entity_type: String,
    pub entity_id: String,
}

impl EntityTriple {
    pub fn new(tenant_id: impl Into<String>, entity_type: impl Into<String>, entity_id: impl Into<String>) -> Self {
        EntityTriple {
            tenant_id: tenant_id.into(),
            entity_type: entity_type.into(),
            entity_id: entity_id.into(),
        }
    }

    pub fn aggregate_id(&self) -> String {
        format!("{}:{}", self.entity_type, self.entity_id)
    }
}

impl Scheduler {
    pub fn new(max_concurrency: usize) -> Self {
        Scheduler {
            semaphore: Semaphore::new(max_concurrency),
            pending_queue: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn acquire(&self, entity: EntityTriple) -> SchedulerPermit<'_> {
        let permit = self.semaphore.acquire().await.unwrap();
        SchedulerPermit {
            _permit: permit,
            entity: Some(entity),
        }
    }

    pub async fn enqueue(&self, entity: EntityTriple) {
        let mut queue = self.pending_queue.lock().await;
        queue.push_back(entity);
    }

    pub async fn dequeue(&self) -> Option<EntityTriple> {
        let mut queue = self.pending_queue.lock().await;
        queue.pop_front()
    }

    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

pub struct SchedulerPermit<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
    entity: Option<EntityTriple>,
}

impl SchedulerPermit<'_> {
    pub fn entity(&self) -> Option<&EntityTriple> {
        self.entity.as_ref()
    }
}
