use crate::context::ServiceContext;

/// Shared state held by the runtime and referenced by generated proxies via `Weak<RuntimeInner>`.
pub struct RuntimeInner {}

impl RuntimeInner {
    /// Stub: no-op; returns `()` (not `Result`) so proxies impose no `From<ServiceError>` bound.
    pub fn enforce_tenant(&self, _ctx: &ServiceContext) {}
}

#[derive(Debug, Clone)]
pub struct RuntimeBuilder {
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub type_id: String,
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub enum RuntimeError {
    ServiceNotFound,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            dependencies: Vec::new(),
        }
    }

    pub fn with_entity<E: 'static>(mut self) -> Self {
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<E>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    pub fn with_projection<P: 'static>(mut self) -> Self {
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<P>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    pub fn with_service<S: 'static>(mut self) -> Self {
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<S>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    pub fn with_service_bundle(mut self, bundle: &str) -> Self {
        self.dependencies.push(Dependency {
            type_id: bundle.to_string(),
            name: None,
            version: None,
        });
        self
    }

    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        Ok(Runtime {})
    }
}

#[derive(Debug, Clone)]
pub struct Runtime {}
