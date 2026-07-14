//! Internal bounded admission queue (CORE-019 Phase 4).
//!
//! **Not public API** (design.md §2, §3): the spec's "no third swappable
//! queue port" requirement means this type is never re-exported from
//! `effects::mod` — only [`crate::effects::acceptor`] (Phase 5) and
//! [`crate::effects::runner`] (Phase 6) see it, both inside this crate.

use tokio::sync::mpsc;

use super::store::AcceptedEffect;

/// The sending half of the bounded admission queue.
///
/// Bounded `tokio::sync::mpsc` (AD-6): `send` blocks while the queue is at
/// capacity rather than dropping — the runtime lifecycle requirement that
/// acceptance backpressure delays the reply, never refuses or loses an
/// already-committed effect.
///
/// **F-01 (PR2 round 5)**: this queue now carries ONLY freshly-accepted
/// effects. The reclaim loop's claimed/already-`InFlight` effects (formerly
/// `QueuedEffect::Reclaimed`, sent via a now-removed `send_reclaimed`) are
/// dispatched directly by [`super::runner::DeliveryRunner`] instead —
/// `send`/`send_reclaimed` block until the bounded queue has capacity, and
/// the ONLY consumer that would ever free that capacity
/// (`EffectQueueReceiver::recv`) is the very same reclaim loop, so routing
/// reclaimed effects back through this queue could self-deadlock whenever
/// `claim_due` returned more due effects than the queue had free capacity.
/// See `runner.rs`'s reclaim-loop doc comment for the full rationale.
#[derive(Clone)]
pub(crate) struct EffectQueue {
    sender: mpsc::Sender<AcceptedEffect>,
}

/// The receiving half — [`super::runner::DeliveryRunner`] is the sole
/// consumer (AD-8's single-consumer invariant applies to the whole subsystem,
/// not just `claim_due`).
pub(crate) struct EffectQueueReceiver {
    receiver: mpsc::Receiver<AcceptedEffect>,
}

impl EffectQueue {
    /// Creates a bounded queue pair with room for `capacity` in-flight sends.
    pub(crate) fn bounded(capacity: usize) -> (Self, EffectQueueReceiver) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Self { sender }, EffectQueueReceiver { receiver })
    }

    /// Enqueues a freshly-accepted `effect`, waiting for capacity rather than
    /// dropping it.
    pub(crate) async fn send(
        &self,
        effect: AcceptedEffect,
    ) -> Result<(), mpsc::error::SendError<AcceptedEffect>> {
        self.sender.send(effect).await
    }
}

impl EffectQueueReceiver {
    /// Receives the next queued effect, or `None` once every `EffectQueue`
    /// sender has been dropped.
    pub(crate) async fn recv(&mut self) -> Option<AcceptedEffect> {
        self.receiver.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample() -> super::super::store::AcceptedEffect {
        use crate::effects::store::{AcceptedEffect, EffectId};
        use ego_domain::{ExternalEffectDescription, IdempotencyKey, TenantId};
        use std::sync::Arc;

        AcceptedEffect {
            id: EffectId::new(),
            tenant: TenantId::new("tenant-a").unwrap(),
            attempt: 0,
            description: Arc::new(ExternalEffectDescription {
                idempotency_key: IdempotencyKey::new("uow-1:0").unwrap(),
                effect_type: "invoice.created".to_string(),
                payload: vec![],
                destination: "https://example.com".to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn send_succeeds_below_capacity_and_receiver_gets_it() {
        let (queue, mut receiver) = EffectQueue::bounded(1);

        queue.send(sample()).await.unwrap();

        assert!(receiver.recv().await.is_some());
    }

    #[tokio::test]
    async fn send_blocks_at_capacity_until_receiver_makes_room_never_dropping() {
        let (queue, mut receiver) = EffectQueue::bounded(1);
        queue.send(sample()).await.unwrap();

        // The queue is now full (capacity 1). A second send must block, not
        // drop the effect or return early.
        let second_send = tokio::spawn({
            let queue = queue.clone();
            async move { queue.send(sample()).await }
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !second_send.is_finished(),
            "send must block while the queue is at capacity, not drop or return early"
        );

        // Draining one slot unblocks the pending send — proving it was
        // queued, not silently dropped.
        receiver.recv().await.unwrap();
        second_send
            .await
            .expect("task joins")
            .expect("second send completes once capacity frees up");

        // Both effects are now real, receivable items — nothing was lost.
        assert!(receiver.recv().await.is_some());
    }
}
