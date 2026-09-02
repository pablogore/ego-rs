//! Layer-map parsing, the allowed-dependency matrix, and the direction +
//! completeness checks (design.md §2, §3 AD-3a/c).

use std::collections::BTreeMap;
use std::path::Path;

pub type Graph = BTreeMap<String, Vec<String>>;
pub type LayerMap = BTreeMap<String, String>;

/// The only layer names `allowed_layers` understands. A `layers.toml` entry
/// naming anything else is a data error, not a crate with zero permitted
/// dependencies — `check_completeness` rejects it explicitly rather than
/// letting it silently behave like `domain`.
pub const KNOWN_LAYERS: &[&str] = &[
    "domain",
    "foundation",
    "cross-cutting",
    "application",
    "infrastructure",
    "sdk",
    "transport",
    "tooling",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    WrongDirection {
        from: String,
        from_layer: String,
        to: String,
        to_layer: String,
    },
    Cycle(Vec<String>),
    UnmappedCrate(String),
    DeadLayerEntry(String),
    InvalidLayer {
        crate_name: String,
        layer: String,
    },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::WrongDirection {
                from,
                from_layer,
                to,
                to_layer,
            } => write!(
                f,
                "wrong-direction dependency: {from} ({from_layer}) -> {to} ({to_layer}) is not permitted by the layer direction rules"
            ),
            Violation::Cycle(members) => {
                write!(f, "dependency cycle: {}", members.join(" -> "))
            }
            Violation::UnmappedCrate(name) => {
                write!(f, "unmapped crate: {name} has no layers.toml entry")
            }
            Violation::DeadLayerEntry(name) => {
                write!(f, "dead layer-map entry: {name} does not name a real workspace crate")
            }
            Violation::InvalidLayer { crate_name, layer } => write!(
                f,
                "invalid layer: {crate_name} is mapped to unknown layer \"{layer}\" (expected one of {})",
                KNOWN_LAYERS.join(", ")
            ),
        }
    }
}

/// The allowed-dependency matrix (design.md §2). `None` means the layer is a
/// sink that may depend on anything (`tooling`).
pub fn allowed_layers(layer: &str) -> Option<&'static [&'static str]> {
    match layer {
        "domain" => Some(&["domain"]),
        "foundation" => Some(&["domain", "foundation"]),
        "cross-cutting" => Some(&["domain"]),
        "application" => Some(&["domain"]),
        "infrastructure" => Some(&[
            "domain",
            "application",
            "foundation",
            "cross-cutting",
            "infrastructure",
        ]),
        "sdk" => Some(&["domain", "foundation", "cross-cutting"]),
        "transport" => Some(&["domain", "application", "cross-cutting", "sdk"]),
        "tooling" => None,
        _ => Some(&[]),
    }
}

/// FR-002: a crate's dependency MUST NOT point to a layer its own layer is
/// not allowed to depend on. Crates missing from `layers` are skipped here;
/// completeness owns that failure class.
pub fn check_direction(graph: &Graph, layers: &LayerMap) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (from, deps) in graph {
        let Some(from_layer) = layers.get(from) else {
            continue;
        };
        for to in deps {
            let Some(to_layer) = layers.get(to) else {
                continue;
            };
            if let Some(allowed) = allowed_layers(from_layer) {
                if !allowed.contains(&to_layer.as_str()) {
                    violations.push(Violation::WrongDirection {
                        from: from.clone(),
                        from_layer: from_layer.clone(),
                        to: to.clone(),
                        to_layer: to_layer.clone(),
                    });
                }
            }
        }
    }
    violations
}

/// FR-001: every crate in `crates` MUST have a `layers` entry, every `layers`
/// entry MUST name a crate present in `crates`, and every `layers` value
/// MUST be one of `KNOWN_LAYERS`.
pub fn check_completeness(crates: &[String], layers: &LayerMap) -> Vec<Violation> {
    let mut violations = Vec::new();
    for c in crates {
        if !layers.contains_key(c) {
            violations.push(Violation::UnmappedCrate(c.clone()));
        }
    }
    let crate_set: std::collections::BTreeSet<&str> = crates.iter().map(String::as_str).collect();
    for (name, layer) in layers {
        if !crate_set.contains(name.as_str()) {
            violations.push(Violation::DeadLayerEntry(name.clone()));
        }
        if !KNOWN_LAYERS.contains(&layer.as_str()) {
            violations.push(Violation::InvalidLayer {
                crate_name: name.clone(),
                layer: layer.clone(),
            });
        }
    }
    violations
}

