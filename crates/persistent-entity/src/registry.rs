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
use crate::scheduler::EntityTriple;

const MAX_PASSIVATED_ENTRIES: usize = 10_000;

/// A live routing entry: the entity's type-erased mailbox handle, the
/// actor-published lifecycle state (read-only from the registry's side), a
/// monotonic epoch identifying which activation created it (ADR-005), and
/// the triple that created it — the authoritative owner, checked against the
/// caller's own triple by SEC-001's defence-in-depth routing check.
struct ActiveEntry {
    mailbox: Arc<dyn Any + Send + Sync>,
    rx: watch::Receiver<EntityState>,
    epoch: u64,
    triple: EntityTriple,
}

/// Outcome of [`EntityRegistry::lookup_or_insert`]'s single-flight critical section.
pub enum RouteOutcome {
    /// A live entry already existed; the caller reuses its mailbox instead of spawning.
    Existing {
        /// The existing entry's type-erased mailbox handle.
        mailbox: Arc<dyn Any + Send + Sync>,
        /// The triple recorded at this entry's insert time — the
        /// authoritative owner. Callers must compare this against the
        /// triple they looked up under and refuse to route on a mismatch
        /// (SEC-001 defence-in-depth): a regression that ever keys the map
        /// by something lossier than the full triple must fail loudly
        /// instead of handing back another triple's live actor.
        owner: EntityTriple,
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
///
/// Entries are keyed by the full [`EntityTriple`] — tenant, entity type, and
/// entity id together — never by the bare `aggregate_id` (which drops the
/// tenant). This means two [`crate::runtime::EntityRuntime`]s pinned to
/// different tenants may safely share one registry: their triples never
/// collide, even when entity type and entity id happen to match (SEC-001).
pub struct EntityRegistry {
    /// Live routing entries, keyed by the full entity triple.
    active: Mutex<HashMap<EntityTriple, ActiveEntry>>,
    /// Monotonic counter stamping each insert with a unique epoch — ABA-safe
    /// teardown identity (ADR-005).
    next_epoch: AtomicU64,
    /// Entities that have passivated (triple → final version). Advisory
    /// bookkeeping only — never gates or forks routing (ADR-004).
    passivated_entities: Arc<std::sync::Mutex<HashMap<EntityTriple, u64>>>,
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
    /// returns the existing entry's erased mailbox if `triple` already has
    /// a live entry, otherwise lazily calls `make_mailbox` (never invoked on
    /// the hit path), inserts a new entry seeded `Recovering`, and hands the
    /// caller its epoch plus the sole [`watch::Sender`] for that entry.
    ///
    /// `make_mailbox` runs synchronously, still under the lock — it must not
    /// `.await` or panic-prone-ly do more than construct a mailbox, matching
    /// ADR-001's "no `.await`, no `tokio::spawn`" critical-section contract.
    pub fn lookup_or_insert<F>(&self, triple: &EntityTriple, make_mailbox: F) -> RouteOutcome
    where
        F: FnOnce() -> Arc<dyn Any + Send + Sync>,
    {
        let mut active = self.active.lock();
        if let Some(entry) = active.get(triple) {
            return RouteOutcome::Existing {
                mailbox: entry.mailbox.clone(),
                owner: entry.triple.clone(),
            };
        }

        let mailbox = make_mailbox();
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = watch::channel(EntityState::Recovering);
        active.insert(
            triple.clone(),
            ActiveEntry {
                mailbox: mailbox.clone(),
                rx,
                epoch,
                triple: triple.clone(),
            },
        );
        RouteOutcome::Inserted { mailbox, epoch, tx }
    }

    /// Returns the live entry's erased mailbox handle, if one exists, without
    /// inserting anything. Presence-only lookup (ADR-008): sufficient for
    /// retry logic that only needs to know "is there still something here."
    pub fn lookup(&self, triple: &EntityTriple) -> Option<Arc<dyn Any + Send + Sync>> {
        self.active
            .lock()
            .get(triple)
            .map(|entry| entry.mailbox.clone())
    }

    /// Removes `triple`'s routing entry only if it is still owned by
    /// `epoch` — "removal is authority-scoped" (FR-001). A stale or
    /// superseded exit path's call is a safe no-op.
    pub fn deactivate_if_mine(&self, triple: &EntityTriple, epoch: u64) {
        let mut active = self.active.lock();
        let is_mine = matches!(active.get(triple), Some(entry) if entry.epoch == epoch);
        if is_mine {
            active.remove(triple);
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
    pub fn mark_passivated(&self, triple: EntityTriple, version: u64) {
        let mut passivated = self.passivated_entities.lock().unwrap();
        if passivated.len() >= MAX_PASSIVATED_ENTRIES {
            if let Some(oldest) = passivated.keys().next().cloned() {
                passivated.remove(&oldest);
            }
        }
        passivated.insert(triple, version);
    }

    /// Test-only backdoor: inserts a live entry keyed by `key` but recording
    /// `owner` as its authoritative triple — the two are normally identical
    /// (see [`Self::lookup_or_insert`]), and this is the only way to build
    /// the mismatched state a routing-key regression would produce, to prove
    /// the defence-in-depth check in `entity_ref_tokio.rs` actually fires.
    #[cfg(test)]
    pub(crate) fn insert_mismatched_owner_for_test(
        &self,
        key: EntityTriple,
        owner: EntityTriple,
        mailbox: Arc<dyn Any + Send + Sync>,
    ) {
        let mut active = self.active.lock();
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let (_tx, rx) = watch::channel(EntityState::Recovering);
        active.insert(
            key,
            ActiveEntry {
                mailbox,
                rx,
                epoch,
                triple: owner,
            },
        );
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

    /// Builds a test triple for a fixed tenant — the tenant only matters for
    /// the SEC-001 cross-tenant test below; every other test just needs a
    /// stable identity to key the registry by.
    fn triple(id: &str) -> EntityTriple {
        EntityTriple::new("tenant-a".to_string(), "kind", id)
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
                match registry.lookup_or_insert(&triple("triple-1"), || {
                    spawn_count.fetch_add(1, Ordering::SeqCst);
                    erased_probe(42usize)
                }) {
                    RouteOutcome::Existing { mailbox, .. } => mailbox,
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

        let tx = match registry.lookup_or_insert(&triple("triple-2"), || erased_probe(0usize)) {
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
        let (epoch1, tx1) =
            match registry.lookup_or_insert(&triple("triple-5"), || erased_probe(0usize)) {
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
        registry.mark_passivated(triple("triple-5"), 1);
        registry.deactivate_if_mine(&triple("triple-5"), epoch1);
        assert_eq!(
            registry.active_count(),
            0,
            "the torn-down entry must no longer be counted"
        );

        // Reactivation: a fresh lookup_or_insert for the same entity_id
        // starts a new Recovering entry under a new epoch.
        let tx2 = match registry.lookup_or_insert(&triple("triple-5"), || erased_probe(0usize)) {
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

        let original = match registry.lookup_or_insert(&triple("triple-3"), || erased_probe(7usize))
        {
            RouteOutcome::Inserted { mailbox, .. } => mailbox,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        let second_call_spawned = AtomicBool::new(false);
        let erased = match registry.lookup_or_insert(&triple("triple-3"), || {
            second_call_spawned.store(true, Ordering::SeqCst);
            erased_probe(String::from("wrong-type"))
        }) {
            RouteOutcome::Existing { mailbox, .. } => mailbox,
            RouteOutcome::Inserted { .. } => {
                panic!("must find the live entry, not insert a new one")
            }
        };
        assert!(
            !second_call_spawned.load(Ordering::SeqCst),
            "no second mailbox must be constructed for a live triple"
        );
        assert!(
            erased.downcast::<String>().is_err(),
            "downcasting a usize-backed entry as String must fail closed, not fall through"
        );

        let re_lookup = match registry
            .lookup_or_insert(&triple("triple-3"), || panic!("must not spawn again"))
        {
            RouteOutcome::Existing { mailbox, .. } => mailbox,
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
        let epoch = match registry.lookup_or_insert(&triple("triple-4"), || erased_probe(1usize)) {
            RouteOutcome::Inserted { epoch, .. } => epoch,
            RouteOutcome::Existing { .. } => panic!("expected a fresh insert"),
        };

        registry.deactivate_if_mine(&triple("triple-4"), epoch + 1);
        assert!(
            registry.lookup(&triple("triple-4")).is_some(),
            "a stale epoch's removal attempt must not remove the live entry"
        );

        registry.deactivate_if_mine(&triple("triple-4"), epoch);
        assert!(
            registry.lookup(&triple("triple-4")).is_none(),
            "the current epoch's removal attempt must remove the entry"
        );
    }

    /// TASK-024 (poison safety, design.md Testing Strategy): a panic from
    /// `make_mailbox` fires WHILE `lookup_or_insert`'s `parking_lot::Mutex`
    /// guard is still held (the panic unwinds through the lock, not after
    /// it's released — unlike the Round-3 `tokio::spawn`-outside-runtime
    /// scenario covered elsewhere). `parking_lot::Mutex` does not poison on
    /// panic, so the registry must remain fully usable for every other
    /// triple afterward.
    #[test]
    fn panic_inside_the_critical_section_does_not_poison_the_registry() {
        let registry = EntityRegistry::new();

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.lookup_or_insert(&triple("triple-poison"), || {
                panic!("boom: construction failure")
            });
        }));
        assert!(
            panicked.is_err(),
            "the panic must propagate, not be swallowed"
        );

        // The lock must not be poisoned: another triple's lookup_or_insert
        // must still succeed, and the panicking triple must have left no
        // partial entry behind (make_mailbox panicked before `active.insert`).
        assert!(
            registry.lookup(&triple("triple-poison")).is_none(),
            "a panic during construction must not leave a partial entry"
        );

        let outcome = registry.lookup_or_insert(&triple("triple-other"), || erased_probe(7usize));
        let mailbox = match outcome {
            RouteOutcome::Inserted { mailbox, .. } => mailbox,
            RouteOutcome::Existing { .. } => {
                panic!("expected a fresh insert for an unrelated triple")
            }
        };
        assert_eq!(
            *mailbox.downcast::<usize>().expect("erased as usize"),
            7,
            "an unrelated triple must activate normally after the other triple's construction panic"
        );
        assert_eq!(
            registry.active_count(),
            0,
            "triple-other is Recovering, not yet Active — active_count must still be usable post-panic"
        );
    }

    /// SEC-001: two triples that share `entity_type` and `entity_id` but
    /// belong to different tenants must route to two distinct entries, never
    /// alias onto the same live actor. This is what makes it safe for
    /// runtimes pinned to different tenants to share one registry.
    #[test]
    fn same_type_and_id_different_tenant_are_distinct_entries() {
        let registry = EntityRegistry::new();
        let tenant_a = EntityTriple::new("tenant-a".to_string(), "kind", "shared-id");
        let tenant_b = EntityTriple::new("tenant-b".to_string(), "kind", "shared-id");

        match registry.lookup_or_insert(&tenant_a, || erased_probe(1usize)) {
            RouteOutcome::Inserted { .. } => {}
            RouteOutcome::Existing { .. } => {
                panic!("tenant-a's triple must be a fresh insert")
            }
        }

        match registry.lookup_or_insert(&tenant_b, || erased_probe(2usize)) {
            RouteOutcome::Inserted { .. } => {}
            RouteOutcome::Existing { .. } => {
                panic!(
                    "tenant-b's triple must also be a fresh insert — sharing entity_type and \
                     entity_id with tenant-a must not alias onto tenant-a's live entry (SEC-001)"
                )
            }
        }
    }
}
