//! Internal bounded admission queue (CORE-019 Phase 4).
//!
//! **Not public API** (design.md §2, §3): the spec's "no third swappable
//! queue port" requirement means this type is never re-exported from
//! `effects::mod` — only [`crate::effects::acceptor`] (Phase 5) and
//! [`crate::effects::runner`] (Phase 6) see it, both inside this crate.
//!
//! **F-02 round 3 (PR4 review round 3, BLOCKER fix):** the previous round's
//! `oldest_pending_age()` reported "the wait time of the effect most
//! recently returned by `recv`" — itself a regression. `runner.rs` only
//! reads that signal from inside its `receiver.recv()` select branch, so
//! the instant the runner stops making dequeue progress (backpressure
//! saturation, a hung executor), the signal is anchored to an
//! already-departed effect and stops reflecting reality — exactly the
//! stall scenario this signal exists to detect, and a violation of
//! design.md §9 / spec.md's normative "age of oldest pending effect"
//! contract. Reintroducing the *previous* round's plain `VecDeque<Instant>`
//! (position-keyed: push on send, pop-front on recv) would fix the freeze
//! but reopen the *original* F-02 race: `EffectQueue` is `Clone`, and two
//! concurrent `send()` calls can land their messages in the channel in an
//! order that does not match the order their own push_backs happened,
//! desyncing a "front of the deque" assumption. The fix here tracks pending
//! enqueue timestamps **by the effect's own identity**
//! (`HashMap<EffectId, Instant>`), immune to any send/dequeue ordering
//! mismatch between concurrent senders — there is no "position" to get
//! wrong.
//!
//! **F-03 (PR4 review round 4, BLOCKER fix):** the identity-keyed redesign
//! above introduced its own new bug: [`EffectQueue::send`]'s `pending_since`
//! insert ran with no RAII guard between it and the channel-send `.await` —
//! if that future was cancelled mid-flight (dropped, e.g. by
//! `acceptor.rs`'s `send_to_queue` racing a shutdown deadline in
//! `tokio::select!` while blocked awaiting queue capacity), the entry leaked
//! forever: never actually enqueued, yet never removed. `send` now
//! constructs a [`PendingGuard`] right after the insert, disarmed only on
//! the genuine success path (ownership transfers to
//! [`EffectQueueReceiver::recv`]); every other exit — the existing `Err`
//! branch and cancellation alike — is handled uniformly by the guard's own
//! `Drop`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;

use super::store::{AcceptedEffect, EffectId};

/// A thin wrapper around the queued effect. **F-02 round 3:** no longer
/// carries an enqueue timestamp — that bookkeeping now lives entirely in
/// `pending_since`, keyed by the effect's own [`EffectId`], so there is
/// nothing left to keep in sync between the channel message and a side
/// structure. Kept as a distinct type (rather than sending `AcceptedEffect`
/// directly) so a future per-message addition doesn't require touching the
/// channel's element type again.
struct QueuedEffect {
    effect: AcceptedEffect,
}

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
    sender: mpsc::Sender<QueuedEffect>,
    /// **F-02 round 3:** enqueue timestamps keyed by the effect's own
    /// identity, shared with [`EffectQueueReceiver`] via the same `Arc`.
    /// Immune to send/dequeue ordering mismatches between concurrent
    /// senders — see the module doc comment.
    pending_since: Arc<Mutex<HashMap<EffectId, Instant>>>,
}

/// The receiving half — [`super::runner::DeliveryRunner`] is the sole
/// consumer (AD-8's single-consumer invariant applies to the whole subsystem,
/// not just `claim_due`).
pub(crate) struct EffectQueueReceiver {
    receiver: mpsc::Receiver<QueuedEffect>,
    /// **F-02 round 3:** same shared identity-keyed map as
    /// [`EffectQueue::pending_since`] — see [`Self::oldest_pending_age`].
    pending_since: Arc<Mutex<HashMap<EffectId, Instant>>>,
}

