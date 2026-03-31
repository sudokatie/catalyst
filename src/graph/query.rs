//! Graph query operations

use std::collections::HashSet;
use std::fmt::Write;

use super::node::{Node, NodeId};
use super::resolver::Graph;
use crate::Label;

/// Query engine for build graphs
pub struct QueryEngine<'a> {
    graph: &'a Graph,
}

impl<'a> QueryEngine<'a> {
    /// Create a new query engine for the given graph
    pub fn new(graph: &'a Graph) -> Self {
        Self { graph }
    }

    /// Get direct dependencies of a target
    pub fn deps(&self, id: NodeId) -> Vec<NodeId> {
        self.graph.deps(id)
    }

    /// Get direct dependents (reverse deps) of a target
    pub fn rdeps(&self, id: NodeId) -> Vec<NodeId> {
        self.graph.rdeps(id)
    }

    /// Get all transitive dependencies
    pub fn transitive_deps(&self, id: NodeId) -> Vec<NodeId> {
        self.graph.transitive_deps(id)
    }

    /// Get all transitive reverse dependencies
    pub fn transitive_rdeps(&self, id: NodeId) -> Vec<NodeId> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut stack = vec![id];

        while let Some(current) = stack.pop() {
            for rdep in self.graph.rdeps(current) {
                if visited.insert(rdep.0) {
                    result.push(rdep);
                    stack.push(rdep);
                }
            }
        }

