//! Interceptor traits and implementations for service invocation.
//!
//! Interceptors allow instrumenting service calls with pre/post/error hooks.
//! They provide a mechanism for cross-cutting concerns like logging, tracing,
//! authentication, and metrics collection.

use crate::context::ServiceContext;
use crate::error::{ServiceError, ServiceErrorTrait};
use async_trait::async_trait;
use std::sync::Arc;

/// An interceptor for service invocations.
///
/// Interceptors provide a way to add cross-cutting concerns to service calls.
/// They can be used for:
/// - Logging and tracing
/// - Authentication and authorization
/// - Metrics collection
/// - Request/response transformation
///
/// Interceptors are executed in a chain, with each interceptor having the opportunity
/// to modify or inspect the service call before and after it executes.
#[async_trait]
pub trait Interceptor: Send + Sync {
    /// Called before a service invocation.
    ///
    /// # Arguments
    /// * `context` - The service context for the current invocation
    ///
    /// # Returns
    /// * `Ok(())` if the interceptor succeeds
    /// * `Err(ServiceError)` if the interceptor fails, which will prevent the service call
    async fn on_request(&self, context: &ServiceContext) -> Result<(), ServiceError>;

    /// Called after a successful service invocation.
    ///
    /// # Arguments
    /// * `context` - The service context for the current invocation
    ///
    /// # Returns
    /// * `Ok(())` if the interceptor succeeds
    /// * `Err(ServiceError)` if the interceptor fails
    async fn on_response(&self, context: &ServiceContext) -> Result<(), ServiceError>;

    /// Called when a service invocation fails.
    ///
    /// Receives a `&dyn ServiceErrorTrait` so interceptors are decoupled from
    /// the concrete error type. The original error is forwarded unchanged to the caller.
    ///
    /// # Arguments
    /// * `context` - The service context for the current invocation
    /// * `error` - The error that occurred, as a trait object
    ///
    /// # Returns
    /// * `Ok(())` if the interceptor succeeds
    /// * `Err(ServiceError)` if the interceptor itself fails
    async fn on_error(
        &self,
        context: &ServiceContext,
        error: &dyn ServiceErrorTrait,
    ) -> Result<(), ServiceError>;
}

/// A chain of interceptors.
///
/// This struct manages a sequence of interceptors that are executed in order
/// during service invocation. The interceptors are executed in the order they
/// are added to the chain.
#[derive(Default)]
pub struct InterceptorChain {
    interceptors: Vec<Arc<dyn Interceptor>>,
}

impl InterceptorChain {
    /// Creates a new interceptor chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an interceptor to the chain.
    pub fn add_interceptor(&mut self, interceptor: Arc<dyn Interceptor>) {
        self.interceptors.push(interceptor);
    }

    /// Runs the on_request hooks for all interceptors in the chain.
    pub async fn on_request(&self, context: &ServiceContext) -> Result<(), ServiceError> {
        for interceptor in &self.interceptors {
            interceptor.on_request(context).await?;
        }
        Ok(())
    }

    /// Runs the on_response hooks for all interceptors in the chain.
    pub async fn on_response(&self, context: &ServiceContext) -> Result<(), ServiceError> {
        for interceptor in &self.interceptors {
            interceptor.on_response(context).await?;
        }
        Ok(())
    }

    /// Runs the on_error hooks for all interceptors in the chain.
    ///
    /// Takes `&dyn ServiceErrorTrait` so interceptors are decoupled from the concrete error type.
    pub async fn on_error(
        &self,
        context: &ServiceContext,
        error: &dyn ServiceErrorTrait,
    ) -> Result<(), ServiceError> {
        for interceptor in &self.interceptors {
            interceptor.on_error(context, error).await?;
        }
        Ok(())
    }
}
