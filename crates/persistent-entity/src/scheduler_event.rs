use std::sync::Arc;

use tokio::sync::Notify;

use crate::types::EntityTriple;

#[derive(Debug)]
pub enum SchedulerEvent {
    SlotFreed,
    CommandArrived(EntityTriple),
    CircuitBreakerExpired(EntityTriple),
}

#[derive(Debug)]
pub struct SchedulerTrigger {
    notify: Arc<Notify>,
}

impl SchedulerTrigger {
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn fire(&self) {
        self.notify.notify_one();
    }

    pub fn waiter(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }
}
