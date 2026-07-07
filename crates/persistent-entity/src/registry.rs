//! Routing authority for persistent entities.
//!
//! This module owns the single-flight routing map that decides "does a live
//! actor already exist for this triple" (ADR-001) and the read-only view over
//! its actor-published lifecycle state used to answer "is it active"
//! (ADR-002/ADR-003). See `openspec/changes/CORE-006A-activation-authority/design.md`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

use crate::lifecycle::EntityState;

const MAX_PASSIVATED_ENTRIES: usize = 10_000;

/// A live routing entry: the entity's type-erased mailbox handle, the
/// actor-published lifecycle state (read-only from the registry's side), and
/// a monotonic epoch identifying which activation created it (ADR-005).
struct ActiveEntry {
    mailbox: Arc<dyn Any + Send + Sync>,
    rx: watch::Receiver<EntityState>,
    epoch: u64,
}

/// Outcome of [`EntityRegistry::lookup_or_insert`]'s single-flight critical section.
pub enum RouteOutcome {
    /// A live entry already existed; the caller reuses its mailbox instead of spawning.
    Existing {
        /// The existing entry's type-erased mailbox handle.
        mailbox: Arc<dyn Any + Send + Sync>,
    },
    /// No live entry existed; one was just inserted (state `Recovering`). The
    /// caller now owns spawning the actor for this epoch and, once Phase 3
    /// wires the actor-owned publish (ADR-003), sending its lifecycle
    /// transitions through `tx`.
    Inserted {
        /// The freshly-created type-erased mailbox handle (the same value `make_mailbox` returned).
        mailbox: Arc<dyn Any + Send + Sync>,
        /// This activation's teardown identity — pass to [`EntityRegistry::deactivate_if_mine`].
        epoch: u64,
        /// The write side of the entry's published-state cell.
        tx: watch::Sender<EntityState>,
    },
}

impl std::fmt::Debug for EntityRegistry {
    /// Erased mailbox handles aren't `Debug`, so this reports only counts.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityRegistry")
            .field("active_count", &self.active_count())
            .field("passivated_count", &self.passivated_count())
            .finish()
    }
}

/// A registry for tracking entity routing and passivation bookkeeping.
pub struct EntityRegistry {
    /// Live routing entries, keyed by `aggregate_id`.
    active: Mutex<HashMap<String, ActiveEntry>>,
    /// Monotonic counter stamping each insert with a unique epoch — ABA-safe
    /// teardown identity (ADR-005).
    next_epoch: AtomicU64,
    /// Entities that have passivated (aggregate_id → final version). Advisory
    /// bookkeeping only — never gates or forks routing (ADR-004).
    passivated_entities: Arc<std::sync::Mutex<HashMap<String, u64>>>,
}

impl EntityRegistry {
    /// Create a new entity registry.
    pub fn new() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
            next_epoch: AtomicU64::new(0),
            passivated_entities: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Single-flight lookup-or-insert (ADR-001). Under one lock acquisition:
    /// returns the existing entry's erased mailbox if `entity_id` already has
    /// a live entry, otherwise lazily calls `make_mailbox` (never invoked on
    /// the hit path), inserts a new entry seeded `Recovering`, and hands the
    /// caller its epoch plus the sole [`watch::Sender`] for that entry.
    ///
    /// `make_mailbox` runs synchronously, still under the lock — it must not
    /// `.await` or panic-prone-ly do more than construct a mailbox, matching
    /// ADR-001's "no `.await`, no `tokio::spawn`" critical-section contract.
    pub fn lookup_or_insert<F>(&self, entity_id: &str, make_mailbox: F) -> RouteOutcome
    where
        F: FnOnce() -> Arc<dyn Any + Send + Sync>,
    {
        let mut active = self.active.lock();
        if let Some(entry) = active.get(entity_id) {
            return RouteOutcome::Existing {
                mailbox: entry.mailbox.clone(),
            };
        }

        let mailbox = make_mailbox();
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(EntityState::Recovering);
        active.insert(
            entity_id.to_string(),
            ActiveEntry {
                mailbox: mailbox.clone(),
                rx,
                epoch,
            },
        );
        RouteOutcome::Inserted { mailbox, epoch, tx }
    }

    /// Returns the live entry's erased mailbox handle, if one exists, without
    /// inserting anything. Presence-only lookup (ADR-008): sufficient for
    /// retry logic that only needs to know "is there still something here."
    pub fn lookup(&self, entity_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.active
            .lock()
            .get(entity_id)
            .map(|entry| entry.mailbox.clone())
    }

