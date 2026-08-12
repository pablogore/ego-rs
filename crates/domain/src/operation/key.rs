//! `OperationKey` identifies one complete, client-supplied business operation
//! for end-to-end idempotent command processing.
//!
//! Deliberately unrelated to [`crate::idempotency::IdempotencyKey`], which
//! identifies a single external effect dispatch attempt derived from the
//! unit-of-work identity. `OperationKey` is the inverse: it is the
//! caller-supplied identity of one whole business operation, potentially
//! spanning multiple aggregates, and it is never derived or minted
//! server-side — a missing key is rejected, not invented.
//!
//! # Construction
//!
//! Use [`OperationKey::parse`]. It rejects empty or whitespace-only strings
//! and any string longer than [`MAX_LEN`] bytes.

use std::fmt;

/// Maximum length, in bytes, of a valid [`OperationKey`].
pub const MAX_LEN: usize = 255;

/// A client-supplied identity for one complete business operation.
///
/// See the module docs for how this differs from
/// [`crate::idempotency::IdempotencyKey`]. `OperationKey` has no `From`
/// conversion to or from `IdempotencyKey` — the two are deliberately
/// unrelated types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct OperationKey(String);

/// Error returned when a raw string is not a valid [`OperationKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKeyError {
    /// The supplied string was empty or whitespace-only.
    Empty,
    /// The supplied string exceeded [`MAX_LEN`] bytes.
    TooLong,
}

impl fmt::Display for OperationKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationKeyError::Empty => write!(f, "operation_key must not be empty"),
            OperationKeyError::TooLong => {
                write!(f, "operation_key must not exceed {MAX_LEN} bytes")
            }
        }
    }
}

impl std::error::Error for OperationKeyError {}

impl OperationKey {
    /// Parses and validates a raw string into an `OperationKey`.
    ///
    /// Rejects empty/whitespace-only strings and strings longer than
    /// [`MAX_LEN`] bytes. This is the sole constructor — the system never
    /// mints a key on the caller's behalf.
    pub fn parse(value: impl Into<String>) -> Result<Self, OperationKeyError> {
        let s = value.into();
        if s.trim().is_empty() {
            return Err(OperationKeyError::Empty);
        }
        if s.len() > MAX_LEN {
            return Err(OperationKeyError::TooLong);
        }
        Ok(Self(s))
    }

