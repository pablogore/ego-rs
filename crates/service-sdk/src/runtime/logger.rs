//! Boundary adapter: config view -> real `KITLogger` constructor.
//!
//! [`build_logger`] is the canonical **public** logger construction entry
//! point — the only supported public way to turn a [`LoggingSettings`] view
//! into a `KITLogger`. The restriction is on the public API surface; a
//! private internal helper used only by this function's own implementation
//! would not violate it. Do not add another *public* helper that constructs
//! `KITLogger` by a different path.
//!
//! Called by the HOST, before `RuntimeBuilder::new()`.

use std::sync::Arc;

use kitlogger::KITLogger;
use kitlogger_formatter::LogFormat;

use super::config_provider::{LogFormatSetting, LoggingSettings};
use super::error::RuntimeInfraError;

/// Turns a [`LoggingSettings`] view into a running `Arc<KITLogger>`.
///
/// Returns `Ok(None)` when `enabled == false` — the only "off" mechanism
/// kitlogger's API supports.
pub fn build_logger(cfg: &LoggingSettings) -> Result<Option<Arc<KITLogger>>, RuntimeInfraError> {
    if !cfg.enabled {
        return Ok(None);
    }
    let fmt = match cfg.format {
        LogFormatSetting::Json => LogFormat::Json,
        LogFormatSetting::Pretty => LogFormat::HumanReadable,
        LogFormatSetting::Compact => LogFormat::Text,
        LogFormatSetting::Text => LogFormat::Text,
    };
    let logger = KITLogger::with_format(fmt);
    // KITLogger::init() -> Result<(), AdapterError>; AdapterError is not re-exported,
    // so map by Debug at the boundary rather than naming the type.
    logger
        .init()
        .map_err(|e| RuntimeInfraError::LoggerInit { reason: format!("{e:?}") })?;
    Ok(Some(Arc::new(logger)))
}

/// Ordered teardown: LIFO over the console-exporter's flush-on-shutdown.
///
/// Held behind `Mutex<TeardownStack>` in `RuntimeInner` — `RuntimeInner` is
/// always shared via `Arc`, so `Runtime::shutdown(&self)` needs interior
/// mutability to drain it.
pub(super) struct TeardownStack {
    entries: Vec<Arc<KITLogger>>,
}

impl TeardownStack {
    pub(super) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(super) fn push(&mut self, l: Arc<KITLogger>) {
        self.entries.push(l);
    }

