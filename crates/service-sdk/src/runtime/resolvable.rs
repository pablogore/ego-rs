//! Resolvable trait and container for typed proxy resolution at runtime.
//!
//! The `#[service]` macro generates a `Resolvable` impl on every service tag,
//! enabling `Runtime::resolve<T: Resolvable>()` to produce typed proxies from
//! raw registry entries.

use std::any::Any;
use std::sync::{Arc, Weak};

use crate::contract::ServiceContract;
use crate::interceptor::InterceptorChain;

use super::{RuntimeError, RuntimeInner};

/// Concrete wrapper that enables storing `Arc<dyn Trait>` as `Arc<dyn Any>`.
///
/// # Why this exists
///
/// `Arc<dyn Any>` can be downcast to concrete types but NOT to `Arc<dyn Trait>`
/// because `dyn Trait` does not implement `Any`. This wrapper provides a concrete
/// `'static` type (`ResolvableContainer<T>`) that holds the `Arc<T>` and can be
/// stored in the registry as `Arc<dyn Any + Send + Sync>`.
///
/// At resolve time, the generated `Resolvable::create_proxy` impl downcasts back
/// to `Arc<ResolvableContainer<dyn Trait>>` and extracts the inner `Arc<dyn Trait>`.
#[repr(transparent)]
pub struct ResolvableContainer<T: ?Sized + Send + Sync + 'static>(pub Arc<T>);

// SAFETY: ResolvableContainer<T> is Send+Sync when T: Send+Sync because Arc<T> is Send+Sync.
unsafe impl<T: ?Sized + Send + Sync + 'static> Send for ResolvableContainer<T> {}
unsafe impl<T: ?Sized + Send + Sync + 'static> Sync for ResolvableContainer<T> {}

/// Trait generated on every service tag to enable typed proxy resolution.
///
/// Implemented by the `#[service]` macro for every annotated trait.
/// Connects a `Tag` type (registry key) to its corresponding `Proxy` type
/// and provides a factory method to create proxies from raw registry entries.
///
/// # Usage
///
/// ```ignore
/// let proxy: OrderServiceRef = runtime.resolve::<OrderServiceTag>().unwrap();
/// ```
pub trait Resolvable: ServiceContract {
    /// The proxy type returned by `Runtime::resolve`.
    type Proxy: Send + Sync;

    /// The trait object this tag fronts — the type link `with_service::<Tag>`
    /// needs to name the `Arc<Tag::Service>` it accepts. This is the only
    /// job of this associated type; it is not a general service descriptor
    /// (see design.md's scope note — further metadata belongs on
    /// `ServiceContract`, not here).
    type Service: ?Sized + Send + Sync + 'static;

    /// Creates a typed proxy from a raw registry entry.
    ///
    /// The `inner` must have been stored as `Arc<ResolvableContainer<dyn Trait>>`
    /// where `Trait` is the service trait associated with this tag.
    fn create_proxy(
        inner: Arc<dyn Any + Send + Sync>,
        chain: Arc<InterceptorChain>,
        runtime: Weak<RuntimeInner>,
    ) -> Result<Self::Proxy, RuntimeError>;
}
