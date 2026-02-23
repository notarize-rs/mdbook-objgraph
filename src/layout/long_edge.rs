/// Eiglsperger implicit long edge segments (DESIGN.md §4.2.2).
///
/// After layer assignment, edges whose actual span exceeds the minimum span
/// are "long edges".  Rather than inserting explicit dummy nodes (as in
/// classical Sugiyama), we use implicit segment entries in each intermediate
/// layer.  This module builds the per-layer item lists and identifies all
/// long edges.

use std::collections::HashMap;

use crate::model::types::{DerivId, Edge, EdgeId, Graph, NodeId};

use super::layer_assign::LayerAssignment;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A long edge that spans multiple layers, represented implicitly
/// rather than with explicit dummy nodes.
#[derive(Debug, Clone)]
pub struct LongEdge {
    pub edge_id: EdgeId,
    pub source_layer: u32,
    pub target_layer: u32,
    /// Position within each intermediate layer's ordering.
    /// Key: layer index, Value: fractional position in that layer's ordering.
    /// Initially set to 0.0; populated by the crossing minimization phase.
    pub positions: HashMap<u32, f64>,
}

/// An item within a layer's ordering.
#[derive(Debug, Clone)]
pub enum LayerItem {
    Node(NodeId),
    Derivation(DerivId),
    /// A long edge segment passing through this layer.
    /// Fields: (edge_id, layer_index).
    Segment(EdgeId, u32),
}

/// A single layer with its ordered items.
#[derive(Debug, Clone)]
pub struct LayerEntry {
    pub items: Vec<LayerItem>,
}

// ---------------------------------------------------------------------------
// Edge endpoint resolution
// ---------------------------------------------------------------------------

