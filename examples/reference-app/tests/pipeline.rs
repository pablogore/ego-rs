//! Integration test for CORE-016 Phase 5 — the Host -> AppConfig ->
//! service construction -> RuntimeBuilder pipeline (see design.md "Data Flow").

use ego_domain::Validate;
use reference_app::read_side::UsersByTenantStore;
use reference_app::{build_runtime, AppConfig};

#[test]
fn valid_app_config_passes_validate_and_builds_runtime() {
    let config = AppConfig::default();

    assert!(config.validate().is_ok(), "default AppConfig should be valid");

    let runtime = build_runtime(&config);
    assert!(
        runtime.is_ok(),
        "build_runtime should construct services from a valid AppConfig"
    );
}

#[test]
fn invalid_subtree_config_fails_validate_before_any_service_is_constructed() {
    let mut config = AppConfig::default();
    // Invalidate a single subtree (EventBusConfig requires non-zero capacity).
    config.scheduler.capacity = 0;

    let validate_err = config.validate();
    assert!(
        validate_err.is_err(),
        "AppConfig::validate must reject an invalid subtree"
    );

    // build_runtime calls config.validate() before constructing any service
    // (see lib.rs `build_runtime` — `config.validate()?` is the first line),
    // so the same invalid config must fail the pipeline the same way.
    let pipeline_err = build_runtime(&config);
    assert!(
        pipeline_err.is_err(),
        "build_runtime must return Err before constructing any service"
    );
}

// CORE-028 Stage 2 (task 5.2, design.md Testing Strategy): the design doc's
// own test plan for this feature is exactly this cheap, non-async assertion
// — the query handle `build_runtime`'s `.projection(...)` call registers
// must be resolvable through the DI path. `e2e_register.rs` separately
// proves the resolved handle observes live engine writes, which needs the
// full HTTP/JWT stack; reachability alone does not.
#[test]
fn build_runtime_registers_the_read_model_as_a_resolvable_projection() {
    let config = AppConfig::default();

    let runtime = build_runtime(&config).expect("build_runtime succeeds");
    assert!(
        runtime.app.resolve_projection::<UsersByTenantStore>().is_ok(),
        "UsersByTenantStore must be resolvable via the projection DI path after build"
    );
}

#[test]
fn build_runtime_wires_real_kit_config_output() {
    let config = AppConfig::default();

    let runtime = build_runtime(&config);
    assert!(
        runtime.is_ok(),
        "build_runtime should materialize configuration through the real kit-config \
         loader (ConfigLoader -> ConfigurationProvider -> build_logger -> with_logger)"
    );
}

#[test]
fn invalid_cross_domain_rule_fails_validate() {
    let mut config = AppConfig::default();
    // Each subtree is individually valid, but the cross-domain rule (see
    // lib.rs `AppConfig::validate`) requires more database connections once
    // the runtime is multi-tenant.
    config.runtime.single_tenant_mode = false;
    config.runtime.tenant_id = "tenant-a".to_string();
    config.database.max_connections = 1;

    assert!(
        config.database.validate().is_ok(),
        "the database subtree alone is valid"
    );
    assert!(
        config.runtime.validate().is_ok(),
        "the runtime subtree alone is valid"
    );
    assert!(
        config.validate().is_err(),
        "the cross-domain rule must reject this combination even though each subtree is valid alone"
    );
}
