/// Core graph model types.
///
/// All index types are newtypes over `u32` for type safety.
/// The `Graph` is the validated, immutable data structure that
/// all downstream phases (trust propagation, layout, rendering)
/// operate on.
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Index newtypes
// ---------------------------------------------------------------------------

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_id!(NodeId);
define_id!(PropId);
define_id!(EdgeId);
define_id!(DomainId);

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// The validated, immutable graph structure with port-level adjacency.
#[derive(Debug, Clone)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub properties: Vec<Property>,
    pub edges: Vec<Edge>,
    pub domains: Vec<Domain>,

    /// Port-level adjacency: PropId -> Vec<EdgeId>.
    pub prop_edges: HashMap<PropId, Vec<EdgeId>>,

    /// Node-level adjacency for anchors only: NodeId -> Vec<EdgeId> (children).
    pub node_children: HashMap<NodeId, Vec<EdgeId>>,

    /// Node-level adjacency for anchors only: NodeId -> EdgeId (parent anchor).
    pub node_parent: HashMap<NodeId, EdgeId>,
}

impl Graph {
    /// Look up a node by identifier string (skips derivation nodes).
    pub fn find_node_by_ident(&self, ident: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.ident.as_deref() == Some(ident))
    }

    /// Look up a property by node ident and property name.
    pub fn find_property(&self, node_ident: &str, prop_name: &str) -> Option<&Property> {
        let node = self.find_node_by_ident(node_ident)?;
        node.properties
            .iter()
            .map(|&pid| &self.properties[pid.index()])
            .find(|p| p.name == prop_name)
    }

    /// Get the parent node of a given node (via its incoming anchor), if any.
    pub fn parent_of(&self, node_id: NodeId) -> Option<NodeId> {
        self.node_parent.get(&node_id).map(|&eid| {
            match &self.edges[eid.index()] {
                Edge::Anchor { parent, .. } => *parent,
                _ => unreachable!("node_parent should only contain Anchor edges"),
            }
        })
    }

    /// Get all edges incident on a property.
    pub fn edges_on_prop(&self, prop_id: PropId) -> &[EdgeId] {
        self.prop_edges
            .get(&prop_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Resolve the source and target nodes of any edge.
    ///
    /// For `Anchor` edges: source = parent, target = child.
    /// For `Constraint` edges: source = node owning source_prop, target = node owning dest_prop.
    pub fn edge_nodes(&self, edge: &Edge) -> (NodeId, NodeId) {
        match edge {
            Edge::Anchor { parent, child, .. } => (*parent, *child),
            Edge::Constraint { source_prop, dest_prop, .. } => (
                self.properties[source_prop.index()].node,
                self.properties[dest_prop.index()].node,
            ),
        }
    }

    /// Convenience: resolve edge nodes by `EdgeId`.
    pub fn edge_node_ids(&self, edge_id: EdgeId) -> (NodeId, NodeId) {
        self.edge_nodes(&self.edges[edge_id.index()])
    }

    /// Get all child anchor edges for a node.
    pub fn children_of(&self, node_id: NodeId) -> &[EdgeId] {
        self.node_children
            .get(&node_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    /// User-visible identifier. `None` for derivation (synthetic) nodes.
    pub ident: Option<String>,
    pub display_name: Option<String>,
    pub properties: Vec<PropId>,
    pub domain: Option<DomainId>,
    pub is_anchored: bool,
    pub is_selected: bool,
}

impl Node {
    /// Returns `true` for synthetic derivation nodes (`ident` is `None`).
    pub fn is_derivation(&self) -> bool {
        self.ident.is_none()
    }

    /// The display label: display_name if set, then ident, then a fallback.
    pub fn label(&self) -> &str {
        self.display_name.as_deref()
            .or(self.ident.as_deref())
            .unwrap_or("<derivation>")
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// Property
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Property {
    pub id: PropId,
    pub node: NodeId,
    pub name: String,
    /// `@critical` — property gates node verification.
    pub critical: bool,
    /// `@constrained` — property is pre-satisfied (annotation-constrained).
    pub constrained: bool,
}

// ---------------------------------------------------------------------------
// Domain
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Domain {
    pub id: DomainId,
    pub display_name: String,
    pub members: Vec<NodeId>,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Edge {
    /// Hierarchical anchor: parent -> child. Anchoring flows parent to child.
    Anchor {
        child: NodeId,
        parent: NodeId,
        operation: Option<String>,
    },

    /// Property constraint: source_prop -> dest_prop. Trust flows source to dest.
    Constraint {
        dest_prop: PropId,
        source_prop: PropId,
        operation: Option<String>,
    },
}

impl Edge {
    /// Returns true if this is an Anchor edge.
    pub fn is_anchor(&self) -> bool {
        matches!(self, Edge::Anchor { .. })
    }

    /// Returns true if this is a Constraint edge.
    pub fn is_constraint(&self) -> bool {
        matches!(self, Edge::Constraint { .. })
    }

    /// Edge weight for layout purposes.
    pub fn weight(&self) -> u32 {
        match self {
            Edge::Anchor { .. } => 3,
            Edge::Constraint { .. } => 1,
        }
    }
}