    /// Reverse construction order. Collects the first error but shuts down all.
    /// Idempotent: a second call drains an already-empty stack and returns `Ok(())`.
    pub(super) fn drain(&mut self) -> Result<(), RuntimeInfraError> {
        let mut first_err = None;
        while let Some(l) = self.entries.pop() {
            // LIFO = reverse order
            if let Err(e) = l.shutdown() {
                // flush-then-close (OnShutdownFlush)
                first_err.get_or_insert(RuntimeInfraError::Teardown { reason: format!("{e:?}") });
            }
        }
        first_err.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use console_exporter::ConsoleExporterImpl;
    use kitlogger_log_domain::Severity;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[test]
    fn build_logger_maps_each_format() {
        let cases = [
            (LogFormatSetting::Json, "json"),
            (LogFormatSetting::Pretty, "pretty"),
            (LogFormatSetting::Compact, "compact"),
            (LogFormatSetting::Text, "text"),
        ];
        for (format, label) in cases {
            let settings = LoggingSettings { enabled: true, format };
            let result = build_logger(&settings);
            assert!(result.is_ok(), "format {label} should construct a logger");
            assert!(result.unwrap().is_some(), "format {label} should yield Some(logger)");
        }
    }

    #[test]
    fn build_logger_disabled_returns_none_with_no_side_effect() {
        let settings = LoggingSettings {
            enabled: false,
            format: LogFormatSetting::Json,
        };
        let result = build_logger(&settings);
        assert!(matches!(result, Ok(None)));
    }

    /// The real, reachable `AdapterError` -> `LoggerInit` mapping.
    ///
    /// `build_logger` itself cannot be driven into this failure through its
    /// own public signature: it always constructs a *fresh* `KITLogger`
    /// (`with_format`) whose exporter starts `Uninitialized`, so its own
    /// `init()` call always succeeds — there is no seam to pre-fail that
    /// internal exporter before `build_logger` runs (per design.md's Testing
    /// Strategy row 4 / TASK-007's "document as untestable... if no seam
    /// exists" guidance).
    ///
    /// However kitlogger *does* expose a real failure surface via
    /// `KITLogger::with_exporter_and_format`: initializing the same shared
    /// exporter twice is an invalid lifecycle transition
    /// (`Running -> Running`), which is exactly the `AdapterError` shape
    /// `build_logger`'s `logger.init().map_err(..)` line maps to
    /// `RuntimeInfraError::LoggerInit`. This test drives that real failure
    /// (not fabricated) through the identical mapping expression to verify
    /// the mapping compiles and behaves as specified.
    #[test]
    fn logger_init_failure_maps_to_logger_init_error() {
        let exporter = Arc::new(ConsoleExporterImpl::new());
        exporter.init().expect("first init succeeds");

        let logger = KITLogger::with_exporter_and_format(exporter, LogFormat::Json);
        let result = logger
            .init()
            .map_err(|e| RuntimeInfraError::LoggerInit { reason: format!("{e:?}") });

        assert!(matches!(result, Err(RuntimeInfraError::LoggerInit { .. })));
    }

    // -- TeardownStack --------------------------------------------------

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

    fn capture_logger() -> (Arc<KITLogger>, CaptureBuffer) {
        let exporter = Arc::new(ConsoleExporterImpl::new());
        let stdout = CaptureBuffer::default();
        exporter.set_writers(Box::new(stdout.clone()), Box::new(CaptureBuffer::default()));
        exporter.init().expect("capture exporter initializes");
        let logger = Arc::new(KITLogger::with_exporter_and_format(exporter, LogFormat::Json));
        (logger, stdout)
    }

    /// Does NOT verify shutdown call order — see below for why, and don't let
    /// the name mislead you into thinking it does.
    ///
    /// `KITLogger::shutdown()` / kitlogger's console-exporter `flush()` are
    /// currently no-ops with respect to any externally observable side
    /// channel (the real `ConsoleExporterImpl::flush()` body is a documented
    /// stub that always returns `Ok(())` without touching the writers, and
    /// `shutdown()` itself only flips internal lifecycle state) — so there is
    /// no black-box signal available to distinguish "entry A was shut down
    /// before entry B" purely from the outside, unlike what design.md's
    /// Testing Strategy row 6 anticipated ("stub/verify order with a capture
    /// logger"). A spy/mock writer doesn't help either, since `flush()` never
    /// calls the writer.
    ///
    /// What this test actually verifies: `push` preserves insertion order in
    /// `entries` (a `Vec` fact, checked directly against the private field),
    /// and that `drain` performs real teardown — not a no-op — on every
    /// pushed entry. `drain`'s `while let Some(l) = self.entries.pop()` *is*
    /// LIFO by Rust's own `Vec::pop` guarantee, which is why this is
    /// correctness-by-construction rather than something worth chasing a
    /// black-box order assertion for — but be aware this test would still
    /// pass if `drain`'s pop were swapped for a forward iterator, since call
    /// order isn't observed. If real order-observability ever matters (e.g.
    /// once a second infra component exists in `TeardownStack`), that will
    /// require either a genericized `TeardownStack` accepting an injectable
    /// teardown trait, or a kitlogger version whose `flush()` is no longer a
    /// stub.
    #[test]
    fn push_preserves_order_and_drain_tears_down_every_entry() {
        let (logger_a, _buf_a) = capture_logger();
        let (logger_b, _buf_b) = capture_logger();

        let mut stack = TeardownStack::new();
        stack.push(logger_a.clone());
        stack.push(logger_b.clone());

        // Insertion order is preserved; Vec::pop is LIFO, so drain() will
        // process logger_b (last pushed) before logger_a (first pushed).
        assert!(Arc::ptr_eq(&stack.entries[0], &logger_a));
        assert!(Arc::ptr_eq(&stack.entries[1], &logger_b));

        assert!(stack.drain().is_ok());

        // Both loggers are unusable now — drain() performed real teardown,
        // not a no-op, on every entry in the stack.
        assert!(logger_a.log(Severity::Info, "after-shutdown").is_err());
        assert!(logger_b.log(Severity::Info, "after-shutdown").is_err());
    }

    #[test]
    fn drain_is_idempotent() {
        let (logger, _buf) = capture_logger();
        let mut stack = TeardownStack::new();
        stack.push(logger);

        assert_eq!(stack.drain(), Ok(()));
        assert_eq!(stack.drain(), Ok(()));
    }

    /// `RuntimeInfraError::Teardown` real, reachable failure path.
    ///
    /// A never-initialized `KITLogger`'s exporter is `Uninitialized`; calling
    /// `shutdown()` on it is an invalid lifecycle transition
    /// (`Uninitialized -> Flushing`), producing a real `AdapterError` — the
    /// same category of genuine failure `logger_init_failure_maps_to_logger_init_error`
    /// already exercises for `LoggerInit`. This drives `drain()`'s error
    /// branch (`Teardown`) through an actually-reachable path, and confirms
    /// the documented "collects the first error but shuts down all" contract:
    /// a healthy entry pushed alongside the failing one still gets torn down.
    #[test]
    fn drain_surfaces_teardown_error_and_still_shuts_down_the_rest() {
        let failing_logger = Arc::new(KITLogger::default()); // never initialized
        let (healthy_logger, _buf) = capture_logger();

        let mut stack = TeardownStack::new();
        stack.push(failing_logger);
        stack.push(healthy_logger.clone());

        let result = stack.drain();
        assert!(matches!(result, Err(RuntimeInfraError::Teardown { .. })));

        // The healthy entry was still torn down despite the other's failure.
        assert!(healthy_logger.log(Severity::Info, "after-shutdown").is_err());
    }
}
