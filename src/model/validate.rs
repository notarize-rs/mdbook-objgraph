//! Validation rules (DESIGN.md §7.2).

use std::collections::{HashMap, HashSet, VecDeque};

use super::types::{DerivId, Edge, Graph, NodeId, PropId};
use crate::ObgraphError;

/// Validate a constructed graph against all rules from DESIGN.md §7.2.
///
/// Returns `Ok(())` if valid, or an error describing the first violation found.
pub fn validate(graph: &Graph) -> Result<(), ObgraphError> {
    check_duplicate_node_idents(graph)?;
    check_duplicate_property_names(graph)?;
    check_nonexistent_node_references(graph)?;
    check_nonexistent_property_references(graph)?;
    check_constraint_on_constrained_prop(graph)?;
    check_root_node_incoming_anchor(graph)?;
    check_non_root_without_incoming_anchor(graph)?;
    check_multiple_incoming_anchors(graph)?;
    check_nullary_derivations(graph)?;
    check_cycles(graph)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Duplicate node identifier
// ---------------------------------------------------------------------------

fn check_duplicate_node_idents(graph: &Graph) -> Result<(), ObgraphError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for node in &graph.nodes {
        if !seen.insert(node.ident.as_str()) {
            return Err(ObgraphError::Validation(format!(
                "duplicate node identifier: '{}'",
                node.ident
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Duplicate property name within a node
// ---------------------------------------------------------------------------

fn check_duplicate_property_names(graph: &Graph) -> Result<(), ObgraphError> {
    for node in &graph.nodes {
        let mut seen: HashSet<&str> = HashSet::new();
        for &pid in &node.properties {
            let prop = &graph.properties[pid.index()];
            if !seen.insert(prop.name.as_str()) {
                return Err(ObgraphError::Validation(format!(
                    "duplicate property name '{}' in node '{}'",
                    prop.name, node.ident
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Reference to nonexistent node
// ---------------------------------------------------------------------------

fn check_nonexistent_node_references(graph: &Graph) -> Result<(), ObgraphError> {
    let node_count = graph.nodes.len();
    for edge in &graph.edges {
        match edge {
            Edge::Anchor { child, parent, .. } => {
                if child.index() >= node_count {
                    return Err(ObgraphError::Validation(format!(
                        "anchor references nonexistent child node id {}",
                        child
                    )));
                }
                if parent.index() >= node_count {
                    return Err(ObgraphError::Validation(format!(
                        "anchor references nonexistent parent node id {}",
                        parent
                    )));
                }
            }
            // Constraint and DerivInput reference properties, not nodes directly.
            Edge::Constraint { .. } | Edge::DerivInput { .. } => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Reference to nonexistent property
// ---------------------------------------------------------------------------

fn check_nonexistent_property_references(graph: &Graph) -> Result<(), ObgraphError> {
    let prop_count = graph.properties.len();
    let deriv_count = graph.derivations.len();

    for edge in &graph.edges {
        match edge {
            Edge::Constraint {
                dest_prop,
                source_prop,
                ..
            } => {
                if dest_prop.index() >= prop_count {
                    return Err(ObgraphError::Validation(format!(
                        "constraint references nonexistent dest property id {}",
                        dest_prop
                    )));
                }
                if source_prop.index() >= prop_count {
                    return Err(ObgraphError::Validation(format!(
                        "constraint references nonexistent source property id {}",
                        source_prop
                    )));
                }
            }
            Edge::DerivInput {
                source_prop,
                target_deriv,
            } => {
                if source_prop.index() >= prop_count {
                    return Err(ObgraphError::Validation(format!(
                        "derivation input references nonexistent source property id {}",
                        source_prop
                    )));
                }
                if target_deriv.index() >= deriv_count {
                    return Err(ObgraphError::Validation(format!(
                        "derivation input references nonexistent derivation id {}",
                        target_deriv
                    )));
                }
            }
            Edge::Anchor { .. } => {}
        }
    }

    // Also validate derivation output_prop references.
    for deriv in &graph.derivations {
        if deriv.output_prop.index() >= prop_count {
            return Err(ObgraphError::Validation(format!(
                "derivation {} has nonexistent output property id {}",
                deriv.id, deriv.output_prop
            )));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Constraint on @constrained property
// ---------------------------------------------------------------------------

fn check_constraint_on_constrained_prop(graph: &Graph) -> Result<(), ObgraphError> {
    for edge in &graph.edges {
        if let Edge::Constraint { dest_prop, .. } = edge {
            let prop = &graph.properties[dest_prop.index()];
            if prop.constrained {
                let node = &graph.nodes[prop.node.index()];
                return Err(ObgraphError::Validation(format!(
                    "property '{}' on node '{}' is @constrained but has an incoming constraint \
                     (contradictory: pre-satisfied properties cannot also be constrained by edges)",
                    prop.name, node.ident
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: @anchored node with incoming anchor edge
// ---------------------------------------------------------------------------

fn check_root_node_incoming_anchor(graph: &Graph) -> Result<(), ObgraphError> {
    for node in &graph.nodes {
        if node.is_anchored && graph.node_parent.contains_key(&node.id) {
            return Err(ObgraphError::Validation(format!(
                "node '{}' is annotated @anchored but appears as the child in an anchor \
                 (root nodes must not have a parent)",
                node.ident
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Non-@anchored node without incoming anchor (orphan)
// ---------------------------------------------------------------------------

fn check_non_root_without_incoming_anchor(graph: &Graph) -> Result<(), ObgraphError> {
    for node in &graph.nodes {
        if !node.is_anchored && !graph.node_parent.contains_key(&node.id) {
            return Err(ObgraphError::Validation(format!(
                "node '{}' is not @anchored but has no incoming anchor \
                 (orphaned nodes must be annotated @anchored or connected to a parent)",
                node.ident
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Multiple incoming anchors
// ---------------------------------------------------------------------------

fn check_multiple_incoming_anchors(graph: &Graph) -> Result<(), ObgraphError> {
    // node_parent is a HashMap<NodeId, EdgeId> so by construction it holds at
    // most one parent per node. We verify consistency by counting Link edges
    // per child directly, which catches any discrepancy between the edge list
    // and the index.
    let mut incoming_count: HashMap<NodeId, usize> = HashMap::new();
    for edge in &graph.edges {
        if let Edge::Anchor { child, .. } = edge {
            *incoming_count.entry(*child).or_insert(0) += 1;
        }
    }
    for (node_id, count) in &incoming_count {
        if *count > 1 {
            let ident = &graph.nodes[node_id.index()].ident;
            return Err(ObgraphError::Validation(format!(
                "node '{}' has {} incoming anchors (at most one is allowed)",
                ident, count
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Nullary derivation
// ---------------------------------------------------------------------------

fn check_nullary_derivations(graph: &Graph) -> Result<(), ObgraphError> {
    for deriv in &graph.derivations {
        if deriv.inputs.is_empty() {
            return Err(ObgraphError::Validation(format!(
                "derivation '{}' (id {}) has zero inputs (derivations must have at least one input)",
                deriv.operation, deriv.id
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rule: Cycle detection (Kahn's algorithm)
// ---------------------------------------------------------------------------
//
// Vertex universe:
//   - Node vertices  : 0              ..  N-1
//   - Prop vertices  : N              ..  N+P-1
//   - Deriv vertices : N+P            ..  N+P+D-1
//
// Edges (trust-flow direction):
//   - Link          : parent-node  -> child-node
//   - Constraint    : source_prop  -> dest_prop
//   - DerivInput    : source_prop  -> deriv-vertex
//   - Deriv output  : deriv-vertex -> output_prop

fn check_cycles(graph: &Graph) -> Result<(), ObgraphError> {
    let n = graph.nodes.len();
    let p = graph.properties.len();
    let d = graph.derivations.len();
    let total = n + p + d;

    let node_vtx = |id: NodeId| -> usize { id.index() };
    let prop_vtx = |id: PropId| -> usize { n + id.index() };
    let deriv_vtx = |id: DerivId| -> usize { n + p + id.index() };

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut in_degree: Vec<usize> = vec![0; total];

    let mut add_edge = |from: usize, to: usize| {
        adj[from].push(to);
        in_degree[to] += 1;
    };

    for edge in &graph.edges {
        match edge {
            Edge::Anchor { parent, child, .. } => {
                add_edge(node_vtx(*parent), node_vtx(*child));
            }
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => {
                add_edge(prop_vtx(*source_prop), prop_vtx(*dest_prop));
            }
            Edge::DerivInput {
                source_prop,
                target_deriv,
            } => {
                add_edge(prop_vtx(*source_prop), deriv_vtx(*target_deriv));
            }
        }
    }

    // Derivation output edges: deriv -> output_prop.
    for deriv in &graph.derivations {
        add_edge(deriv_vtx(deriv.id), prop_vtx(deriv.output_prop));
    }

    // Kahn's algorithm.
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (v, &deg) in in_degree.iter().enumerate().take(total) {
        if deg == 0 {
            queue.push_back(v);
        }
    }

    let mut visited = 0usize;
    while let Some(v) = queue.pop_front() {
        visited += 1;
        for &u in &adj[v] {
            in_degree[u] -= 1;
            if in_degree[u] == 0 {
                queue.push_back(u);
            }
        }
    }

    if visited != total {
        return Err(ObgraphError::Validation(
            "cycle detected in the graph (anchors, constraints, and derivation edges form a cycle)"
                .to_string(),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::*;
    use std::collections::HashMap;

    // ---------------------------------------------------------------------------
    // Helpers to build minimal graphs for testing
    // ---------------------------------------------------------------------------

    fn empty_graph() -> Graph {
        Graph {
            nodes: Vec::new(),
            properties: Vec::new(),
            derivations: Vec::new(),
            edges: Vec::new(),
            domains: Vec::new(),
            prop_edges: HashMap::new(),
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        }
    }

    /// Build a minimal valid single-root graph with no properties or edges.
    fn single_root_graph() -> Graph {
        let mut g = empty_graph();
        g.nodes.push(Node {
            id: NodeId(0),
            ident: "root".to_string(),
            display_name: None,
            properties: vec![],
            domain: None,
            is_anchored: true,
            is_selected: false,
        });
        g
    }

    /// Build a valid two-node graph: root -> child via Link.
    fn two_node_graph() -> Graph {
        let mut g = empty_graph();
        g.nodes.push(Node {
            id: NodeId(0),
            ident: "root".to_string(),
            display_name: None,
            properties: vec![],
            domain: None,
            is_anchored: true,
            is_selected: false,
        });
        g.nodes.push(Node {
            id: NodeId(1),
            ident: "child".to_string(),
            display_name: None,
            properties: vec![],
            domain: None,
            is_anchored: false,
            is_selected: false,
        });
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        });
        g.node_parent.insert(NodeId(1), EdgeId(0));
        g.node_children
            .entry(NodeId(0))
            .or_default()
            .push(EdgeId(0));
        g
    }

    // ---------------------------------------------------------------------------
    // A valid graph passes
    // ---------------------------------------------------------------------------

    #[test]
    fn valid_single_root_passes() {
        assert!(validate(&single_root_graph()).is_ok());
    }

    #[test]
    fn valid_two_node_graph_passes() {
        assert!(validate(&two_node_graph()).is_ok());
    }

    #[test]
    fn valid_graph_with_constraint_passes() {
        // root(p0) -> child(p1); p0 --constraint--> p1
        let mut g = two_node_graph();

        // Add properties
        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "sig".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties.push(PropId(0));

        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(1),
            name: "sig".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[1].properties.push(PropId(1));

        let constraint_eid = EdgeId(g.edges.len() as u32);
        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        });
        g.prop_edges.entry(PropId(0)).or_default().push(constraint_eid);
        g.prop_edges.entry(PropId(1)).or_default().push(constraint_eid);

        assert!(validate(&g).is_ok());
    }

    #[test]
    fn valid_graph_with_derivation_passes() {
        // root(p0, p1) -> child; deriv(p0 -> p1)
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "a".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "b".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(0)],
            output_prop: PropId(1),
        });

        let di_eid = EdgeId(g.edges.len() as u32);
        g.edges.push(Edge::DerivInput {
            source_prop: PropId(0),
            target_deriv: DerivId(0),
        });
        g.prop_edges.entry(PropId(0)).or_default().push(di_eid);

        assert!(validate(&g).is_ok());
    }

    // ---------------------------------------------------------------------------
    // Duplicate node identifier
    // ---------------------------------------------------------------------------

    #[test]
    fn duplicate_node_ident_fails() {
        let mut g = empty_graph();
        for i in 0..2u32 {
            g.nodes.push(Node {
                id: NodeId(i),
                ident: "duplicate".to_string(),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: i == 0,
                is_selected: false,
            });
        }
        // Give the second node a parent so orphan check doesn't fire first.
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        });
        g.node_parent.insert(NodeId(1), EdgeId(0));

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate node identifier"),
            "unexpected error: {msg}"
        );
        assert!(msg.contains("duplicate"), "unexpected error: {msg}");
    }

    // ---------------------------------------------------------------------------
    // Duplicate property name within a node
    // ---------------------------------------------------------------------------

    #[test]
    fn duplicate_property_name_fails() {
        let mut g = single_root_graph();
        for i in 0..2u32 {
            g.properties.push(Property {
                id: PropId(i),
                node: NodeId(0),
                name: "sig".to_string(),
                critical: true,
            constrained: false,
            });
            g.nodes[0].properties.push(PropId(i));
        }

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate property name"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Reference to nonexistent node (via Link)
    // ---------------------------------------------------------------------------

    #[test]
    fn nonexistent_child_node_in_anchor_fails() {
        let mut g = single_root_graph();
        // Edge references NodeId(99) which doesn't exist.
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(99),
            operation: None,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent child node"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn nonexistent_parent_node_in_anchor_fails() {
        let mut g = single_root_graph();
        g.edges.push(Edge::Anchor {
            parent: NodeId(99),
            child: NodeId(0),
            operation: None,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent parent node"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Reference to nonexistent property (via Constraint / DerivInput)
    // ---------------------------------------------------------------------------

    #[test]
    fn nonexistent_dest_prop_in_constraint_fails() {
        let mut g = single_root_graph();
        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "p".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties.push(PropId(0));

        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(99), // nonexistent
            operation: None,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent dest property"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn nonexistent_source_prop_in_deriv_input_fails() {
        let mut g = single_root_graph();
        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "out".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties.push(PropId(0));
        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(99)],
            output_prop: PropId(0),
        });
        g.edges.push(Edge::DerivInput {
            source_prop: PropId(99), // nonexistent
            target_deriv: DerivId(0),
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nonexistent source property"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Constraint on @constrained property
    // ---------------------------------------------------------------------------

    #[test]
    fn constraint_on_constrained_prop_fails() {
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "source".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "constrained_prop".to_string(),
            critical: false,
            constrained: true, // @constrained
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1), // dest is @constrained — invalid
            operation: None,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("@constrained") && msg.contains("constraint"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn constraint_targeting_non_constrained_passes() {
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "source".to_string(),
            critical: false,
            constrained: true, // source is constrained — ok, only dest matters
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "dest".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        let eid = EdgeId(g.edges.len() as u32);
        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        });
        g.prop_edges.entry(PropId(0)).or_default().push(eid);
        g.prop_edges.entry(PropId(1)).or_default().push(eid);

        assert!(validate(&g).is_ok());
    }

    // ---------------------------------------------------------------------------
    // @anchored node with incoming anchor
    // ---------------------------------------------------------------------------

    #[test]
    fn root_node_with_incoming_anchor_fails() {
        let mut g = two_node_graph();
        // Make the child a root as well — contradictory.
        g.nodes[1].is_anchored = true;

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("@anchored") && msg.contains("child"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Non-@anchored node without incoming anchor (orphan)
    // ---------------------------------------------------------------------------

    #[test]
    fn non_root_orphan_fails() {
        let mut g = single_root_graph();
        // Add a second node that is NOT root and has no parent.
        g.nodes.push(Node {
            id: NodeId(1),
            ident: "orphan".to_string(),
            display_name: None,
            properties: vec![],
            domain: None,
            is_anchored: false,
            is_selected: false,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("orphan") || msg.contains("no incoming anchor"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Multiple incoming anchors
    // ---------------------------------------------------------------------------

    #[test]
    fn multiple_incoming_anchors_fails() {
        let mut g = empty_graph();
        // Three nodes: two parents + one child.
        for i in 0..3u32 {
            g.nodes.push(Node {
                id: NodeId(i),
                ident: format!("n{}", i),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: i < 2,
                is_selected: false,
            });
        }
        // Two anchors both pointing to NodeId(2).
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(2),
            operation: None,
        });
        g.edges.push(Edge::Anchor {
            parent: NodeId(1),
            child: NodeId(2),
            operation: None,
        });
        // node_parent records only one (simulating the HashMap constraint),
        // but both edges exist — our checker should catch the duplication.
        g.node_parent.insert(NodeId(2), EdgeId(0));

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("incoming anchors") || msg.contains("multiple"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Nullary derivation
    // ---------------------------------------------------------------------------

    #[test]
    fn nullary_derivation_fails() {
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "out".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties.push(PropId(0));

        // Derivation with zero inputs.
        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "constant".to_string(),
            inputs: vec![], // zero inputs!
            output_prop: PropId(0),
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("zero inputs") || msg.contains("nullary"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn unary_derivation_passes() {
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "in".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "out".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(0)],
            output_prop: PropId(1),
        });

        let di_eid = EdgeId(g.edges.len() as u32);
        g.edges.push(Edge::DerivInput {
            source_prop: PropId(0),
            target_deriv: DerivId(0),
        });
        g.prop_edges.entry(PropId(0)).or_default().push(di_eid);

        assert!(validate(&g).is_ok());
    }

    // ---------------------------------------------------------------------------
    // Cycle detection — direct cycle
    // ---------------------------------------------------------------------------

    #[test]
    fn direct_constraint_cycle_fails() {
        // root has two properties p0 and p1 with p0->p1 and p1->p0 constraints.
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "a".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "b".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        // p0 -> p1
        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        });
        // p1 -> p0  (cycle!)
        g.edges.push(Edge::Constraint {
            source_prop: PropId(1),
            dest_prop: PropId(0),
            operation: None,
        });

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "unexpected error: {msg}");
    }

    #[test]
    fn direct_anchor_cycle_fails() {
        // Two nodes each claiming to be the parent of the other.
        let mut g = empty_graph();
        for i in 0..2u32 {
            g.nodes.push(Node {
                id: NodeId(i),
                ident: format!("n{}", i),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: false,
                is_selected: false,
            });
        }
        // n0 -> n1 (anchor)
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        });
        // n1 -> n0 (anchor, cycle!)
        g.edges.push(Edge::Anchor {
            parent: NodeId(1),
            child: NodeId(0),
            operation: None,
        });
        // Both nodes appear as children.
        g.node_parent.insert(NodeId(0), EdgeId(1));
        g.node_parent.insert(NodeId(1), EdgeId(0));

        // Cycle check fires before root/orphan checks because they are both
        // registered as having parents, but cycle detection still catches it.
        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        // May be cycle or other structural error; cycle detection is key.
        assert!(
            msg.contains("cycle") || msg.contains("@anchored"),
            "unexpected error: {msg}"
        );
    }

    // ---------------------------------------------------------------------------
    // Cycle detection — transitive cycle through derivation
    // ---------------------------------------------------------------------------

    #[test]
    fn transitive_deriv_cycle_fails() {
        // root has properties p0, p1.
        // Derivation D: input=p1, output=p0.
        // Constraint: p0 -> p1.
        // Cycle: p0 --(constraint)--> p1 --(DerivInput)--> D --(output)--> p0.
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "a".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "b".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        // Derivation: input p1 -> output p0
        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(1)],
            output_prop: PropId(0),
        });

        // DerivInput edge: p1 -> D0
        g.edges.push(Edge::DerivInput {
            source_prop: PropId(1),
            target_deriv: DerivId(0),
        });
        // Constraint: p0 -> p1
        g.edges.push(Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        });
        // D0 output to p0 is encoded in Derivation.output_prop, not as an edge
        // in the edge list — but check_cycles reads it from derivations directly.

        let err = validate(&g).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cycle"), "unexpected error: {msg}");
    }

    // ---------------------------------------------------------------------------
    // Cycle detection — acyclic derivation graph passes
    // ---------------------------------------------------------------------------

    #[test]
    fn acyclic_deriv_graph_passes() {
        // root: p0 --(DerivInput)--> D0 --(output)--> p1
        // No back-edge.
        let mut g = single_root_graph();

        g.properties.push(Property {
            id: PropId(0),
            node: NodeId(0),
            name: "in".to_string(),
            critical: true,
            constrained: false,
        });
        g.properties.push(Property {
            id: PropId(1),
            node: NodeId(0),
            name: "out".to_string(),
            critical: true,
            constrained: false,
        });
        g.nodes[0].properties = vec![PropId(0), PropId(1)];

        g.derivations.push(Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(0)],
            output_prop: PropId(1),
        });

        let di_eid = EdgeId(g.edges.len() as u32);
        g.edges.push(Edge::DerivInput {
            source_prop: PropId(0),
            target_deriv: DerivId(0),
        });
        g.prop_edges.entry(PropId(0)).or_default().push(di_eid);

        assert!(validate(&g).is_ok());
    }

    // ---------------------------------------------------------------------------
    // Cycle detection — chain of three nodes is acyclic
    // ---------------------------------------------------------------------------

    #[test]
    fn three_node_chain_passes() {
        // root -> mid -> leaf (all via Link)
        let mut g = empty_graph();
        for (i, name) in ["root", "mid", "leaf"].iter().enumerate() {
            g.nodes.push(Node {
                id: NodeId(i as u32),
                ident: name.to_string(),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: i == 0,
                is_selected: false,
            });
        }
        g.edges.push(Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        });
        g.edges.push(Edge::Anchor {
            parent: NodeId(1),
            child: NodeId(2),
            operation: None,
        });
        g.node_parent.insert(NodeId(1), EdgeId(0));
        g.node_parent.insert(NodeId(2), EdgeId(1));
        g.node_children.entry(NodeId(0)).or_default().push(EdgeId(0));
        g.node_children.entry(NodeId(1)).or_default().push(EdgeId(1));

        assert!(validate(&g).is_ok());
    }
}