/// RAII guard for [`EffectQueue::send`]'s `pending_since` insert (**F-03,
/// PR4 review round 4, BLOCKER fix**): if the `send` future is dropped
/// mid-`.await` (cancelled — e.g. `acceptor.rs`'s `send_to_queue` racing a
/// shutdown deadline in `tokio::select!`), Rust drops every live local of
/// the cancelled future's generated state machine, including this guard.
/// Its `Drop` removes the now-orphaned entry, since the effect was never
/// actually enqueued. Disarmed only on the genuine success path, where
/// responsibility for removing the entry transfers to
/// [`EffectQueueReceiver::recv`]. See `send`'s own doc comment for the full
/// rationale.
struct PendingGuard<'a> {
    map: &'a Mutex<HashMap<EffectId, Instant>>,
    id: EffectId,
    armed: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.map.lock().unwrap().remove(&self.id);
        }
    }
}

impl EffectQueue {
    /// Creates a bounded queue pair with room for `capacity` in-flight sends.
    pub(crate) fn bounded(capacity: usize) -> (Self, EffectQueueReceiver) {
        let (sender, receiver) = mpsc::channel(capacity);
        let pending_since = Arc::new(Mutex::new(HashMap::new()));
        (
            Self {
                sender,
                pending_since: pending_since.clone(),
            },
            EffectQueueReceiver {
                receiver,
                pending_since,
            },
        )
    }

    /// Enqueues a freshly-accepted `effect`, waiting for capacity rather than
    /// dropping it. **F-02 round 3:** the enqueue timestamp is recorded in
    /// `pending_since`, keyed by the effect's own id, *before* the channel
    /// send — so by the time the message can possibly become visible to a
    /// receiver, its identity-keyed entry already exists. Ordering relative
    /// to any OTHER concurrent send's own insert doesn't matter: there is no
    /// shared "position" for two sends to disagree about.
    ///
    /// **Cancellation safety (F-03, PR4 review round 4, BLOCKER fix):** the
    /// insert above is synchronous, so if THIS future is dropped mid-`.await`
    /// below — cancelled, e.g. by `acceptor.rs`'s `send_to_queue` racing a
    /// shutdown deadline in `tokio::select!` while blocked awaiting queue
    /// capacity — the effect is never actually enqueued, yet the insert
    /// already happened. [`PendingGuard`], constructed right after the
    /// insert, exists exactly for this: its `Drop` removes the entry unless
    /// explicitly disarmed, and Rust guarantees every live local of a
    /// cancelled future's generated state machine — including this guard —
    /// is dropped when the future itself is dropped. Disarmed only on the
    /// genuine success path, where responsibility for removing the entry
    /// transfers to [`EffectQueueReceiver::recv`].
    pub(crate) async fn send(
        &self,
        effect: AcceptedEffect,
    ) -> Result<(), SendError<AcceptedEffect>> {
        let id = effect.id;
        self.pending_since
            .lock()
            .unwrap()
            .insert(id, Instant::now());

        let mut guard = PendingGuard {
            map: &self.pending_since,
            id,
            armed: true,
        };

        let queued = QueuedEffect { effect };
        match self.sender.send(queued).await {
            Ok(()) => {
                // Ownership of removing this entry transfers to `recv` once
                // the effect is actually dequeued — disarm so the guard's
                // `Drop` doesn't race that removal.
                guard.armed = false;
                Ok(())
            }
            Err(SendError(queued)) => {
                // Never actually queued (all receivers dropped) — the
                // guard's own `Drop`, below, removes the now-stale entry.
                Err(SendError(queued.effect))
            }
        }
    }

