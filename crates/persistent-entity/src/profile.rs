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
    /// (PROD-013) — the only capabilities with a real, generic
    /// composition-root registration slot to observe. Read-side/projection
    /// persistence has no such slot yet and is deliberately not governed
    /// here; PROD-014 introduces it and extends this same policy to cover
    /// it. See PROD-013/PROD-014.
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
pub fn require_configured(
    profile: Profile,
    configured: bool,
    capability: &'static str,
    fix: &'static str,
) -> Result<(), PersistenceCompositionError> {
    match profile {
        Profile::Production if !configured => {
            Err(PersistenceCompositionError::NotConfigured { capability, fix })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AD-3's rule, the full matrix: {Dev, Production} x {configured, not}.
    /// Only `Profile::Production` with `configured == false` refuses.
    #[test]
    fn require_configured_matrix() {
        for (profile, configured, should_err) in [
            (Profile::Dev, true, false),
            (Profile::Dev, false, false),
            (Profile::Production, true, false),
            (Profile::Production, false, true),
        ] {
            let result = require_configured(
                profile,
                configured,
                "event store",
                "with_event_store(store)",
            );
            assert_eq!(
                result.is_err(),
                should_err,
                "profile={profile:?} configured={configured} expected_err={should_err}"
            );
        }
    }
}
