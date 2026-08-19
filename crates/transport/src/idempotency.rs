//! HTTP carrier for the `OperationKey` extraction contract.
//!
//! Reads the `Idempotency-Key` header and nothing else: no request body, no
//! query string, no other header. Validation and the missing-key policy stay
//! entirely in `resolve_operation_key` — this adapter contributes only a
//! location, never a rule (mirrors `security.rs`'s `AxumRequestContext` and
//! `propagation.rs`'s inbound `traceparent` reader in scope and shape).

use axum::http::HeaderMap;
use ego_service_sdk::idempotency::{OperationKeyCarrier, RawOperationKey};

/// The header this carrier reads the raw operation key from.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";

/// Classifies what one location held, given its entries and a way to read one
/// as text.
///
/// Shared by both carriers on purpose. The two transports disagree about
/// nothing here — they differ only in which map they consult and how a value
/// becomes a string — so keeping the classification in one place is what makes
/// "both adapters answer identically" a property of the code rather than of
/// two authors having reached the same conclusion twice.
///
/// # Several entries are ambiguous only when they disagree
///
/// More than one entry does not by itself mean the caller supplied two keys.
/// Duplicated entries are routinely produced by infrastructure rather than by a
/// broken client: a proxy that adds the header when the caller already set it,
/// or a retry wrapper that appends instead of replacing on each attempt. The
/// second of those is why "any duplicate is ambiguous" is actively harmful —
/// attempt one fails transiently, attempt two arrives carrying the same key
/// twice, and a rule that rejected it would convert a transient fault into
/// permanent command loss on a non-retryable status.
///
/// So entries that all agree resolve to the one key they agree on. There is no
/// guess to make: every candidate is the same value. Ambiguity is reserved for
/// the case that genuinely has no honest answer — entries that differ, where
/// choosing one would be inventing which key the caller meant. An unreadable
/// value among several counts as disagreement, because equality cannot be
/// established against something that never became text.
///
/// A lone unreadable value stays unreadable rather than ambiguous: one entry
/// arrived, and what is wrong with it is its bytes, not its multiplicity.
///
/// # A known gap this deliberately does not close
///
/// An intermediary may fold repeated occurrences of one field into a single
/// comma-separated value, and that folding is invisible here: two keys arrive
/// as one entry, so the joined string is handed on as a key. `OperationKey`
/// will accept it — measured, not assumed: the type is deliberately opaque and
/// rejects only empty or over-long strings — so such a request resolves under a
/// key no client ever sent.
///
/// Catching it means rejecting a separator, which is a backward-incompatible
/// narrowing: a composite key that legitimately contains a comma would start
/// failing with a non-retryable status on a path that previously worked. That
/// is a decision with its own impact analysis, not a detail of this one, so the
/// gap is recorded rather than closed here.
fn classify_entries<'a, V: 'a>(
    mut entries: impl Iterator<Item = &'a V>,
    readable: impl Fn(&'a V) -> Option<&'a str>,
) -> RawOperationKey<'a> {
    let Some(first) = entries.next() else {
        return RawOperationKey::Absent;
    };
    let Some(head) = readable(first) else {
        return if entries.next().is_some() {
            RawOperationKey::Ambiguous
        } else {
            RawOperationKey::Unreadable
        };
    };
    for rest in entries {
        if readable(rest) != Some(head) {
            return RawOperationKey::Ambiguous;
        }
    }
    RawOperationKey::Present(head)
}

/// Wraps axum's `HeaderMap` so the shared extraction contract can read the
/// `Idempotency-Key` header without any HTTP-specific knowledge living
/// outside this crate.
pub struct HeaderCarrier<'a>(pub &'a HeaderMap);