    /// How many accepted effects are currently sitting in the queue
    /// (`queue_depth` signal). **F-02 fix:** reads the channel's actual
    /// current occupancy directly (`max_capacity - capacity`, both exposed
    /// by `tokio::sync::mpsc::Sender`) — there is no separate counter or
    /// structure that could ever desync from the channel's own real state.
    /// Production code reads depth via [`EffectQueueReceiver::depth`]
    /// instead (the receiver is what `DeliveryRunner::run_inner` actually
    /// holds, CORE-019 rebase reconciliation) — this sender-side accessor
    /// is exercised directly by this module's own tests below. Unchanged by
    /// the F-02 round 3 fix — already correct and race-free.
    #[allow(dead_code)]
    pub(crate) fn depth(&self) -> usize {
        self.sender.max_capacity() - self.sender.capacity()
    }

    /// Test-only: exact number of live `pending_since` entries. Used to
    /// prove a cancelled `send` future's entry does not survive (F-03, PR4
    /// review round 4) — `oldest_pending_age`/`depth` alone can't distinguish
    /// "one genuinely-queued effect" from "one genuinely-queued effect plus
    /// one orphaned entry from a cancelled send".
    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.pending_since.lock().unwrap().len()
    }
}

impl EffectQueueReceiver {
    /// Receives the next queued effect, or `None` once every `EffectQueue`
    /// sender has been dropped. **F-02 round 3:** removes the dequeued
    /// effect's own entry from `pending_since` by its exact id — never "pop
    /// the front" of anything, so there is no assumption that insertion
    /// order matches dequeue order.
    pub(crate) async fn recv(&mut self) -> Option<AcceptedEffect> {
        let queued = self.receiver.recv().await?;
        self.pending_since.lock().unwrap().remove(&queued.effect.id);
        Some(queued.effect)
    }

    /// Mirrors [`EffectQueue::depth`] — reads the same underlying channel's
    /// real occupancy directly; the receiver and sender halves of a
    /// `tokio::sync::mpsc` channel always agree on capacity/max_capacity,
    /// so there is nothing to share or synchronize beyond the channel
    /// itself (PR2 round 5, F-01: the self-deadlock risk of holding both
    /// halves in `DeliveryRunner` stays closed).
    pub(crate) fn depth(&self) -> usize {
        self.receiver.max_capacity() - self.receiver.capacity()
    }