    /// Returns the key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OperationKey {
    type Error = OperationKeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl fmt::Display for OperationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An opaque fingerprint of an operation's content.
///
/// Distinguishes a legitimate replay (same [`OperationKey`], same
/// fingerprint) from a permanent conflict (same key, different fingerprint —
/// see the `idempotent-command-processing` spec's "Fingerprint Determines
/// Replay vs. Conflict" requirement). `OperationFingerprint` carries no
/// opinion about how the fingerprint is computed from a request payload —
/// that is a caller concern; this type only guarantees stable equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OperationFingerprint(String);

impl OperationFingerprint {
    /// Wraps a precomputed fingerprint value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the fingerprint as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How many hex characters of the digest are kept. CORE-019 §12.
const HASH_HEX_LEN: usize = 16;

/// The redacted form of an [`OperationKey`], and the only form telemetry may
/// carry.
///
/// # Why this is a type rather than a formatting rule
///
/// An `OperationKey` is client-supplied and may carry business identifiers — an
/// invoice number, an email, a customer reference. AD-10 forbids emitting it
/// raw, and a rule stated only in prose is a rule every future call site has to
/// remember. This type makes the redaction unavoidable instead: the sole
/// constructor takes an `OperationKey` and hashes it, so there is no path from a
/// raw string to a value telemetry accepts. Nothing here can be built *around*
/// the hashing.
///
/// That is the same posture
/// [`SpanAttributes`](crate::tracer::SpanAttributes) already takes for tenancy —
/// redaction enforced structurally at a type, not by a filter in an adapter that
/// could be bypassed or forgotten.
///
/// # Truncation is deliberate, and it is not a security weakening
///
/// 16 hex characters is 64 bits. That is chosen for *cardinality*, not for
/// preimage resistance: the value exists to group a retry with its original in a
/// trace, and a full 64-character digest would cost four times the bytes on every
/// span for no extra grouping power. It is not a commitment, not a lookup key,
/// and nothing authenticates against it.
///
/// # A span attribute only, never a metric attribute
///
/// The value is unbounded in the number of distinct values it can take — one per
/// operation key — so as a metric dimension it would multiply time series
/// without limit. It is not expressible as one either: [`Observability::metric`]
/// takes a name and a value and has no attribute parameter at all, so this is
/// held by the shape of that port rather than by anyone's discipline.
///
/// [`Observability::metric`]: crate::observability::Observability::metric
///
/// # What this type establishes, and what it does not
///
/// It makes the digest **representable** and the raw key **not** representable
/// wherever an `OperationKeyHash` is required — which, so far, is
/// [`SpanAttributes`](crate::tracer::SpanAttributes). That is a precondition,
/// deliberately landed before any instrumentation exists that could emit a key.
///
/// It is not evidence that idempotency telemetry redacts anything, because no
/// idempotency span is emitted yet. Showing that requires the whole chain —
/// `OperationKey` → `OperationKeyHash` → `SpanAttributes` → an observed
/// exporter — driven from a real reservation, and that arrives with the spans.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperationKeyHash(String);

impl OperationKeyHash {
    /// Hashes `key` and keeps the leading [`HASH_HEX_LEN`] hex characters.
    ///
    /// **The only constructor, deliberately.** There is no `new`, no
    /// `From<String>`, and no way in from an already-rendered value: any of those
    /// would let a raw operation key be passed where a digest is expected, and
    /// the type would then guarantee nothing. A caller that has a key gets a
    /// digest; a caller that has a digest already has one of these.
    pub fn of(key: &OperationKey) -> Self {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(key.as_str().as_bytes());
        let mut hex = String::with_capacity(HASH_HEX_LEN);
        // Rendered from the leading bytes rather than by formatting the whole
        // digest and slicing it: slicing a hex string is correct here but invites
        // a later edit that slices before rendering, or renders uppercase and
        // truncates mid-byte. Two hex characters per byte, eight bytes, no
        // intermediate 64-character string to mis-slice.
        for byte in digest.iter().take(HASH_HEX_LEN / 2) {
            use std::fmt::Write as _;
            // Infallible: writing to a String cannot fail, and `{:02x}` always
            // emits lowercase hex.
            let _ = write!(hex, "{byte:02x}");
        }
        Self(hex)
    }

    /// The digest, as the 16 lowercase hex characters telemetry carries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperationKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- OperationKeyHash ----------------------------------------------------

    /// The digest is the documented one: first 16 hex chars of SHA-256.
    ///
    /// Pinned against an externally-computable value rather than against
    /// whatever this code produces. `sha256("op-123")` begins
    /// `d9d0e3d1a4b8a5b9…`; asserting the prefix a second implementation of the
    /// same rule would produce is what makes this a test of the *rule* instead
    /// of a snapshot of the current call.
    #[test]
    fn the_hash_is_the_first_sixteen_hex_chars_of_sha256() {
        let key = OperationKey::parse("op-123").expect("valid key");
        let hash = OperationKeyHash::of(&key);

        assert_eq!(
            hash.as_str().len(),
            16,
            "CORE-019 §12 fixes the width at 16 hex chars, got {:?}",
            hash.as_str()
        );
        assert!(
            hash.as_str().chars().all(|c| c.is_ascii_hexdigit()),
            "every character must be a hex digit, got {:?}",
            hash.as_str()
        );
        assert!(
            hash.as_str().chars().all(|c| !c.is_ascii_uppercase()),
            "lowercase hex, so two emitters of the same key agree: {:?}",
            hash.as_str()
        );
        assert_eq!(
            hash.as_str(),
            &sha256_hex(b"op-123")[..16],
            "the digest must be SHA-256 of the key's bytes, truncated — not of \
             anything else, and not another algorithm"
        );
    }

