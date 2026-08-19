//! Cross-adapter equivalence: HTTP and gRPC must resolve the same input to the
//! same outcome, under both enforcement modes.
//!
//! The conformance harness already proves each adapter satisfies the shared
//! contract on its own. That is necessary and not sufficient: two carriers can
//! each be self-consistent and still disagree with one another, and a
//! disagreement is precisely what "the idempotency guarantee is protocol-neutral"
//! claims cannot happen. This file compares them directly.
//!
//! Gated on the `grpc` feature because half the comparison does not exist
//! without tonic.
#![cfg(feature = "grpc")]

use axum::http::{HeaderMap, HeaderValue};
use ego_service_sdk::idempotency::{
    resolve_operation_key, OperationKeyCarrier, OperationKeyRejection,
};
use ego_service_sdk::runtime::IdempotencyEnforcementMode;
use ego_transport::idempotency::{GrpcMetadataCarrier, HeaderCarrier};
use tonic::metadata::{AsciiMetadataValue, MetadataMap};

/// The key as each transport spells it on the wire. Written as literals rather
/// than imported from the carriers, so a rename of either constant shows up
/// here as a failure instead of being followed silently.
const HTTP_KEY: &str = "Idempotency-Key";
const GRPC_KEY: &str = "idempotency-key";

/// A resolution outcome with the per-adapter diagnostic erased.
///
/// The two adapters are *supposed* to differ in exactly one respect: a
/// rejection names the location it came from, `"http:Idempotency-Key"` versus
/// `"grpc:idempotency-key"`. That difference is the point of `carrier_name`, so
/// comparing raw `Result`s would report it as a divergence on every rejecting
/// row. Projecting it away leaves precisely the part that must match: which
/// rule fired, and whether a key came out the other side.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Resolved(String),
    NoKey,
    Missing,
    Invalid,
    Unreadable,
    Ambiguous,
}

fn outcome_of(carrier: &dyn OperationKeyCarrier, mode: IdempotencyEnforcementMode) -> Outcome {
    match resolve_operation_key(carrier, mode) {
        Ok(Some(key)) => Outcome::Resolved(key.as_str().to_string()),
        Ok(None) => Outcome::NoKey,
        Err(OperationKeyRejection::Missing { .. }) => Outcome::Missing,
        Err(OperationKeyRejection::Invalid { .. }) => Outcome::Invalid,
        Err(OperationKeyRejection::Unreadable { .. }) => Outcome::Unreadable,
        Err(OperationKeyRejection::Ambiguous { .. }) => Outcome::Ambiguous,
    }
}

/// What arrived at the location, described once and then rendered onto each
/// transport. Describing the input per-transport instead would let the two
/// sides drift apart, and a comparison of two different inputs proves nothing.
enum Arrival {
    Nothing,
    OneValue(&'static str),
    OneUnreadableValue,
    TwoValues(&'static str, &'static str),
    ValueThenUnreadable(&'static str),
}

fn http_map(arrival: &Arrival) -> HeaderMap {
    let mut map = HeaderMap::new();
    match arrival {
        Arrival::Nothing => {}
        Arrival::OneValue(v) => {
            map.append(
                HTTP_KEY,
                HeaderValue::from_str(v).expect("valid header value"),
            );
        }
        Arrival::OneUnreadableValue => {
            map.append(
                HTTP_KEY,
                HeaderValue::from_bytes(&[0xff, 0xfe]).expect("valid bytes"),
            );
        }
        Arrival::TwoValues(a, b) => {
            map.append(
                HTTP_KEY,
                HeaderValue::from_str(a).expect("valid header value"),
            );
            map.append(
                HTTP_KEY,
                HeaderValue::from_str(b).expect("valid header value"),
            );
        }
        Arrival::ValueThenUnreadable(a) => {
            map.append(
                HTTP_KEY,
                HeaderValue::from_str(a).expect("valid header value"),
            );
            map.append(
                HTTP_KEY,
                HeaderValue::from_bytes(&[0xff, 0xfe]).expect("valid bytes"),
            );
        }
    }
    map
}

fn grpc_map(arrival: &Arrival) -> MetadataMap {
    let ascii = |v: &str| AsciiMetadataValue::try_from(v).expect("valid ASCII metadata value");
    let unreadable =
        || AsciiMetadataValue::try_from(&[0xff, 0xfe][..]).expect("valid metadata bytes");
    let mut map = MetadataMap::new();
    match arrival {
        Arrival::Nothing => {}
        Arrival::OneValue(v) => {
            map.append(GRPC_KEY, ascii(v));
        }
        Arrival::OneUnreadableValue => {
            map.append(GRPC_KEY, unreadable());
        }
        Arrival::TwoValues(a, b) => {
            map.append(GRPC_KEY, ascii(a));
            map.append(GRPC_KEY, ascii(b));
        }
        Arrival::ValueThenUnreadable(a) => {
            map.append(GRPC_KEY, ascii(a));
            map.append(GRPC_KEY, unreadable());
        }
    }
    map
}

/// Asserts both adapters agree *and* that what they agree on is right.
///
/// Both halves are load-bearing. Comparing the two adapters alone would pass
/// happily if they were identically wrong — which is the most likely way this
/// ever breaks, since they share a classification helper. Asserting the
/// expected outcome alone would not notice one adapter drifting. Neither
/// assertion substitutes for the other.
fn assert_both_adapters_resolve_to(
    label: &str,
    arrival: Arrival,
    mode: IdempotencyEnforcementMode,
    expected: Outcome,
) {
    let headers = http_map(&arrival);
    let metadata = grpc_map(&arrival);
    let http = outcome_of(&HeaderCarrier(&headers), mode);
    let grpc = outcome_of(&GrpcMetadataCarrier(&metadata), mode);

    assert_eq!(
        http, grpc,
        "{label} under {mode:?}: HTTP and gRPC disagreed. A divergence here is a defect \
         in whichever adapter differs, never a protocol-specific rule — the guarantee \
         does not get to mean different things on different transports"
    );
    assert_eq!(
        http, expected,
        "{label} under {mode:?}: both adapters agreed on the wrong outcome. Agreement \
         alone is not correctness; they share a classification helper, so identical \
         wrongness is the most likely way this breaks"
    );
}

#[test]
fn an_absent_key_resolves_identically_on_both_adapters() {
    assert_both_adapters_resolve_to(
        "absent",
        Arrival::Nothing,
        IdempotencyEnforcementMode::MandatoryKey,
        Outcome::Missing,
    );
    // The one row where the mode changes the answer, and it changes it the same
    // way on both transports.
    assert_both_adapters_resolve_to(
        "absent",
        Arrival::Nothing,
        IdempotencyEnforcementMode::Compatibility,
        Outcome::NoKey,
    );
}

#[test]
fn a_present_valid_key_resolves_identically_on_both_adapters() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "present and valid",
            Arrival::OneValue("op-123"),
            mode,
            Outcome::Resolved("op-123".to_string()),
        );
    }
}

