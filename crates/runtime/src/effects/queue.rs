//! Internal bounded admission queue (CORE-019 Phase 4).
//!
//! **Not public API** (design.md §2, §3): the spec's "no third swappable
//! queue port" requirement means this type is never re-exported from
//! `effects::mod` — only [`crate::effects::acceptor`] (Phase 5) and
//! [`crate::effects::runner`] (Phase 6) see it, both inside this crate.

use tokio::sync::mpsc;

use super::store::AcceptedEffect;

/// Distinguishes a freshly-accepted effect — which still needs
/// `mark_in_flight` before dispatch — from one the reclaim loop already
/// claimed via `claim_due` and transitioned to `InFlight` itself, before ever
/// enqueueing it (F-01, PR2 round 4).
///
/// Before this distinction existed, `claim_due` didn't transition state at
/// all — an effect stayed `Pending`/`RetryableFailed` until `drain_one`
/// eventually reached `mark_in_flight`, so the same effect could be claimed
/// and re-enqueued on every reclaim tick until its first queue entry was
/// finally dequeued, inflating the queue with duplicate entries for one
/// effect. Now the reclaim loop claims-then-transitions before it ever
/// enqueues (see [`super::runner::DeliveryRunner::reclaim_due`]), and this
/// enum tells [`super::runner::DeliveryRunner::run_inner`]'s receive loop
/// which of `drain_one`/`drain_reclaimed` to call — the latter must NOT
/// call `mark_in_flight` again (it would immediately fail with
/// `InvalidTransition`, since the effect is no longer `Pending`/
/// `RetryableFailed`).
pub(crate) enum QueuedEffect {
    /// Needs `mark_in_flight` — the normal, direct-from-acceptance path.
    Fresh(AcceptedEffect),
    /// Already `InFlight` — the reclaim loop transitioned it before
    /// enqueueing.
    Reclaimed(AcceptedEffect),
}

/// The sending half of the bounded admission queue.
///
/// Bounded `tokio::sync::mpsc` (AD-6): `send`/`send_reclaimed` block while
/// the queue is at capacity rather than dropping — the runtime lifecycle
/// requirement that acceptance backpressure delays the reply, never refuses
/// or loses an already-committed effect.
#[derive(Clone)]
pub(crate) struct EffectQueue {
    sender: mpsc::Sender<QueuedEffect>,
}

/// The receiving half — [`super::runner::DeliveryRunner`] is the sole
/// consumer (AD-8's single-consumer invariant applies to the whole subsystem,
/// not just `claim_due`).
pub(crate) struct EffectQueueReceiver {
    receiver: mpsc::Receiver<QueuedEffect>,
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
    ) -> Result<(), mpsc::error::SendError<QueuedEffect>> {
        self.sender.send(QueuedEffect::Fresh(effect)).await
    }

    /// F-01 (PR2 round 4): enqueues an effect the reclaim loop already
    /// transitioned to `InFlight` — see [`QueuedEffect::Reclaimed`].
    pub(crate) async fn send_reclaimed(
        &self,
        effect: AcceptedEffect,
    ) -> Result<(), mpsc::error::SendError<QueuedEffect>> {
        self.sender.send(QueuedEffect::Reclaimed(effect)).await
    }
}

impl EffectQueueReceiver {
    /// Receives the next queued effect, or `None` once every `EffectQueue`
    /// sender has been dropped.
    pub(crate) async fn recv(&mut self) -> Option<QueuedEffect> {
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
