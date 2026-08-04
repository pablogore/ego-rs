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
    resolve_operation_key, OperationKeyCarrier, OperationKeyRejection, RawOperationKey,
};
use ego_service_sdk::runtime::IdempotencyEnforcementMode;

/// Asserts that three instances of the same [`OperationKeyCarrier`]
/// implementation — one carrying a valid raw key, one carrying none, and one
/// whose location holds something unreadable — resolve through
/// [`resolve_operation_key`] exactly as every other conforming carrier must,
/// against the one shared policy table.
///
/// An adapter's own integration test wires up all three instances of its
/// carrier and passes them here — that is the entire conformance obligation. Nothing about a specific protocol is
/// exercised beyond what [`OperationKeyCarrier`] itself already narrows to:
/// one location reported as absent, present, or unreadable, plus one stable
/// diagnostic name.
///
/// # Why three carrier instances
///
/// The original sketch for this helper took a single carrier reference. One
/// instance reports one fixed `raw_operation_key()` value, so it cannot
/// exercise the policy table against the identical adapter — and the table has
/// one row per state the contract admits. There are three such states, so
/// three instances.
///
/// The third is not optional, and that is a deliberate trade. A carrier whose
/// location physically cannot hold an unreadable value — one reading from a
/// `String` field, say — cannot supply it, and therefore cannot use this
/// harness unchanged. The alternative was an opt-out parameter, which would
/// let any adapter skip the case silently; a harness that can be satisfied
/// without exercising a rule is how a contract quietly stops being enforced.
/// Requiring it makes the gap visible at the call site instead.
///
/// # Panics
///
/// Panics with a descriptive message on the first behaviour that does not
/// match the shared contract: any instance not reporting the state it was
/// passed as, a `carrier_name()` that is empty or that differs between
/// instances, or any resolution outcome that diverges from the policy table
/// every conforming carrier must satisfy.
pub fn assert_carrier_conformance<C: OperationKeyCarrier>(
    with_key: &C,
    without_key: &C,
    unreadable_key: &C,
) {
    let raw = match with_key.raw_operation_key() {
        RawOperationKey::Present(raw) => raw,
        other => panic!(
            "conformance precondition failed: `with_key` must carry a present, readable \
             raw operation key, got {other:?}"
        ),
    };
    let expected = OperationKey::parse(raw).unwrap_or_else(|err| {
        panic!("`with_key`'s raw value {raw:?} must itself be a valid OperationKey: {err}")
    });

    assert_eq!(
        RawOperationKey::Absent,
        without_key.raw_operation_key(),
        "conformance precondition failed: `without_key` must report the key as absent — \
         reporting it unreadable instead would be a different state with a different \
         resolution rule"
    );

    assert_eq!(
        RawOperationKey::Unreadable,
        unreadable_key.raw_operation_key(),
        "conformance precondition failed: `unreadable_key` must report the value as \
         unreadable — reporting it absent instead is the exact collapse this state \
         exists to prevent, since an absent key is admissible under compatibility"
    );

    assert!(
        !with_key.carrier_name().is_empty(),
        "carrier_name() must be a non-empty diagnostic name"
    );
    // All three instances must report the same name. The generic bound already
    // forces them to be the same type; this catches a name derived from
    // per-instance state instead of from the adapter itself, which would make
    // the diagnostic location depend on which request happened to be rejected.
    assert_eq!(
        with_key.carrier_name(),
        without_key.carrier_name(),
        "every instance of one carrier must report the identical name, so a \
         rejection always names the same location"
    );
    assert_eq!(
        with_key.carrier_name(),
        unreadable_key.carrier_name(),
        "every instance of one carrier must report the identical name, so a \
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
        resolve_operation_key(unreadable_key, IdempotencyEnforcementMode::MandatoryKey),
        Err(OperationKeyRejection::Unreadable {
            carrier: unreadable_key.carrier_name()
        }),
        "an unreadable key must be rejected under the default mandatory mode"
    );
    // The row that matters most: unlike an absent key, an unreadable one is
    // rejected under compatibility too. The caller did supply a key, so the
    // missing-key policy does not apply to it, and admitting it would drop the
    // guarantee for exactly the malformed input most likely to signal a broken
    // client.
    assert_eq!(
        resolve_operation_key(unreadable_key, IdempotencyEnforcementMode::Compatibility),
        Err(OperationKeyRejection::Unreadable {
            carrier: unreadable_key.carrier_name()
        }),
        "an unreadable key must be rejected under compatibility mode as well"
    );

    assert_eq!(
        resolve_operation_key(without_key, IdempotencyEnforcementMode::Compatibility),
        Ok(None),
        "a missing key must be admitted only under the explicit compatibility mode"
    );
}

