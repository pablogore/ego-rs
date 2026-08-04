//! `assert_carrier_conformance` — a table-driven conformance check for
//! [`OperationKeyCarrier`] implementations.
//!
//! It belongs here rather than beside any single transport adapter: it
//! defines what conforming to the carrier contract *means*, so an adapter's
//! own test calls this function against its own type and is judged against
//! the one shared contract [`resolve_operation_key`] enforces, never against
//! its own author's reading of that contract.

use ego_domain::operation::OperationKey;
use ego_service_sdk::idempotency::{
    resolve_operation_key, OperationKeyCarrier, OperationKeyRejection,
};
use ego_service_sdk::runtime::IdempotencyEnforcementMode;

/// Asserts that `with_key` and `without_key` — two instances of the same
/// [`OperationKeyCarrier`] implementation, one carrying a valid raw key and
/// one carrying none — resolve through [`resolve_operation_key`] exactly as
/// every other conforming carrier must, against the one shared policy table.
///
/// An adapter's own integration test wires up both a with-key and a
/// without-key instance of its carrier and passes both here — that is the
/// entire conformance obligation. Nothing about a specific protocol is
/// exercised beyond what [`OperationKeyCarrier`] itself already narrows to
/// (one raw string, one diagnostic name).
///
/// # Why two carrier instances, not one
///
/// The original sketch for this helper took a single carrier reference. A
/// single instance can only ever report one
/// fixed `raw_operation_key()` value, so it cannot exercise both halves of
/// `resolve_operation_key`'s policy table — key present and key absent —
/// against the identical adapter implementation. Flagged here the same way
/// `crate::runtime::tenant`'s `CanonicalTenant` flags its own deviation from
/// a design sketch: the literal shape does not typecheck against what the
/// function must actually prove, so it takes the smallest change consistent
/// with the design's intent instead.
///
/// # Panics
///
/// Panics with a descriptive message on the first behaviour that does not
/// match the shared contract: `with_key` not actually carrying a key,
/// `without_key` carrying one anyway, a `carrier_name()` that is empty or
/// unstable across calls, or any resolution outcome that diverges from the
/// policy table every conforming carrier must satisfy.
pub fn assert_carrier_conformance<C: OperationKeyCarrier>(with_key: &C, without_key: &C) {
    let raw = with_key
        .raw_operation_key()
        .expect("conformance precondition failed: `with_key` must carry a raw operation key");
    let expected = OperationKey::parse(raw).unwrap_or_else(|err| {
        panic!("`with_key`'s raw value {raw:?} must itself be a valid OperationKey: {err}")
    });

    assert!(
        without_key.raw_operation_key().is_none(),
        "conformance precondition failed: `without_key` must carry no operation key"
    );

    assert!(
        !with_key.carrier_name().is_empty(),
        "carrier_name() must be a non-empty diagnostic name"
    );
    // Both instances must report the same name. The generic bound already
    // forces them to be the same type; this catches a name derived from
    // per-instance state instead of from the adapter itself, which would make
    // the diagnostic location depend on which request happened to be rejected.
    assert_eq!(
        with_key.carrier_name(),
        without_key.carrier_name(),
        "both instances of one carrier must report the identical name, so a \
         rejection always names the same location"
    );

    // Table-driven: a present, valid key resolves the same way regardless of
    // enforcement mode — the mode governs only the missing-key policy.
    assert_eq!(
        resolve_operation_key(with_key, IdempotencyEnforcementMode::MandatoryKey),
        Ok(Some(expected.clone())),
        "a present, valid key must resolve under the default mandatory mode"
    );
    assert_eq!(
        resolve_operation_key(with_key, IdempotencyEnforcementMode::Compatibility),
        Ok(Some(expected)),
        "a present, valid key must resolve identically under compatibility mode"
    );

    // A missing key is mode-dependent: rejected under the fail-closed
    // default, admitted only when the compatibility variant was explicitly
    // configured.
    assert_eq!(
        resolve_operation_key(without_key, IdempotencyEnforcementMode::MandatoryKey),
        Err(OperationKeyRejection::Missing {
            carrier: without_key.carrier_name()
        }),
        "a missing key must be rejected under the default mandatory mode"
    );
    assert_eq!(
        resolve_operation_key(without_key, IdempotencyEnforcementMode::Compatibility),
        Ok(None),
        "a missing key must be admitted only under the explicit compatibility mode"
    );
}

#[cfg(test)]
mod tests {
    use ego_service_sdk::idempotency::OperationKeyCarrier;

    use super::assert_carrier_conformance;

    /// A minimal, test-local [`OperationKeyCarrier`] — reads one string and
    /// nothing else, per the contract's explicit non-goal.
    struct FakeCarrier {
        raw: Option<&'static str>,
        name: &'static str,
    }

    impl OperationKeyCarrier for FakeCarrier {
        fn raw_operation_key(&self) -> Option<&str> {
            self.raw
        }

        fn carrier_name(&self) -> &'static str {
            self.name
        }
    }

    #[test]
    fn a_correctly_implemented_carrier_pair_satisfies_conformance() {
        let with_key = FakeCarrier {
            raw: Some("op-1"),
            name: "fake:key",
        };
        let without_key = FakeCarrier {
            raw: None,
            name: "fake:key",
        };

        // Must not panic — this is the "conforms" case.
        assert_carrier_conformance(&with_key, &without_key);
    }

    #[test]
    #[should_panic(expected = "`without_key` must carry no operation key")]
    fn a_without_key_carrier_that_still_reports_a_key_fails_conformance() {
        let with_key = FakeCarrier {
            raw: Some("op-1"),
            name: "fake:key",
        };
        // Mislabeled: this "without_key" instance still reports a raw key,
        // violating the precondition `assert_carrier_conformance` requires.
        let mislabeled_without_key = FakeCarrier {
            raw: Some("op-1"),
            name: "fake:key",
        };

        assert_carrier_conformance(&with_key, &mislabeled_without_key);
    }

    /// Two instances of one carrier that disagree about their own name must fail
    /// conformance. A name derived from per-instance state rather than from the
    /// adapter itself would make the diagnostic location depend on which request
    /// happened to be rejected, which is precisely the instability the harness
    /// exists to rule out.
    ///
    /// Note what the generic bound already prevents and this test therefore
    /// cannot express: passing two *different* carrier implementations. That is a
    /// compile error now, not a runtime assertion, which is the stronger place
    /// for it.
    #[test]
    #[should_panic(expected = "must report the identical name")]
    fn one_carrier_reporting_two_different_names_fails_conformance() {
        let with_key = FakeCarrier {
            raw: Some("op-1"),
            name: "fake:key",
        };
        let without_key = FakeCarrier {
            raw: None,
            name: "fake:some-other-location",
        };

        assert_carrier_conformance(&with_key, &without_key);
    }

    /// An empty name fails too: a rejection has to be able to say where the key
    /// was expected.
    #[test]
    #[should_panic(expected = "non-empty diagnostic name")]
    fn an_empty_carrier_name_fails_conformance() {
        let with_key = FakeCarrier {
            raw: Some("op-1"),
            name: "",
        };
        let without_key = FakeCarrier {
            raw: None,
            name: "",
        };

        assert_carrier_conformance(&with_key, &without_key);
    }
}