    /// Age of the oldest accepted effect waiting to enter or leave the
    /// delivery queue (`oldest_pending_age` signal). `None` when nothing is
    /// currently pending — including after the queue fully drains.
    ///
    /// **Deliberately includes backpressure wait time:** an effect is
    /// tracked from the moment `send()` is called — including any time
    /// spent blocked awaiting channel capacity — not only once it physically
    /// occupies an mpsc slot. The effect was already durably accepted at
    /// that point and is genuinely waiting on delivery, so counting
    /// backpressure time is the operationally useful signal (a producer
    /// stuck on a saturated queue is exactly the kind of stall an operator
    /// needs "oldest pending" to surface). `queue_depth` stays a distinct,
    /// narrower signal — the mpsc channel's actual physical occupancy only.
    ///
    /// **F-02 round 3 fix:** the previous round reported the wait time of
    /// the effect most recently returned by [`Self::recv`] — honest only at
    /// the instant of dequeue, and frozen on the wrong effect's timestamp
    /// the moment the runner stops making dequeue progress (see the module
    /// doc comment). This restores the true "age of oldest pending effect"
    /// contract (design.md §9 / spec.md's Observability requirement): an
    /// O(depth) scan for the minimum timestamp still in `pending_since`,
    /// keyed by each effect's own identity rather than queue position. The
    /// queue is bounded/small, so this scan is not a performance concern.
    pub(crate) fn oldest_pending_age(&self) -> Option<Duration> {
        self.pending_since
            .lock()
            .unwrap()
            .values()
            .min()
            .map(|oldest| oldest.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;
    use tokio::sync::Notify;

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

    // -- CORE-019 Phase 11: queue_depth / oldest_pending_age -----------------

    #[tokio::test]
    async fn depth_is_zero_when_empty() {
        let (queue, receiver) = EffectQueue::bounded(4);

        assert_eq!(queue.depth(), 0);
        assert_eq!(receiver.depth(), 0);
    }

    #[tokio::test]
    async fn oldest_pending_age_is_none_when_nothing_sent() {
        let (_queue, receiver) = EffectQueue::bounded(4);

        assert!(receiver.oldest_pending_age().is_none());
    }

    /// **F-02 round 3, failure mode 3.** Once every sent effect has actually
    /// been received, `oldest_pending_age()` must go back to `None` —
    /// reflecting "nothing pending" — not stay frozen at a stale non-`None`
    /// value forever (the previous "just-dequeued" design never returned to
    /// `None` again after its first `Some`).
    #[tokio::test]
    async fn oldest_pending_age_returns_to_none_once_fully_drained() {
        let (queue, mut receiver) = EffectQueue::bounded(4);

        queue.send(sample()).await.unwrap();
        queue.send(sample()).await.unwrap();

        receiver.recv().await.unwrap();
        assert!(receiver.oldest_pending_age().is_some());

        receiver.recv().await.unwrap();
        assert!(
            receiver.oldest_pending_age().is_none(),
            "once every sent effect has been received, oldest_pending_age must go back to None"
        );
    }

    #[tokio::test]
    async fn depth_tracks_sends_and_receives_fifo() {
        let (queue, mut receiver) = EffectQueue::bounded(4);

        queue.send(sample()).await.unwrap();
        queue.send(sample()).await.unwrap();
        assert_eq!(queue.depth(), 2);

        receiver.recv().await.unwrap();
        assert_eq!(queue.depth(), 1);

        receiver.recv().await.unwrap();
        assert_eq!(queue.depth(), 0);
    }

    /// **F-02 round 3 (PR4 review round 3, BLOCKER fix).** Replaces the
    /// previous test that encoded the "just-dequeued effect's own wait time"
    /// semantics — that WAS the bug: `oldest_pending_age()` must report the
    /// age of whatever is genuinely STILL sitting in the queue, computable
    /// even *before* anything has been dequeued, and must keep tracking the
    /// new minimum as the current oldest entry is drained.
    #[tokio::test]
    async fn oldest_pending_age_reflects_the_minimum_still_pending_enqueue_time() {
        let (queue, mut receiver) = EffectQueue::bounded(4);

        queue.send(sample()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        queue.send(sample()).await.unwrap();

        // Both effects are still queued — readable WITHOUT dequeuing
        // anything (impossible under the old "just-dequeued" semantics,
        // which required at least one `recv()` first).
        let age = receiver.oldest_pending_age().unwrap();
        assert!(age >= Duration::from_millis(30));

        // Dequeuing the oldest leaves only the more-recently-enqueued
        // effect — the new minimum, smaller than before.
        receiver.recv().await.unwrap();
        let age_after_dequeuing_oldest = receiver.oldest_pending_age().unwrap();
        assert!(age_after_dequeuing_oldest < age);

        // Fully drained — back to None, not a frozen stale value.
        receiver.recv().await.unwrap();
        assert!(receiver.oldest_pending_age().is_none());
    }

    /// **F-02 round 3, failure mode 1: the freeze-during-a-stall bug.**
    /// `log_oldest_pending_age` is only called from `runner.rs`'s
    /// `receiver.recv()` select branch — if the runner stops making dequeue
    /// progress (backpressure saturation, a hung executor), no new value is
    /// ever observed again unless `oldest_pending_age()` itself keeps
    /// honestly tracking whatever is STILL enqueued. The previous
    /// "just-dequeued" design reported the wrong effect's age forever once
    /// dequeuing stalled: it kept computing an ever-growing number, but
    /// anchored to the wrong (already-departed) effect's enqueue time,
    /// rather than the effect actually still sitting in the queue.
    #[tokio::test]
    async fn oldest_pending_age_tracks_still_queued_effect_not_the_last_dequeued_one() {
        let (queue, mut receiver) = EffectQueue::bounded(4);

        // Effect A enqueued first...
        queue.send(sample()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        // ...effect B enqueued ~50ms later.
        queue.send(sample()).await.unwrap();

        // The runner dequeues A only, then stalls — simulating backpressure
        // saturation or a hung executor by simply never calling `recv()`
        // again.
        receiver.recv().await.unwrap();

        // B is still sitting in the queue, just enqueued — its true age is
        // small. A design that reports "the just-dequeued effect's (A's)
        // own wait time" would incorrectly show >= 50ms here, even though
        // nothing that old remains queued.
        let age = receiver.oldest_pending_age().unwrap();
        assert!(
            age < Duration::from_millis(50),
            "oldest_pending_age must report the age of the effect STILL in the queue (B, just \
             enqueued), not the just-dequeued effect's (A's) own wait time"
        );
    }

    /// **F-02 round 3, failure mode 2: naive position-keyed tracking
    /// mismatches under concurrent senders.** Standalone reproduction (same
    /// established pattern as
    /// `old_design_shape_permanently_desyncs_when_recv_races_sends_side_channel_update`
    /// below) proving a naive `VecDeque<Instant>` push_back/pop_front
    /// tracker — even with "push before send" ordering per sender — can
    /// still assign the WRONG effect's timestamp when two senders race,
    /// because it tracks by queue *position*, not by the effect's own
    /// identity. This is exactly the hazard an identity-keyed
    /// `HashMap<EffectId, Instant>` sidesteps entirely: there is no
    /// "position" to get wrong.
    #[tokio::test]
    async fn old_position_keyed_pop_front_assigns_wrong_effects_timestamp_under_concurrent_senders()
    {
        let pending_since: Arc<StdMutex<VecDeque<Instant>>> =
            Arc::new(StdMutex::new(VecDeque::new()));
        let (tx, mut rx) = mpsc::channel::<&'static str>(4);

        let release_b = Arc::new(Notify::new());
        let release_a_send = Arc::new(Notify::new());

        // Sender A pushes its (earlier) timestamp immediately — "push
        // before send" — but is held back from actually landing its
        // message in the channel until sender B has already landed its own
        // message first.
        let a_enqueued_at = Instant::now();
        let sender_a = {
            let pending_since = pending_since.clone();
            let release_a_send = release_a_send.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                pending_since.lock().unwrap().push_back(a_enqueued_at);
                release_a_send.notified().await;
                tx.send("A").await.unwrap();
            })
        };

        // Sender B pushes ITS timestamp, then lands its message in the
        // channel FIRST — a legitimate interleaving between two concurrent
        // senders that "push before send" per-sender ordering does nothing
        // to prevent.
        let sender_b = {
            let pending_since = pending_since.clone();
            let release_b = release_b.clone();
            let release_a_send = release_a_send.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                release_b.notified().await;
                let b_enqueued_at = Instant::now();
                pending_since.lock().unwrap().push_back(b_enqueued_at);
                tx.send("B").await.unwrap();
                release_a_send.notify_one();
            })
        };

        release_b.notify_one();

        // B's message is received FIRST from the channel (it landed first)...
        let first_received = rx.recv().await.unwrap();
        assert_eq!(first_received, "B");
        // ...but a naive position-keyed tracker pops the FRONT of the
        // queue, which is A's (earlier-pushed) timestamp — handing B the
        // WRONG effect's enqueue time.
        let popped_for_b = pending_since.lock().unwrap().pop_front().unwrap();
        assert_eq!(
            popped_for_b, a_enqueued_at,
            "the naive push_back/pop_front shape hands B the WRONG (A's) enqueue timestamp \
             because it tracks by position, not by the effect's own identity — exactly the \
             hazard an identity-keyed HashMap<EffectId, Instant> avoids"
        );

        sender_a.await.unwrap();
        sender_b.await.unwrap();
    }

