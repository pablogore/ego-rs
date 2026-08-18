//! Runs the shared conformance harness against the gRPC `GrpcMetadataCarrier`.
//!
//! Deliberately the *same* `assert_carrier_conformance` the HTTP carrier is
//! judged by — not a gRPC-flavoured variant. A second harness would prove only
//! that this adapter satisfies its own author's reading of the contract, which
//! is exactly the claim a second transport exists to stop being taken on trust.
//!
//! The whole file is gated on the `grpc` feature, so an HTTP-only build
//! compiles it to nothing rather than failing to resolve `tonic`.
#![cfg(feature = "grpc")]

use ego_testkit::assert_carrier_conformance;
use ego_transport::idempotency::{GrpcMetadataCarrier, IDEMPOTENCY_KEY_METADATA};
use tonic::metadata::{AsciiMetadataValue, MetadataMap};

/// The key a real client actually puts on the wire, written out as a literal.
///
/// Deliberately **not** `IDEMPOTENCY_KEY_METADATA`. Building the fixtures from
/// the same constant the carrier reads would make every test below move with
/// the carrier: rename the constant and both sides agree on the new name, so
/// the suite keeps passing while no client can reach the key any more. Pinning
/// the wire name here is what turns "the carrier reads its own constant" into
/// "the carrier reads the agreed key". `the_constant_is_pinned_to_the_wire_key`
/// ties the two together in the one place that should know both.
const WIRE_KEY: &str = "idempotency-key";

#[test]
fn grpc_metadata_carrier_conforms_to_the_shared_extraction_contract() {
    let mut with_key_metadata = MetadataMap::new();
    with_key_metadata.insert(
        WIRE_KEY,
        "op-123"
            .parse()
            .expect("`op-123` is a valid ASCII metadata value"),
    );
    let with_key = GrpcMetadataCarrier(&with_key_metadata);

    let without_key_metadata = MetadataMap::new();
    let without_key = GrpcMetadataCarrier(&without_key_metadata);

    // Real bytes that no ASCII reader can accept, not a stand-in, and reached
    // through tonic's own safe constructor rather than an `unsafe` shortcut or
    // a `-bin` key that would change which state is under test.
    //
    // The gap this exploits is genuine and measured, not assumed: an ASCII
    // metadata value admits any byte in 32..=255 except 127, while `to_str`
    // admits only visible ASCII, 32..=126. `[0xff, 0xfe]` therefore constructs
    // successfully and then fails to read — the same two-step the HTTP carrier
    // relies on, which is why both adapters can answer the third state at all.
    let mut unreadable_metadata = MetadataMap::new();
    unreadable_metadata.insert(
        WIRE_KEY,
        AsciiMetadataValue::try_from(&[0xff, 0xfe][..])
            .expect("bytes 128..=255 are valid in an ASCII metadata value"),
    );
    let unreadable_key = GrpcMetadataCarrier(&unreadable_metadata);

    assert_carrier_conformance(&with_key, &without_key, &unreadable_key);
}

/// The premise the unreadable case rests on, enforced rather than assumed: the
/// fixture must be constructible *and* unreadable. If a future tonic tightened
/// its ASCII constructor to reject these bytes, the conformance test above
/// would still compile and would fail somewhere far less obvious; this test
/// names the real reason instead.
#[test]
fn the_unreadable_fixture_is_constructible_yet_unreadable() {
    let value = AsciiMetadataValue::try_from(&[0xff, 0xfe][..])
        .expect("the ASCII constructor must accept bytes 128..=255");

    assert!(
        value.to_str().is_err(),
        "the fixture must be unreadable as text; if `to_str` accepts it, the \
         unreadable state is no longer being exercised on this transport"
    );
}

/// The key must be classified ASCII, not binary. tonic decides that purely by a
/// `-bin` suffix, and a binary key would be invisible to `MetadataMap::get` —
/// the carrier would report `Absent` for a key that was actually sent, turning
/// the distinction the contract exists to preserve into a silent lie.
#[test]
fn the_metadata_key_is_ascii_typed_so_get_can_see_it() {
    assert!(
        !WIRE_KEY.ends_with("-bin"),
        "a `-bin` key would be binary-typed and unreadable through `get`"
    );

    let mut metadata = MetadataMap::new();
    metadata.insert(
        WIRE_KEY,
        "op-123"
            .parse()
            .expect("`op-123` is a valid ASCII metadata value"),
    );

    assert!(
        metadata.get(WIRE_KEY).is_some(),
        "the ASCII accessor must find a value stored under this key"
    );
}

/// The carrier's constant must name the key clients actually send. This is the
/// only test that mentions both, and it exists so renaming the constant is a
/// visible contract change rather than a silent one that every other test
/// follows along with.
#[test]
fn the_constant_is_pinned_to_the_wire_key() {
    assert_eq!(
        IDEMPOTENCY_KEY_METADATA, WIRE_KEY,
        "the carrier must read the key clients put on the wire; changing the \
         constant without changing the agreed wire name makes the key \
         unreachable for every real client"
    );
}
