use crate::persistence::PersistenceFacade;
use crate::error::EntityError;
use async_trait::async_trait;
use ego_domain::event::DomainEvent;

#[async_trait]
pub trait StateRecovery: Send + 'static {
    type State: Send + 'static;
    type Event: DomainEvent + Clone + serde::de::DeserializeOwned + 'static;

    async fn recover(
        &self,
        persistence: &PersistenceFacade<Self::Event>,
        aggregate_id: &str,
        tenant_id: Option<&str>,
    ) -> Result<(Self::State, u64), EntityError>;
}
