//! The shared extraction contract: one definition of what a valid
//! `OperationKey` is, and one definition of the missing-key policy, that
//! every transport adapter consumes rather than re-implementing.

use ego_domain::operation::{OperationKey, OperationKeyError};

use crate::runtime::IdempotencyEnforcementMode;

/// A transport-specific carrier of the raw, unvalidated operation key.
///
/// Reads **one string and nothing else** — no request, no headers, no
/// protocol knowledge. This is deliberately not a generic `Transport` trait
/// and deliberately never will be: an adapter contributes a location, never a
/// rule. Validation and the missing-key policy live only in
/// [`resolve_operation_key`], so two adapters cannot disagree about what a
/// valid key is or what happens when one is absent.
/// What a carrier found at its location.
///
/// Four answers rather than two. A carrier that could only say "here is a
/// string" or "nothing" has to report an unreadable value as absent, and an
/// absent key is admissible under the compatibility variant — so malformed
/// input would silently disable the guarantee instead of being rejected for
/// what it is. The third answer exists precisely so that cannot happen.
///
/// The fourth answers the mirror-image failure. A carrier that can only hand
/// back one string has to choose one when its location held several, and any
/// such choice is a guess about which key the caller meant. Reporting the
/// multiplicity keeps that decision in the shared policy instead of leaving
/// every adapter to resolve it quietly on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOperationKey<'a> {
    /// The carrier's location held nothing.
    Absent,
    /// The carrier's location held this raw, still-unvalidated value.
    Present(&'a str),
    /// The location held something, but it cannot be read as text — for
    /// instance a header whose bytes are not valid UTF-8. Distinct from
    /// [`RawOperationKey::Absent`] because the caller did supply a key; it is
    /// simply unusable, and treating it as absent would let it through under
    /// the compatibility variant.
    Unreadable,
    /// The location held more than one entry, so which of them the caller
    /// meant is unknowable. Distinct from [`RawOperationKey::Present`] because
    /// taking the first is a guess, and picking on the caller's behalf is not
    /// a decision a carrier is entitled to make; distinct from
    /// [`RawOperationKey::Unreadable`] because each individual value may be
    /// perfectly readable — the defect is the multiplicity, not the bytes.
    ///
    /// Collapsing it into [`RawOperationKey::Present`] is the whole reason this
    /// state exists: a request carrying two *different* keys would be admitted
    /// under whichever one happened to come first, and a request whose first
    /// value is readable and whose second is not would have the unreadable one
    /// disappear entirely.
    Ambiguous,
}

pub trait OperationKeyCarrier {
    /// What this transport found at its location.
    fn raw_operation_key(&self) -> RawOperationKey<'_>;

    /// A stable diagnostic name for this carrier, e.g.
    /// `"http:Idempotency-Key"`. Never derived from user input — used only
    /// for rejection diagnostics and telemetry, so it must stay a fixed
    /// string rather than anything a caller can influence.
    fn carrier_name(&self) -> &'static str;
}

/// Why [`resolve_operation_key`] rejected a request.
///
/// Carries the offending carrier's stable [`OperationKeyCarrier::carrier_name`]
/// so a caller can report which transport-specific location was consulted,
/// without ever including the raw (potentially business-identifying) key
/// value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationKeyRejection {
    /// No key was present, and the configured
    /// [`IdempotencyEnforcementMode`] requires one.
    Missing {
        /// The carrier that reported no key.
        carrier: &'static str,
    },
    /// A key was present but failed [`OperationKey::parse`] validation.
    ///
    /// Never admitted under [`IdempotencyEnforcementMode::Compatibility`]:
    /// that variant loosens only the *missing*-key policy, not what counts
    /// as a valid key.
    Invalid {
        /// The carrier that reported the invalid key.
        carrier: &'static str,
        /// Why the raw value failed validation.
        source: OperationKeyError,
    },
    /// A key was present but could not be read as text at all.
    ///
    /// Never admitted under any mode, for the same reason as
    /// [`OperationKeyRejection::Invalid`]: the caller supplied a key, so the
    /// missing-key policy does not apply to it. Kept separate from `Invalid`
    /// because no [`OperationKeyError`] describes it — that type judges a
    /// string's validity, and this value never became one.
    Unreadable {
        /// The carrier that reported the unreadable value.
        carrier: &'static str,
    },
    /// More than one key was supplied at the same location.
    ///
    /// Never admitted under any mode, for the same reason as
    /// [`OperationKeyRejection::Invalid`] and
    /// [`OperationKeyRejection::Unreadable`]: the caller did supply keys, so
    /// the missing-key policy does not apply to them. Kept separate from
    /// `Unreadable` because nothing here need be malformed — each value may
    /// read perfectly well, and it is having several of them that leaves no
    /// honest way to choose one.
    Ambiguous {
        /// The carrier that reported more than one key.
        carrier: &'static str,
    },
}