#[cfg(test)]
mod tests {
    use ego_service_sdk::idempotency::{OperationKeyCarrier, RawOperationKey};

    use super::assert_carrier_conformance;

    /// A minimal, test-local [`OperationKeyCarrier`] — reads one location and
    /// nothing else, contributing no rule of its own.
    struct FakeCarrier {
        raw: RawOperationKey<'static>,
        name: &'static str,
    }

    impl OperationKeyCarrier for FakeCarrier {
        fn raw_operation_key(&self) -> RawOperationKey<'_> {
            self.raw
        }

        fn carrier_name(&self) -> &'static str {
            self.name
        }
    }

    #[test]
    fn a_correctly_implemented_carrier_pair_satisfies_conformance() {
        let with_key = FakeCarrier {
            raw: RawOperationKey::Present("op-1"),
            name: "fake:key",
        };
        let without_key = FakeCarrier {
            raw: RawOperationKey::Absent,
            name: "fake:key",
        };

        let unreadable_key = FakeCarrier {
            raw: RawOperationKey::Unreadable,
            name: "fake:key",
        };

        // Must not panic — this is the "conforms" case.
        assert_carrier_conformance(&with_key, &without_key, &unreadable_key);
    }

    #[test]
    #[should_panic(expected = "`without_key` must report the key as absent")]
    fn a_without_key_carrier_that_still_reports_a_key_fails_conformance() {
        let with_key = FakeCarrier {
            raw: RawOperationKey::Present("op-1"),
            name: "fake:key",
        };
        // Mislabeled: this "without_key" instance still reports a raw key,
        // violating the precondition `assert_carrier_conformance` requires.
        let mislabeled_without_key = FakeCarrier {
            raw: RawOperationKey::Present("op-1"),
            name: "fake:key",
        };

        let unreadable_key = FakeCarrier {
            raw: RawOperationKey::Unreadable,
            name: "fake:key",
        };

        assert_carrier_conformance(&with_key, &mislabeled_without_key, &unreadable_key);
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
            raw: RawOperationKey::Present("op-1"),
            name: "fake:key",
        };
        let without_key = FakeCarrier {
            raw: RawOperationKey::Absent,
            name: "fake:some-other-location",
        };

        let unreadable_key = FakeCarrier {
            raw: RawOperationKey::Unreadable,
            name: with_key.name,
        };

        assert_carrier_conformance(&with_key, &without_key, &unreadable_key);
    }

    /// An empty name fails too: a rejection has to be able to say where the key
    /// was expected.
    #[test]
    #[should_panic(expected = "non-empty diagnostic name")]
    fn an_empty_carrier_name_fails_conformance() {
        let with_key = FakeCarrier {
            raw: RawOperationKey::Present("op-1"),
            name: "",
        };
        let without_key = FakeCarrier {
            raw: RawOperationKey::Absent,
            name: "",
        };

        let unreadable_key = FakeCarrier {
            raw: RawOperationKey::Unreadable,
            name: with_key.name,
        };

        assert_carrier_conformance(&with_key, &without_key, &unreadable_key);
    }

    /// The precondition that makes the third state worth having: an instance
    /// passed as unreadable which actually reports the value as absent must fail
    /// conformance. Collapsing those two is exactly the defect the third state
    /// was introduced to prevent, since an absent key is admissible under the
    /// compatibility variant and an unreadable one never is.
    #[test]
    #[should_panic(expected = "`unreadable_key` must report the value as unreadable")]
    fn an_unreadable_instance_that_reports_absent_fails_conformance() {
        let with_key = FakeCarrier {
            raw: RawOperationKey::Present("op-1"),
            name: "fake:key",
        };
        let without_key = FakeCarrier {
            raw: RawOperationKey::Absent,
            name: "fake:key",
        };
        // Mislabeled: passed as the unreadable instance, but it collapses the
        // state back to absent.
        let mislabeled_unreadable = FakeCarrier {
            raw: RawOperationKey::Absent,
            name: "fake:key",
        };

        assert_carrier_conformance(&with_key, &without_key, &mislabeled_unreadable);
    }
}
