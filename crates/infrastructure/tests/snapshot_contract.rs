mod common;

use ego_infrastructure::persistence::in_memory::InMemorySnapshotStore;

#[test]
fn in_memory_snapshot_store_passes_contract_tests() {
    let store = InMemorySnapshotStore::new();
    common::snapshot_contract_tests(store);
}
