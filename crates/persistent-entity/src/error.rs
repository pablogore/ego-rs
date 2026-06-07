use ego_domain::persistence::PersistenceError;

#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    #[error("entity not found: {0}")]
    EntityNotFound(String),

    #[error("version conflict: expected {expected}, current {current}")]
    VersionConflict { expected: u64, current: u64 },

    #[error("entity is passivating, retry later")]
    EntityPassivating,

    #[error("mailbox at capacity ({0})")]
    MailboxFull(usize),

    #[error("handler error: {0}")]
    Handler(String),

    #[error("runtime error: {0}")]
    Runtime(String),
}

impl From<PersistenceError> for EntityError {
    fn from(err: PersistenceError) -> Self {
        match err {
            PersistenceError::Conflict { .. } => {
                EntityError::VersionConflict {
                    expected: 0,
                    current: 0,
                }
            }
            PersistenceError::NotFound { .. } => {
                EntityError::Runtime(err.to_string())
            }
            PersistenceError::MissingTenant => {
                EntityError::Runtime("missing tenant".into())
            }
            PersistenceError::Internal(msg) => {
                EntityError::Runtime(msg)
            }
        }
    }
}
