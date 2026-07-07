//! Bounded FIFO mailbox for command queuing.
//!
//! This module implements a bounded FIFO mailbox for queuing commands to entities.

use crate::error::EntityError;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// Type alias for erased command results.
///
/// Boxed as `Box<dyn Any + Send>` so that callers can downcast back to the
/// concrete [`CommandResult<E, S>`](crate::persistent_entity::CommandResult)
/// they expect. The `Any` bound is required for `downcast`.
pub type CommandErasedResult = Box<dyn std::any::Any + Send>;

/// A bounded FIFO mailbox for commands.
///
/// `Clone` is implemented manually so that the clone bound does NOT propagate
/// to `T`.  All inner fields are `Arc`-backed, so cloning merely increments
/// reference counts and both halves refer to the same underlying queue.
#[derive(Debug)]
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

impl<T> Clone for BoundedMailbox<T> {
    /// Clones the mailbox, sharing all underlying `Arc`-backed state.
    ///
    /// Both the original and the clone refer to the same queue, so a send on
    /// either is visible to a recv on the other.  No `T: Clone` bound is
    /// required because `T` is never stored by value inside `Clone`.
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
            capacity: self.capacity,
            not_full: Arc::clone(&self.not_full),
            not_empty: Arc::clone(&self.not_empty),
            closed: Arc::clone(&self.closed),
        }
    }
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

    /// Mark the mailbox closed without notifying anyone. Shared by `close()`
    /// and `close_and_drain()`, which each need different notify timing.
    fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Close the mailbox. Any `recv()` calls waiting on an empty queue will
    /// return `Err(EntityError::MailboxClosed)` after the queue drains. Senders
    /// blocked on a full queue are also woken so they can observe the closed flag.
    pub fn close(&self) {
        self.mark_closed();
        self.not_empty.notify_waiters();
        self.not_full.notify_waiters();
    }

    /// Synchronously close the mailbox and drain every currently-queued
    /// command, handing them back to the caller.
    ///
    /// Unlike [`close`](Self::close), this also empties the queue in the same
    /// call — callable from a synchronous `Drop` (e.g. the Phase 3 teardown
    /// guard), which cannot `.await` a lock. The `parking_lot::Mutex` backing
    /// the queue never poisons, so this is safe to call during panic unwind.
    /// The caller is responsible for terminally answering each returned
    /// item's reply channel — this method only closes and drains.
    ///
    /// Drains before notifying waiters: a woken `send()`/`recv()` re-checks
    /// the queue under its own lock acquisition, so notifying first would
    /// open an avoidable contention window against this method's own drain.
    pub fn close_and_drain(&self) -> VecDeque<T> {
        self.mark_closed();
        let drained = std::mem::take(&mut *self.queue.lock());
        self.not_empty.notify_waiters();
        self.not_full.notify_waiters();
        drained
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
                let mut queue = self.queue.lock();
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
                let mut queue = self.queue.lock();
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
    pub fn is_empty(&self) -> bool {
        self.queue.lock().is_empty()
    }

    /// Check if the mailbox is full.
    pub fn is_full(&self) -> bool {
        self.queue.lock().len() >= self.capacity
    }

    /// Get the current size of the mailbox.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    /// Minimal stand-in for `ActorEnvelope<C>` — just enough to prove
    /// `close_and_drain()` hands back every queued item so a caller (the
    /// Phase 3 teardown guard) can reply on each one.
    struct TestEnvelope {
        reply: oneshot::Sender<Result<(), EntityError>>,
    }

    #[tokio::test]
    async fn close_and_drain_returns_every_queued_envelope_for_replying() {
        let mailbox: BoundedMailbox<TestEnvelope> = BoundedMailbox::new(4);
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        mailbox
            .send(TestEnvelope { reply: tx1 })
            .await
            .expect("send 1");
        mailbox
            .send(TestEnvelope { reply: tx2 })
            .await
            .expect("send 2");

        let drained = mailbox.close_and_drain();
        assert_eq!(drained.len(), 2, "both queued envelopes must be drained");

        for envelope in drained {
            envelope
                .reply
                .send(Err(EntityError::EntityNotActive))
                .expect("receiver still open");
        }

        assert!(matches!(
            rx1.await.unwrap(),
            Err(EntityError::EntityNotActive)
        ));
        assert!(matches!(
            rx2.await.unwrap(),
            Err(EntityError::EntityNotActive)
        ));
    }

    #[tokio::test]
    async fn close_and_drain_on_empty_mailbox_returns_empty_queue() {
        let mailbox: BoundedMailbox<TestEnvelope> = BoundedMailbox::new(4);

        let drained = mailbox.close_and_drain();

        assert!(drained.is_empty(), "nothing was queued, nothing to drain");
    }

    /// Regression test for the lock-ordering guarantee: `send()` checks the
    /// `closed` flag inside the same `queue` lock critical section as the
    /// `push_back`, so there is no window in which `close_and_drain()` can
    /// take the queue after a message was accepted but before it lands in
    /// the drained output. Every one of `N` concurrent `send()` attempts
    /// must end up EITHER drained by `close_and_drain()` OR rejected with
    /// `MailboxClosed` — never silently lost.
    ///
    /// Capacity is deliberately half of `N` so roughly half the sends fill
    /// the queue while the other half block on `not_full.notified()`,
    /// exercising the backpressure/lost-wakeup-guard path (PC-R7) as well as
    /// plain capacity exhaustion — not just the always-fits case.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn close_and_drain_races_concurrent_sends_without_losing_envelopes() {
        const N: usize = 64;
        let mailbox: BoundedMailbox<usize> = BoundedMailbox::new(N / 2);

        let handles: Vec<_> = (0..N)
            .map(|id| {
                let mailbox = mailbox.clone();
                tokio::spawn(async move { (id, mailbox.send(id).await) })
            })
            .collect();

        // Race close_and_drain() against both the queue-filling sends and the
        // blocked-on-not_full sends above.
        let drained = mailbox.close_and_drain();
        let drained_ids: std::collections::HashSet<usize> = drained.iter().copied().collect();
        assert_eq!(
            drained.len(),
            drained_ids.len(),
            "close_and_drain() must not return the same envelope twice"
        );

        let mut accepted = 0usize;
        let mut rejected = 0usize;
        for handle in handles {
            let (id, result) = handle.await.expect("send task panicked");
            match result {
                Ok(()) => {
                    assert!(
                        drained_ids.contains(&id),
                        "send({id}) returned Ok but {id} was not present in close_and_drain() output"
                    );
                    accepted += 1;
                }
                Err(EntityError::MailboxClosed) => rejected += 1,
                Err(other) => panic!("unexpected error from send(): {other:?}"),
            }
        }

        assert_eq!(
            accepted + rejected,
            N,
            "every send attempt must be either accepted-and-drained or rejected as closed — none silently lost"
        );
    }
}