/// Identifies the "layout vertex" and its assigned layer for one endpoint of
/// an edge.
#[derive(Debug, Clone, Copy)]
enum EndpointVertex {
    Node(#[allow(dead_code)] NodeId),
    Deriv(#[allow(dead_code)] DerivId),
}

/// Resolve the upstream (source) and downstream (target) layout vertices for
/// an edge, together with their assigned layers.
fn edge_endpoints(
    edge: &Edge,
    graph: &Graph,
    assignment: &LayerAssignment,
) -> Option<(EndpointVertex, u32, EndpointVertex, u32)> {
    match edge {
        Edge::Anchor { parent, child, .. } => {
            let src_layer = *assignment.node_layers.get(parent)?;
            let tgt_layer = *assignment.node_layers.get(child)?;
            Some((
                EndpointVertex::Node(*parent),
                src_layer,
                EndpointVertex::Node(*child),
                tgt_layer,
            ))
        }
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let dst_node = graph.properties[dest_prop.index()].node;
            let src_layer = *assignment.node_layers.get(&src_node)?;
            let tgt_layer = *assignment.node_layers.get(&dst_node)?;
            Some((
                EndpointVertex::Node(src_node),
                src_layer,
                EndpointVertex::Node(dst_node),
                tgt_layer,
            ))
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let src_layer = *assignment.node_layers.get(&src_node)?;
            let tgt_layer = *assignment.deriv_layers.get(target_deriv)?;
            Some((
                EndpointVertex::Node(src_node),
                src_layer,
                EndpointVertex::Deriv(*target_deriv),
                tgt_layer,
            ))
        }
    }
}

/// Compute the minimum span for an edge based on the vertex types of its
/// endpoints.
fn min_span_for_edge(edge: &Edge, _graph: &Graph) -> u32 {
    match edge {
        Edge::Anchor { .. } => 2,          // Node -> Node
        Edge::Constraint { .. } => 2,    // Node -> Node
        Edge::DerivInput { .. } => 1,    // Node -> Deriv
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Build layer entries from a layer assignment, identifying long edges
/// and inserting segment entries in intermediate layers.
///
/// Returns a vector of `LayerEntry` (indexed by layer number, 0..num_layers)
/// and a vector of `LongEdge` descriptors for edges that span more than
/// their minimum span.
pub fn build_layers(
    assignment: &LayerAssignment,
    graph: &Graph,
) -> (Vec<LayerEntry>, Vec<LongEdge>) {
    // Step 1: Create empty layers.
    let num_layers = assignment.num_layers as usize;
    let mut layers: Vec<LayerEntry> = (0..num_layers)
        .map(|_| LayerEntry { items: Vec::new() })
        .collect();

    // Step 2: Place nodes in their assigned layers.
    for (&node_id, &layer) in &assignment.node_layers {
        if (layer as usize) < num_layers {
            layers[layer as usize].items.push(LayerItem::Node(node_id));
        }
    }

    // Step 3: Place derivations in their assigned layers.
    for (&deriv_id, &layer) in &assignment.deriv_layers {
        if (layer as usize) < num_layers {
            layers[layer as usize]
                .items
                .push(LayerItem::Derivation(deriv_id));
        }
    }

    // Step 4: Identify long edges and insert segment entries.
    let mut long_edges: Vec<LongEdge> = Vec::new();

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);

        let Some((_src_vertex, src_layer, _tgt_vertex, tgt_layer)) =
            edge_endpoints(edge, graph, assignment)
        else {
            continue;
        };

        let min_span = min_span_for_edge(edge, graph);
        let actual_span = tgt_layer.saturating_sub(src_layer);

        if actual_span > min_span {
            // This is a long edge.  Insert segment entries in every
            // intermediate layer between source and target (exclusive of both
            // endpoints).
            let mut positions: HashMap<u32, f64> = HashMap::new();

            for layer_idx in (src_layer + 1)..tgt_layer {
                if (layer_idx as usize) < num_layers {
                    layers[layer_idx as usize]
                        .items
                        .push(LayerItem::Segment(edge_id, layer_idx));
                    positions.insert(layer_idx, 0.0);
                }
            }

            long_edges.push(LongEdge {
                edge_id,
                source_layer: src_layer,
                target_layer: tgt_layer,
                positions,
            });
        }
    }

    // Step 5: Sort items within each layer for deterministic ordering.
    // Nodes and derivations are sorted by their ID; segments by edge ID then layer.
    for layer in &mut layers {
        layer.items.sort_by_key(layer_item_sort_key);
    }

    (layers, long_edges)
}

/// Sort key for deterministic ordering within a layer.
/// Returns (kind, primary_id, secondary_id) where kind is:
///   0 = Node, 1 = Derivation, 2 = Segment
fn layer_item_sort_key(item: &LayerItem) -> (u8, u32, u32) {
    match item {
        LayerItem::Node(nid) => (0, nid.0, 0),
        LayerItem::Derivation(did) => (1, did.0, 0),
        LayerItem::Segment(eid, layer) => (2, eid.0, *layer),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::*;

    /// Helper: build a minimal Graph.
    fn make_graph(
        nodes: Vec<Node>,
        properties: Vec<Property>,
        derivations: Vec<Derivation>,
        edges: Vec<Edge>,
    ) -> Graph {
        let mut prop_edges: HashMap<PropId, Vec<EdgeId>> = HashMap::new();
        let mut node_children: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut node_parent: HashMap<NodeId, EdgeId> = HashMap::new();

        for (idx, edge) in edges.iter().enumerate() {
            let eid = EdgeId(idx as u32);
            match edge {
                Edge::Anchor { parent, child, .. } => {
                    node_children.entry(*parent).or_default().push(eid);
                    node_parent.insert(*child, eid);
                }
                Edge::Constraint {
                    source_prop,
                    dest_prop,
                    ..
                } => {
                    prop_edges.entry(*source_prop).or_default().push(eid);
                    prop_edges.entry(*dest_prop).or_default().push(eid);
                }
                Edge::DerivInput {
                    source_prop,
                    ..
                } => {
                    prop_edges.entry(*source_prop).or_default().push(eid);
                }
            }
        }

        Graph {
            nodes,
            properties,
            derivations,
            edges,
            domains: vec![],
            prop_edges,
            node_children,
            node_parent,
        }
    }

    fn make_node(id: u32, ident: &str, props: &[u32]) -> Node {
        Node {
            id: NodeId(id),
            ident: ident.to_string(),
            display_name: None,
            properties: props.iter().map(|&p| PropId(p)).collect(),
            domain: None,
            is_root: id == 0,
            is_selected: false,
        }
    }

    fn make_prop(id: u32, node: u32, name: &str) -> Property {
        Property {
            id: PropId(id),
            node: NodeId(node),
            name: name.to_string(),
            critical: true, constrained: false,
        }
    }

    // ----- Test 1: Simple two-node graph, no long edges -----

    #[test]
    fn test_no_long_edges() {
        let nodes = vec![make_node(0, "a", &[]), make_node(1, "b", &[])];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let assignment = LayerAssignment {
            node_layers: HashMap::from([(NodeId(0), 0), (NodeId(1), 2)]),
            deriv_layers: HashMap::new(),
            num_layers: 3,
        };

        let (layers, long_edges) = build_layers(&assignment, &graph);

        // No long edges: span is 2, min_span is 2.
        assert!(long_edges.is_empty(), "expected no long edges");

        // Layer 0 has node a, layer 2 has node b, layer 1 is empty.
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].items.len(), 1);
        assert!(matches!(layers[0].items[0], LayerItem::Node(NodeId(0))));
        assert_eq!(layers[1].items.len(), 0);
        assert_eq!(layers[2].items.len(), 1);
        assert!(matches!(layers[2].items[0], LayerItem::Node(NodeId(1))));
    }

