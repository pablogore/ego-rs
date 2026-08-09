mod builder;
mod config_provider;
mod error;
mod idempotency;
mod logger;
mod permit;
mod resolvable;
mod runtime_builder;
mod tenant;

pub use builder::{Runtime, RuntimeBuilder, RuntimeResolver};
pub use config_provider::{ConfigurationProvider, LogFormatSetting, LoggingSettings};
pub use error::RuntimeInfraError;
pub use idempotency::{
    IdempotencyEnforcementMode, ReservationConfig, ReservationConfigError, ReservationDecision,
    ReservationPermit, ReservationRejection, StoredResponseError,
};
pub use logger::build_logger;
pub use permit::CrossTenantPermit;
pub use resolvable::{HasServiceTag, Resolvable, ResolvableContainer};
pub use runtime_builder::{DependencyKind, RuntimeError, RuntimeInner, SecurityDenialKind};
pub use tenant::{CanonicalTenant, TenantEnforcementMode, TenantResolver};
// Crate-internal only (AD-014 Established Fact type) — `crate::context`
// needs `CrossTenantGrant` to expose `ServiceContext::cross_tenant_grant`,
// which is not part of this crate's external API. `EstablishedTenantFacts`
// stays a `crate::runtime`-internal detail: every caller that needs it
// (`runtime_builder::enforce_tenant`, `tenant`'s own tests) already lives in
// this module tree and imports it directly via `super::tenant::...`
// (code-review fix — no crate-wide re-export needed).
pub(crate) use tenant::CrossTenantGrant;

/// CORE-017 Phase 5 integration tests (TASK-021/TASK-022).
///
/// Per the testing skill's Decision Gates table, these exercise only
/// in-memory state (kitlogger's capture-buffer exporter, `serde_json::json!`
/// values) — no real DB/broker/HTTP — so they live as ordinary
/// `#[cfg(test)]` modules here, which is where anything needing no
/// infrastructure belongs.
#[cfg(test)]
mod integration_tests {
    use super::IdempotencyEnforcementMode;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use console_exporter::ConsoleExporterImpl;
    use kitlogger::KITLogger;
    use kitlogger_formatter::LogFormat;
    use kitlogger_log_domain::Severity;
    use serde_json::json;

    use super::{build_logger, ConfigurationProvider, RuntimeBuilder};
    use crate::context::ServiceContext;

    #[derive(Clone, Default)]
    struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// TASK-021: bootstrap path only.
    ///
    /// `ConfigurationProvider::from_value(..)` -> `.logging()` -> `build_logger(..)`
    /// -> `RuntimeBuilder::new().with_logger(logger).build()` ->
    /// `ServiceContext::new().with_logger(rt.logger().unwrap().clone())`.
    ///
    /// Uses the public `Runtime::logger()` facade, not the hidden
    /// `RuntimeInner::logger()` — this test demonstrates the same path
    /// application code should follow (see `examples/logging_bootstrap.rs`).
    ///
    /// Manual `ServiceContext::new().with_logger(..)` construction is
    /// intentional here — there is no generated-dispatcher path that
    /// assembles `ServiceContext` today; `examples/reference-app` constructs
    /// `.with_security(..)` the same manual way.
    #[test]
    fn bootstrap_path_wires_logger_from_config_to_service_context() {
        let provider = ConfigurationProvider::from_value(json!({
            "logging": { "enabled": true, "format": "json" }
        }));
        let settings = provider.logging().expect("valid logging config");
        let logger = build_logger(&settings)
            .expect("logger constructs")
            .expect("enabled settings yield a logger");

        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_logger(logger)
            .build();
        let rt_logger = rt.logger().expect("runtime holds the logger").clone();

        let ctx = ServiceContext::new().with_logger(rt_logger.clone());

        assert!(ctx.logger().is_some());
        assert!(Arc::ptr_eq(
            ctx.logger.as_ref().expect("ctx holds the logger"),
            &rt_logger
        ));
    }

    /// TASK-022: shutdown path only, isolated from TASK-021 so a failure
    /// points at one component instead of six.
    ///
    /// Builds a `Runtime` with `.with_logger(..)` wired to a capture-buffer
    /// exporter, calls `rt.shutdown()`, and asserts the buffer received every
    /// record logged before shutdown (no lost records). Per
    /// `ConsoleExporterImpl::export`, each `log()` call writes to the router
    /// synchronously — `shutdown()`'s flush is a documented no-op stub — so
    /// "no lost records" here means shutdown does not corrupt or drop what
    /// was already written, and the pipeline is still fully drained/closed.
    #[test]
    fn shutdown_path_flushes_capture_buffer_with_no_lost_records() {
        let exporter = Arc::new(ConsoleExporterImpl::new());
        let stdout = CaptureBuffer::default();
        exporter.set_writers(Box::new(stdout.clone()), Box::new(CaptureBuffer::default()));
        exporter.init().expect("capture exporter initializes");

        let logger = Arc::new(KITLogger::with_exporter_and_format(
            exporter,
            LogFormat::Json,
        ));
        logger
            .log(Severity::Info, "record-1")
            .expect("record 1 logs");
        logger
            .log(Severity::Info, "record-2")
            .expect("record 2 logs");
        logger
            .log(Severity::Info, "record-3")
            .expect("record 3 logs");

        let rt = RuntimeBuilder::new()
            .with_idempotency_enforcement_mode(IdempotencyEnforcementMode::Compatibility)
            .with_logger(logger)
            .build();

        assert!(rt.shutdown().is_ok());

        let captured = stdout.0.lock().unwrap();
        let text = String::from_utf8_lossy(&captured);
        assert!(text.contains("record-1"), "record-1 must not be lost");
        assert!(text.contains("record-2"), "record-2 must not be lost");
        assert!(text.contains("record-3"), "record-3 must not be lost");
    }
}
