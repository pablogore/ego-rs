//! Dependency-cycle detection via Tarjan's strongly-connected-components
//! algorithm (design.md AD-3b). Any SCC of size > 1 is a cycle.

use crate::layers::Graph;
use std::collections::HashMap;

/// Returns every strongly-connected component of size > 1 in `graph`,
/// members sorted alphabetically within each SCC and SCCs sorted by their
/// first member, for deterministic reporting.
pub fn find_cycles(graph: &Graph) -> Vec<Vec<String>> {
    let mut tarjan = Tarjan::new(graph);
    for node in graph.keys() {
        if !tarjan.indices.contains_key(node) {
            tarjan.strongconnect(node);
        }
    }
    let mut cycles: Vec<Vec<String>> = tarjan
        .sccs
        .into_iter()
        .filter(|scc| scc.len() > 1)
        .map(|mut scc| {
            scc.sort();
            scc
        })
        .collect();
    cycles.sort();
    cycles
}

struct Tarjan<'a> {
    graph: &'a Graph,
    index_counter: usize,
    stack: Vec<String>,
    on_stack: std::collections::HashSet<String>,
    indices: HashMap<String, usize>,
    lowlink: HashMap<String, usize>,
    sccs: Vec<Vec<String>>,
}

impl<'a> Tarjan<'a> {
    fn new(graph: &'a Graph) -> Self {
        Tarjan {
            graph,
            index_counter: 0,
            stack: Vec::new(),
            on_stack: std::collections::HashSet::new(),
            indices: HashMap::new(),
            lowlink: HashMap::new(),
            sccs: Vec::new(),
        }
    }

    fn strongconnect(&mut self, v: &str) {
        self.indices.insert(v.to_string(), self.index_counter);
        self.lowlink.insert(v.to_string(), self.index_counter);
        self.index_counter += 1;
        self.stack.push(v.to_string());
        self.on_stack.insert(v.to_string());

        let empty: Vec<String> = Vec::new();
        let deps = self.graph.get(v).unwrap_or(&empty).clone();
        for w in &deps {
            if !self.indices.contains_key(w) {
                self.strongconnect(w);
                let w_low = self.lowlink[w];
                let v_low = self.lowlink[v];
                self.lowlink.insert(v.to_string(), v_low.min(w_low));
            } else if self.on_stack.contains(w) {
                let w_idx = self.indices[w];
                let v_low = self.lowlink[v];
                self.lowlink.insert(v.to_string(), v_low.min(w_idx));
            }
        }

        if self.lowlink[v] == self.indices[v] {
            let mut scc = Vec::new();
            loop {
                let w = self
                    .stack
                    .pop()
                    .expect("stack non-empty within its own SCC");
                self.on_stack.remove(&w);
                let is_v = w == v;
                scc.push(w);
                if is_v {
                    break;
                }
            }
            self.sccs.push(scc);
        }
    }
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

    #[test]
    fn detects_a_three_crate_cycle() {
        let graph = graph_from(&[("a", &["b"]), ("b", &["c"]), ("c", &["a"])]);

        let cycles = find_cycles(&graph);

        assert_eq!(cycles.len(), 1);
        let mut members = cycles[0].clone();
        members.sort();
        assert_eq!(
            members,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn reports_no_cycle_for_an_acyclic_graph() {
        let graph = graph_from(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);

        assert!(find_cycles(&graph).is_empty());
    }
}