    /// **F-02 round 3, GREEN proof for failure mode 2.** The real,
    /// identity-keyed `EffectQueue`/`EffectQueueReceiver` stays correct
    /// under concurrent sends regardless of which one actually lands (and
    /// is dequeued) first — there is no shared "position" for two racing
    /// senders to desync.
    #[tokio::test]
    async fn oldest_pending_age_stays_correct_regardless_of_concurrent_send_dequeue_order() {
        let (queue, mut receiver) = EffectQueue::bounded(4);

        let effect_x = sample();
        let effect_y = sample();
        let id_x = effect_x.id;
        let id_y = effect_y.id;

        let send_x = {
            let queue = queue.clone();
            tokio::spawn(async move { queue.send(effect_x).await.unwrap() })
        };
        let send_y = {
            let queue = queue.clone();
            tokio::spawn(async move { queue.send(effect_y).await.unwrap() })
        };
        send_x.await.unwrap();
        send_y.await.unwrap();

        assert_eq!(receiver.depth(), 2);

        let first = receiver.recv().await.unwrap();
        let remaining_id = if first.id == id_x { id_y } else { id_x };

        assert!(
            receiver.oldest_pending_age().is_some(),
            "the still-pending effect's age must be tracked by its own identity, regardless of \
             which of the two racing sends landed/was dequeued first"
        );

        let second = receiver.recv().await.unwrap();
        assert_eq!(
            second.id, remaining_id,
            "identity-keyed tracking cannot mix up which timestamp belongs to which effect, \
             unlike a position-keyed VecDeque"
        );

        assert!(receiver.oldest_pending_age().is_none());
    }

