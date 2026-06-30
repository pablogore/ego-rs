//! PrincipalMapper SPI — maps verified claim sets to domain identity.
//!
//! This trait is the contract between JWT/introspection infrastructure and
//! the domain identity model. All implementations are in `security-jwt`.

use ego_domain::auth::{AuthenticationError, ClaimSet, Claims};

use crate::principal::Principal;

/// Maps a verified [`ClaimSet`] to a `(Principal, Claims)` pair.
///
/// Called for every successfully verified token — no provider hardcodes
/// claim extraction (INV-3). Custom implementations may map vendor-specific
/// claims (e.g. `preferred_username`) without touching the interceptor.
///
/// The `&ClaimSet` parameter keeps `security-sdk` free of `serde_json` (INV-8).
pub trait PrincipalMapper: Send + Sync {
    /// Map the verified claim set to a principal + claims pair.
    ///
    /// # Errors
    ///
    /// Returns [`AuthenticationError::MissingClaim`] when a required claim
    /// (e.g. `sub`) is absent.
    fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError>;
}

// Compile-time assertions live in tests, not here.
// Object-safety test below proves `Arc<dyn PrincipalMapper>` compiles.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use ego_domain::auth::{ClaimSet, ClaimValue, Claims};
    use crate::principal::{Principal, PrincipalKind, SubjectId};

    // --- Object-safety and Send + Sync ---

    #[test]
    fn principal_mapper_is_object_safe() {
        struct StubMapper;
        impl PrincipalMapper for StubMapper {
            fn map(&self, _: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError> {
                unimplemented!()
            }
        }
        let _: Arc<dyn PrincipalMapper> = Arc::new(StubMapper);
    }

    #[test]
    fn principal_mapper_dyn_is_send_sync() {
        fn assert_send_sync<T: ?Sized + Send + Sync>() {}
        assert_send_sync::<dyn PrincipalMapper>();
    }

    // --- Minimal stub impl works without importing security-jwt ---

    struct FixedMapper;

    impl PrincipalMapper for FixedMapper {
        fn map(&self, claims: &ClaimSet) -> Result<(Principal, Claims), AuthenticationError> {
            let sub = claims
                .subject()
                .ok_or_else(|| AuthenticationError::MissingClaim("sub".into()))?;
            let principal = Principal::new(
                PrincipalKind::User,
                SubjectId::new(sub)
                    .map_err(|_| AuthenticationError::InvalidToken("bad sub".into()))?,
            );
            Ok((principal, Claims::empty()))
        }
    }

    #[test]
    fn stub_mapper_maps_sub_to_principal() {
        let mut raw = BTreeMap::new();
        raw.insert("sub".to_string(), ClaimValue::String("user-42".into()));
        let cs = ClaimSet::new(raw);
        let mapper = FixedMapper;
        let (principal, _claims) = mapper.map(&cs).unwrap();
        assert_eq!(principal.subject_id.as_str(), "user-42");
    }

    #[test]
    fn stub_mapper_returns_missing_claim_for_absent_sub() {
        let cs = ClaimSet::new(BTreeMap::new());
        let err = FixedMapper.map(&cs).unwrap_err();
        assert!(matches!(err, AuthenticationError::MissingClaim(ref s) if s == "sub"));
    }
}
