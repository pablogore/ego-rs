//! `ReadSideClaimStore` — the port through which one worker obtains single
//! valid processing ownership of a `(projection_id, tag, tenant)` stream
//! before fetching or handling any event (PROD-014C).
//!
//! Every mutating call (`renew`, `release`) MUST verify the full
//! `claim_id + owner_id + fencing_token` triple inside the same statement
//! that mutates. A caller whose claim was taken over receives
//! [`ClaimError::StaleOwner`] and its call MUST leave the claim unmodified.

use chrono::{DateTime, Utc};

use super::event_tag::EventTag;
use crate::operation::reservation::{FencingToken, OwnerId};

/// The claim identity — exactly `projection_offsets`' primary key shape:
/// `(projection_id, tag, tenant)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaimId {
    /// The projection this claim guards processing for.
    pub projection_id: String,
    /// The event tag stream within that projection.
    pub tag: EventTag,
    /// The tenant scope.
    pub tenant: String,
}

/// The full verification triple every mutating call presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFence {
    /// The claim being mutated.
    pub claim_id: ClaimId,
    /// The caller claiming ownership.
    pub owner_id: OwnerId,
    /// The fencing token the caller was granted when it last (re)gained the
    /// claim.
    pub fencing_token: FencingToken,
}

/// Errors returned by [`ReadSideClaimStore`] operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    /// The presented fence no longer matches the current claim — typically
    /// because the claim was taken over. The claim is guaranteed unmodified
    /// by this call.
    #[error("stale owner: the presented fence no longer matches the current claim")]
    StaleOwner,
    /// No strictly greater token can be minted, so takeover cannot proceed
    /// safely. Unreachable in practice; represented rather than wrapped.
    #[error("fencing token sequence exhausted for this claim")]
    FencingExhausted,
    /// A transient error (e.g. connection issue) — safe to retry.
    #[error("transient claim store error: {0}")]
    Transient(String),
    /// A fatal error (e.g. data corruption) — not safe to retry.
    #[error("fatal claim store error: {0}")]
    Fatal(String),
}

/// The capability port through which one worker obtains single valid
/// processing ownership of a `(projection_id, tag, tenant)` stream.
///
/// Every mutating call (`renew`, `release`) MUST verify the full
/// `claim_id + owner_id + fencing_token` triple inside the same statement
/// that mutates. A caller whose claim was taken over receives
/// `ClaimError::StaleOwner` and its call MUST leave the claim unmodified.
#[async_trait::async_trait]
pub trait ReadSideClaimStore: Send + Sync {
    /// Whether claims obtained through this store survive a process restart.
    ///
    /// Defaults to `false`, mirroring `OffsetStore::is_durable` — honest for
    /// every implementation that has not considered the question.
    /// `Profile::Production` reads this.
    fn is_durable(&self) -> bool {
        false
    }

    /// Obtains the claim, or reports that a live claim already holds it.
    ///
    /// `Ok(None)` is a refusal, not a failure: another worker holds an
    /// unexpired lease. The caller MUST NOT fetch or invoke the handler.
    /// `Ok(Some(fence))` is granted, whether fresh or taken over from a
    /// lapsed owner; the fence carries a strictly greater token than any
    /// this identity previously issued.
    ///
    /// `lease_until` is computed by the caller (`clock.now() + configured
    /// lease`), never by the store.
    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError>;

    /// Extends an owned, still-valid claim to `lease_until`.
    ///
    /// MUST reject a stale fence AND an already-lapsed lease with
    /// `StaleOwner`, leaving the claim unmodified — a lapsed holder
    /// resurrecting its claim would defeat a takeover that was already
    /// legitimate.
    async fn renew(
        &self,
        fence: &ClaimFence,
        lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError>;

    /// Releases an owned, still-valid claim, making the stream immediately
    /// claimable without waiting for expiry. Same fence rule as `renew`.
    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError>;
}

/// Forwards through a shared handle, so a composition root can hold the
/// store as `Arc<dyn ReadSideClaimStore + Send + Sync>` and still hand that
/// exact value to whatever spawns the claiming session. Without this, the
/// registered store and the spawned store could never be the same value
/// (PROD-014A EC-2).
#[async_trait::async_trait]
impl<T: ReadSideClaimStore + Send + Sync + ?Sized> ReadSideClaimStore for std::sync::Arc<T> {
    /// **Load-bearing.** Omitting this silently inherits the trait's `false`
    /// default, and every registered store would be classified volatile no
    /// matter what the host wrapped — the gate would refuse a correct
    /// durable composition and pass nothing.
    fn is_durable(&self) -> bool {
        (**self).is_durable()
    }