    // ----- Test 2: Long edge spanning 4 layers -----

    #[test]
    fn test_long_edge_detection() {
        // Nodes a (layer 0) and b (layer 6) connected by a Link.
        // Min span for Node->Node is 2, actual span is 6 => long edge.
        // Segments should appear in layers 1, 2, 3, 4, 5.
        let nodes = vec![make_node(0, "a", &[]), make_node(1, "b", &[])];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let assignment = LayerAssignment {
            node_layers: HashMap::from([(NodeId(0), 0), (NodeId(1), 6)]),
            deriv_layers: HashMap::new(),
            num_layers: 7,
        };

        let (layers, long_edges) = build_layers(&assignment, &graph);

        assert_eq!(long_edges.len(), 1);
        let le = &long_edges[0];
        assert_eq!(le.edge_id, EdgeId(0));
        assert_eq!(le.source_layer, 0);
        assert_eq!(le.target_layer, 6);

        // Segments in layers 1..6 (exclusive of 0 and 6).
        assert_eq!(le.positions.len(), 5);
        for l in 1..=5 {
            assert!(le.positions.contains_key(&l), "missing segment at layer {}", l);
        }

        // Check that intermediate layers have segment items.
        for l in 1..=5 {
            let has_segment = layers[l as usize].items.iter().any(|item| {
                matches!(item, LayerItem::Segment(EdgeId(0), layer) if *layer == l)
            });
            assert!(has_segment, "layer {} should have a segment for edge 0", l);
        }
    }

    // ----- Test 3: Mixed nodes and derivations -----

