mod common;

use ego_infrastructure::persistence::in_memory::InMemoryRepository;

#[test]
fn in_memory_repository_passes_contract_tests() {
    let repo = InMemoryRepository::<String>::new();
    common::repository_contract_tests(repo);
}
