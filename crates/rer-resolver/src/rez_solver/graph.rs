//! Small directed-graph utilities for the solver's `finalise` step: cycle
//! detection and dependency ordering. These stand in for rez's vendored
//! `pygraph` `find_cycle`, `accessibility` and `_get_dependency_order`.

use std::collections::{HashMap, HashSet};

/// A directed graph keyed by package family name.
#[derive(Debug, Default)]
pub struct DepGraph {
    nodes: HashSet<String>,
    /// `node -> direct successors`.
    edges: HashMap<String, Vec<String>>,
}

impl DepGraph {
    /// An empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node (idempotent).
    pub fn add_node(&mut self, node: impl Into<String>) {
        self.nodes.insert(node.into());
    }

    /// Add a directed edge `from -> to`, adding both endpoints as nodes.
    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        let from = from.into();
        let to = to.into();
        self.nodes.insert(from.clone());
        self.nodes.insert(to.clone());
        let succ = self.edges.entry(from).or_default();
        if !succ.contains(&to) {
            succ.push(to);
        }
    }

    fn successors(&self, node: &str) -> &[String] {
        self.edges.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Find one dependency cycle, returned as the list of nodes forming it, or
    /// an empty vec if the graph is acyclic. Mirrors pygraph's `find_cycle`
    /// closely enough for failure reporting — the *exact* cycle returned only
    /// affects the error message, not solve correctness.
    pub fn find_cycle(&self) -> Vec<String> {
        // DFS with a node colouring: 0 = unvisited, 1 = on the current path,
        // 2 = fully explored. A grey (1) target means a back-edge → cycle.
        let mut colour: HashMap<&str, u8> = HashMap::new();
        let mut path: Vec<String> = Vec::new();

        // Visit nodes in sorted order for determinism.
        let mut roots: Vec<&String> = self.nodes.iter().collect();
        roots.sort();

        for root in roots {
            if colour.get(root.as_str()).copied().unwrap_or(0) == 0 {
                if let Some(cycle) = self.dfs_cycle(root, &mut colour, &mut path) {
                    return cycle;
                }
            }
        }
        Vec::new()
    }

    fn dfs_cycle<'a>(
        &'a self,
        node: &'a str,
        colour: &mut HashMap<&'a str, u8>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        colour.insert(node, 1);
        path.push(node.to_string());

        let mut succ: Vec<&String> = self.successors(node).iter().collect();
        succ.sort();
        for next in succ {
            match colour.get(next.as_str()).copied().unwrap_or(0) {
                0 => {
                    if let Some(cycle) = self.dfs_cycle(next, colour, path) {
                        return Some(cycle);
                    }
                }
                1 => {
                    // Back-edge: the cycle is the path slice from `next`.
                    let start = path.iter().position(|n| n == next).unwrap();
                    return Some(path[start..].to_vec());
                }
                _ => {}
            }
        }

        path.pop();
        colour.insert(node, 2);
        None
    }

    /// For each node, the set of nodes reachable from it (including itself).
    /// Mirrors pygraph's `accessibility`.
    fn accessibility(&self) -> HashMap<String, HashSet<String>> {
        let mut result: HashMap<String, HashSet<String>> = HashMap::new();
        for node in &self.nodes {
            let mut reachable = HashSet::new();
            let mut stack = vec![node.clone()];
            while let Some(n) = stack.pop() {
                if reachable.insert(n.clone()) {
                    for s in self.successors(&n) {
                        if !reachable.contains(s) {
                            stack.push(s.clone());
                        }
                    }
                }
            }
            result.insert(node.clone(), reachable);
        }
        result
    }

    /// Order nodes as close as possible to `node_list`, but with child nodes
    /// (dependencies) earlier than their parents. Ported from rez's
    /// `_get_dependency_order` (`solver.py:1192`).
    pub fn dependency_order(&self, node_list: &[String]) -> Vec<String> {
        let access = self.accessibility();
        // deps[k] = nodes reachable from k, excluding k itself.
        let deps: HashMap<&String, HashSet<String>> = access
            .iter()
            .map(|(k, v)| {
                let mut d = v.clone();
                d.remove(k);
                (k, d)
            })
            .collect();

        // node_list, then the remaining graph nodes sorted.
        let in_list: HashSet<&String> = node_list.iter().collect();
        let mut extra: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| !in_list.contains(n))
            .cloned()
            .collect();
        extra.sort();

        let mut nodes: Vec<String> = node_list.to_vec();
        nodes.extend(extra);

        let mut ordered: Vec<String> = Vec::new();
        while !nodes.is_empty() {
            let n_ = nodes[0].clone();
            let n_deps = deps.get(&n_);
            if ordered.contains(&n_) || n_deps.is_none() {
                nodes.remove(0);
                continue;
            }
            let n_deps = n_deps.unwrap();

            // Find the first later node that `n_` depends on; if found, move it
            // to the front and retry.
            let mut moved = false;
            for i in 1..nodes.len() {
                if n_deps.contains(&nodes[i]) {
                    let dep = nodes.remove(i);
                    nodes.insert(0, dep);
                    moved = true;
                    break;
                }
            }
            if !moved {
                ordered.push(n_);
                nodes.remove(0);
            }
        }
        ordered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_cycle() {
        let mut g = DepGraph::new();
        g.add_edge("a", "b");
        g.add_edge("b", "c");
        assert!(g.find_cycle().is_empty());
    }

    #[test]
    fn test_finds_cycle() {
        let mut g = DepGraph::new();
        g.add_edge("a", "b");
        g.add_edge("b", "c");
        g.add_edge("c", "a");
        let cycle = g.find_cycle();
        assert!(!cycle.is_empty());
        // every node in the cycle has a successor also in the cycle
        let set: HashSet<&String> = cycle.iter().collect();
        for n in &cycle {
            assert!(g.successors(n).iter().any(|s| set.contains(s)));
        }
    }

    #[test]
    fn test_dependency_order_children_first() {
        // app -> lib -> base ; request order [app]
        let mut g = DepGraph::new();
        g.add_edge("app", "lib");
        g.add_edge("lib", "base");
        let order = g.dependency_order(&["app".to_string()]);
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(pos("base") < pos("lib"));
        assert!(pos("lib") < pos("app"));
    }
}
