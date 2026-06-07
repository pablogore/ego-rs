use async_trait::async_trait;

#[async_trait]
pub trait EventPublisher<E>: Send + Sync {
    async fn publish(&self, events: &[E]) -> Result<(), ()>;
}
