mod runtime_builder;

pub use runtime_builder::{Runtime, RuntimeBuilder, RuntimeError, RuntimeInner};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runtime_builder_creation() {
        let builder = RuntimeBuilder::new();
        assert_eq!(builder.services.len(), 0);
        assert_eq!(builder.dependencies.len(), 0);
    }

    #[tokio::test]
    async fn test_with_entity() {
        struct TestEntity;
        let builder = RuntimeBuilder::new().with_entity::<TestEntity>();
        assert_eq!(builder.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_with_projection() {
        struct TestProjection;
        let builder = RuntimeBuilder::new().with_projection::<TestProjection>();
        assert_eq!(builder.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_with_service() {
        struct TestService;
        let builder = RuntimeBuilder::new().with_service::<TestService>();
        assert_eq!(builder.dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_with_service_bundle() {
        let builder = RuntimeBuilder::new().with_service_bundle("test-bundle");
        assert_eq!(builder.dependencies.len(), 1);
    }
}
