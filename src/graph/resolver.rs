//! Dependency resolution and graph construction

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Dfs;
use petgraph::Direction;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::node::{Node, NodeId, TargetNode};
use crate::{build_file_to_targets, Error, Label, Parser, Target};

/// A directed graph of build dependencies
pub struct Graph {
    /// The underlying petgraph
    inner: DiGraph<Node, ()>,
    /// Map from labels to node indices
    label_to_index: HashMap<Label, NodeIndex>,
    /// Map from paths to node indices (for file nodes)
    path_to_index: HashMap<PathBuf, NodeIndex>,
}

impl Graph {
    /// Create a new empty graph
    pub fn new() -> Self {
        Self {
            inner: DiGraph::new(),
            label_to_index: HashMap::new(),
            path_to_index: HashMap::new(),
        }
    }

    /// Add a target node to the graph
    pub fn add_target(&mut self, label: Label) -> NodeId {
        if let Some(&idx) = self.label_to_index.get(&label) {
            return NodeId(idx.index());
        }

        let node = Node::target(label.clone());
        let idx = self.inner.add_node(node);
        self.label_to_index.insert(label, idx);
        NodeId(idx.index())
    }

    /// Add a target node with rule type
    pub fn add_target_with_rule(&mut self, label: Label, rule_type: String) -> NodeId {
        if let Some(&idx) = self.label_to_index.get(&label) {
            return NodeId(idx.index());
        }

        let node = Node::Target(TargetNode::with_rule_type(label.clone(), rule_type));
        let idx = self.inner.add_node(node);
        self.label_to_index.insert(label, idx);
        NodeId(idx.index())
    }

    /// Add a source file node
    pub fn add_source_file(&mut self, path: PathBuf) -> NodeId {
        if let Some(&idx) = self.path_to_index.get(&path) {
            return NodeId(idx.index());
        }

        let node = Node::source_file(path.clone());
        let idx = self.inner.add_node(node);
        self.path_to_index.insert(path, idx);
        NodeId(idx.index())
    }

    /// Add a dependency edge from one node to another
    pub fn add_edge(&mut self, from: NodeId, to: NodeId) {
        let from_idx = NodeIndex::new(from.0);
        let to_idx = NodeIndex::new(to.0);
        self.inner.add_edge(from_idx, to_idx, ());
    }

    /// Get a node by label
    pub fn get_by_label(&self, label: &Label) -> Option<&Node> {
        self.label_to_index
            .get(label)
            .map(|&idx| &self.inner[idx])
    }