impl OperationKeyCarrier for HeaderCarrier<'_> {
    /// Looks the header up case-insensitively, which is what `HeaderMap`
    /// already does for a `&str` name. That matters in practice rather than in
    /// theory: HTTP/2 transmits every header name lowercased, so a real client
    /// sends `idempotency-key` and a case-sensitive lookup would silently find
    /// nothing and reject a request that was perfectly well formed.
    ///
    /// A value that is not valid UTF-8 is reported as **unreadable**, not as
    /// absent. The distinction is load-bearing: an absent key is admissible
    /// under the compatibility variant, so collapsing the two would let
    /// malformed input silently disable the guarantee for exactly the requests
    /// most likely to come from a broken client. Unreadable is rejected under
    /// every mode.
    fn raw_operation_key(&self) -> RawOperationKey<'_> {
        classify_entries(self.0.get_all(IDEMPOTENCY_KEY_HEADER).iter(), |value| {
            value.to_str().ok()
        })
    }

    fn carrier_name(&self) -> &'static str {
        "http:Idempotency-Key"
    }
}

/// The metadata key the gRPC carrier reads the raw operation key from.
///
/// Lowercase deliberately: gRPC transmits metadata keys over HTTP/2, which
/// lowercases every name on the wire, and tonic classifies a key as binary
/// purely by a `-bin` suffix. Without that suffix this is an *ASCII* key, so
/// the value arrives as `MetadataValue<Ascii>` and stays readable as text —
/// which is what lets this adapter answer the same three states as HTTP.
#[cfg(feature = "grpc")]
pub const IDEMPOTENCY_KEY_METADATA: &str = "idempotency-key";

/// Wraps tonic's `MetadataMap` so the shared extraction contract can read the
/// `idempotency-key` metadata entry, exactly as [`HeaderCarrier`] does for an
/// HTTP header. Same newtype-over-a-borrowed-map shape, same three answers,
/// no protocol-specific rule of its own.
#[cfg(feature = "grpc")]
pub struct GrpcMetadataCarrier<'a>(pub &'a tonic::metadata::MetadataMap);

