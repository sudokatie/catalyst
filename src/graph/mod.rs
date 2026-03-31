//! Build graph representation and operations

mod node;
mod resolver;

pub use node::{ActionNode, FileKind, FileNode, Node, NodeId, TargetNode};
pub use resolver::{Graph, Resolver};