        result
    }

    /// Get topological order (dependencies first)
    pub fn topo_order(&self) -> Result<Vec<NodeId>, crate::Error> {
        self.graph.topo_order()
    }

    /// Get node by ID
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.graph.get(id)
    }

    /// Get node by label
    pub fn get_by_label(&self, label: &Label) -> Option<&Node> {
        self.graph.get_by_label(label)
    }

    /// Export graph to DOT format for visualization
    pub fn to_dot(&self) -> String {
        let mut dot = String::new();
        writeln!(dot, "digraph build {{").unwrap();
        writeln!(dot, "    rankdir=BT;").unwrap();
        writeln!(dot, "    node [shape=box];").unwrap();

        // Add nodes
        for label in self.graph.labels() {
            let node_name = label.to_string().replace(':', "_").replace('/', "_");
            let display = label.to_string();
            writeln!(dot, "    {} [label=\"{}\"];", node_name, display).unwrap();
        }

        // Add edges
        for label in self.graph.labels() {
            if let Some(node) = self.graph.get_by_label(label) {
                if let Node::Target(_) = node {
                    // Find node ID for this label
                    let from_name = label.to_string().replace(':', "_").replace('/', "_");

                    // Get the NodeId by looking it up
                    if let Some(from_id) = self.find_node_id(label) {
                        for dep_id in self.graph.deps(from_id) {
                            if let Some(Node::Target(dep)) = self.graph.get(dep_id) {
                                let to_name =
                                    dep.label.to_string().replace(':', "_").replace('/', "_");
                                writeln!(dot, "    {} -> {};", from_name, to_name).unwrap();
                            }
                        }
                    }
                }
            }
        }

        writeln!(dot, "}}").unwrap();
        dot
    }

    /// Export subgraph rooted at a target to DOT format
    pub fn subgraph_to_dot(&self, root: NodeId) -> String {
        let mut dot = String::new();
        writeln!(dot, "digraph build {{").unwrap();
        writeln!(dot, "    rankdir=BT;").unwrap();
        writeln!(dot, "    node [shape=box];").unwrap();

        // Collect all nodes in subgraph (root + transitive deps)
        let mut nodes = vec![root];
        nodes.extend(self.transitive_deps(root));
        let node_set: HashSet<usize> = nodes.iter().map(|n| n.0).collect();

        // Add nodes
        for &id in &nodes {
            if let Some(Node::Target(t)) = self.graph.get(id) {
                let node_name = t.label.to_string().replace(':', "_").replace('/', "_");
                let display = t.label.to_string();
                writeln!(dot, "    {} [label=\"{}\"];", node_name, display).unwrap();
            }
        }

        // Add edges
        for &id in &nodes {
            if let Some(Node::Target(t)) = self.graph.get(id) {
                let from_name = t.label.to_string().replace(':', "_").replace('/', "_");

                for dep_id in self.graph.deps(id) {
                    if node_set.contains(&dep_id.0) {
                        if let Some(Node::Target(dep)) = self.graph.get(dep_id) {
                            let to_name =
                                dep.label.to_string().replace(':', "_").replace('/', "_");
                            writeln!(dot, "    {} -> {};", from_name, to_name).unwrap();
                        }
                    }
                }
            }
        }

        writeln!(dot, "}}").unwrap();
        dot
    }

    /// Find node ID by label (helper)
    fn find_node_id(&self, label: &Label) -> Option<NodeId> {
        // Iterate through all nodes to find the matching one
        for i in 0..self.graph.node_count() {
            let id = NodeId(i);
            if let Some(Node::Target(t)) = self.graph.get(id) {
                if &t.label == label {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Get all targets that match a pattern
    pub fn filter_targets<F>(&self, predicate: F) -> Vec<NodeId>
    where
        F: Fn(&Label) -> bool,
    {
        let mut result = Vec::new();
        for i in 0..self.graph.node_count() {
            let id = NodeId(i);
            if let Some(Node::Target(t)) = self.graph.get(id) {
                if predicate(&t.label) {
                    result.push(id);
                }
            }
        }
        result
    }

    /// Get all targets in a package
    pub fn targets_in_package(&self, package: &str) -> Vec<NodeId> {
        self.filter_targets(|label| label.package == package)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Resolver;
    use std::fs;
    use std::io::Write as IoWrite;
    use tempfile::TempDir;

    fn create_test_workspace() -> TempDir {
        TempDir::new().unwrap()
    }

    fn write_build_file(dir: &std::path::Path, package: &str, content: &str) {
        let pkg_dir = dir.join(package);
        fs::create_dir_all(&pkg_dir).unwrap();
        let build_path = pkg_dir.join("BUILD");
        let mut file = fs::File::create(build_path).unwrap();
        write!(file, "{}", content).unwrap();
    }

    fn setup_test_graph() -> (TempDir, Resolver) {
        let ws = create_test_workspace();

        write_build_file(
            ws.path(),
            "base",
            r#"
rust_library(
    name = "base",
)
"#,
        );
        write_build_file(
            ws.path(),
            "util",
            r#"
rust_library(
    name = "util",
    deps = ["//base:base"],
)
"#,
        );
        write_build_file(
            ws.path(),
            "app",
            r#"
rust_binary(
    name = "app",
    deps = ["//util:util"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        resolver.resolve(&Label::new("app", "app")).unwrap();

        (ws, resolver)
    }

    #[test]
    fn get_transitive_deps() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        // Find app node
        let app_id = query
            .filter_targets(|l| l.name == "app")
            .into_iter()
            .next()
            .unwrap();

        let trans_deps = query.transitive_deps(app_id);

        // app depends transitively on util and base
        assert_eq!(trans_deps.len(), 2);
    }

    #[test]
    fn get_reverse_deps() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        // Find base node
        let base_id = query
            .filter_targets(|l| l.name == "base")
            .into_iter()
            .next()
            .unwrap();

        let rdeps = query.rdeps(base_id);

        // base is directly depended on by util
        assert_eq!(rdeps.len(), 1);
        if let Some(Node::Target(t)) = query.get(rdeps[0]) {
            assert_eq!(t.label.name, "util");
        } else {
            panic!("expected target");
        }
    }

    #[test]
    fn get_transitive_rdeps() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        // Find base node
        let base_id = query
            .filter_targets(|l| l.name == "base")
            .into_iter()
            .next()
            .unwrap();

        let trans_rdeps = query.transitive_rdeps(base_id);

        // base is transitively depended on by util and app
        assert_eq!(trans_rdeps.len(), 2);
    }

    #[test]
    fn topo_order_valid() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        let order = query.topo_order().unwrap();
        assert_eq!(order.len(), 3);

        // Find positions
        let base_pos = order
            .iter()
            .position(|&id| {
                if let Some(Node::Target(t)) = query.get(id) {
                    t.label.name == "base"
                } else {
                    false
                }
            })
            .unwrap();

        let util_pos = order
            .iter()
            .position(|&id| {
                if let Some(Node::Target(t)) = query.get(id) {
                    t.label.name == "util"
                } else {
                    false
                }
            })
            .unwrap();

        let app_pos = order
            .iter()
            .position(|&id| {
                if let Some(Node::Target(t)) = query.get(id) {
                    t.label.name == "app"
                } else {
                    false
                }
            })
            .unwrap();

        // Dependencies must come before dependents
        assert!(base_pos < util_pos);
        assert!(util_pos < app_pos);
    }

    #[test]
    fn export_to_dot() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        let dot = query.to_dot();

        // Check basic structure
        assert!(dot.contains("digraph build"));
        assert!(dot.contains("rankdir=BT"));

        // Check nodes are present
        assert!(dot.contains("base"));
        assert!(dot.contains("util"));
        assert!(dot.contains("app"));

        // Check edges exist
        assert!(dot.contains("->"));
    }

    #[test]
    fn subgraph_to_dot() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        // Get util node
        let util_id = query
            .filter_targets(|l| l.name == "util")
            .into_iter()
            .next()
            .unwrap();

        let dot = query.subgraph_to_dot(util_id);

        // Should contain util and base, but not app
        assert!(dot.contains("util"));
        assert!(dot.contains("base"));
        // app is not in util's transitive deps
    }

    #[test]
    fn filter_targets() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        // Filter by name containing 'a'
        let matching = query.filter_targets(|l| l.name.contains('a'));
        assert_eq!(matching.len(), 2); // base and app
    }

    #[test]
    fn targets_in_package() {
        let (_ws, resolver) = setup_test_graph();
        let query = QueryEngine::new(resolver.graph());

        let util_targets = query.targets_in_package("util");
        assert_eq!(util_targets.len(), 1);

        if let Some(Node::Target(t)) = query.get(util_targets[0]) {
            assert_eq!(t.label.name, "util");
        } else {
            panic!("expected target");
        }
    }
}
