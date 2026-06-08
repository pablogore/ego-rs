//! A simple event publisher.
//!
//! This module provides a basic event publisher for domain events.

use std::sync::Arc;
use tokio::sync::Mutex;

/// A simple event publisher trait.
#[async_trait::async_trait]
pub trait EventPublisher<E> {
    /// Publish events.
    async fn publish(&self, events: &[E]) -> Result<(), String>;
}

/// A simple event publisher implementation.
#[derive(Debug)]
pub struct SimpleEventPublisher<E> {
    /// The published events.
    events: Arc<Mutex<Vec<E>>>,
}

impl<E: Send> SimpleEventPublisher<E> {
    /// Create a new event publisher.
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl<E: Send> EventPublisher<E> for SimpleEventPublisher<E> {
    /// Publish events.
    async fn publish(&self, _events: &[E]) -> Result<(), String> {
        // For now, do nothing
        Ok(())
    }
}