/// Parses `layers.toml`'s `[layers]` table.
pub fn load_layers_toml(path: &Path) -> anyhow::Result<LayerMap> {
    #[derive(serde::Deserialize)]
    struct File {
        layers: LayerMap,
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let file: File =
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
    Ok(file.layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_from(pairs: &[(&str, &[&str])]) -> Graph {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn layers_from(pairs: &[(&str, &str)]) -> LayerMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn direction_check_fails_on_edge_target_layer_not_in_allowed_set() {
        // application -> infrastructure is forbidden (application may only
        // depend on domain, design.md §2 matrix).
        let graph = graph_from(&[("app-crate", &["infra-crate"]), ("infra-crate", &[])]);
        let layers = layers_from(&[
            ("app-crate", "application"),
            ("infra-crate", "infrastructure"),
        ]);

        let violations = check_direction(&graph, &layers);

        assert_eq!(
            violations,
            vec![Violation::WrongDirection {
                from: "app-crate".into(),
                from_layer: "application".into(),
                to: "infra-crate".into(),
                to_layer: "infrastructure".into(),
            }]
        );
    }

    #[test]
    fn direction_check_passes_when_edge_target_layer_is_allowed() {
        // application -> domain is allowed.
        let graph = graph_from(&[("app-crate", &["domain-crate"]), ("domain-crate", &[])]);
        let layers = layers_from(&[("app-crate", "application"), ("domain-crate", "domain")]);

        assert!(check_direction(&graph, &layers).is_empty());
    }

    #[test]
    fn completeness_fails_when_workspace_crate_missing_from_map() {
        let crates = vec!["mapped".to_string(), "unmapped".to_string()];
        let layers = layers_from(&[("mapped", "domain")]);

        let violations = check_completeness(&crates, &layers);

        assert_eq!(
            violations,
            vec![Violation::UnmappedCrate("unmapped".into())]
        );
    }

    #[test]
    fn completeness_fails_when_map_entry_names_nonexistent_crate() {
        let crates = vec!["real-crate".to_string()];
        let layers = layers_from(&[("real-crate", "domain"), ("runtime-slice", "domain")]);

        let violations = check_completeness(&crates, &layers);

        assert_eq!(
            violations,
            vec![Violation::DeadLayerEntry("runtime-slice".into())]
        );
    }

    #[test]
    fn completeness_passes_when_map_matches_crate_set_exactly() {
        let crates = vec!["a".to_string(), "b".to_string()];
        let layers = layers_from(&[("a", "domain"), ("b", "foundation")]);

        assert!(check_completeness(&crates, &layers).is_empty());
    }

    #[test]
    fn direction_check_passes_on_domain_to_domain_self_edge() {
        // CORE-PERSIST-A AD-1/SC-7: a domain-layer crate MAY depend on
        // another domain-layer crate (the ego-domain -> ego-persistence-api
        // edge). This is the narrow same-layer self-edge, not a wider hole.
        let graph = graph_from(&[("ego-domain", &["ego-persistence-api"]), ("ego-persistence-api", &[])]);
        let layers = layers_from(&[
            ("ego-domain", "domain"),
            ("ego-persistence-api", "domain"),
        ]);

        assert!(check_direction(&graph, &layers).is_empty());
    }

    #[test]
    fn direction_check_still_fails_domain_to_foundation() {
        let graph = graph_from(&[("domain-crate", &["foundation-crate"]), ("foundation-crate", &[])]);
        let layers = layers_from(&[
            ("domain-crate", "domain"),
            ("foundation-crate", "foundation"),
        ]);

        assert_eq!(
            check_direction(&graph, &layers),
            vec![Violation::WrongDirection {
                from: "domain-crate".into(),
                from_layer: "domain".into(),
                to: "foundation-crate".into(),
                to_layer: "foundation".into(),
            }]
        );
    }

    #[test]
    fn direction_check_still_fails_domain_to_infrastructure() {
        let graph = graph_from(&[("domain-crate", &["infra-crate"]), ("infra-crate", &[])]);
        let layers = layers_from(&[
            ("domain-crate", "domain"),
            ("infra-crate", "infrastructure"),
        ]);

        assert_eq!(
            check_direction(&graph, &layers),
            vec![Violation::WrongDirection {
                from: "domain-crate".into(),
                from_layer: "domain".into(),
                to: "infra-crate".into(),
                to_layer: "infrastructure".into(),
            }]
        );
    }

    #[test]
    fn direction_check_still_fails_domain_to_sdk() {
        let graph = graph_from(&[("domain-crate", &["sdk-crate"]), ("sdk-crate", &[])]);
        let layers = layers_from(&[("domain-crate", "domain"), ("sdk-crate", "sdk")]);

        assert_eq!(
            check_direction(&graph, &layers),
            vec![Violation::WrongDirection {
                from: "domain-crate".into(),
                from_layer: "domain".into(),
                to: "sdk-crate".into(),
                to_layer: "sdk".into(),
            }]
        );
    }

    #[test]
    fn completeness_rejects_unknown_layer_name() {
        let crates = vec!["a".to_string()];
        let layers = layers_from(&[("a", "domian")]); // typo, not a KNOWN_LAYERS value

        let violations = check_completeness(&crates, &layers);

        assert_eq!(
            violations,
            vec![Violation::InvalidLayer {
                crate_name: "a".into(),
                layer: "domian".into(),
            }]
        );
    }
}