#[cfg(feature = "grpc")]
impl OperationKeyCarrier for GrpcMetadataCarrier<'_> {
    /// Structurally identical to [`HeaderCarrier::raw_operation_key`], and
    /// that is the point rather than a coincidence: the two adapters differ
    /// only in which map they read, never in what the three answers mean.
    ///
    /// `get` is the ASCII accessor. It returns `None` for a binary (`-bin`)
    /// key, so reading through it is what keeps a missing key and a
    /// wrongly-typed key from being told apart here — neither is this
    /// adapter's business, and [`IDEMPOTENCY_KEY_METADATA`] is ASCII by
    /// construction.
    ///
    /// A value that cannot be read as text is reported as **unreadable**, not
    /// as absent. That distinction survives here for the same reason it does
    /// over HTTP: an absent key is admissible under the compatibility variant,
    /// so collapsing the two would silently disable the guarantee for exactly
    /// the requests most likely to come from a broken client.
    fn raw_operation_key(&self) -> RawOperationKey<'_> {
        classify_entries(self.0.get_all(IDEMPOTENCY_KEY_METADATA).iter(), |value| {
            value.to_str().ok()
        })
    }

    fn carrier_name(&self) -> &'static str {
        "grpc:idempotency-key"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_header_value_when_present() {
        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_KEY_HEADER, "op-123".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-123")
        );
    }

    /// HTTP/2 lowercases every header name on the wire, so this is the shape a
    /// real client actually sends. Pinned because a lookup that compared names
    /// literally would pass every other test here and still reject every
    /// HTTP/2 request.
    #[test]
    fn reads_the_header_regardless_of_the_case_the_client_sent() {
        let mut headers = HeaderMap::new();
        headers.insert("idempotency-key", "op-123".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-123")
        );
    }

    /// A non-UTF-8 value reports as unreadable, never as absent — the
    /// distinction is what keeps a malformed key from being admitted under the
    /// compatibility variant.
    #[test]
    fn a_non_utf8_header_value_reports_as_unreadable_not_absent() {
        let mut headers = HeaderMap::new();
        headers.insert(
            IDEMPOTENCY_KEY_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Unreadable);
    }

    #[test]
    fn reports_absent_when_the_header_is_absent() {
        let headers = HeaderMap::new();
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Absent);
    }

    #[test]
    fn carrier_name_names_the_http_header_it_reads() {
        let headers = HeaderMap::new();
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.carrier_name(), "http:Idempotency-Key");
    }

    // --- multiplicity cases, and the documented coalescence gap -------------
    //
    // Only one mechanism is at work here: counting entries, without reading any
    // value. The last test in this group is not another case of it — it pins
    // the gap that mechanism cannot see, where a value an intermediary already
    // folded arrives as a single entry and resolves as an ordinary key.

    /// Two entries. Rejected on the count alone.
    #[test]
    fn two_entries_report_as_ambiguous_not_as_the_first_of_them() {
        let mut headers = HeaderMap::new();
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-A".parse().unwrap());
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-B".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// Entries that all agree resolve to the key they agree on. This is the
    /// shape a retry wrapper that appends rather than replaces produces, and
    /// rejecting it would turn a transient fault into permanent command loss —
    /// on a status the client cannot retry.
    #[test]
    fn two_identical_entries_resolve_as_the_one_key_they_agree_on() {
        let mut headers = HeaderMap::new();
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-A".parse().unwrap());
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-A".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-A")
        );
    }

    /// A readable value followed by an unreadable one is **ambiguous**, and the
    /// reason matters: it is disqualified for being two entries, not for the
    /// second one's bytes, which are never examined. Were this reported as
    /// unreadable it would be right by accident, and it would stop being right
    /// the moment the unreadable entry came first.
    #[test]
    fn a_readable_entry_followed_by_an_unreadable_one_is_ambiguous() {
        let mut headers = HeaderMap::new();
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-A".parse().unwrap());
        headers.append(
            IDEMPOTENCY_KEY_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// The mirror of the case above, and the reason it is not redundant: with
    /// the unreadable entry first, a carrier that read before counting would
    /// answer `Unreadable` here and `Present` in the previous test. Both are
    /// wrong, and only having both cases makes the read-before-count ordering
    /// detectable at all.
    #[test]
    fn an_unreadable_entry_followed_by_a_readable_one_is_also_ambiguous() {
        let mut headers = HeaderMap::new();
        headers.append(
            IDEMPOTENCY_KEY_HEADER,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        headers.append(IDEMPOTENCY_KEY_HEADER, "op-A".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// The known gap, pinned as a gap rather than left to be rediscovered: a
    /// value an intermediary already folded reads as ONE entry and resolves as
    /// an ordinary key. Asserting the current behaviour is what makes closing
    /// this later a visible, deliberate change rather than a silent one — and
    /// the second assertion records why the domain cannot catch it either.
    #[test]
    fn a_coalesced_pair_currently_resolves_as_a_single_key() {
        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_KEY_HEADER, "op-A, op-B".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-A, op-B"),
            "coalesced values are deliberately out of scope here; closing this \
             narrows what a key may be and needs its own impact analysis"
        );
        assert!(
            ego_domain::operation::OperationKey::parse("op-A, op-B").is_ok(),
            "and the domain will not catch it either: OperationKey is opaque by \
             design, rejecting only empty or over-long strings"
        );
    }

    /// A single ordinary value stays present. Without this the rules above
    /// would be satisfied by a carrier that called everything ambiguous.
    #[test]
    fn a_lone_separator_free_value_is_still_present() {
        let mut headers = HeaderMap::new();
        headers.insert(IDEMPOTENCY_KEY_HEADER, "op-123".parse().unwrap());
        let carrier = HeaderCarrier(&headers);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-123")
        );
    }
}

/// The gRPC carrier's own unit tests.
///
/// A separate module rather than more cases in `tests`, because the whole set
/// only exists when `tonic` does. Kept deliberately parallel to the HTTP cases
/// above, case for case: the two adapters are supposed to answer identically,
/// and two test sets that drifted apart in *shape* would make a real
/// divergence in *behaviour* much harder to notice.
#[cfg(all(test, feature = "grpc"))]
mod grpc_tests {
    use super::*;
    use tonic::metadata::{AsciiMetadataValue, MetadataMap};

    fn ascii(value: &str) -> AsciiMetadataValue {
        AsciiMetadataValue::try_from(value).expect("fixture value is valid ASCII metadata")
    }

    fn unreadable() -> AsciiMetadataValue {
        AsciiMetadataValue::try_from(&[0xff, 0xfe][..])
            .expect("bytes 128..=255 are valid in an ASCII metadata value")
    }

    #[test]
    fn reads_the_metadata_value_when_present() {
        let mut metadata = MetadataMap::new();
        metadata.insert(IDEMPOTENCY_KEY_METADATA, ascii("op-123"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-123")
        );
    }

    #[test]
    fn reports_absent_when_the_metadata_entry_is_absent() {
        let metadata = MetadataMap::new();
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Absent);
    }

    /// One entry whose bytes are not readable as text stays **unreadable**.
    /// Pinned separately from the ambiguous cases because a carrier that
    /// reported everything unusable as ambiguous would satisfy those and lose
    /// this distinction entirely.
    #[test]
    fn a_lone_unreadable_metadata_value_reports_as_unreadable() {
        let mut metadata = MetadataMap::new();
        metadata.insert(IDEMPOTENCY_KEY_METADATA, unreadable());
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Unreadable);
    }

    // --- multiplicity cases, and the documented coalescence gap -------------

    /// Two entries, rejected on the count alone.
    #[test]
    fn two_entries_report_as_ambiguous_not_as_the_first_of_them() {
        let mut metadata = MetadataMap::new();
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-A"));
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-B"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// Entries that all agree resolve to the key they agree on, identically to
    /// HTTP. A rule that differed here would make the guarantee depend on which
    /// transport carried the request, which is the one thing it must not do.
    #[test]
    fn two_identical_entries_resolve_as_the_one_key_they_agree_on() {
        let mut metadata = MetadataMap::new();
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-A"));
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-A"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-A")
        );
    }

    /// A readable entry followed by an unreadable one is **ambiguous**, and
    /// the reason is the count, not the bytes: the second value is never
    /// examined. Reporting it unreadable would be right by accident here and
    /// wrong the moment the unreadable entry arrived first.
    #[test]
    fn a_readable_entry_followed_by_an_unreadable_one_is_ambiguous() {
        let mut metadata = MetadataMap::new();
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-A"));
        metadata.append(IDEMPOTENCY_KEY_METADATA, unreadable());
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// The mirror of the case above, and the reason it is not redundant: with
    /// the unreadable entry first, a carrier that read before counting would
    /// answer `Unreadable` here and `Present` in the previous test. Both are
    /// wrong, and only having both cases makes the read-before-count ordering
    /// detectable at all.
    #[test]
    fn an_unreadable_entry_followed_by_a_readable_one_is_also_ambiguous() {
        let mut metadata = MetadataMap::new();
        metadata.append(IDEMPOTENCY_KEY_METADATA, unreadable());
        metadata.append(IDEMPOTENCY_KEY_METADATA, ascii("op-A"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(carrier.raw_operation_key(), RawOperationKey::Ambiguous);
    }

    /// The same known gap on this transport, pinned identically. Both adapters
    /// must be out of scope together — a gap closed on one and open on the
    /// other is the divergence the equivalence test exists to prevent.
    #[test]
    fn a_coalesced_pair_currently_resolves_as_a_single_key() {
        let mut metadata = MetadataMap::new();
        metadata.insert(IDEMPOTENCY_KEY_METADATA, ascii("op-A, op-B"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-A, op-B"),
            "coalesced values are deliberately out of scope on both transports"
        );
    }

    /// A single ordinary value stays present. Without this the rules above
    /// would be satisfied by a carrier that called everything ambiguous.
    #[test]
    fn a_lone_separator_free_value_is_still_present() {
        let mut metadata = MetadataMap::new();
        metadata.insert(IDEMPOTENCY_KEY_METADATA, ascii("op-123"));
        let carrier = GrpcMetadataCarrier(&metadata);

        assert_eq!(
            carrier.raw_operation_key(),
            RawOperationKey::Present("op-123")
        );
    }
}
