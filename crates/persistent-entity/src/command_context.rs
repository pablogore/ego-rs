//! Command context information available during command processing.
//!
//! This module provides context information that is available during command processing,
//! including metadata about the command execution environment.

use ego_domain::operation::OperationIdentity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context information available during command processing.
///
/// This struct contains metadata about the command execution environment,
/// including tenant information, version information, and other relevant data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContext {
    /// The tenant identifier for the command.
    pub tenant_id: Option<String>,

    /// The entity type identifier.
    pub entity_type: String,

    /// The entity identifier.
    pub entity_id: String,

    /// The expected version for optimistic concurrency control.
    pub expected_version: Option<u64>,

    /// The causation identifier for the command.
    pub causation_id: Option<String>,

    /// Additional metadata for the command.
    pub metadata: HashMap<String, String>,

    /// The identity of the operation this command belongs to — which operation,
    /// and which request it came from — carried unchanged from the reservation
    /// that accepted it.
    ///
    /// Part of the public contract, and additive: it carries data, never
    /// behaviour. Three rules make it trustworthy.
    ///
    /// **The two halves are one value.** A key without a fingerprint would not
    /// be a partial identity, it would be an identity the receipt gate must
    /// ignore: with only the key, a retry cannot be told apart from a different
    /// command reusing that key. [`OperationIdentity`] makes that state
    /// unconstructible, so a service body cannot transfer one half and silently
    /// leave the guarantee off for this aggregate while appearing to switch it
    /// on.
    ///
    /// **The actor never recomputes it.** A fingerprint derived a second time
    /// from a re-serialised request can differ from the first for reasons that
    /// have nothing to do with the request changing — map ordering, float
    /// formatting, an added default field. A retry would then look like a
    /// different request and be refused, so the value is carried rather than
    /// recovered.
    ///
    /// **`None` keeps the non-idempotent path.** A command that arrived without
    /// idempotency enforcement has no identity to record, and inventing one
    /// would manufacture an identity the caller never asked for.
    pub identity: Option<OperationIdentity>,
}

impl CommandContext {
    /// Create a new command context.
    ///
    /// # Arguments
    /// * `entity_type` - The type of the entity
    ///
    /// # Returns
    /// * `CommandContext` - A new command context with default values
    pub fn new(entity_type: String) -> Self {
        Self {
            tenant_id: None,
            entity_type,
            entity_id: String::new(),
            expected_version: None,
            causation_id: None,
            metadata: HashMap::new(),
            identity: None,
        }
    }

    /// Carries an operation identity into this context, returning it.
    ///
    /// `None` is a legitimate argument and means this command belongs to no
    /// reserved operation — a dispatch that never reserved has no identity to
    /// hand down, and the receipt gate stays inactive rather than gating on one
    /// that nothing authorised.
    ///
    /// Written as one call per aggregate so a service body's transfer is a
    /// single act that either happened or did not, rather than two assignments
    /// one of which can be forgotten.
    pub fn carrying(mut self, identity: Option<OperationIdentity>) -> Self {
        self.identity = identity;
        self
    }
}
