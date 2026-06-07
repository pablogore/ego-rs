//! Event tag — a string tag used to partition event streams.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A partition key defining a logical event stream.
///
/// Tags are precomputed by `EventTagger` at event creation time.
/// The runtime never recalculates tags.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventTag {
    /// The tag value (e.g., "order-123", "payment").
    value: String,
}

impl EventTag {
    /// Creates a new `EventTag`.
    ///
    /// # Panics
    /// Panics if `value` is empty.
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        assert!(!value.is_empty(), "EventTag value must not be empty");
        Self { value }
    }

    /// Returns the tag value.
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for EventTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl std::str::FromStr for EventTag {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Err("EventTag value must not be empty")
        } else {
            Ok(Self::new(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tag() {
        let tag = EventTag::new("order");
        assert_eq!(tag.value(), "order");
    }

    #[test]
    #[should_panic(expected = "EventTag value must not be empty")]
    fn test_new_tag_empty() {
        EventTag::new("");
    }

    #[test]
    fn test_display() {
        let tag = EventTag::new("payment");
        assert_eq!(format!("{}", tag), "payment");
    }

    #[test]
    fn test_from_str() {
        let tag: EventTag = "shipping".parse().unwrap();
        assert_eq!(tag.value(), "shipping");
    }

    #[test]
    fn test_from_str_empty() {
        assert!("".parse::<EventTag>().is_err());
    }

    #[test]
    fn test_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(EventTag::new("order"));
        set.insert(EventTag::new("payment"));
        assert_eq!(set.len(), 2);
        assert!(set.contains(&EventTag::new("order")));
    }

    #[test]
    fn test_equality() {
        let t1 = EventTag::new("test");
        let t2 = EventTag::new("test");
        let t3 = EventTag::new("other");
        assert_eq!(t1, t2);
        assert_ne!(t1, t3);
    }
}
