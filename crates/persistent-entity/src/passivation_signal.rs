//! Runtime-agnostic passivation signal trait.
//!
//! Provides [`PassivationSignal`], which the actor loop polls to decide when
//! to stop processing commands and passivate.  Decoupling this from Tokio's
//! `sleep` directly allows the actor to be tested with a [`ManualSignal`] that
//! fires on demand without waiting for a real clock.
//!
//! # Implementations
//!
//! - [`TokioPassivationSignal`]: production impl backed by `tokio::time::sleep`.
//! - [`ManualSignal`]: test impl that fires when [`ManualSignal::trigger`] is called.
//!
//! [`EntityActor`]: crate::actor::EntityActor

use std::future::Future;
use std::sync::Arc;
use tokio::sync::Notify;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A signal that fires when an entity should passivate.
///
/// The actor loop selects on `mailbox.recv()` and `signal.passivated()`.
/// When `passivated()` resolves, the loop exits and passivation begins.
pub trait PassivationSignal: Send {
    /// Returns a future that resolves when the signal fires.
    ///
    /// The future is single-use: once it resolves, the actor stops.
    fn passivated(&self) -> impl Future<Output = ()> + Send;
}

// ---------------------------------------------------------------------------
// TokioPassivationSignal
// ---------------------------------------------------------------------------

/// Production passivation signal backed by `tokio::time::sleep`.
///
/// Each call to [`passivated`](PassivationSignal::passivated) creates a fresh
/// `sleep` future. Because `process_commands` calls `passivated()` inside a
/// `tokio::select!` loop, Tokio cancels the sleep branch when a command
/// arrives and a new sleep starts on the next iteration.  This implements
/// **idle-based passivation**: the timer resets on every received command, and
/// the actor only passivates after `timeout` elapses with no activity.  Under
/// sustained load the actor will never passivate.
pub struct TokioPassivationSignal {
    /// Duration of inactivity before passivation fires.
    pub timeout: std::time::Duration,
}

impl TokioPassivationSignal {
    /// Creates a new signal that fires after `timeout`.
    pub fn new(timeout: std::time::Duration) -> Self {
        Self { timeout }
    }
}

impl PassivationSignal for TokioPassivationSignal {
    fn passivated(&self) -> impl Future<Output = ()> + Send {
        let duration = self.timeout;
        async move { tokio::time::sleep(duration).await }
    }
}

// ---------------------------------------------------------------------------
// ManualSignal
// ---------------------------------------------------------------------------

/// Test passivation signal that fires when [`ManualSignal::trigger`] is called.
///
/// Uses a `tokio::sync::Notify` internally so `passivated()` awaits
/// `Notify::notified()`.
#[derive(Clone)]
pub struct ManualSignal {
    notify: Arc<Notify>,
}

impl ManualSignal {
    /// Creates a new, untriggered signal.
    pub fn new() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
        }
    }

    /// Triggers the signal, resolving any pending `passivated()` future.
    pub fn trigger(&self) {
        self.notify.notify_one();
    }
}

impl PassivationSignal for ManualSignal {
    fn passivated(&self) -> impl Future<Output = ()> + Send {
        let notify = Arc::clone(&self.notify);
        async move { notify.notified().await }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn tokio_signal_zero_duration_resolves_immediately() {
        let sig = TokioPassivationSignal::new(Duration::from_millis(0));
        // Should complete without hanging.
        tokio::time::timeout(Duration::from_millis(100), sig.passivated())
            .await
            .expect("TokioPassivationSignal with zero duration must resolve immediately");
    }

    #[tokio::test]
    async fn manual_signal_not_triggered_does_not_resolve() {
        let sig = ManualSignal::new();
        // The future should NOT resolve within a short window.
        let result =
            tokio::time::timeout(Duration::from_millis(20), sig.passivated()).await;
        assert!(
            result.is_err(),
            "ManualSignal not triggered must not resolve"
        );
    }

    #[tokio::test]
    async fn manual_signal_triggered_resolves() {
        let sig = ManualSignal::new();
        let sig2 = sig.clone();
        // Trigger from a sibling task.
        tokio::spawn(async move {
            sig2.trigger();
        });
        tokio::time::timeout(Duration::from_millis(200), sig.passivated())
            .await
            .expect("ManualSignal::trigger must resolve passivated()");
    }
}