    #[test]
    fn test_with_derivation_layers() {
        // Node A (layer 0), Deriv D (layer 1), Node B (layer 2).
        // Edge: DerivInput from A.p0 to D. Span=1, min_span=1. Not long.
        // Edge: Link from A to B. Span=2, min_span=2. Not long.
        let nodes = vec![make_node(0, "A", &[0]), make_node(1, "B", &[1])];
        let props = vec![make_prop(0, 0, "p0"), make_prop(1, 1, "p1")];
        let derivations = vec![Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(0)],
            output_prop: PropId(1),
        }];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::DerivInput {
                source_prop: PropId(0),
                target_deriv: DerivId(0),
            },
        ];
        let graph = make_graph(nodes, props, derivations, edges);

        let assignment = LayerAssignment {
            node_layers: HashMap::from([(NodeId(0), 0), (NodeId(1), 2)]),
            deriv_layers: HashMap::from([(DerivId(0), 1)]),
            num_layers: 3,
        };

        let (layers, long_edges) = build_layers(&assignment, &graph);

        assert!(long_edges.is_empty(), "no long edges expected");

        // Layer 0: Node A
        assert_eq!(layers[0].items.len(), 1);
        assert!(matches!(layers[0].items[0], LayerItem::Node(NodeId(0))));

        // Layer 1: Derivation D
        assert_eq!(layers[1].items.len(), 1);
        assert!(matches!(layers[1].items[0], LayerItem::Derivation(DerivId(0))));

        // Layer 2: Node B
        assert_eq!(layers[2].items.len(), 1);
        assert!(matches!(layers[2].items[0], LayerItem::Node(NodeId(1))));
    }

    // ----- Test 4: Multiple long edges -----

    #[test]
    fn test_multiple_long_edges() {
        // Three nodes in a chain: a(0) -> b(2) -> c(8).
        // a->b: span 2, min 2 => not long.
        // b->c: span 6, min 2 => long, segments in 3,4,5,6,7.
        //
        // Also add a constraint from a to c (via properties).
        // a->c: span 8, min 2 => long, segments in 1,2,3,4,5,6,7.
        // But a's node is at 0 and c's node is at 8.
        // Wait, the constraint is from a property on a to a property on c.
        // Since a is at layer 0 and c is at layer 8, the constraint span is 8,
        // min_span=2, so segments in layers 1..7.
        let nodes = vec![
            make_node(0, "a", &[0]),
            make_node(1, "b", &[]),
            make_node(2, "c", &[1]),
        ];
        let props = vec![make_prop(0, 0, "out"), make_prop(1, 2, "in")];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(1),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, props, vec![], edges);

        let assignment = LayerAssignment {
            node_layers: HashMap::from([
                (NodeId(0), 0),
                (NodeId(1), 2),
                (NodeId(2), 8),
            ]),
            deriv_layers: HashMap::new(),
            num_layers: 9,
        };

        let (layers, long_edges) = build_layers(&assignment, &graph);

        // Edge 0 (a->b): span 2, not long.
        // Edge 1 (b->c): span 6, long.
        // Edge 2 (a->c constraint): span 8, long.
        assert_eq!(long_edges.len(), 2);

        // Sort by edge_id for deterministic checks.
        let mut sorted = long_edges.clone();
        sorted.sort_by_key(|le| le.edge_id.0);

        // Edge 1 (b->c): layers 3..7 inclusive.
        assert_eq!(sorted[0].edge_id, EdgeId(1));
        assert_eq!(sorted[0].source_layer, 2);
        assert_eq!(sorted[0].target_layer, 8);
        assert_eq!(sorted[0].positions.len(), 5); // layers 3,4,5,6,7

        // Edge 2 (a->c): layers 1..7 inclusive.
        assert_eq!(sorted[1].edge_id, EdgeId(2));
        assert_eq!(sorted[1].source_layer, 0);
        assert_eq!(sorted[1].target_layer, 8);
        assert_eq!(sorted[1].positions.len(), 7); // layers 1,2,3,4,5,6,7

        // Verify a busy intermediate layer (e.g., layer 5) has segments from both.
        let layer5_segments: Vec<_> = layers[5]
            .items
            .iter()
            .filter(|item| matches!(item, LayerItem::Segment(_, _)))
            .collect();
        assert_eq!(layer5_segments.len(), 2, "layer 5 should have 2 segments");
    }

    // ----- Test 5: Empty graph -----

    #[test]
    fn test_empty_graph() {
        let graph = make_graph(vec![], vec![], vec![], vec![]);
        let assignment = LayerAssignment {
            node_layers: HashMap::new(),
            deriv_layers: HashMap::new(),
            num_layers: 0,
        };

        let (layers, long_edges) = build_layers(&assignment, &graph);
        assert!(layers.is_empty());
        assert!(long_edges.is_empty());
    }

    // ----- Test 6: Integration with network simplex -----

    #[test]
    fn test_integration_with_layer_assign() {
        use super::super::layer_assign::network_simplex;

        // Chain: a -> b -> c (all links).
        let nodes = vec![
            make_node(0, "a", &[]),
            make_node(1, "b", &[]),
            make_node(2, "c", &[]),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let assignment = network_simplex(&graph).unwrap();
        let (layers, long_edges) = build_layers(&assignment, &graph);

        // All edges at minimum span => no long edges.
        assert!(
            long_edges.is_empty(),
            "chain with minimum spans should have no long edges"
        );

        // Layers: 0 (a), 1 (empty), 2 (b), 3 (empty), 4 (c).
        assert_eq!(assignment.num_layers as usize, layers.len());
        assert_eq!(layers[0].items.len(), 1);
        assert_eq!(layers[2].items.len(), 1);
        assert_eq!(layers[4].items.len(), 1);
    }
}