    /// Removes `entity_id`'s routing entry only if it is still owned by
    /// `epoch` — "removal is authority-scoped" (FR-001). A stale or
    /// superseded exit path's call is a safe no-op.
    pub fn deactivate_if_mine(&self, entity_id: &str, epoch: u64) {
        let mut active = self.active.lock();
        let is_mine = matches!(active.get(entity_id), Some(entry) if entry.epoch == epoch);
        if is_mine {
            active.remove(entity_id);
        }
    }

    /// Get the count of active entities — counts only entries whose
    /// published state is `EntityState::Active` (ADR-003/FR-002).
    /// `Recovering`, transitional, or duplicate entries are never counted.
    pub fn active_count(&self) -> usize {
        self.active
            .lock()
            .values()
            .filter(|entry| *entry.rx.borrow() == EntityState::Active)
            .count()
    }

    /// Get the count of passivated entities.
    pub fn passivated_count(&self) -> usize {
        self.passivated_entities.lock().unwrap().len()
    }

    /// Record an entity's passivation for observability — advisory bookkeeping
    /// that never gates or forks routing (ADR-004). Routing-entry removal is
    /// the caller's responsibility via [`Self::deactivate_if_mine`].
    ///
    /// Caps the passivated map at `MAX_PASSIVATED_ENTRIES` by evicting one
    /// arbitrary entry when the limit is reached, bounding memory in
    /// high-churn deployments.
    pub fn mark_passivated(&self, entity_id: String, version: u64) {
        let mut passivated = self.passivated_entities.lock().unwrap();
        if passivated.len() >= MAX_PASSIVATED_ENTRIES {
            if let Some(oldest) = passivated.keys().next().cloned() {
                passivated.remove(&oldest);
            }
        }
        passivated.insert(entity_id, version);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;

    fn erased_probe<T: Any + Send + Sync + 'static>(value: T) -> Arc<dyn Any + Send + Sync> {
        Arc::new(value) as Arc<dyn Any + Send + Sync>
    }

    /// TASK-003 (FR-001, FR-005, NFR-002): N concurrent `lookup_or_insert`
    /// calls for one triple must invoke `make_mailbox` exactly once and all
    /// resolve to the same mailbox `Arc` — instrumented by counting
    /// invocations of the "spawn" closure itself (spawn-count instrumentation),
    /// never by inspecting map/ID-set cardinality (NFR-002).
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_lookups_for_one_triple_spawn_exactly_once() {
        let registry = Arc::new(EntityRegistry::new());
        let spawn_count = Arc::new(AtomicUsize::new(0));
        const N: usize = 20;

        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let registry = registry.clone();
            let spawn_count = spawn_count.clone();
            handles.push(tokio::spawn(async move {
                match registry.lookup_or_insert("triple-1", || {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    erased_probe(42usize)
                }) {
                    RouteOutcome::Existing { mailbox } => mailbox,
                    RouteOutcome::Inserted { mailbox, .. } => mailbox,
                }
            }));
        }

        let mut mailboxes = Vec::with_capacity(N);
        for handle in handles {
            mailboxes.push(handle.await.expect("task should not panic"));
        }

