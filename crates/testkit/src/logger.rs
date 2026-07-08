//! Capturing logger — [`CapturingLogger`] and [`CapturedRecord`] (CORE-022
//! Phase 7, design.md AD-6).
//!
//! `CapturingLogger` builds a real `Arc<KITLogger>` — the exact type a
//! service receives via `ServiceContext::with_logger`/`RuntimeBuilder::with_logger`
//! — and redirects its `ConsoleExporterImpl` writers into an in-memory
//! buffer so a test can inspect what was logged. There is no fake logger and
//! no parallel trait: `kitlogger`'s construction API
//! (`KITLogger::with_exporter_and_format`) takes a concrete
//! `Arc<ConsoleExporterImpl>`, so capture happens purely on the
//! already-serialized output side.

use std::collections::HashMap;
use std::io::Write;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use console_exporter::ConsoleExporterImpl;
use kitlogger::KITLogger;
use kitlogger_formatter::LogFormat;
use kitlogger_log_domain::Severity;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Grounding check (task 7.1) — confirmed against the real `kitlogger`
// `develop` source checked out locally at commit `bfb30ae`
// (`~/.cargo/git/checkouts/kitlogger-*/bfb30ae/`), matching
// `Cargo.lock`'s pinned rev (`bfb30ae45e255deba6417268eebf0f84a7240cbb`).
// ---------------------------------------------------------------------------
//
// (a) JSON keys (`kitlogger-formatter/src/json.rs::JsonFormatter::format`):
//     the formatter does NOT wrap structured data under a `"fields"` key.
//     It emits a single flat object, in order: `"ts"` (RFC3339 string),
//     `"level"` (uppercase severity label, e.g. `"INFO"`, `"FATAL"`),
//     `"msg"` (the message string), an optional `"logger"` key (from
//     `LogContext`'s `"logger"` attribute, if present), and then every
//     `LogRecord` attribute and `LogContext` attribute (excluding
//     `"logger"`) flattened directly at the TOP level, e.g.
//     `{"ts":"...","level":"INFO","msg":"login ok","service":"api"}`.
//     Design.md's Open Questions anticipated needing to confirm this exact
//     shape before parsing — confirmed: `CapturedRecord::fields` is
//     reconstructed here by taking every key except `ts`/`level`/`msg`/
//     `logger`, not by reading a `"fields"` key (none exists).
//
// (b) Two logging entry points on `KITLogger` (`kitlogger/src/lib.rs`):
//     - `log_record(&self, record: &LogRecord, context: Option<&LogContext>)
//       -> Result<(), AdapterError>` — the structured, field-carrying entry
//       point design.md hinted at as "`log_record`-style". Confirmed exact
//       name and signature.
//     - `log(&self, severity: Severity, message: &str) -> Result<(),
//       AdapterError>` — the simple path.
//     No third public logging entry point exists on `KITLogger`; AD-6's
//     "exactly two documented entry points" is confirmed accurate.
//
// (c) DISCREPANCY vs. design.md (documented per task 7.1's explicit
//     instruction, not silently adapted): design.md's AD-6 states the
//     simple `log(Severity, &str)` path "still serializes through the same
//     formatter ... but it carries no structured fields". This is
//     **incorrect** for the real source at this commit. `KITLogger::log`'s
//     own doc comment reads "back-compat path, no formatter involved", and
//     its body confirms this:
//     `self.exporter.export(message, severity)` — it calls the exporter
//     directly, completely bypassing `self.formatter`. The formatter (and
//     therefore the JSON envelope carrying `"level"`) is applied ONLY by
//     `log_record`. `ConsoleExporterImpl::export` then routes the raw
//     `message` string to one of exactly two physical streams
//     (`stdout`/`stderr`) based on `Severity` via `StreamRouter`
//     (`console-exporter/src/stream_router.rs`), and writes only the raw
//     message text — the `Severity` itself is never encoded into the
//     written bytes, only used to pick a destination stream. Both entry
//     points still route through this same `ConsoleExporterImpl`, so
//     AD-6's "two documented entry points, no more no less" claim holds —
//     but the claim that both are JSON-formatted does not.
//
//     Consequence for `CapturedRecord`: for the simple `log(Severity, &str)`
//     path, the exact `Severity` is **not recoverable** from the captured
//     bytes — `ConsoleExporterImpl` has exactly two output streams, so at
//     best a writer-side capture could distinguish two severity buckets
//     (stdout vs. stderr), never the original six-value `Severity`. Rather
//     than guess a specific `Severity` and risk silently reporting a wrong
//     level (explicitly forbidden by task 7.1: "do not guess field names
//     and hope"), `CapturedRecord::level` is `Option<Severity>`: `Some(_)`
//     for records recovered from a `log_record` (JSON) line, `None` for
//     lines that came from the back-compat `log` path (message and empty
//     `fields` are still faithfully recovered for that path).
//
// (d) `ConsoleExporterImpl::set_writers(&self, stdout: Box<dyn Write +
//     Send>, stderr: Box<dyn Write + Send>)` (`console-exporter/src/exporter.rs`)
//     — confirmed real signature; takes `&self` (internally
//     `Mutex`-guarded), so it can be called on an already-`Arc`-wrapped
//     exporter before handing that `Arc` to `KITLogger::with_exporter_and_format`.
//     `KITLogger::with_exporter_and_format(exporter: Arc<ConsoleExporterImpl>,
//     format: LogFormat) -> Self` is the confirmed real constructor name —
//     its own doc comment reads "Intended for testing: callers supply a
//     `ConsoleExporterImpl` with custom `set_writers` capture buffers
//     already attached and initialized," which is exactly the sequence
//     `CapturingLogger::new` follows below. `ConsoleExporterImpl::init()`
//     must also be called — a fresh exporter starts `Uninitialized`, and
//     `export()` rejects writes until `init()` transitions it to `Running`
//     (`console-exporter/src/lifecycle.rs`).

