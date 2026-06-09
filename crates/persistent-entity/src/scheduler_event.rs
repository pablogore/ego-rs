use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::Notify;

use crate::scheduler::EntityTriple;

/// Default capacity for the bounded scheduler event bus.
pub const DEFAULT_EVENT_BUS_CAPACITY: usize = 4096;

/// Default capacity for the in-scheduler event replay buffer.
pub const DEFAULT_REPLAY_BUFFER_CAPACITY: usize = 1024;

/// Channel item: (monotonic_sequence_id, event_payload).
type BusItem = (u64, SchedulerEvent);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the scheduler event bus.
#[derive(Debug, Clone)]
pub struct SchedulerEventBusConfig {
    /// Maximum number of events buffered in the channel.
    pub capacity: usize,
}

impl Default for SchedulerEventBusConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_EVENT_BUS_CAPACITY,
        }
    }
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

/// Reactive scheduler event emitted by the Actor.
#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    SlotFreed,
    CommandArrived(EntityTriple),
    CircuitBreakerExpired(EntityTriple),
    /// Emitted by Actor after executing a command.
    ExecutionCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
    /// Emitted by Actor when its internal state changes.
    EntityStateUpdated {
        entity: EntityTriple,
        state_version: u64,
    },
    /// Emitted by Actor after recovery finishes.
    RecoveryCompleted {
        entity: EntityTriple,
        state_version: u64,
    },
}

// ---------------------------------------------------------------------------
// SchedulerEventBus — bounded, non-blocking, fire-and-forget
// ---------------------------------------------------------------------------

/// Non-blocking event sender for Actor -> Scheduler feedback.
///
/// Bounded capacity prevents unbounded memory growth. Each event carries
/// a monotonic sequence ID for deterministic ordering guarantees.
/// If the buffer is full, the event is silently dropped (newest-drop policy).
///
/// The Actor execution path is NEVER blocked by the event bus.
#[derive(Debug, Clone)]
pub struct SchedulerEventSender {
    tx: mpsc::Sender<BusItem>,
    sequence_counter: Arc<AtomicU64>,
}

impl SchedulerEventSender {
    pub fn new(tx: mpsc::Sender<BusItem>) -> Self {
        Self {
            tx,
            sequence_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create a no-op sender that discards all events.
    /// Useful for tests that bypass the scheduler loop.
    pub fn noop() -> Self {
        let (tx, _rx) = mpsc::channel(1);
        Self {
            tx,
            sequence_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Fire-and-forget event emission with bounded buffer.
    ///
    /// Assigns a monotonic sequence ID and attempts to enqueue.
    /// Returns `true` if the event was accepted, `false` if dropped
    /// (buffer full or channel closed).
    pub fn emit(&self, event: SchedulerEvent) -> bool {
        let seq = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        self.tx.try_send((seq, event)).is_ok()
    }
}

/// Event receiver consumed by the Scheduler to drain pending events.
pub struct SchedulerEventReceiver {
    rx: mpsc::Receiver<BusItem>,
}

impl std::fmt::Debug for SchedulerEventReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerEventReceiver").finish()
    }
}

impl SchedulerEventReceiver {
    pub fn new(rx: mpsc::Receiver<BusItem>) -> Self {
        Self { rx }
    }

    /// Drain all currently buffered events without blocking.
    /// Returns drained items paired with their sequence IDs.
    pub fn drain_all(&mut self) -> Vec<BusItem> {
        let mut items = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(item) => items.push(item),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        items
    }
}

// ---------------------------------------------------------------------------
// SchedulerState — internal scheduler snapshot for deterministic decisions
// ---------------------------------------------------------------------------

/// Deterministic scheduler state snapshot.
///
/// Updated atomically during drain cycles. Provides the policy engine
/// with a consistent view of the system without coupling to Actor state.
#[derive(Debug, Clone)]
pub struct SchedulerState {
    /// Total events consumed since scheduler creation.
    pub total_events_consumed: u64,
    /// Events consumed in the last drain cycle.
    pub last_drain_count: usize,
    /// Sequence ID of the last event consumed.
    pub last_sequence_id: Option<u64>,
    /// Number of detected sequence gaps (lost events due to buffer overflow).
    pub detected_gaps: u64,
    /// The last activation suggestion produced.
    pub last_suggestion: Option<EntityTriple>,
    /// Bounded replay buffer of recent (sequence_id, event) pairs.
    /// Maintains the last N events for deterministic replay.
    replay_buffer: VecDeque<(u64, SchedulerEvent)>,
}

impl SchedulerState {
    pub fn new() -> Self {
        Self {
            total_events_consumed: 0,
            last_drain_count: 0,
            last_sequence_id: None,
            detected_gaps: 0,
            last_suggestion: None,
            replay_buffer: VecDeque::with_capacity(DEFAULT_REPLAY_BUFFER_CAPACITY),
        }
    }

    /// Process a batch of drained events and update internal counters.
    ///
    /// Detects sequence gaps (indicating buffer overflow), maintains
    /// the bounded replay buffer, and updates totals.
    pub fn apply_drained_events(&mut self, events: Vec<(u64, SchedulerEvent)>) {
        self.last_drain_count = events.len();
        self.total_events_consumed = self.total_events_consumed.wrapping_add(events.len() as u64);

        for &(seq, ref event) in &events {
            if let Some(last_seq) = self.last_sequence_id {
                if seq != last_seq.wrapping_add(1) {
                    self.detected_gaps = self.detected_gaps.wrapping_add(1);
                }
            }
            self.last_sequence_id = Some(seq);

            if self.replay_buffer.len() >= DEFAULT_REPLAY_BUFFER_CAPACITY {
                self.replay_buffer.pop_front();
            }
            self.replay_buffer.push_back((seq, event.clone()));
        }
    }

    /// Access the bounded replay buffer.
    pub fn replay_buffer(&self) -> &VecDeque<(u64, SchedulerEvent)> {
        &self.replay_buffer
    }

    /// Clear the replay buffer (e.g., after a snapshot is taken).
    pub fn clear_replay_buffer(&mut self) {
        self.replay_buffer.clear();
    }
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SchedulerTrigger — async notification primitive
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Channel constructors
// ---------------------------------------------------------------------------

/// Create a new event bus channel with default capacity (4096).
pub fn event_bus_channel() -> (SchedulerEventSender, SchedulerEventReceiver) {
    event_bus_channel_with_config(SchedulerEventBusConfig::default())
}

/// Create a new event bus channel with the given configuration.
pub fn event_bus_channel_with_config(
    config: SchedulerEventBusConfig,
) -> (SchedulerEventSender, SchedulerEventReceiver) {
    let (tx, rx) = mpsc::channel(config.capacity);
    (
        SchedulerEventSender::new(tx),
        SchedulerEventReceiver::new(rx),
    )
}
