//! Priority-based resolver (internal).

use std::collections::HashMap;

use crate::error::ConfigurationError;
use crate::source::ConfigurationSource;
use crate::value::ConfigValue;

/// Conflict-resolution policy for sources at equal priority.
pub(crate) enum ConflictPolicy {
    /// First (highest-priority) source wins; equal-priority ties silently use the first encountered.
    FirstWins,
    /// Any two equal-priority sources sharing a key is a hard error.
    Strict,
}

/// Resolves a set of post-load [`ConfigurationSource`] snapshots according to a conflict policy.
///
/// Operates on immutable snapshots; all resolution is done at construction time.
pub(crate) struct ConfigurationResolver {
    /// Snapshots sorted **descending** by priority (highest priority first).
    sources: Vec<(u32, ConfigurationSource)>,
    policy: ConflictPolicy,
}

impl ConfigurationResolver {
    /// Builds a resolver from loaded source snapshots and a conflict policy.
    ///
    /// Sorts `sources` descending by priority (stable sort — ties preserve insertion order).
    pub(crate) fn new(
        mut sources: Vec<(u32, ConfigurationSource)>,
        policy: ConflictPolicy,
    ) -> Self {
        // Stable sort descending so equal-priority sources retain insertion order.
        sources.sort_by_key(|(p, _)| std::cmp::Reverse(*p));
        Self { sources, policy }
    }

    /// Resolves all keys, returning a flat map of `key → (string_value, provider_name)`.
    ///
    /// Under `FirstWins`: highest-priority source that contains a key wins.
    /// Under `Strict`: equal-priority sources sharing a key produce `Conflict` errors;
    /// all conflicts are collected before returning.
    pub(crate) fn resolve(
        &self,
    ) -> Result<HashMap<String, (String, String)>, Vec<ConfigurationError>> {
        match self.policy {
            ConflictPolicy::FirstWins => Ok(self.resolve_first_wins()),
            ConflictPolicy::Strict => self.resolve_strict(),
        }
    }

    fn resolve_first_wins(&self) -> HashMap<String, (String, String)> {
        let mut out: HashMap<String, (String, String)> = HashMap::new();
        for (_, src) in &self.sources {
            for key in src.keys() {
                // Only the first (highest-priority) source claiming this key wins.
                if !out.contains_key(key) {
                    let val = value_to_string(src.get(key).unwrap());
                    out.insert(key.to_owned(), (val, src.provider_name().to_owned()));
                }
            }
        }
        out
    }

    fn resolve_strict(&self) -> Result<HashMap<String, (String, String)>, Vec<ConfigurationError>> {
        let mut errors: Vec<ConfigurationError> = Vec::new();
        let mut out: HashMap<String, (String, String)> = HashMap::new();

        // Process sources grouped into priority tiers.
        let mut i = 0;
        while i < self.sources.len() {
            let tier_priority = self.sources[i].0;
            let tier_start = i;
            while i < self.sources.len() && self.sources[i].0 == tier_priority {
                i += 1;
            }
            let tier = &self.sources[tier_start..i];

            // Collect which providers in this tier have each key.
            let mut tier_key_owners: HashMap<String, Vec<String>> = HashMap::new();
            for (_, src) in tier {
                for key in src.keys() {
                    tier_key_owners
                        .entry(key.to_owned())
                        .or_default()
                        .push(src.provider_name().to_owned());
                }
            }

            // For each key in this tier, either record a conflict or the resolved value.
            for (key, providers) in &tier_key_owners {
                if providers.len() > 1 {
                    errors.push(ConfigurationError::Conflict {
                        key: key.clone(),
                        sources: providers.clone(),
                    });
                } else if !out.contains_key(key) {
                    // Exactly one provider in this tier has the key — safe to take it.
                    let (_, src) = tier
                        .iter()
                        .find(|(_, s)| s.get(key).is_some())
                        .unwrap();
                    let val = value_to_string(src.get(key).unwrap());
                    out.insert(key.clone(), (val, src.provider_name().to_owned()));
                }
            }
        }

        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        }
    }
}