/// Whitespace-only is the invalid case because it is the one `OperationKey`
/// actually rejects on content: the type is deliberately opaque and admits any
/// non-empty string within its length bound.
#[test]
fn an_invalid_key_is_rejected_identically_on_both_adapters_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "present but invalid",
            Arrival::OneValue("   "),
            mode,
            Outcome::Invalid,
        );
    }
}

#[test]
fn an_unreadable_value_is_rejected_identically_on_both_adapters_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "single unreadable value",
            Arrival::OneUnreadableValue,
            mode,
            Outcome::Unreadable,
        );
    }
}

/// Several entries that disagree: the message carried two different keys and
/// there is no honest way to choose one. Entries that agree are a separate row
/// and resolve, because agreement leaves nothing to guess.
#[test]
fn disagreeing_entries_are_rejected_identically_on_both_adapters_under_both_modes() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "two entries with different values",
            Arrival::TwoValues("op-A", "op-B"),
            mode,
            Outcome::Ambiguous,
        );
        assert_both_adapters_resolve_to(
            "two entries carrying the identical value",
            Arrival::TwoValues("op-A", "op-A"),
            mode,
            Outcome::Resolved("op-A".to_string()),
        );
        // Ambiguous, not unreadable: an unreadable value cannot be shown equal
        // to a readable one, so the entries disagree.
        assert_both_adapters_resolve_to(
            "a readable entry followed by an unreadable one",
            Arrival::ValueThenUnreadable("op-A"),
            mode,
            Outcome::Ambiguous,
        );
    }
}

/// One entry an intermediary already folded. Deliberately out of scope here,
/// and pinned as such: it resolves as an ordinary key on BOTH adapters, so the
/// gap is symmetric. A gap closed on one transport and open on the other is
/// exactly the divergence this file exists to prevent, which makes asserting
/// the shared current behaviour worth more than asserting nothing.
#[test]
fn a_coalesced_key_currently_resolves_identically_on_both_adapters() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "one entry holding a coalesced pair",
            Arrival::OneValue("op-A, op-B"),
            mode,
            Outcome::Resolved("op-A, op-B".to_string()),
        );
    }
}

/// The negative control for the whole file. Every row above asserts something
/// is rejected or resolved; without this one, an adapter pair that rejected
/// *everything* would satisfy most of them. What it pins specifically is that
/// an ordinary single entry is **not swept up by the multiplicity rule** —
/// one entry is one key, whatever the value happens to contain.
#[test]
fn an_ordinary_single_key_is_not_swept_up_by_the_multiplicity_rule() {
    for mode in [
        IdempotencyEnforcementMode::MandatoryKey,
        IdempotencyEnforcementMode::Compatibility,
    ] {
        assert_both_adapters_resolve_to(
            "an ordinary key with no separator",
            Arrival::OneValue("op-with-dashes-and.dots"),
            mode,
            Outcome::Resolved("op-with-dashes-and.dots".to_string()),
        );
    }
}
