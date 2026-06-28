//! Configuration value types.

/// A typed configuration value.
///
/// # Note on `Float` equality
///
/// `ConfigValue` derives `PartialEq` but not `Eq` because `f64` does not satisfy `Eq`.
/// `ConfigValue::Float(f64::NAN) != ConfigValue::Float(f64::NAN)` — NaN values from
/// configuration files should be treated as a parse error by callers.
#[non_exhaustive]
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
    /// A list of scalar values.
    #[doc(hidden)]
    List(Vec<ConfigValue>),
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
    fn float_nan_is_not_equal_to_itself() {
        let a = ConfigValue::Float(f64::NAN);
        let b = ConfigValue::Float(f64::NAN);
        assert_ne!(a, b, "NaN != NaN per IEEE 754 — callers must handle this");
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
}