/// Converts a [`ConfigValue`] to its canonical UTF-8 string for storage in [`Configuration`].
///
/// [`Configuration`]: crate::config::Configuration
fn value_to_string(v: &ConfigValue) -> String {
    match v {
        ConfigValue::Str(s) => s.clone(),
        ConfigValue::Int(n) => n.to_string(),
        ConfigValue::Float(f) => f.to_string(),
        ConfigValue::Bool(b) => b.to_string(),
        ConfigValue::List(_) => String::new(), // ponytail: List reserved for OQ-04
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::ConfigValue;
    use std::collections::HashMap;

    fn make_source(provider: &str, pairs: &[(&str, &str)], priority: u32) -> (u32, ConfigurationSource) {
        let mut map = HashMap::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), ConfigValue::Str(v.to_string()));
        }
        let source = ConfigurationSource::new(map, provider.to_string())
            .expect("valid test source");
        (priority, source)
    }

    // S-02: FirstWins — higher priority wins on overlapping key
    #[test]
    fn first_wins_higher_priority_wins() {
        let cli = make_source("cli", &[("port", "9090")], 30);
        let env = make_source("env", &[("port", "8080")], 20);
        let resolver = ConfigurationResolver::new(vec![cli, env], ConflictPolicy::FirstWins);
        let result = resolver.resolve().expect("no conflict under FirstWins");
        assert_eq!(result["port"].0, "9090", "cli (priority 30) must win over env (priority 20)");
        assert_eq!(result["port"].1, "cli");
    }

    // S-02 triangulation: lower-priority source's unique keys are still included
    #[test]
    fn first_wins_includes_non_overlapping_keys_from_lower_priority() {
        let cli = make_source("cli", &[("port", "9090")], 30);
        let env = make_source("env", &[("port", "8080"), ("host", "localhost")], 20);
        let resolver = ConfigurationResolver::new(vec![cli, env], ConflictPolicy::FirstWins);
        let result = resolver.resolve().expect("no conflict");
        assert_eq!(result["port"].0, "9090");
        assert_eq!(result["host"].0, "localhost", "unique key from env must be present");
    }

    // S-03: Strict — equal-priority sources with same key → Conflict error
    #[test]
    fn strict_equal_priority_conflict() {
        let a = make_source("toml-a", &[("db.url", "postgres://a")], 10);
        let b = make_source("toml-b", &[("db.url", "postgres://b")], 10);
        let resolver = ConfigurationResolver::new(vec![a, b], ConflictPolicy::Strict);
        let errs = resolver.resolve().expect_err("should be a conflict");
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            ConfigurationError::Conflict { key, sources } => {
                assert_eq!(key, "db.url");
                assert!(sources.contains(&"toml-a".to_string()));
                assert!(sources.contains(&"toml-b".to_string()));
            }
            other => panic!("expected Conflict, got: {other:?}"),
        }
    }

    // S-03 triangulation: Strict no-conflict — distinct keys at same priority succeed
    #[test]
    fn strict_equal_priority_no_conflict_distinct_keys() {
        let a = make_source("toml-a", &[("a.key", "1")], 10);
        let b = make_source("toml-b", &[("b.key", "2")], 10);
        let resolver = ConfigurationResolver::new(vec![a, b], ConflictPolicy::Strict);
        let result = resolver.resolve().expect("no conflict — distinct keys");
        assert_eq!(result["a.key"].0, "1");
        assert_eq!(result["b.key"].0, "2");
    }

    // S-13: empty sources → empty result, no error
    #[test]
    fn empty_sources_returns_empty_map() {
        let resolver = ConfigurationResolver::new(vec![], ConflictPolicy::FirstWins);
        let result = resolver.resolve().expect("no error on empty sources");
        assert!(result.is_empty(), "empty sources must produce empty map");
    }

    // Strict — multiple conflicts are all collected before returning (no early exit)
    #[test]
    fn strict_collects_all_conflicts() {
        let a = make_source("src-a", &[("x", "1"), ("y", "2")], 10);
        let b = make_source("src-b", &[("x", "10"), ("y", "20")], 10);
        let resolver = ConfigurationResolver::new(vec![a, b], ConflictPolicy::Strict);
        let errs = resolver.resolve().expect_err("both x and y conflict");
        assert_eq!(errs.len(), 2, "both conflicts must be collected; got: {errs:?}");
    }

    // ponytail: List → "" is the documented ceiling until OQ-04; this test pins the behavior.
    #[test]
    fn list_value_to_string_returns_empty() {
        let list = ConfigValue::List(vec![ConfigValue::Str("a".into())]);
        assert_eq!(value_to_string(&list), "");
    }

    // Priority-sort invariant: insertion order must not affect resolution
    #[test]
    fn priority_sort_invariant_insertion_order_irrelevant() {
        // Insert low-priority first, high-priority last — resolver must still sort correctly.
        let env = make_source("env", &[("key", "from-env")], 20);
        let cli = make_source("cli", &[("key", "from-cli")], 30);
        let resolver = ConfigurationResolver::new(vec![env, cli], ConflictPolicy::FirstWins);
        let result = resolver.resolve().expect("no conflict");
        assert_eq!(result["key"].0, "from-cli", "priority sort must work regardless of insertion order");
    }
}