    async fn try_claim(
        &self,
        claim_id: &ClaimId,
        owner_id: &OwnerId,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<ClaimFence>, ClaimError> {
        (**self).try_claim(claim_id, owner_id, lease_until).await
    }

    async fn renew(
        &self,
        fence: &ClaimFence,
        lease_until: DateTime<Utc>,
    ) -> Result<(), ClaimError> {
        (**self).renew(fence, lease_until).await
    }

    async fn release(&self, fence: &ClaimFence) -> Result<(), ClaimError> {
        (**self).release(fence).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn claim_id(tenant: &str) -> ClaimId {
        ClaimId {
            projection_id: "proj".to_string(),
            tag: EventTag::new("users-by-tenant"),
            tenant: tenant.to_string(),
        }
    }

    fn fence(tenant: &str, owner: &str, token: u64) -> ClaimFence {
        ClaimFence {
            claim_id: claim_id(tenant),
            owner_id: OwnerId::new(owner),
            fencing_token: FencingToken::from_value(token),
        }
    }

    struct BareClaimStore;

    #[async_trait::async_trait]
    impl ReadSideClaimStore for BareClaimStore {
        async fn try_claim(
            &self,
            _claim_id: &ClaimId,
            _owner_id: &OwnerId,
            _lease_until: DateTime<Utc>,
        ) -> Result<Option<ClaimFence>, ClaimError> {
            unreachable!("this fixture exists only to exercise is_durable's default")
        }

        async fn renew(
            &self,
            _fence: &ClaimFence,
            _lease_until: DateTime<Utc>,
        ) -> Result<(), ClaimError> {
            unreachable!()
        }

        async fn release(&self, _fence: &ClaimFence) -> Result<(), ClaimError> {
            unreachable!()
        }
    }

    /// A bare implementation that never overrides `is_durable()` must be
    /// classified volatile by default.
    #[test]
    fn bare_impl_defaults_is_durable_to_false() {
        assert!(!BareClaimStore.is_durable());
    }

    #[derive(Default)]
    struct DurableClaimStore {
        held: std::sync::Mutex<Option<ClaimFence>>,
    }

    #[async_trait::async_trait]
    impl ReadSideClaimStore for DurableClaimStore {
        fn is_durable(&self) -> bool {
            true
        }

        async fn try_claim(
            &self,
            claim_id: &ClaimId,
            owner_id: &OwnerId,
            _lease_until: DateTime<Utc>,
        ) -> Result<Option<ClaimFence>, ClaimError> {
            let granted = ClaimFence {
                claim_id: claim_id.clone(),
                owner_id: owner_id.clone(),
                fencing_token: FencingToken::initial(),
            };
            *self.held.lock().unwrap() = Some(granted.clone());
            Ok(Some(granted))
        }

        async fn renew(
            &self,
            _fence: &ClaimFence,
            _lease_until: DateTime<Utc>,
        ) -> Result<(), ClaimError> {
            Ok(())
        }

        async fn release(&self, _fence: &ClaimFence) -> Result<(), ClaimError> {
            *self.held.lock().unwrap() = None;
            Ok(())
        }
    }

    /// PROD-014A EC-2/AD-3 landmine: `Arc<T>` MUST forward `is_durable()`.
    /// Without this, a composition root holding the store as
    /// `Arc<dyn ReadSideClaimStore + Send + Sync>` would classify every
    /// registered store volatile no matter what it wraps.
    #[test]
    fn arc_forwards_is_durable() {
        let store: Arc<dyn ReadSideClaimStore + Send + Sync> =
            Arc::new(DurableClaimStore::default());
        assert!(store.is_durable(), "Arc<T> must forward is_durable()");
    }

    /// `Arc<T>` must also forward `try_claim`/`renew`/`release`, proving the
    /// same `Arc` handle a composition root registers is fully usable by
    /// whatever spawns the claiming session.
    #[tokio::test]
    async fn arc_forwards_try_claim_renew_and_release() {
        let store: Arc<dyn ReadSideClaimStore + Send + Sync> =
            Arc::new(DurableClaimStore::default());
        let id = claim_id("tenant-a");
        let owner = OwnerId::new("owner-1");
        let now = Utc::now();

        let granted = store
            .try_claim(&id, &owner, now)
            .await
            .unwrap()
            .expect("the durable double always grants");

        store.renew(&granted, now).await.unwrap();
        store.release(&granted).await.unwrap();
    }

    #[test]
    fn claim_id_equality_is_by_full_triple() {
        let a = claim_id("tenant-a");
        let b = claim_id("tenant-a");
        let c = claim_id("tenant-b");
        assert_eq!(a, b);
        assert_ne!(a, c);

        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut ha = DefaultHasher::new();
        a.hash(&mut ha);
        let mut hb = DefaultHasher::new();
        b.hash(&mut hb);
        assert_eq!(ha.finish(), hb.finish(), "equal ids must hash equal");
    }

    #[test]
    fn claim_fence_equality_is_by_full_triple() {
        let f1 = fence("tenant-a", "owner-1", 1);
        let f2 = fence("tenant-a", "owner-1", 1);
        assert_eq!(f1, f2);

        assert_ne!(f1, fence("tenant-b", "owner-1", 1), "claim_id must differ");
        assert_ne!(f1, fence("tenant-a", "owner-2", 1), "owner_id must differ");
        assert_ne!(
            f1,
            fence("tenant-a", "owner-1", 2),
            "fencing_token must differ"
        );
    }
}
