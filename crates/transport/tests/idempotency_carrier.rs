//! Runs the shared conformance harness against the HTTP `HeaderCarrier`.
//! Proves this adapter is judged against the one shared extraction contract
//! rather than against its own author's reading of it.

use axum::http::{HeaderMap, HeaderValue};
use ego_testkit::assert_carrier_conformance;
use ego_transport::idempotency::HeaderCarrier;

#[test]
fn header_carrier_conforms_to_the_shared_extraction_contract() {
    let mut with_key_headers = HeaderMap::new();
    with_key_headers.insert("Idempotency-Key", "op-123".parse().unwrap());
    let with_key = HeaderCarrier(&with_key_headers);

    let without_key_headers = HeaderMap::new();
    let without_key = HeaderCarrier(&without_key_headers);

    // Real non-UTF-8 bytes, not a stand-in. This is the state that motivated
    // giving the contract a third answer: over HTTP a header value is a byte
    // string, so a client genuinely can send one that is not text, and the
    // harness has to see that this adapter reports it as unreadable rather than
    // collapsing it to absent.
    let mut unreadable_headers = HeaderMap::new();
    unreadable_headers.insert(
        "Idempotency-Key",
        HeaderValue::from_bytes(&[0xff, 0xfe]).expect("bytes are valid in a header value"),
    );
    let unreadable_key = HeaderCarrier(&unreadable_headers);

    // What a duplicate looks like on this transport: HTTP lets one header name
    // appear as many times as a client cares to send it, so two entries under
    // `Idempotency-Key` is a well-formed request that simply cannot be answered.
    // `append` is what builds it — `insert` replaces, which would quietly reduce
    // the fixture back to the single-value case it exists to escape. Both values
    // are deliberately valid keys: nothing is wrong with either one, and the
    // request is unusable all the same.
    let mut ambiguous_headers = HeaderMap::new();
    ambiguous_headers.append("Idempotency-Key", "op-first".parse().unwrap());
    ambiguous_headers.append("Idempotency-Key", "op-second".parse().unwrap());
    let ambiguous_key = HeaderCarrier(&ambiguous_headers);

    assert_carrier_conformance(&with_key, &without_key, &unreadable_key, &ambiguous_key);
}
