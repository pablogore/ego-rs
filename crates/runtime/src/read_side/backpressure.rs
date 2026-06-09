//! Backpressure control for read-side projections.

use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

/// Global semaphore for enforcing max_in_flight limits.
pub struct Backpressure {
    semaphore: Arc<Semaphore>,
}

impl Backpressure {
    /// Creates a new backpressure controller with the given limit.
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_in_flight)),
        }
    }

    /// Attempts to acquire a permit for processing a batch.
    /// Returns a permit that must be held until processing is complete.
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, tokio::sync::AcquireError> {
        self.semaphore.clone().acquire_owned().await
    }

    /// Checks if we can process a batch without exceeding limits.
    pub async fn can_process(&self) -> bool {
        self.semaphore.try_acquire().is_ok()
    }
}
