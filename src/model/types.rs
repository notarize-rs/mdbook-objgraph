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
define_id!(DerivId);
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
    pub derivations: Vec<Derivation>,
    pub edges: Vec<Edge>,
    pub domains: Vec<Domain>,

    /// Port-level adjacency: PropId -> Vec<EdgeId>.
    /// Includes edges to/from derivation ports.
    pub prop_edges: HashMap<PropId, Vec<EdgeId>>,

    /// Node-level adjacency for anchors only: NodeId -> Vec<EdgeId> (children).
    pub node_children: HashMap<NodeId, Vec<EdgeId>>,

    /// Node-level adjacency for anchors only: NodeId -> EdgeId (parent anchor).
    pub node_parent: HashMap<NodeId, EdgeId>,
}

impl Graph {
    /// Look up a node by identifier string.
    pub fn find_node_by_ident(&self, ident: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.ident == ident)
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

    /// Get all child anchor edges for a node.
    pub fn children_of(&self, node_id: NodeId) -> &[EdgeId] {
        self.node_children
            .get(&node_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Total number of elements (nodes + derivations) for layout purposes.
    pub fn element_count(&self) -> usize {
        self.nodes.len() + self.derivations.len()
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub ident: String,
    pub display_name: Option<String>,
    pub properties: Vec<PropId>,
    pub domain: Option<DomainId>,
    pub is_root: bool,
    pub is_selected: bool,
}

impl Node {
    /// The display label: display_name if set, otherwise ident.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.ident)
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
// Derivation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Derivation {
    pub id: DerivId,
    pub operation: String,
    pub inputs: Vec<PropId>,
    pub output_prop: PropId,
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

    /// Derivation input: source_prop feeds into target_deriv.
    DerivInput {
        source_prop: PropId,
        target_deriv: DerivId,
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

    /// Returns true if this is a DerivInput edge.
    pub fn is_deriv_input(&self) -> bool {
        matches!(self, Edge::DerivInput { .. })
    }

    /// Edge weight for layout purposes.
    pub fn weight(&self) -> u32 {
        match self {
            Edge::Anchor { .. } => 3,
            Edge::DerivInput { .. } => 2,
            Edge::Constraint { .. } => 1,
        }
    }
}
