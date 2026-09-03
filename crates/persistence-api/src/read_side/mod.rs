//! Read-side projection SPIs — offset tracking, dedup, event fetch, and
//! state storage (CORE-PERSIST-A S1). Relocated verbatim from
//! `ego_domain::read_side::{offset,dedup,store,projection_state_store,
//! event_tag,state,event_stream}` (D-6); `ego-domain` re-exports each
//! module at its original name and path.

pub mod claim;
pub mod dedup;
pub mod event_stream;
pub mod event_tag;
pub mod offset;
pub mod projection_state;
pub mod state;
pub mod store;

pub use claim::{ClaimError, ClaimFence, ClaimId, ReadSideClaimStore};