/// A single log record recovered from a [`CapturingLogger`]'s buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedRecord {
    /// The severity the service logged at, when recoverable.
    ///
    /// `Some(_)` for records logged through `KITLogger::log_record` (the
    /// structured path, which the real JSON formatter renders). `None` for
    /// records logged through the back-compat `KITLogger::log(Severity,
    /// &str)` path — that path bypasses the formatter entirely and writes
    /// only the raw message, so the exact `Severity` is not present in the
    /// captured bytes (see grounding note (c) above).
    pub level: Option<Severity>,
    /// The logged message text.
    pub message: String,
    /// Structured fields attached to the record. Always empty for records
    /// captured via the back-compat `log(Severity, &str)` path, since that
    /// path carries no structured data — this is what the service actually
    /// emitted, not a `CapturingLogger` limitation.
    pub fields: HashMap<String, Value>,
}

/// A [`Write`] sink that appends every write into a shared, mutex-guarded
/// buffer. Two independent instances never share a buffer.
struct SharedBufferWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer mutex is never poisoned in tests")
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A real [`KITLogger`] whose output is captured in memory instead of going
/// to the real console, so a test can inspect what a service logged.
///
/// The logger handed out via [`CapturingLogger::logger`] is the same
/// `Arc<KITLogger>` a production `ServiceContext`/`Runtime` would carry —
/// there is no fake logger or parallel logging trait (design.md AD-6).
/// Capture is purely on the writer side: `CapturingLogger` owns its own
/// `ConsoleExporterImpl` and buffer, so two instances never cross-capture.
pub struct CapturingLogger {
    logger: Arc<KITLogger>,
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CapturingLogger {
    /// Builds a real `KITLogger` (JSON format) wired to a fresh, isolated
    /// in-memory buffer instead of the real console.
    pub fn new() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let exporter = Arc::new(ConsoleExporterImpl::new());
        exporter.set_writers(
            Box::new(SharedBufferWriter(buffer.clone())),
            Box::new(SharedBufferWriter(buffer.clone())),
        );
        exporter
            .init()
            .expect("a freshly constructed ConsoleExporterImpl always initializes");
        let logger = Arc::new(KITLogger::with_exporter_and_format(
            exporter,
            LogFormat::Json,
        ));
        Self { logger, buffer }
    }

