//! Configuration value types.

/// A typed configuration value produced by a provider.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    /// A UTF-8 string value.
    Str(String),
    /// A 64-bit signed integer value.
    Int(i64),
    /// A 64-bit floating-point value.
    Float(f64),
    /// A boolean value.
    Bool(bool),
    /// A list of values. Reserved for OQ-04; no provider populates this in v1.
    List(Vec<ConfigValue>),
}

/// Identifies which provider loaded a specific configuration key.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceAttribution {
    /// The name of the provider that supplied this value.
    pub provider_name: String,
    /// The configuration key.
    pub key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_variant_stores_string() {
        let v = ConfigValue::Str("hello".to_string());
        match v {
            ConfigValue::Str(s) => assert_eq!(s, "hello"),
            _ => panic!("expected Str variant"),
        }
    }

    #[test]
    fn str_variant_different_value() {
        let v = ConfigValue::Str("world".to_string());
        match v {
            ConfigValue::Str(s) => assert_eq!(s, "world"),
            _ => panic!("expected Str variant"),
        }
    }

    #[test]
    fn bool_variant_construction() {
        let t = ConfigValue::Bool(true);
        let f = ConfigValue::Bool(false);
        match (t, f) {
            (ConfigValue::Bool(true), ConfigValue::Bool(false)) => {}
            _ => panic!("bool variants incorrect"),
        }
    }

    #[test]
    fn int_variant_construction() {
        let v = ConfigValue::Int(42);
        match v {
            ConfigValue::Int(n) => assert_eq!(n, 42),
            _ => panic!("expected Int variant"),
        }
    }

    #[test]
    fn float_variant_construction() {
        let v = ConfigValue::Float(3.14);
        match v {
            ConfigValue::Float(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            _ => panic!("expected Float variant"),
        }
    }

    #[test]
    fn list_variant_compiles() {
        // ponytail: List is reserved for OQ-04 — just verify the variant exists and compiles
        let v = ConfigValue::List(vec![ConfigValue::Str("a".to_string())]);
        match v {
            ConfigValue::List(items) => assert_eq!(items.len(), 1),
            _ => panic!("expected List variant"),
        }
    }

    #[test]
    fn source_attribution_fields() {
        let attr = SourceAttribution {
            provider_name: "env".to_string(),
            key: "server.port".to_string(),
        };
        assert_eq!(attr.provider_name, "env");
        assert_eq!(attr.key, "server.port");
    }

    #[test]
    fn source_attribution_different_provider() {
        let attr = SourceAttribution {
            provider_name: "toml:/etc/app.toml".to_string(),
            key: "database.url".to_string(),
        };
        assert_eq!(attr.provider_name, "toml:/etc/app.toml");
        assert_eq!(attr.key, "database.url");
    }
}
