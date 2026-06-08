//! Shared activation coordination for single-flight entity activation.
//!
//! Provides [`SharedActivation`] to synchronize concurrent activation attempts
//! so only one actor is spawned per entity.

use tokio::sync::{Mutex, watch};

/// Coordinates concurrent activation attempts for a single entity.
///
/// Uses a mutex for single-flight activation and a watch channel to broadcast
/// the activation result to all waiters.
pub struct SharedActivation {
    /// Mutex ensuring only one activation proceeds at a time.
    pub lock: Mutex<()>,
    /// Sends the activation result to waiters.
    pub result_tx: watch::Sender<Option<super::error::EntityError>>,
    /// Receives the activation result.
    pub result_rx: watch::Receiver<Option<super::error::EntityError>>,
}

impl SharedActivation {
    /// Creates a new [`SharedActivation`] with an initially empty result.
    pub fn new() -> Self {
        let (result_tx, result_rx) = watch::channel(None);
        SharedActivation {
            lock: Mutex::new(()),
            result_tx,
            result_rx,
        }
    }
}
