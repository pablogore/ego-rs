//! Fluent configuration builder.

use std::collections::HashMap;

use crate::config::Configuration;
use crate::error::ConfigurationError;
use crate::provider::ConfigurationProvider;
use crate::resolver::{ConfigurationResolver, ConflictPolicy as ResolverPolicy};
use crate::source::ConfigurationSource;
use crate::value::SourceAttribution;

/// Default priority tier constants for [`ConfigurationBuilder::with_default_precedence`].
pub mod priority {
    /// CLI arguments — highest precedence.
    pub const CLI: u32 = 30;
    /// Environment variables.
    pub const ENV: u32 = 20;
    /// File-based providers (TOML, YAML, JSON).
    pub const FILES: u32 = 10;
    /// Built-in defaults — lowest precedence.
    pub const DEFAULTS: u32 = 0;
}

/// Determines how the builder handles two equal-priority sources that share a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Highest-priority source wins; equal-priority ties use the first registered source.
    FirstWins,
    /// Equal-priority sources sharing a key produce a hard [`ConfigurationError::Conflict`].
    Strict,
}

/// Composable builder for constructing a resolved, immutable [`Configuration`].
///
/// Calls [`ConfigurationProvider::load`] on every registered source exactly once.
/// All errors are collected before returning — `build` never short-circuits.
pub struct ConfigurationBuilder {
    sources: Vec<(u32, Box<dyn ConfigurationProvider>)>,
    conflict_policy: ConflictPolicy,
    required_keys: Vec<String>,
}

