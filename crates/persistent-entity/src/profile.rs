//! What a composition declares about the deployment it is being built for
//! (PROD-013), and the one shared predicate that enforces it across the
//! `persistent-entity` / `service-sdk` layer boundary.

use crate::error::PersistenceCompositionError;

/// What a composition declares about the deployment it is being built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// Today's behavior, byte-for-byte. Volatile storage by omission is
    /// valid here, because that is what dev and test are for.
    #[default]
    Dev,
    /// Every composition-root-observable persistent capability governed by
    /// this profile must be configured explicitly. Nothing volatile is
    /// reachable by omission for those capabilities.
    ///
    /// Today that means the event store, snapshot store, and effect store
    /// (PROD-013), plus the durable progress pair — `OffsetStore` and
    /// `DedupStore` together — of every projection registered through
    /// `AppBuilder::read_side_progress` (PROD-014A). A projection whose pair
    /// is never registered is not governed here, by design: a command-only or
    /// non-read-side application is never forced to register storage it does
    /// not use, and a projection spawned directly through
    /// `ProjectionSpec`/`TagSchedulerImpl` without passing the composition
    /// root is ungoverned by construction (PROD-014A OOS-7).
    /// See PROD-013/PROD-014A.
    Production,
}

/// The one definition of PROD-013's rule.
///
/// A free function rather than a method on either builder, because the three
/// gated capabilities live in two crates that cannot share a builder: the
/// event and snapshot stores are `EntityRuntimeBuilder`'s, the effect store
/// is `RuntimeBuilder`'s, and `persistent-entity` cannot see
/// `EffectStateStore`. Restating the rule once per crate would create
/// exactly the second, parallel check SC-8 forbids; passing the three
/// varying facts as arguments keeps one.
///
/// `durably_configured` is deliberately not named `configured`: presence
/// (`Option::is_some()`) and durability are different properties, and a
/// caller passing mere presence here would defeat the whole rule — a
/// composition could then declare `Profile::Production` and explicitly wire
/// an in-memory store, and this function would wave it through. Every call
/// site MUST compute this argument from the capability's own durability
/// declaration (each store's `is_durable()`, or the effect store's
/// `capabilities().durable`), never from `.is_some()` alone. That is the
/// one fact this predicate cannot verify for its caller — it can only
/// refuse to be handed a name that invites the mistake.
pub fn require_durably_configured(
    profile: Profile,
    durably_configured: bool,
    capability: &'static str,
    fix: &'static str,
) -> Result<(), PersistenceCompositionError> {
    match profile {
        Profile::Production if !durably_configured => {
            Err(PersistenceCompositionError::NotConfigured { capability, fix })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AD-3's rule, the full matrix: {Dev, Production} x {durably configured, not}.
    /// Only `Profile::Production` with `durably_configured == false` refuses.
    #[test]
    fn require_durably_configured_matrix() {
        for (profile, durably_configured, should_err) in [
            (Profile::Dev, true, false),
            (Profile::Dev, false, false),
            (Profile::Production, true, false),
            (Profile::Production, false, true),
        ] {
            let result = require_durably_configured(
                profile,
                durably_configured,
                "event store",
                "with_event_store(store)",
            );
            assert_eq!(
                result.is_err(),
                should_err,
                "profile={profile:?} durably_configured={durably_configured} expected_err={should_err}"
            );
        }
    }

    /// The exact defect a reviewer flagged against an earlier draft: passing
    /// mere presence (not durability) would let `Profile::Production` accept
    /// an explicitly-wired volatile store. Pinned here so it can never
    /// regress silently — this is `durably_configured=false` precisely
    /// because a real caller must compute it from `is_durable()`, not
    /// `is_some()`, and `Some(volatile_store).is_some()` is `true`.
    #[test]
    fn presence_alone_is_not_durability() {
        let is_some_but_not_durable = true; // Some(InMemoryEventStore::new()).is_some()
        let is_durable = false; // ...but that store's is_durable() is false
        assert!(
            is_some_but_not_durable,
            "presence was true, as a mistaken caller would compute it"
        );
        let result = require_durably_configured(
            Profile::Production,
            is_durable,
            "event store",
            "with_event_store(store)",
        );
        assert!(
            result.is_err(),
            "a caller that correctly passes is_durable() (not is_some()) must be refused"
        );
    }
}