    // -- F-03 (PR4 review round 4, BLOCKER): send cancellation leak ---------

    /// **F-03 (PR4 review round 4, BLOCKER fix).** `send()`'s `pending_since`
    /// insert happens synchronously, before the `.await` on the channel send
    /// — there is no RAII guard between them. `acceptor.rs`'s
    /// `send_to_queue` races `self.queue.send(effect)` inside a
    /// `tokio::select!` against `wait_for_deadline(...)`; if the deadline
    /// branch wins while the send is blocked awaiting capacity,
    /// `tokio::select!` drops the send future mid-flight — the insert
    /// already happened, but the effect was never actually enqueued, and
    /// nothing ever removes its entry.
    ///
    /// RED (deterministic, no timing): a `biased` `tokio::select!` polls the
    /// blocked `send` branch first — guaranteeing its synchronous insert
    /// runs and observing it `Pending` (the queue is already at capacity) —
    /// then completes on an already-`Ready` second branch, causing
    /// `tokio::select!` to drop the still-pending `send` future. This is the
    /// exact production race in `send_to_queue`, reproduced without relying
    /// on timing. Before this fix, `pending_count()` stays at 2 (the
    /// cancelled second effect's entry survives forever). After the RAII
    /// guard below, it returns to 1 — only the genuinely still-queued first
    /// effect remains.
    #[tokio::test]
    async fn send_cancelled_while_blocked_on_capacity_removes_its_pending_since_entry() {
        let (queue, _receiver) = EffectQueue::bounded(1);

        // Fill the queue to capacity with a real, still-pending effect.
        queue.send(sample()).await.unwrap();
        assert_eq!(queue.pending_count(), 1);

        // A second send now blocks awaiting capacity. `biased` guarantees
        // the send branch is polled first (running its synchronous insert),
        // observed `Pending`, and then dropped once the always-ready branch
        // completes the select — deterministic cancellation, no timing.
        tokio::select! {
            biased;
            _ = queue.send(sample()) => {
                panic!("send must not complete — the queue is already at capacity");
            }
            _ = std::future::ready(()) => {}
        }

        assert_eq!(
            queue.pending_count(),
            1,
            "the cancelled second send's pending_since entry must not survive — only the \
             genuinely still-queued first effect should remain"
        );
    }