impl Default for ConfigurationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigurationBuilder {
    /// Creates a new builder with no sources, `FirstWins` conflict policy, and no required keys.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            conflict_policy: ConflictPolicy::FirstWins,
            required_keys: Vec::new(),
        }
    }

    /// Registers a provider at the given `priority` tier (higher = more precedent).
    pub fn add_source(mut self, priority: u32, provider: impl ConfigurationProvider + 'static) -> Self {
        self.sources.push((priority, Box::new(provider)));
        self
    }

    /// Sets the conflict resolution policy.
    pub fn with_conflict_policy(mut self, policy: ConflictPolicy) -> Self {
        self.conflict_policy = policy;
        self
    }

    /// Declares that `key` must be present in the resolved configuration.
    ///
    /// If the key is absent after all sources are loaded and merged, `build` returns a
    /// [`ConfigurationError::Missing`] for it.
    pub fn require(mut self, key: impl Into<String>) -> Self {
        self.required_keys.push(key.into());
        self
    }

    /// Documents the default priority tiers (`CLI=30`, `ENV=20`, `FILES=10`, `DEFAULTS=0`).
    ///
    /// This method is a no-op — it does not add any providers.  Callers still need to
    /// [`add_source`](Self::add_source) for each provider; this exists to make the priority
    /// constants discoverable and to document intent.
    pub fn with_default_precedence(self) -> Self {
        self
    }

    /// Loads all registered providers and resolves their key-value pairs into a [`Configuration`].
    ///
    /// All errors (provider load failures, conflicts, and missing required keys) are collected
    /// before returning.  Returns `Ok` only when zero errors occurred.
    pub fn build(self) -> Result<Configuration, Vec<ConfigurationError>> {
        let mut errors: Vec<ConfigurationError> = Vec::new();
        let mut loaded: Vec<(u32, ConfigurationSource)> = Vec::new();
        let mut all_names: Vec<String> = Vec::new();

        for (priority, provider) in self.sources {
            let name = provider.name().to_owned();
            match provider.load() {
                Ok(src) => {
                    all_names.push(name);
                    loaded.push((priority, src));
                }
                Err(e) => errors.push(e),
            }
        }

        let resolver_policy = match self.conflict_policy {
            ConflictPolicy::FirstWins => ResolverPolicy::FirstWins,
            ConflictPolicy::Strict => ResolverPolicy::Strict,
        };

        let resolved: HashMap<String, (String, String)> = if !loaded.is_empty() {
            match ConfigurationResolver::new(loaded, resolver_policy).resolve() {
                Ok(map) => map,
                Err(mut conflict_errors) => {
                    errors.append(&mut conflict_errors);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        for key in &self.required_keys {
            if !resolved.contains_key(key.as_str()) {
                errors.push(ConfigurationError::Missing {
                    key: key.clone(),
                    searched_providers: all_names.clone(),
                });
            }
        }

        if errors.is_empty() {
            let entries = resolved
                .into_iter()
                .map(|(key, (value, provider_name))| {
                    let attr = SourceAttribution {
                        provider_name,
                        key: key.clone(),
                    };
                    (key, (value, attr))
                })
                .collect();
            Ok(Configuration::new(entries))
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigurationError;
    use crate::provider::ConfigurationProvider;
    use crate::source::ConfigurationSource;
    use crate::value::ConfigValue;
    use std::collections::HashMap;

    struct StubProvider {
        name: String,
        data: HashMap<String, ConfigValue>,
    }

    impl ConfigurationProvider for StubProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
            ConfigurationSource::new(self.data.clone(), self.name.clone())
        }
    }

    struct AlwaysErrProvider {
        name: String,
    }

    impl ConfigurationProvider for AlwaysErrProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn load(&self) -> Result<ConfigurationSource, ConfigurationError> {
            Err(ConfigurationError::ProviderLoad {
                provider_name: self.name.clone(),
                cause: "always fails".to_string(),
            })
        }
    }

    fn stub(name: &str, pairs: &[(&str, &str)]) -> StubProvider {
        StubProvider {
            name: name.to_string(),
            data: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), ConfigValue::Str(v.to_string())))
                .collect(),
        }
    }

    #[test]
    fn s01_single_provider_happy_path() {
        let cfg = ConfigurationBuilder::new()
            .add_source(10, stub("stub", &[("x", "42")]))
            .build()
            .expect("single provider should succeed");
        assert_eq!(cfg.get("x").unwrap(), "42");
    }

    #[test]
    fn s04_require_missing_key_returns_err() {
        let errs = ConfigurationBuilder::new()
            .add_source(10, stub("stub", &[]))
            .require("server.port")
            .build()
            .expect_err("missing required key must fail");
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], ConfigurationError::Missing { .. }));
    }

    #[test]
    fn s05_two_required_absent_keys_both_collected() {
        let errs = ConfigurationBuilder::new()
            .add_source(10, stub("stub", &[]))
            .require("a.key")
            .require("b.key")
            .build()
            .expect_err("both missing keys must fail");
        assert_eq!(errs.len(), 2);
        assert!(errs.iter().all(|e| matches!(e, ConfigurationError::Missing { .. })));
    }

    #[test]
    fn s10_provider_error_and_missing_key_both_collected() {
        let errs = ConfigurationBuilder::new()
            .add_source(10, AlwaysErrProvider { name: "bad".into() })
            .require("any.key")
            .build()
            .expect_err("provider load error + missing key must both appear");
        assert_eq!(errs.len(), 2, "expected ProviderLoad + Missing, got: {errs:?}");
        let has_load_err = errs.iter().any(|e| matches!(e, ConfigurationError::ProviderLoad { .. }));
        let has_missing = errs.iter().any(|e| matches!(e, ConfigurationError::Missing { .. }));
        assert!(has_load_err, "missing ProviderLoad error");
        assert!(has_missing, "missing Missing error");
    }

    #[test]
    fn s13_no_providers_no_required_keys_succeeds() {
        let cfg = ConfigurationBuilder::new()
            .build()
            .expect("empty builder should succeed");
        assert_eq!(cfg.keys().count(), 0);
    }

    #[test]
    fn default_conflict_policy_is_first_wins() {
        let cfg = ConfigurationBuilder::new()
            .add_source(20, stub("high", &[("x", "from-high")]))
            .add_source(10, stub("low", &[("x", "from-low")]))
            .build()
            .expect("first-wins should not conflict");
        assert_eq!(cfg.get("x").unwrap(), "from-high");
    }

    #[test]
    fn with_default_precedence_is_noop() {
        let cfg = ConfigurationBuilder::new()
            .with_default_precedence()
            .add_source(priority::FILES, stub("toml", &[("key", "val")]))
            .build()
            .expect("with_default_precedence is a no-op");
        assert_eq!(cfg.get("key").unwrap(), "val");
    }
}
