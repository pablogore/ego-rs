/// A runtime builder.
#[derive(Debug, Clone)]
pub struct RuntimeBuilder {
    /// The services to include in the runtime.
    pub services: Vec<String>,
    /// The dependencies to include in the runtime.
    pub dependencies: Vec<Dependency>,
}

/// A dependency in the runtime.
#[derive(Debug, Clone)]
pub struct Dependency {
    /// The type ID of the dependency.
    pub type_id: String,
    /// The name of the dependency.
    pub name: Option<String>,
    /// The version of the dependency.
    pub version: Option<String>,
}

/// An error that can occur in the runtime.
#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// A service was not found.
    ServiceNotFound,
    /// A dependency was not found.
    DependencyNotFound,
    /// A dependency cycle was detected.
    DependencyCycle,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    /// Creates a new runtime builder.
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    /// Registers an entity with the runtime.
    pub fn with_entity<E>(mut self) -> Self
    where
        E: 'static,
    {
        // Register entity type in dependency resolver
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<E>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a projection with the runtime.
    pub fn with_projection<P>(mut self) -> Self
    where
        P: 'static,
    {
        // Register projection type in dependency resolver
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<P>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a service with the runtime.
    pub fn with_service<S>(mut self) -> Self
    where
        S: 'static,
    {
        // Register service type in dependency resolver
        self.dependencies.push(Dependency {
            type_id: std::any::type_name::<S>().to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Registers a service bundle with the runtime.
    pub fn with_service_bundle(mut self, bundle: &str) -> Self {
        // Register bundle in dependency resolver
        self.dependencies.push(Dependency {
            type_id: bundle.to_string(),
            name: None,
            version: None,
        });
        self
    }

    /// Builds the runtime.
    pub async fn build(self) -> Result<Runtime, RuntimeError> {
        // Validate all dependencies
        // Construct service graph
        // Return operational runtime
        Ok(Runtime {})
    }
}

/// A runtime.
#[derive(Debug, Clone)]
pub struct Runtime {
    // Runtime state
}
