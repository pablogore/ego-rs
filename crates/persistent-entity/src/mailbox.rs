//! Bounded FIFO mailbox for command queuing.
//!
//! This module implements a bounded FIFO mailbox for queuing commands to entities.

use crate::error::EntityError;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Set to true once the mailbox has been closed; unblocks any waiting recv().
    closed: Arc<AtomicBool>,
}

impl<T> BoundedMailbox<T> {
    /// Create a new bounded mailbox.
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
            not_full: Arc::new(Notify::new()),
            not_empty: Arc::new(Notify::new()),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Close the mailbox. Any `recv()` calls waiting on an empty queue will
    /// return `Err(EntityError::MailboxClosed)` after the queue drains. Senders
    /// blocked on a full queue are also woken so they can observe the closed flag.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.not_empty.notify_waiters();
        self.not_full.notify_waiters();
    }

    /// Send a command to the mailbox.
    ///
    /// Returns `Err(EntityError::MailboxClosed)` immediately if the mailbox
    /// has already been closed, preventing silent command loss during passivation.
    pub async fn send(&self, command: T) -> Result<(), EntityError> {
        loop {
            // Create Notified before lock to avoid missing a wakeup from a
            // concurrent recv() that fires notify_waiters() between our lock
            // release and the .await below.
            let notified = self.not_full.notified();
            {
                let mut queue = self.queue.lock().await;
                if self.closed.load(Ordering::Acquire) {
                    return Err(EntityError::MailboxClosed);
                }
                if queue.len() < self.capacity {
                    queue.push_back(command);
                    self.not_empty.notify_waiters();
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    /// Receive a command from the mailbox.
    ///
    /// Returns `Err(EntityError::MailboxClosed)` when the mailbox has been
    /// closed and the queue is fully drained.
    pub async fn recv(&self) -> Result<T, EntityError> {
        loop {
            // Create Notified BEFORE acquiring the lock. A concurrent send()
            // that calls notify_waiters() between our lock release and the
            // .await below would otherwise be a lost wakeup.
            let notified = self.not_empty.notified();
            {
                let mut queue = self.queue.lock().await;
                if let Some(command) = queue.pop_front() {
                    self.not_full.notify_waiters();
                    return Ok(command);
                }
                if self.closed.load(Ordering::Acquire) {
                    return Err(EntityError::MailboxClosed);
                }
            }
            notified.await;
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
