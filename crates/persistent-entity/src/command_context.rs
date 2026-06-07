use ego_domain::context::{CorrelationId, CausationId, Metadata, RequestId};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CommandContext {
    correlation_id: Option<CorrelationId>,
    causation_id: Option<CausationId>,
    request_id: Option<RequestId>,
    metadata: Metadata,
}

impl CommandContext {
    pub fn new() -> Self {
        CommandContext {
            correlation_id: None,
            causation_id: None,
            request_id: None,
            metadata: HashMap::new(),
        }
    }

    pub fn with_correlation_id(mut self, id: CorrelationId) -> Self {
        self.correlation_id = Some(id);
        self
    }

    pub fn with_causation_id(mut self, id: CausationId) -> Self {
        self.causation_id = Some(id);
        self
    }

    pub fn with_request_id(mut self, id: RequestId) -> Self {
        self.request_id = Some(id);
        self
    }

    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn correlation_id(&self) -> Option<&CorrelationId> {
        self.correlation_id.as_ref()
    }

    pub fn causation_id(&self) -> Option<&CausationId> {
        self.causation_id.as_ref()
    }

    pub fn request_id(&self) -> Option<&RequestId> {
        self.request_id.as_ref()
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

impl Default for CommandContext {
    fn default() -> Self {
        Self::new()
    }
}