/// The single place validation and missing-key policy live.
///
/// One definition of what a valid key is (delegated to
/// [`OperationKey::parse`]) and one definition of the missing-key policy
/// (governed by `mode`) — every transport adapter calls this function
/// instead of re-implementing either rule, which is what keeps the guarantee
/// from diverging per protocol.
///
/// A key present but failing validation is **always** rejected as
/// [`OperationKeyRejection::Invalid`], a key present but unreadable is
/// **always** rejected as [`OperationKeyRejection::Unreadable`], and several
/// keys supplied at once are **always** rejected as
/// [`OperationKeyRejection::Ambiguous`], regardless of `mode`.
/// [`IdempotencyEnforcementMode::Compatibility`] bounds only what happens when
/// a key is *absent*, never what counts as usable.
pub fn resolve_operation_key(
    carrier: &dyn OperationKeyCarrier,
    mode: IdempotencyEnforcementMode,
) -> Result<Option<OperationKey>, OperationKeyRejection> {
    match carrier.raw_operation_key() {
        RawOperationKey::Present(raw) => {
            OperationKey::parse(raw)
                .map(Some)
                .map_err(|source| OperationKeyRejection::Invalid {
                    carrier: carrier.carrier_name(),
                    source,
                })
        }
        // A supplied-but-unusable value is never admitted, and neither is a
        // supplied-but-ambiguous one: one cannot be read, the other cannot be
        // chosen between, and either way the caller did supply something. The
        // mode governs only what happens when nothing was supplied at all.
        RawOperationKey::Unreadable => Err(OperationKeyRejection::Unreadable {
            carrier: carrier.carrier_name(),
        }),
        RawOperationKey::Ambiguous => Err(OperationKeyRejection::Ambiguous {
            carrier: carrier.carrier_name(),
        }),
        RawOperationKey::Absent => match mode {
            IdempotencyEnforcementMode::MandatoryKey => Err(OperationKeyRejection::Missing {
                carrier: carrier.carrier_name(),
            }),
            // Admitted only because this variant was deliberately
            // configured. Never a silent default — a deployment cannot end up
            // here without someone choosing it.
            IdempotencyEnforcementMode::Compatibility => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use ego_domain::operation::{OperationKey, OperationKeyError};

    use crate::idempotency::{
        resolve_operation_key, OperationKeyCarrier, OperationKeyRejection, RawOperationKey,
    };
    use crate::runtime::IdempotencyEnforcementMode;

    /// A minimal, test-only carrier — reads one string and nothing else, per
    /// the contract's explicit non-goal (no request, no headers, no
    /// protocol knowledge).
    struct TestCarrier {
        raw: RawOperationKey<'static>,
    }

    impl OperationKeyCarrier for TestCarrier {
        fn raw_operation_key(&self) -> RawOperationKey<'_> {
            self.raw
        }

        fn carrier_name(&self) -> &'static str {
            "test:key"
        }
    }

    #[test]
    fn present_valid_key_resolves_under_the_default_mandatory_mode() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Present("op-123"),
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::MandatoryKey);

        assert_eq!(resolved, Ok(Some(OperationKey::parse("op-123").unwrap())));
    }

    #[test]
    fn present_valid_key_resolves_identically_under_compatibility_mode() {
        // The mode governs the missing-key policy only — a present, valid
        // key resolves the same way regardless of mode.
        let carrier = TestCarrier {
            raw: RawOperationKey::Present("op-123"),
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::Compatibility);

        assert_eq!(resolved, Ok(Some(OperationKey::parse("op-123").unwrap())));
    }

    #[test]
    fn missing_key_rejected_under_the_default_mandatory_mode() {
        // The default is fail-closed: no key means no dispatch.
        let carrier = TestCarrier {
            raw: RawOperationKey::Absent,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::MandatoryKey);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Missing {
                carrier: "test:key"
            })
        );
    }

    #[test]
    fn missing_key_admitted_only_under_the_explicit_compatibility_mode() {
        // Admission happens only because the compatibility variant was
        // explicitly configured, which is what keeps the loosening auditable.
        let carrier = TestCarrier {
            raw: RawOperationKey::Absent,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::Compatibility);

        assert_eq!(resolved, Ok(None));
    }

    #[test]
    fn present_but_invalid_key_is_rejected_under_the_default_mandatory_mode() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Present("   "),
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::MandatoryKey);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Invalid {
                carrier: "test:key",
                source: OperationKeyError::Empty,
            })
        );
    }

    #[test]
    fn present_but_invalid_key_is_rejected_even_under_compatibility_mode() {
        // Compatibility only loosens the missing-key policy — it never
        // admits a key that failed validation. A malformed key is not
        // "absent"; treating it as such would silently widen what counts as
        // a valid `OperationKey`.
        let carrier = TestCarrier {
            raw: RawOperationKey::Present("   "),
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::Compatibility);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Invalid {
                carrier: "test:key",
                source: OperationKeyError::Empty,
            })
        );
    }

    /// An unreadable value is rejected under the fail-closed default, as any
    /// unusable key must be.
    #[test]
    fn unreadable_key_is_rejected_under_the_default_mandatory_mode() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Unreadable,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::MandatoryKey);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Unreadable {
                carrier: "test:key"
            })
        );
    }

    /// And rejected under compatibility too — which is the whole point of
    /// distinguishing it from an absent key. The compatibility variant loosens
    /// what happens when a caller sent *no* key; a caller who sent an unusable
    /// one did send a key, so admitting it would silently drop the guarantee
    /// for exactly the malformed input most likely to indicate a broken client.
    #[test]
    fn unreadable_key_is_rejected_under_compatibility_mode_too() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Unreadable,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::Compatibility);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Unreadable {
                carrier: "test:key"
            })
        );
    }

    /// Several keys at one location are rejected under the fail-closed default.
    /// Every value may be readable and even valid, so nothing here is malformed
    /// — what makes the request unusable is that no rule can say which key the
    /// caller meant, and inventing one is the guess this rejection refuses.
    #[test]
    fn ambiguous_key_is_rejected_under_the_default_mandatory_mode() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Ambiguous,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::MandatoryKey);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Ambiguous {
                carrier: "test:key"
            })
        );
    }

    /// And rejected under compatibility too. That variant loosens what happens
    /// when a caller sent *no* key; a caller who sent several sent keys, so
    /// admitting the request would mean quietly picking one of them — the very
    /// choice the state exists to keep an adapter from making, now made by the
    /// shared policy instead.
    #[test]
    fn ambiguous_key_is_rejected_under_compatibility_mode_too() {
        let carrier = TestCarrier {
            raw: RawOperationKey::Ambiguous,
        };

        let resolved = resolve_operation_key(&carrier, IdempotencyEnforcementMode::Compatibility);

        assert_eq!(
            resolved,
            Err(OperationKeyRejection::Ambiguous {
                carrier: "test:key"
            })
        );
    }
}
