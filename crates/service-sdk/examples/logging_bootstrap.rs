//! Logging Bootstrap Example — CORE-017 config-to-logger host flow.
//!
//! This example demonstrates the intended host bootstrap sequence:
//!
//! ```ignore
//! // 1. Host materializes config (kit_config::ConfigLoader, external — simulated
//! //    here with a plain serde_json::Value)
//! // 2. ConfigurationProvider::from_value(..).logging() -> LoggingSettings
//! // 3. build_logger(&settings) -> Option<Arc<KITLogger>>, BEFORE RuntimeBuilder::new()
//! // 4. RuntimeBuilder::new().with_logger(logger).build() -> Runtime
//! // 5. Services obtain the logger via ServiceContext, never by constructing it
//! ```
//!
//! ## Note — `format` only applies through `log_record`, not `log`
//!
//! `KITLogger::log(severity, message)` is documented upstream as a "back-compat,
//! no formatter involved" path — it writes the raw message regardless of the
//! configured `format`. Only `KITLogger::log_record(&LogRecord, ..)` applies the
//! configured formatter (`json`/`pretty`/`compact`/`text`). This example calls
//! both, side by side, so the difference is visible rather than assumed.

use ego_service_sdk::context::ServiceContext;
use ego_service_sdk::runtime::{build_logger, ConfigurationProvider, RuntimeBuilder};
use kitlogger_log_domain::{LogRecord, Severity};
use serde_json::json;

fn main() {
    // Host materializes config and extracts the logging view.
    // ConfigurationProvider is a thin host-boundary adapter; kit-config remains
    // the canonical configuration model.
    let config = json!({ "logging": { "enabled": true, "format": "pretty" } });
    let provider = ConfigurationProvider::from_value(config);
    let settings = provider.logging().expect("valid logging config");

    // Host constructs the logger BEFORE RuntimeBuilder::new() — RuntimeBuilder
    // receives fully-constructed services rather than configuration objects
    // (CORE-016), never a config object.
    let logger = build_logger(&settings)
        .expect("logger constructs")
        .expect("enabled=true yields a logger");

    // RuntimeBuilder takes ownership; Runtime owns lifecycle from here on.
    let runtime = RuntimeBuilder::new()
        .with_idempotency_enforcement_mode(
            ego_service_sdk::runtime::IdempotencyEnforcementMode::Compatibility,
        )
        .with_logger(logger.clone())
        .build();

    // Services access the logger through ServiceContext, mirroring `security`.
    let ctx = ServiceContext::new().with_logger(logger);
    let logger = ctx.logger().expect("context holds the logger");

    println!("--- via .log() — ignores `format`, always raw ---");
    logger
        .log(Severity::Info, "order created")
        .expect("log succeeds");

    println!("--- via .log_record() — applies `format` (\"pretty\" here) ---");
    let record = LogRecord::new(
        std::time::SystemTime::now(),
        Severity::Info,
        "order created".to_string(),
        vec![],
    )
    .expect("non-empty message");
    logger
        .log_record(&record, None)
        .expect("log_record succeeds");

    // Runtime owns shutdown: flush-then-close, idempotent.
    runtime.shutdown().expect("shutdown succeeds");
}