        assert_eq!(
            spawn_count.load(Ordering::SeqCst),
            1,
            "exactly one caller should have won the single-flight race and constructed a mailbox"
        );
        let first = &mailboxes[0];
        for mailbox in &mailboxes {
            assert!(
                Arc::ptr_eq(first, mailbox),
                "every concurrent caller must resolve to the same mailbox Arc"
            );
        }
    }

    /// TASK-004 (FR-002/ADR-003): `active_count()` excludes an entry that is
    /// still `Recovering`, and counts it once its published state becomes `Active`.
    #[test]
    fn active_count_excludes_recovering_counts_active() {
        let registry = EntityRegistry::new();

        let tx = match registry.lookup_or_insert("triple-2", || erased_probe(0usize)) {
            RouteOutcome::Inserted { tx, .. } => tx,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        assert_eq!(
            registry.active_count(),
            0,
            "freshly-inserted entry is Recovering, not counted"
        );

        tx.send(EntityState::Active)
            .expect("receiver still held by the registry entry");

        assert_eq!(
            registry.active_count(),
            1,
            "entry must be counted once its published state is Active"
        );
    }

    /// NFR-003 (FR-004): the identical claim as
    /// `active_count_excludes_recovering_counts_active`, proven for the
    /// reactivation-from-`Passivated` path instead of the cold path. After a
    /// triple's first incarnation reaches `Active` and is torn down
    /// (`deactivate_if_mine`), a second `lookup_or_insert` for the same
    /// `entity_id` is a reactivation: it starts a fresh entry under a new
    /// epoch, seeded `Recovering` exactly like a cold insert. That entry
    /// must be excluded from `active_count()` until its own state reaches
    /// `Active` — the visibility contract must not differ by origin
    /// (FR-004 mirrors FR-003).
    #[test]
    fn reactivation_active_count_excludes_recovering_counts_active() {
        let registry = EntityRegistry::new();

        // Cold activation, then teardown (Active -> removed), simulating a
        // passivation cycle from the registry's point of view.
        let (epoch1, tx1) = match registry.lookup_or_insert("triple-5", || erased_probe(0usize)) {
            RouteOutcome::Inserted { epoch, tx, .. } => (epoch, tx),
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };
        tx1.send(EntityState::Active)
            .expect("receiver still held by the registry entry");
        assert_eq!(
            registry.active_count(),
            1,
            "cold activation must be counted once Active"
        );
        registry.mark_passivated("triple-5".to_string(), 1);
        registry.deactivate_if_mine("triple-5", epoch1);
        assert_eq!(
            registry.active_count(),
            0,
            "the torn-down entry must no longer be counted"
        );

        // Reactivation: a fresh lookup_or_insert for the same entity_id
        // starts a new Recovering entry under a new epoch.
        let tx2 = match registry.lookup_or_insert("triple-5", || erased_probe(0usize)) {
            RouteOutcome::Inserted { tx, .. } => tx,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert on reactivation"),
        };

        assert_eq!(
            registry.active_count(),
            0,
            "reactivation's freshly-inserted entry is Recovering, not counted (FR-004)"
        );

        tx2.send(EntityState::Active)
            .expect("receiver still held by the registry entry");

        assert_eq!(
            registry.active_count(),
            1,
            "the reactivated entry must be counted once its published state is Active (FR-004)"
        );
    }

    /// TASK-006 (ADR-002, Judgment Day CRITICAL 1 / FR-001's type-mismatch
    /// scenario): a downcast mismatch against a live entry must never be
    /// treated as "no live entry" — the entry is left exactly as-is and no
    /// second `make_mailbox` invocation (i.e. no competing spawn) occurs.
    #[test]
    fn live_entry_is_unaffected_by_a_mismatched_lookup() {
        let registry = EntityRegistry::new();

        let original = match registry.lookup_or_insert("triple-3", || erased_probe(7usize)) {
            RouteOutcome::Inserted { mailbox, .. } => mailbox,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        let second_call_spawned = AtomicBool::new(false);
        let erased = match registry.lookup_or_insert("triple-3", || {
            second_call_spawned.store(true, Ordering::SeqCst);
            erased_probe(String::from("wrong-type"))
        }) {
            RouteOutcome::Existing { mailbox } => mailbox,
            RouteOutcome::Inserted { .. } => panic!("must find the live entry, not insert a new one"),
        };
        assert!(
            !second_call_spawned.load(Ordering::SeqCst),
            "no second mailbox must be constructed for a live triple"
        );
        assert!(
            erased.downcast::<String>().is_err(),
            "downcasting a usize-backed entry as String must fail closed, not fall through"
        );

        let re_lookup = match registry.lookup_or_insert("triple-3", || panic!("must not spawn again")) {
            RouteOutcome::Existing { mailbox } => mailbox,
            RouteOutcome::Inserted { .. } => panic!("triple-3 must still be live"),
        };
        assert!(
            Arc::ptr_eq(&original, &re_lookup),
            "the original entry must be untouched by the failed mismatch lookup"
        );
    }

    #[test]
    fn deactivate_if_mine_is_a_noop_for_a_stale_epoch() {
        let registry = EntityRegistry::new();
        let epoch = match registry.lookup_or_insert("triple-4", || erased_probe(1usize)) {
            RouteOutcome::Inserted { epoch, .. } => epoch,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        registry.deactivate_if_mine("triple-4", epoch + 1);
        assert!(
            registry.lookup("triple-4").is_some(),
            "a stale epoch's removal attempt must not remove the live entry"
        );

        registry.deactivate_if_mine("triple-4", epoch);
        assert!(
            registry.lookup("triple-4").is_none(),
            "the current epoch's removal attempt must remove the entry"
        );
    }
}
