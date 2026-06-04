use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationId(String);

impl CorrelationId {
    pub fn new(value: String) -> Result<Self, EmptyCorrelationId> {
        if value.is_empty() {
            Err(EmptyCorrelationId)
        } else {
            Ok(CorrelationId(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyCorrelationId;

impl fmt::Display for EmptyCorrelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "correlation_id must not be empty")
    }
}

impl Error for EmptyCorrelationId {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    correlation_id: Option<CorrelationId>,
}

impl CommandContext {
    pub fn new(correlation_id: Option<CorrelationId>) -> Self {
        CommandContext { correlation_id }
    }

    pub fn correlation_id(&self) -> Option<&CorrelationId> {
        self.correlation_id.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_correlation_id() {
        let cid = CorrelationId::new("abc-123".to_string()).unwrap();
        assert_eq!(cid.as_str(), "abc-123");
    }

    #[test]
    fn test_empty_string_rejected() {
        let result = CorrelationId::new("".to_string());
        assert!(matches!(result, Err(EmptyCorrelationId)));
    }

    #[test]
    fn test_clone_preserves_value() {
        let cid = CorrelationId::new("clone-test".to_string()).unwrap();
        let cloned = cid.clone();
        assert_eq!(cid.as_str(), cloned.as_str());
    }

    #[test]
    fn test_command_context_with_correlation_id() {
        let cid = CorrelationId::new("abc-123".to_string()).unwrap();
        let ctx = CommandContext::new(Some(cid.clone()));
        assert_eq!(ctx.correlation_id(), Some(&cid));
    }

    #[test]
    fn test_none_correlation_id() {
        let ctx = CommandContext::new(None);
        assert!(ctx.correlation_id().is_none());
    }

    #[test]
    fn test_immutability() {
        let ctx = CommandContext::new(None);
        let _ = ctx.correlation_id();
    }

    #[test]
    fn test_clone_preserves_id() {
        let cid = CorrelationId::new("clone-test".to_string()).unwrap();
        let ctx = CommandContext::new(Some(cid));
        let cloned = ctx.clone();
        assert_eq!(ctx.correlation_id().map(|c| c.as_str()), cloned.correlation_id().map(|c| c.as_str()));
    }

    #[test]
    fn test_retry_reuses_same_context() {
        let cid = CorrelationId::new("retry-1".to_string()).unwrap();
        let ctx = CommandContext::new(Some(cid));
        let retry_ctx = ctx.clone();
        assert_eq!(ctx.correlation_id().map(|c| c.as_str()), retry_ctx.correlation_id().map(|c| c.as_str()));
    }
}
