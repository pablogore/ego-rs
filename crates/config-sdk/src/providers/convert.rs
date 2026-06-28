//! Converts kit-config `serde_json::Value` into ego-config-sdk `ConfigValue` entries.
//!
//! Nested objects are flattened into dotted-path keys:
//! `{"server": {"host": "localhost"}}` → `"server.host" = Str("localhost")`

use std::collections::HashMap;

use serde_json::Value;

use crate::value::ConfigValue;

/// Flattens a `serde_json::Value` (potentially nested) into a flat map of
/// dotted-path keys to `ConfigValue`.
pub(crate) fn value_to_config_map(value: Value) -> HashMap<String, ConfigValue> {
    let mut out = HashMap::new();
    flatten("", &value, &mut out);
    out
}

fn flatten(prefix: &str, value: &Value, out: &mut HashMap<String, ConfigValue>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_owned(), ConfigValue::Str(s.clone()));
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.insert(prefix.to_owned(), ConfigValue::Int(i));
            } else if let Some(f) = n.as_f64() {
                out.insert(prefix.to_owned(), ConfigValue::Float(f));
            }
        }
        Value::Bool(b) => {
            out.insert(prefix.to_owned(), ConfigValue::Bool(*b));
        }
        Value::Array(arr) => {
            // ponytail: List reserved for OQ-04; scalar items mapped shallowly,
            // nested objects inside arrays are dropped (no dotted-key path available).
            let items: Vec<ConfigValue> = arr
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(ConfigValue::Str(s.clone())),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Some(ConfigValue::Int(i))
                        } else {
                            n.as_f64().map(ConfigValue::Float)
                        }
                    }
                    Value::Bool(b) => Some(ConfigValue::Bool(*b)),
                    _ => None, // objects and nulls inside arrays are dropped
                })
                .collect();
            out.insert(prefix.to_owned(), ConfigValue::List(items));
        }
        Value::Null => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flat_string_value() {
        let v = json!({"key": "value"});
        let m = value_to_config_map(v);
        assert_eq!(m["key"], ConfigValue::Str("value".into()));
    }

    #[test]
    fn nested_object_flattens_to_dotted_key() {
        let v = json!({"server": {"host": "localhost", "port": 8080}});
        let m = value_to_config_map(v);
        assert_eq!(m["server.host"], ConfigValue::Str("localhost".into()));
        assert_eq!(m["server.port"], ConfigValue::Int(8080));
    }

    #[test]
    fn bool_and_float_values() {
        let v = json!({"enabled": true, "ratio": 0.5});
        let m = value_to_config_map(v);
        assert_eq!(m["enabled"], ConfigValue::Bool(true));
        assert_eq!(m["ratio"], ConfigValue::Float(0.5));
    }

    #[test]
    fn null_values_are_skipped() {
        let v = json!({"present": "yes", "absent": null});
        let m = value_to_config_map(v);
        assert!(m.contains_key("present"));
        assert!(!m.contains_key("absent"));
    }

    #[test]
    fn deeply_nested_flattens_correctly() {
        let v = json!({"a": {"b": {"c": "deep"}}});
        let m = value_to_config_map(v);
        assert_eq!(m["a.b.c"], ConfigValue::Str("deep".into()));
    }

    #[test]
    fn string_array_becomes_list() {
        let v = json!({"tags": ["a", "b", "c"]});
        let m = value_to_config_map(v);
        assert_eq!(
            m["tags"],
            ConfigValue::List(vec![
                ConfigValue::Str("a".into()),
                ConfigValue::Str("b".into()),
                ConfigValue::Str("c".into()),
            ])
        );
    }

    #[test]
    fn integer_array_becomes_list() {
        let v = json!({"ports": [8080, 8443]});
        let m = value_to_config_map(v);
        assert_eq!(
            m["ports"],
            ConfigValue::List(vec![ConfigValue::Int(8080), ConfigValue::Int(8443)])
        );
    }

    #[test]
    fn float_array_items_are_preserved() {
        let v = json!({"ratios": [0.5, 1.5]});
        let m = value_to_config_map(v);
        assert_eq!(
            m["ratios"],
            ConfigValue::List(vec![ConfigValue::Float(0.5), ConfigValue::Float(1.5)])
        );
    }

    #[test]
    fn nested_objects_inside_array_are_dropped() {
        // Objects inside arrays have no addressable dotted-key path — dropped intentionally.
        let v = json!({"items": [{"key": "val"}, "scalar"]});
        let m = value_to_config_map(v);
        assert_eq!(
            m["items"],
            ConfigValue::List(vec![ConfigValue::Str("scalar".into())])
        );
    }
}
