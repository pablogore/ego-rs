//! Interceptor traits and implementations for service invocation.
//!
//! Interceptors allow instrumenting service calls with pre/post/error hooks.

pub use builtin::TracingInterceptor;
pub use chain::{Interceptor, InterceptorChain};

mod builtin;
mod chain;