    // -- F-02 (PR4 review, BLOCKER): send/recv metrics race -----------------

    /// **RED proof.** Reproduces, deterministically (not via flaky timing),
    /// the exact desync the previous design was exposed to: a side
    /// `VecDeque<Instant>` updated by two independent steps —
    /// `sender.send(effect).await` completing, THEN separately
    /// `pending_since.push_back(..)` — racing a receiver whose
    /// `receiver.recv().await` returning THEN separately
    /// `pending_since.pop_front()` can run in between those two steps.
    /// Forced via an explicit `Notify` handshake (mirrors this same
    /// module's/`acceptor.rs`'s established "gated" test pattern) rather
    /// than a timing-based race: the simulated sender is deliberately
    /// paused, by construction, strictly between its channel-visible send
    /// and its side-structure update, so the concurrently-running receiver
    /// is guaranteed to observe the empty side structure and pop nothing
    /// from it — a permanent phantom entry once the sender resumes.
    #[tokio::test]
    async fn old_design_shape_permanently_desyncs_when_recv_races_sends_side_channel_update() {
        let pending_since: Arc<StdMutex<VecDeque<Instant>>> =
            Arc::new(StdMutex::new(VecDeque::new()));
        let pause_before_push_back = Arc::new(Notify::new());
        let (tx, mut rx) = mpsc::channel::<Instant>(4);

        // Old-shape `send`: channel-send completes, THEN (after being
        // deliberately paused) updates the separate side structure — the
        // exact two-step sequence the real pre-fix `EffectQueue::send` used.
        let sender_task = {
            let pending_since = pending_since.clone();
            let pause = pause_before_push_back.clone();
            tokio::spawn(async move {
                tx.send(Instant::now()).await.unwrap();
                pause.notified().await;
                pending_since.lock().unwrap().push_back(Instant::now());
            })
        };

        // Old-shape `recv`: channel-recv completes, THEN immediately (no
        // await in between, matching the real pre-fix code) pops the side
        // structure — which, at this point, the paused sender above has not
        // yet pushed onto.
        let received = rx.recv().await;
        assert!(
            received.is_some(),
            "the channel item is visible before the sender's side update"
        );
        pending_since.lock().unwrap().pop_front();

        // Only now let the sender's delayed side-structure update run.
        pause_before_push_back.notify_one();
        sender_task.await.unwrap();

        assert_eq!(
            pending_since.lock().unwrap().len(),
            1,
            "the old dual-tracking shape permanently desyncs: a phantom entry survives even \
             though the channel is empty and the effect was fully processed — this is exactly \
             the corrupted `queue_depth`/`oldest_pending_age` bug F-02 fixes"
        );
    }

    /// **GREEN proof.** With enqueue timestamps carried atomically inside
    /// the channel message itself, and depth derived directly from the
    /// channel's own real occupancy, there is no separate structure left to
    /// desync. Proven under real concurrent send/recv pressure (many
    /// interleaved producers/consumer): `depth()` always returns to exactly
    /// 0 once every sent effect has actually been received — never a
    /// phantom non-zero value, unlike the old shape reproduced above.
    #[tokio::test]
    async fn depth_never_desyncs_under_concurrent_send_and_recv() {
        const TOTAL: usize = 200;
        let (queue, mut receiver) = EffectQueue::bounded(8);

        let producer = {
            let queue = queue.clone();
            tokio::spawn(async move {
                for _ in 0..TOTAL {
                    queue.send(sample()).await.unwrap();
                }
            })
        };

        let mut received = 0;
        while received < TOTAL {
            if receiver.recv().await.is_some() {
                received += 1;
            }
        }
        producer.await.unwrap();

        assert_eq!(
            queue.depth(),
            0,
            "depth must return to exactly 0 once every sent effect has been received — the \
             atomic-with-the-message redesign has no side structure left that could desync"
        );
        assert_eq!(receiver.depth(), 0);
    }
}
