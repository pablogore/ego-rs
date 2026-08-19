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
        match self.0.get(IDEMPOTENCY_KEY_HEADER) {
            None => RawOperationKey::Absent,
            Some(value) => match value.to_str() {
                Ok(raw) => RawOperationKey::Present(raw),
                Err(_) => RawOperationKey::Unreadable,
            },
        }
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
        match self.0.get(IDEMPOTENCY_KEY_METADATA) {
            None => RawOperationKey::Absent,
            Some(value) => match value.to_str() {
                Ok(raw) => RawOperationKey::Present(raw),
                Err(_) => RawOperationKey::Unreadable,
            },
        }
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
}