    /// Get a node by ID
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.inner.node_weight(NodeIndex::new(id.0))
    }

    /// Get direct dependencies of a node
    pub fn deps(&self, id: NodeId) -> Vec<NodeId> {
        let idx = NodeIndex::new(id.0);
        self.inner
            .neighbors_directed(idx, Direction::Outgoing)
            .map(|i| NodeId(i.index()))
            .collect()
    }

    /// Get direct dependents (reverse deps) of a node
    pub fn rdeps(&self, id: NodeId) -> Vec<NodeId> {
        let idx = NodeIndex::new(id.0);
        self.inner
            .neighbors_directed(idx, Direction::Incoming)
            .map(|i| NodeId(i.index()))
            .collect()
    }

    /// Get all transitive dependencies of a node
    pub fn transitive_deps(&self, id: NodeId) -> Vec<NodeId> {
        let idx = NodeIndex::new(id.0);
        let mut visited = HashSet::new();
        let mut result = Vec::new();

        let mut dfs = Dfs::new(&self.inner, idx);
        // Skip the starting node
        dfs.next(&self.inner);

        while let Some(next) = dfs.next(&self.inner) {
            if visited.insert(next.index()) {
                result.push(NodeId(next.index()));
            }
        }

        result
    }

    /// Check if the graph has a cycle, returning the cycle path if found
    pub fn find_cycle(&self) -> Option<Vec<Label>> {
        use petgraph::algo::is_cyclic_directed;

        if !is_cyclic_directed(&self.inner) {
            return None;
        }

        // Find and report one cycle
        // Use DFS with visited tracking
        let mut visited = HashSet::new();
        let mut rec_stack = Vec::new();

        for start in self.inner.node_indices() {
            if visited.contains(&start) {
                continue;
            }

            if let Some(cycle) = self.dfs_find_cycle(start, &mut visited, &mut rec_stack) {
                return Some(cycle);
            }
        }

        None
    }

    fn dfs_find_cycle(
        &self,
        node: NodeIndex,
        visited: &mut HashSet<NodeIndex>,
        rec_stack: &mut Vec<NodeIndex>,
    ) -> Option<Vec<Label>> {
        visited.insert(node);
        rec_stack.push(node);

        for neighbor in self.inner.neighbors_directed(node, Direction::Outgoing) {
            if !visited.contains(&neighbor) {
                if let Some(cycle) = self.dfs_find_cycle(neighbor, visited, rec_stack) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(&neighbor) {
                // Found a cycle - extract the cycle path
                let start_pos = rec_stack.iter().position(|&n| n == neighbor).unwrap();
                let mut cycle: Vec<Label> = rec_stack[start_pos..]
                    .iter()
                    .filter_map(|&idx| {
                        if let Node::Target(t) = &self.inner[idx] {
                            Some(t.label.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                // Add the first element again to show the cycle
                if !cycle.is_empty() {
                    cycle.push(cycle[0].clone());
                }
                return Some(cycle);
            }
        }

        rec_stack.pop();
        None
    }

    /// Get topological order of nodes (dependencies first)
    pub fn topo_order(&self) -> Result<Vec<NodeId>, Error> {
        use petgraph::algo::toposort;

        match toposort(&self.inner, None) {
            Ok(order) => {
                // petgraph toposort gives us dependents before dependencies,
                // but we want dependencies first for build order, so reverse
                let mut result: Vec<_> = order.into_iter().map(|i| NodeId(i.index())).collect();
                result.reverse();
                Ok(result)
            }
            Err(_) => {
                // Extract cycle for error message
                if let Some(cycle) = self.find_cycle() {
                    let cycle_strs: Vec<_> = cycle.iter().map(|l| l.to_string()).collect();
                    Err(Error::Cycle(cycle_strs))
                } else {
                    Err(Error::Cycle(vec!["unknown cycle".to_string()]))
                }
            }
        }
    }

    /// Number of nodes in the graph
    pub fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Number of edges in the graph
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    /// Iterate over all labels in the graph
    pub fn labels(&self) -> impl Iterator<Item = &Label> {
        self.label_to_index.keys()
    }
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolver for loading BUILD files and constructing the dependency graph
pub struct Resolver {
    /// Workspace root directory
    workspace_root: PathBuf,
    /// Loaded targets by label
    targets: HashMap<Label, Target>,
    /// The dependency graph
    graph: Graph,
    /// Currently resolving (for cycle detection)
    resolving: HashSet<Label>,
}

impl Resolver {
    /// Create a new resolver with the given workspace root
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            targets: HashMap::new(),
            graph: Graph::new(),
            resolving: HashSet::new(),
        }
    }

    /// Load a package's BUILD file
    pub fn load_package(&mut self, package: &str) -> Result<Vec<Label>, Error> {
        let build_path = self.workspace_root.join(package).join("BUILD");
        self.load_build_file(&build_path, package)
    }

    /// Load a BUILD file and return the labels defined in it
    fn load_build_file(&mut self, path: &Path, package: &str) -> Result<Vec<Label>, Error> {
        let content = fs::read_to_string(path).map_err(|e| Error::Io(e))?;

        let mut parser = Parser::new(&content).map_err(|e| Error::Parse {
            file: path.to_string_lossy().into_owned(),
            line: e.line,
            col: e.col,
            message: e.message,
        })?;

        let build_file = parser.parse().map_err(|e| Error::Parse {
            file: path.to_string_lossy().into_owned(),
            line: e.line,
            col: e.col,
            message: e.message,
        })?;

        let targets = build_file_to_targets(&build_file, package)?;
        let mut labels = Vec::new();

        for target in targets {
            let label = target.label.clone();
            self.graph
                .add_target_with_rule(label.clone(), target.rule_type.clone());
            self.targets.insert(label.clone(), target);
            labels.push(label);
        }

        Ok(labels)
    }

    /// Resolve a target and all its dependencies
    pub fn resolve(&mut self, label: &Label) -> Result<NodeId, Error> {
        // Check for cycle during resolution
        if self.resolving.contains(label) {
            // Build cycle path from resolving set
            let mut cycle_path: Vec<String> = self
                .resolving
                .iter()
                .map(|l| l.to_string())
                .collect();
            cycle_path.push(label.to_string());
            return Err(Error::Cycle(cycle_path));
        }

        // If already fully resolved, just return the node ID
        if let Some(&idx) = self.graph.label_to_index.get(label) {
            // Check if this was fully resolved (not just added)
            if !self.resolving.contains(label) {
                return Ok(NodeId(idx.index()));
            }
        }

        // Load the package if not already loaded
        if !self.targets.contains_key(label) {
            self.load_package(&label.package)?;
        }

        let target = self
            .targets
            .get(label)
            .ok_or_else(|| Error::UnknownTarget(label.to_string()))?
            .clone();

        let node_id = self
            .graph
            .label_to_index
            .get(label)
            .map(|idx| NodeId(idx.index()))
            .unwrap();

        // Mark as currently resolving
        self.resolving.insert(label.clone());

        // Resolve all dependencies
        for dep_label in &target.deps {
            let dep_id = self.resolve(dep_label)?;
            self.graph.add_edge(node_id, dep_id);
        }

        // Done resolving
        self.resolving.remove(label);

        Ok(node_id)
    }

    /// Get the constructed graph
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Get a target by label
    pub fn get_target(&self, label: &Label) -> Option<&Target> {
        self.targets.get(label)
    }

    /// Get all loaded targets
    pub fn targets(&self) -> impl Iterator<Item = (&Label, &Target)> {
        self.targets.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_workspace() -> TempDir {
        let dir = TempDir::new().unwrap();
        dir
    }

    fn write_build_file(dir: &Path, package: &str, content: &str) {
        let pkg_dir = dir.join(package);
        fs::create_dir_all(&pkg_dir).unwrap();
        let build_path = pkg_dir.join("BUILD");
        let mut file = fs::File::create(build_path).unwrap();
        write!(file, "{}", content).unwrap();
    }

    #[test]
    fn load_single_package() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "mylib",
            r#"
rust_library(
    name = "mylib",
    srcs = ["lib.rs"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let labels = resolver.load_package("mylib").unwrap();

        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name, "mylib");
        assert_eq!(resolver.graph().node_count(), 1);
    }

    #[test]
    fn resolve_target_with_no_deps() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "simple",
            r#"
rust_binary(
    name = "app",
    srcs = ["main.rs"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let label = Label::new("simple", "app");
        let node_id = resolver.resolve(&label).unwrap();

        assert_eq!(resolver.graph().deps(node_id).len(), 0);
    }

    #[test]
    fn resolve_target_with_deps() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "lib",
            r#"
rust_library(
    name = "util",
    srcs = ["util.rs"],
)
"#,
        );
        write_build_file(
            ws.path(),
            "app",
            r#"
rust_binary(
    name = "main",
    srcs = ["main.rs"],
    deps = ["//lib:util"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let label = Label::new("app", "main");
        let node_id = resolver.resolve(&label).unwrap();

        // main depends on util
        let deps = resolver.graph().deps(node_id);
        assert_eq!(deps.len(), 1);

        // Check the dep is util
        let dep_node = resolver.graph().get(deps[0]).unwrap();
        if let Node::Target(t) = dep_node {
            assert_eq!(t.label.name, "util");
        } else {
            panic!("expected target node");
        }
    }

    #[test]
    fn detect_cycle() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "a",
            r#"
rust_library(
    name = "a",
    deps = ["//b:b"],
)
"#,
        );
        write_build_file(
            ws.path(),
            "b",
            r#"
rust_library(
    name = "b",
    deps = ["//a:a"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let label = Label::new("a", "a");
        let result = resolver.resolve(&label);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, Error::Cycle(_)));
    }

    #[test]
    fn error_on_missing_dependency() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "broken",
            r#"
rust_binary(
    name = "app",
    deps = ["//missing:lib"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let label = Label::new("broken", "app");
        let result = resolver.resolve(&label);

        assert!(result.is_err());
        // Should fail when trying to load the missing package
    }

    #[test]
    fn graph_transitive_deps() {
        let ws = create_test_workspace();
        write_build_file(
            ws.path(),
            "base",
            r#"
rust_library(
    name = "base",
    srcs = ["base.rs"],
)
"#,
        );
        write_build_file(
            ws.path(),
            "middle",
            r#"
rust_library(
    name = "middle",
    deps = ["//base:base"],
)
"#,
        );
        write_build_file(
            ws.path(),
            "top",
            r#"
rust_binary(
    name = "top",
    deps = ["//middle:middle"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        let label = Label::new("top", "top");
        let node_id = resolver.resolve(&label).unwrap();

        // Transitive deps should include both middle and base
        let trans_deps = resolver.graph().transitive_deps(node_id);
        assert_eq!(trans_deps.len(), 2);
    }

    #[test]
    fn graph_topo_order() {
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
            "app",
            r#"
rust_binary(
    name = "app",
    deps = ["//base:base"],
)
"#,
        );

        let mut resolver = Resolver::new(ws.path().to_path_buf());
        resolver.resolve(&Label::new("app", "app")).unwrap();

        let order = resolver.graph().topo_order().unwrap();
        assert_eq!(order.len(), 2);

        // Base should come before app in topo order
        let base_pos = order
            .iter()
            .position(|&id| {
                if let Some(Node::Target(t)) = resolver.graph().get(id) {
                    t.label.name == "base"
                } else {
                    false
                }
            })
            .unwrap();

        let app_pos = order
            .iter()
            .position(|&id| {
                if let Some(Node::Target(t)) = resolver.graph().get(id) {
                    t.label.name == "app"
                } else {
                    false
                }
            })
            .unwrap();

        assert!(base_pos < app_pos);
    }
}
