//! Opaque subject identifier type.

use crate::error::SecurityError;

/// Opaque subject identifier — a non-empty string chosen by the provider.
///
/// No format is enforced at the core level. Examples like `"user:123"` or
/// `"service:billing"` are illustrative; the `AuthenticationProvider`
/// decides the actual structure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubjectId(String);

impl SubjectId {
    /// Creates a `SubjectId` from a non-empty string.
    ///
    /// # Errors
    /// Returns [`SecurityError::InvalidSubjectId`] if `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, SecurityError> {
        let v = value.into();
        if v.is_empty() {
            return Err(SecurityError::InvalidSubjectId(
                "subject id must not be empty".into(),
            ));
        }
        Ok(Self(v))
    }

    /// Returns the full subject id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::SubjectId;
    use crate::error::SecurityError;

    #[test]
    fn non_empty_string_accepted() {
        let result = SubjectId::new("user:123");
        assert!(result.is_ok(), "expected Ok for non-empty string");
        assert_eq!(result.unwrap().as_str(), "user:123");
    }

    #[test]
    fn arbitrary_non_empty_accepted() {
        let result = SubjectId::new("service:billing");
        assert!(result.is_ok(), "expected Ok for 'service:billing'");
    }

    #[test]
    fn empty_string_rejected() {
        let result = SubjectId::new("");
        assert!(
            matches!(result, Err(SecurityError::InvalidSubjectId(_))),
            "expected Err(InvalidSubjectId) for empty string, got: {:?}",
            result
        );
    }

    #[test]
    fn as_str_roundtrip() {
        let sid = SubjectId::new("agent:x").unwrap();
        assert_eq!(sid.as_str(), "agent:x");
    }
}
