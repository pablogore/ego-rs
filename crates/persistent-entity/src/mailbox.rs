//! Bounded FIFO mailbox for command queuing.
//!
//! This module implements a bounded FIFO mailbox for queuing commands to entities.

use crate::error::EntityError;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

/// Type alias for erased command results.
pub type CommandErasedResult = Box<dyn Send>;

/// A bounded FIFO mailbox for commands.
#[derive(Debug, Clone)]
pub struct BoundedMailbox<T> {
    /// The underlying queue.
    queue: Arc<Mutex<VecDeque<T>>>,
    /// The maximum capacity of the mailbox.
    capacity: usize,
    /// A notification for when the mailbox is not full.
    not_full: Arc<Notify>,
    /// A notification for when the mailbox is not empty.
    not_empty: Arc<Notify>,
}

impl<T> BoundedMailbox<T> {
    /// Create a new bounded mailbox.
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            not_full: Arc::new(Notify::new()),
            not_empty: Arc::new(Notify::new()),
        }
    }

    /// Send a command to the mailbox.
    pub async fn send(&self, command: T) -> Result<(), EntityError> {
        loop {
            {
                let mut queue = self.queue.lock().await;
                if queue.len() < self.capacity {
                    queue.push_back(command);
                    self.not_empty.notify_waiters();
                    return Ok(());
                }
            }
            // Wait until the mailbox is not full
            self.not_full.notified().await;
        }
    }

    /// Receive a command from the mailbox.
    pub async fn recv(&self) -> Result<T, EntityError> {
        loop {
            {
                let mut queue = self.queue.lock().await;
                if let Some(command) = queue.pop_front() {
                    self.not_full.notify_waiters();
                    return Ok(command);
                }
            }
            // Wait until the mailbox is not empty
            self.not_empty.notified().await;
        }
    }

    /// Check if the mailbox is empty.
    pub async fn is_empty(&self) -> bool {
        self.queue.lock().await.is_empty()
    }

    /// Check if the mailbox is full.
    pub async fn is_full(&self) -> bool {
        self.queue.lock().await.len() >= self.capacity
    }

    /// Get the current size of the mailbox.
    pub async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }
}
