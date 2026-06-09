//! Interceptor traits and implementations for service invocation.
//!
//! Interceptors allow instrumenting service calls with pre/post/error hooks.

pub use chain::{Interceptor, InterceptorChain};
// pub use builtin::{TracingInterceptor};

mod builtin;
mod chain;