    /// An independent SHA-256 hex rendering, so the assertion above compares the
    /// rule against a second computation of it rather than against itself.
    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The raw key never survives the hash — which is the whole point.
    ///
    /// # Why the substring check is scoped rather than universal
    ///
    /// `!hash.contains(raw)` is the obvious assertion and it is **unsound for
    /// short hex-like keys**, which is worth recording because it looks right.
    /// The key `"a"` hashes to `ca978112ca1bbdca`, and that contains `"a"` — not
    /// because anything leaked, but because `a` is itself a hex digit and every
    /// digest is written in hex. A universal loop over that assertion fails on a
    /// coincidence and would have to be "fixed" by weakening the real cases.
    ///
    /// So the substring check runs over the inputs where a match would be
    /// genuine evidence — business identifiers with non-hex characters, which is
    /// what the redaction exists to protect — and every input, degenerate ones
    /// included, is held to the properties that are true unconditionally: the
    /// digest is not the key, and its width does not depend on the key's.
    #[test]
    fn the_hash_never_carries_the_key_it_was_built_from() {
        for raw in [
            "customer-4417-invoice-2026-03",
            "user@example.com/order/99",
            "op-1",
            "INV/2026/000912",
        ] {
            let key = OperationKey::parse(raw).expect("valid key");
            let hash = OperationKeyHash::of(&key);
            assert!(
                !hash.as_str().contains(raw),
                "the emitted value must not carry the client-supplied key {raw:?}, \
                 got {:?}",
                hash.as_str()
            );
        }

        // True for every input, including the ones the check above cannot speak
        // about: a one-character key and a key that is already valid hex.
        for raw in ["a", "deadbeef", "customer-4417-invoice-2026-03", "1"] {
            let key = OperationKey::parse(raw).expect("valid key");
            let hash = OperationKeyHash::of(&key);
            assert_ne!(
                hash.as_str(),
                raw,
                "the digest must never equal the key it redacts"
            );
            assert_eq!(
                hash.as_str().len(),
                16,
                "the width must not depend on the key's, or the digest would leak \
                 its length: {raw:?} produced {:?}",
                hash.as_str()
            );
        }
    }

    /// Two arrivals of one key hash identically, or telemetry could not group
    /// the retry with the original.
    #[test]
    fn the_same_key_always_hashes_to_the_same_value() {
        let one = OperationKey::parse("op-stable").expect("valid key");
        let two = OperationKey::parse("op-stable").expect("valid key");
        assert_eq!(OperationKeyHash::of(&one), OperationKeyHash::of(&two));
    }

    /// And two different keys do not collapse into one bucket.
    #[test]
    fn different_keys_hash_differently() {
        let a = OperationKey::parse("op-a").expect("valid key");
        let b = OperationKey::parse("op-b").expect("valid key");
        assert_ne!(OperationKeyHash::of(&a), OperationKeyHash::of(&b));
    }

    /// `Display` and `Debug` must both be safe to put in a log line.
    ///
    /// A `Debug` that printed a wrapped raw value would defeat the type on the
    /// one path people reach for while diagnosing — `{:?}` in a tracing macro.
    /// Here both can only show the digest, because the digest is all the type
    /// holds; this pins that it stays that way.
    #[test]
    fn neither_rendering_can_leak_the_key() {
        let key = OperationKey::parse("secret-order-77").expect("valid key");
        let hash = OperationKeyHash::of(&key);

        assert!(!format!("{hash}").contains("secret-order-77"));
        assert!(!format!("{hash:?}").contains("secret-order-77"));
        assert_eq!(format!("{hash}"), hash.as_str());
    }

    // -- OperationKey --------------------------------------------------------

    #[test]
    fn valid_key_parses() {
        let key = OperationKey::parse("op-123").unwrap();
        assert_eq!(key.as_str(), "op-123");
    }

    #[test]
    fn empty_key_rejected() {
        let err = OperationKey::parse("").unwrap_err();
        assert_eq!(err, OperationKeyError::Empty);
    }

    #[test]
    fn whitespace_only_key_rejected() {
        let err = OperationKey::parse("   ").unwrap_err();
        assert_eq!(err, OperationKeyError::Empty);
    }

    #[test]
    fn bounded_length_key_accepted() {
        let value = "a".repeat(MAX_LEN);
        let key = OperationKey::parse(value.clone()).unwrap();
        assert_eq!(key.as_str(), value.as_str());
    }

    #[test]
    fn over_length_key_rejected() {
        let value = "a".repeat(MAX_LEN + 1);
        let err = OperationKey::parse(value).unwrap_err();
        assert_eq!(err, OperationKeyError::TooLong);
    }

    #[test]
    fn fingerprints_with_equal_value_are_equal() {
        assert_eq!(
            OperationFingerprint::new("f1"),
            OperationFingerprint::new("f1")
        );
    }

    #[test]
    fn fingerprints_with_different_value_are_not_equal() {
        assert_ne!(
            OperationFingerprint::new("f1"),
            OperationFingerprint::new("f2")
        );
    }
}