    /// The real logger to hand to a service under test, e.g. via
    /// `ServiceContext::with_logger`/`RuntimeBuilder::with_logger`.
    pub fn logger(&self) -> Arc<KITLogger> {
        self.logger.clone()
    }

    /// Parses every line captured so far into a [`CapturedRecord`], in the
    /// order they were logged.
    pub fn records(&self) -> Vec<CapturedRecord> {
        let buffer = self
            .buffer
            .lock()
            .expect("capture buffer mutex is never poisoned in tests");
        String::from_utf8_lossy(&buffer)
            .lines()
            .map(parse_captured_line)
            .collect()
    }
}

impl Default for CapturingLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses one captured output line into a [`CapturedRecord`].
///
/// See grounding notes (a) and (c) at the top of this file for the exact
/// JSON shape and the back-compat fallback this implements.
///
/// Caveat inherited from kitlogger's flat JSON envelope, not introduced here:
/// `ts`/`level`/`msg`/`logger` share the same object as every logged
/// attribute, with no reserved-name protection. A structured attribute
/// literally named one of those keys collides with the fixed field and the
/// last-written value wins during JSON parsing — this can silently corrupt
/// `level`/`message` recovery for that one record.
fn parse_captured_line(line: &str) -> CapturedRecord {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(mut obj)) => {
            let level = obj
                .remove("level")
                .and_then(|v| v.as_str().map(str::to_owned))
                .and_then(|s| Severity::from_str(&s).ok());
            let message = obj
                .remove("msg")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            obj.remove("ts");
            obj.remove("logger");
            let fields = obj.into_iter().collect();
            CapturedRecord {
                level,
                message,
                fields,
            }
        }
        // Back-compat `log(Severity, &str)` path: no formatter, no JSON —
        // the raw message is exactly what was written (grounding note (c)).
        _ => CapturedRecord {
            level: None,
            message: line.to_string(),
            fields: HashMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitlogger_log_domain::{LogAttribute, LogAttributeValue, LogRecord};
    use std::time::SystemTime;

    #[test]
    fn log_record_captures_matching_level_message_and_fields() {
        let capturing = CapturingLogger::new();
        let record = LogRecord::new(
            SystemTime::now(),
            Severity::Info,
            "login ok".to_string(),
            vec![LogAttribute::new(
                "service".to_string(),
                LogAttributeValue::String("api".to_string()),
            )
            .unwrap()],
        )
        .unwrap();

        capturing.logger().log_record(&record, None).unwrap();

        let records = capturing.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Some(Severity::Info));
        assert_eq!(records[0].message, "login ok");
        assert_eq!(
            records[0].fields.get("service"),
            Some(&Value::String("api".to_string()))
        );
    }

    #[test]
    fn simple_log_path_captures_message_with_empty_fields() {
        // Grounding note (c): KITLogger::log bypasses the formatter, so the
        // exact Severity is not recoverable from the captured bytes — only
        // message and (necessarily empty) fields are.
        let capturing = CapturingLogger::new();

        capturing
            .logger()
            .log(Severity::Warn, "disk almost full")
            .unwrap();

        let records = capturing.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, None);
        assert_eq!(records[0].message, "disk almost full");
        assert!(records[0].fields.is_empty());
    }

    #[test]
    fn independent_instances_never_cross_capture() {
        let a = CapturingLogger::new();
        let b = CapturingLogger::new();

        a.logger().log(Severity::Info, "from a").unwrap();
        b.logger().log(Severity::Info, "from b").unwrap();

        let a_records = a.records();
        let b_records = b.records();
        assert_eq!(a_records.len(), 1);
        assert_eq!(a_records[0].message, "from a");
        assert_eq!(b_records.len(), 1);
        assert_eq!(b_records[0].message, "from b");
    }
}
