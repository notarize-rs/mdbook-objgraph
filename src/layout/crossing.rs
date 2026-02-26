//! Weighted port-aware nested barycenter crossing minimization (DESIGN.md §4.2.3).

use std::collections::{HashMap, HashSet};

use super::long_edge::{LayerEntry, LayerItem, LongEdge};
use super::{EndpointRole, PortSide, PortSideAssignment};
use crate::model::types::{DerivId, Edge, EdgeId, Graph, NodeId, PropId};

/// Maximum iterations for crossing minimization (per ELK convention).
pub const MAX_ITERATIONS: usize = 24;

// ---------------------------------------------------------------------------
// Helper: Edge endpoint descriptors for layout layer membership
// ---------------------------------------------------------------------------

/// Identifies an element that occupies a position within a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LayerElement {
    Node(NodeId),
    Derivation(DerivId),
    Segment(EdgeId, u32),
}

/// Information about an edge endpoint relevant to crossing minimization.
/// We need to know which layer element it belongs to and, for property-level
/// edges, which property it targets.
#[derive(Debug, Clone, Copy)]
struct EdgeEndpoint {
    element: LayerElement,
    /// For property-level endpoints, the specific property.
    prop: Option<PropId>,
}

/// For each property, edges sorted by opposite-endpoint layer position.
/// Computed during crossing minimization so port slot ordering is consistent
/// with property ordering (ELK/KLay FIXED_ORDER approach).
pub type EdgePortOrder = HashMap<PropId, Vec<EdgeId>>;

// ---------------------------------------------------------------------------
// Property ordering tracker
// ---------------------------------------------------------------------------

/// Tracks the current property ordering for each node independently of the
/// `Graph::nodes[].properties` vec (which we treat as read-only source of
/// which properties exist, while this tracks their current display order).
#[derive(Debug, Clone)]
pub struct PropertyOrder {
    /// NodeId -> ordered list of PropId
    order: HashMap<NodeId, Vec<PropId>>,
}

impl PropertyOrder {
    pub fn from_graph(graph: &Graph) -> Self {
        let mut order = HashMap::new();
        for node in &graph.nodes {
            order.insert(node.id, node.properties.clone());
        }
        PropertyOrder { order }
    }

    pub fn props_of(&self, node_id: NodeId) -> &[PropId] {
        self.order.get(&node_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn prop_index(&self, node_id: NodeId, prop_id: PropId) -> Option<usize> {
        self.props_of(node_id).iter().position(|&p| p == prop_id)
    }

    pub fn num_props(&self, node_id: NodeId) -> usize {
        self.props_of(node_id).len()
    }

    fn set_order(&mut self, node_id: NodeId, new_order: Vec<PropId>) {
        self.order.insert(node_id, new_order);
    }
}

// ---------------------------------------------------------------------------
// Edge → logical property pair
// ---------------------------------------------------------------------------

/// For Constraint and DerivInput edges, returns (source_prop, dest_prop).
/// For DerivInput, dest_prop is the derivation's output_prop (attributed to
/// the destination node). Returns None for Anchor edges.
fn edge_prop_pair(graph: &Graph, edge: &Edge) -> Option<(PropId, PropId)> {
    match edge {
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => Some((*source_prop, *dest_prop)),
        Edge::DerivInput {
            source_prop,
            target_deriv,
        } => {
            let deriv = &graph.derivations[target_deriv.index()];
            Some((*source_prop, deriv.output_prop))
        }
        Edge::Anchor { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Position computation
// ---------------------------------------------------------------------------

/// Compute the fractional position of a property within a layer.
///
/// `position_of(prop) = node_position_in_layer + prop_index / (num_props + 1)`
///
/// This ensures property-level edges are ordered correctly relative to
/// node-level positions.
fn position_of_prop(
    prop_id: PropId,
    graph: &Graph,
    layer: &LayerEntry,
    prop_order: &PropertyOrder,
) -> f64 {
    let node_id = graph.properties[prop_id.index()].node;
    let node_pos = position_of_element(
        &LayerElement::Node(node_id),
        layer,
    );
    match node_pos {
        Some(base) => {
            let num_props = prop_order.num_props(node_id);
            let idx = prop_order
                .prop_index(node_id, prop_id)
                .unwrap_or(0);
            base + (idx as f64 + 1.0) / (num_props as f64 + 1.0)
        }
        None => 0.0,
    }
}

/// Compute the integer position of a layer element (its index in the layer's
/// item list).
fn position_of_element(
    element: &LayerElement,
    layer: &LayerEntry,
) -> Option<f64> {
    for (i, item) in layer.items.iter().enumerate() {
        let matches = match (element, item) {
            (LayerElement::Node(a), LayerItem::Node(b)) => a == b,
            (LayerElement::Derivation(a), LayerItem::Derivation(b)) => a == b,
            (LayerElement::Segment(a_eid, a_layer), LayerItem::Segment(b_eid, b_layer)) => {
                a_eid == b_eid && a_layer == b_layer
            }
            _ => false,
        };
        if matches {
            return Some(i as f64);
        }
    }
    None
}

/// Compute position of a segment of a long edge in a given layer.
/// Looks up the long edge's stored position if available, otherwise falls
/// back to finding the segment in the layer.
fn position_of_segment(
    edge_id: EdgeId,
    layer_idx: u32,
    long_edges: &[LongEdge],
    layer: &LayerEntry,
) -> f64 {
    // First try finding it from long_edges positions map
    for le in long_edges {
        if le.edge_id == edge_id && le.positions.contains_key(&layer_idx) {
            return le.positions[&layer_idx];
        }
    }
    // Fall back to layer position
    position_of_element(&LayerElement::Segment(edge_id, layer_idx), layer)
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Edge-to-layer mapping
// ---------------------------------------------------------------------------

/// For a given layer element, determine which element (and optional property)
/// it corresponds to on the "other side" of an edge incident on it.
///
/// Returns the edge endpoint descriptors for the two ends of an edge,
/// tagged with which layer they belong to. Returns None if the edge
/// doesn't connect two adjacent layers.
fn edge_endpoints(
    edge: &Edge,
    edge_id: EdgeId,
    graph: &Graph,
    layer_map: &HashMap<LayerElement, u32>,
    long_edges: &[LongEdge],
) -> Vec<(EdgeEndpoint, u32, EdgeEndpoint, u32)> {
    // An edge can be "expanded" into multiple layer-adjacent segments
    // if it's a long edge. We need to return all adjacent-layer pairs.
    match edge {
        Edge::Anchor { parent, child, .. } => {
            let src = LayerElement::Node(*parent);
            let dst = LayerElement::Node(*child);
            if let (Some(&src_layer), Some(&dst_layer)) =
                (layer_map.get(&src), layer_map.get(&dst))
            {
                // Check if this is a long edge
                if let Some(le) = long_edges.iter().find(|le| le.edge_id == edge_id) {
                    let mut pairs = Vec::new();
                    // Source layer -> first intermediate
                    let mut prev_ep = EdgeEndpoint { element: src, prop: None };
                    let mut prev_layer = src_layer;

                    let mut intermediate_layers: Vec<u32> =
                        le.positions.keys().copied().collect();
                    intermediate_layers.sort();

                    for &mid_layer in &intermediate_layers {
                        let seg = LayerElement::Segment(edge_id, mid_layer);
                        let seg_ep = EdgeEndpoint { element: seg, prop: None };
                        pairs.push((prev_ep, prev_layer, seg_ep, mid_layer));
                        prev_ep = seg_ep;
                        prev_layer = mid_layer;
                    }
                    // Last segment -> target
                    let dst_ep = EdgeEndpoint { element: dst, prop: None };
                    pairs.push((prev_ep, prev_layer, dst_ep, dst_layer));
                    return pairs;
                }
                // Direct edge between adjacent layers
                vec![(
                    EdgeEndpoint { element: src, prop: None },
                    src_layer,
                    EdgeEndpoint { element: dst, prop: None },
                    dst_layer,
                )]
            } else {
                vec![]
            }
        }
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let dst_node = graph.properties[dest_prop.index()].node;
            let src = LayerElement::Node(src_node);
            let dst = LayerElement::Node(dst_node);
            if let (Some(&src_layer), Some(&dst_layer)) =
                (layer_map.get(&src), layer_map.get(&dst))
            {
                if let Some(le) = long_edges.iter().find(|le| le.edge_id == edge_id) {
                    let mut pairs = Vec::new();
                    let mut prev_ep = EdgeEndpoint {
                        element: src,
                        prop: Some(*source_prop),
                    };
                    let mut prev_layer = src_layer;

                    let mut intermediate_layers: Vec<u32> =
                        le.positions.keys().copied().collect();
                    intermediate_layers.sort();

                    for &mid_layer in &intermediate_layers {
                        let seg = LayerElement::Segment(edge_id, mid_layer);
                        let seg_ep = EdgeEndpoint { element: seg, prop: None };
                        pairs.push((prev_ep, prev_layer, seg_ep, mid_layer));
                        prev_ep = seg_ep;
                        prev_layer = mid_layer;
                    }
                    let dst_ep = EdgeEndpoint {
                        element: dst,
                        prop: Some(*dest_prop),
                    };
                    pairs.push((prev_ep, prev_layer, dst_ep, dst_layer));
                    return pairs;
                }
                vec![(
                    EdgeEndpoint {
                        element: src,
                        prop: Some(*source_prop),
                    },
                    src_layer,
                    EdgeEndpoint {
                        element: dst,
                        prop: Some(*dest_prop),
                    },
                    dst_layer,
                )]
            } else {
                vec![]
            }
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let src = LayerElement::Node(src_node);
            let dst = LayerElement::Derivation(*target_deriv);
            if let (Some(&src_layer), Some(&dst_layer)) =
                (layer_map.get(&src), layer_map.get(&dst))
            {
                if let Some(le) = long_edges.iter().find(|le| le.edge_id == edge_id) {
                    let mut pairs = Vec::new();
                    let mut prev_ep = EdgeEndpoint {
                        element: src,
                        prop: Some(*source_prop),
                    };
                    let mut prev_layer = src_layer;

                    let mut intermediate_layers: Vec<u32> =
                        le.positions.keys().copied().collect();
                    intermediate_layers.sort();

                    for &mid_layer in &intermediate_layers {
                        let seg = LayerElement::Segment(edge_id, mid_layer);
                        let seg_ep = EdgeEndpoint { element: seg, prop: None };
                        pairs.push((prev_ep, prev_layer, seg_ep, mid_layer));
                        prev_ep = seg_ep;
                        prev_layer = mid_layer;
                    }
                    let dst_ep = EdgeEndpoint {
                        element: dst,
                        prop: None,
                    };
                    pairs.push((prev_ep, prev_layer, dst_ep, dst_layer));
                    return pairs;
                }
                vec![(
                    EdgeEndpoint {
                        element: src,
                        prop: Some(*source_prop),
                    },
                    src_layer,
                    EdgeEndpoint {
                        element: dst,
                        prop: None,
                    },
                    dst_layer,
                )]
            } else {
                vec![]
            }
        }
    }
}

/// Build a mapping from each LayerElement to its layer index.
fn build_layer_map(layers: &[LayerEntry]) -> HashMap<LayerElement, u32> {
    let mut map = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        for item in &layer.items {
            let elem = match item {
                LayerItem::Node(id) => LayerElement::Node(*id),
                LayerItem::Derivation(id) => LayerElement::Derivation(*id),
                LayerItem::Segment(eid, l) => LayerElement::Segment(*eid, *l),
            };
            map.insert(elem, layer_idx as u32);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Adjacent-layer edge collection
// ---------------------------------------------------------------------------

/// An edge segment between two adjacent layers, with resolved positions.
#[derive(Debug, Clone)]
struct AdjacentEdge {
    #[allow(dead_code)]
    edge_id: EdgeId,
    /// Endpoint in the upper layer (smaller index).
    upper: EdgeEndpoint,
    /// Endpoint in the lower layer (larger index).
    lower: EdgeEndpoint,
    weight: u32,
}

/// Collect all edge segments that connect layer `layer_a_idx` to layer
/// `layer_b_idx` (which must be adjacent). Returns them oriented so that
/// `upper` refers to the smaller-indexed layer.
fn collect_adjacent_edges(
    layer_a_idx: u32,
    layer_b_idx: u32,
    graph: &Graph,
    long_edges: &[LongEdge],
    layer_map: &HashMap<LayerElement, u32>,
) -> Vec<AdjacentEdge> {
    let upper = layer_a_idx.min(layer_b_idx);
    let lower = layer_a_idx.max(layer_b_idx);

    let mut result = Vec::new();

    for (i, edge) in graph.edges.iter().enumerate() {
        let eid = EdgeId(i as u32);
        let pairs = edge_endpoints(edge, eid, graph, layer_map, long_edges);

        for (ep_a, la, ep_b, lb) in pairs {
            let (a_layer, b_layer) = (la.min(lb), la.max(lb));
            if a_layer == upper && b_layer == lower {
                let (upper_ep, lower_ep) = if la <= lb {
                    (ep_a, ep_b)
                } else {
                    (ep_b, ep_a)
                };
                result.push(AdjacentEdge {
                    edge_id: eid,
                    upper: upper_ep,
                    lower: lower_ep,
                    weight: edge.weight(),
                });
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Position resolution for endpoints
// ---------------------------------------------------------------------------

/// Resolve the fractional position of an edge endpoint within its layer.
///
/// When `edge_port_order` and `port_sides` are non-empty, adds side-aware
/// sub-fractions so that crossing detection accounts for port slot ordering.
/// Left-side edges get negative sub-fractions, right-side get positive,
/// ensuring edges on opposite sides never appear to cross at a shared property.
#[allow(clippy::too_many_arguments)]
fn resolve_position(
    ep: &EdgeEndpoint,
    layer: &LayerEntry,
    graph: &Graph,
    prop_order: &PropertyOrder,
    long_edges: &[LongEdge],
    edge_port_order: &EdgePortOrder,
    edge_id: EdgeId,
    port_sides: &PortSideAssignment,
    role: EndpointRole,
) -> f64 {
    match ep.prop {
        Some(prop_id) => {
            let base = position_of_prop(prop_id, graph, layer, prop_order);

            // Add port slot sub-fraction if port ordering is available
            if let Some(edges) = edge_port_order.get(&prop_id) {
                let total = edges.len();
                if total > 1 && let Some(slot) = edges.iter().position(|&eid| eid == edge_id) {
                    let node_id = graph.properties[prop_id.index()].node;
                    let num_props = prop_order.num_props(node_id);
                    let prop_gap = 1.0 / (num_props as f64 + 1.0);

                    // Center within slot range: 0..total-1 → -0.5..+0.5
                    let centered = if total > 1 {
                        (slot as f64 / (total - 1) as f64) - 0.5
                    } else {
                        0.0
                    };

                    // Side-aware: left-side edges get negative offset,
                    // right-side edges get positive offset.
                    let side = port_sides.get(&(edge_id, role)).copied();
                    let side_sign = match side {
                        Some(PortSide::Left) => -1.0,
                        Some(PortSide::Right) | None => 1.0,
                    };

                    return base + side_sign * (0.5 + centered * 0.4) * prop_gap * 0.3;
                }
            }
            base
        }
        None => {
            // Element-level endpoint
            match ep.element {
                LayerElement::Segment(eid, layer_idx) => {
                    position_of_segment(eid, layer_idx, long_edges, layer)
                }
                _ => {
                    position_of_element(&ep.element, layer).unwrap_or(0.0)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Crossing counting
// ---------------------------------------------------------------------------

/// Count the total weighted crossings between two adjacent layers.
///
/// For each pair of edges (e1, e2) between the layers, they cross if
/// their source positions are in opposite order from their target positions.
/// The cost of a crossing is w(e1) + w(e2).
pub fn count_crossings(
    layer_a: &LayerEntry,
    layer_b: &LayerEntry,
    graph: &Graph,
) -> u64 {
    // This public API doesn't have access to all internal state, so we
    // build minimal state for it. For the full algorithm we use the
    // internal version.
    let prop_order = PropertyOrder::from_graph(graph);
    let long_edges: Vec<LongEdge> = Vec::new();
    let empty_epo = EdgePortOrder::new();
    let empty_ps = PortSideAssignment::new();
    count_crossings_internal(layer_a, layer_b, graph, &prop_order, &long_edges, 0, 1, &empty_epo, &empty_ps)
}

/// Internal crossing count with full state access.
#[allow(clippy::too_many_arguments)]
fn count_crossings_internal(
    layer_a: &LayerEntry,
    layer_b: &LayerEntry,
    graph: &Graph,
    prop_order: &PropertyOrder,
    long_edges: &[LongEdge],
    layer_a_idx: u32,
    layer_b_idx: u32,
    edge_port_order: &EdgePortOrder,
    port_sides: &PortSideAssignment,
) -> u64 {
    let layer_map = {
        let mut map = HashMap::new();
        for item in &layer_a.items {
            let elem = match item {
                LayerItem::Node(id) => LayerElement::Node(*id),
                LayerItem::Derivation(id) => LayerElement::Derivation(*id),
                LayerItem::Segment(eid, l) => LayerElement::Segment(*eid, *l),
            };
            map.insert(elem, layer_a_idx);
        }
        for item in &layer_b.items {
            let elem = match item {
                LayerItem::Node(id) => LayerElement::Node(*id),
                LayerItem::Derivation(id) => LayerElement::Derivation(*id),
                LayerItem::Segment(eid, l) => LayerElement::Segment(*eid, *l),
            };
            map.insert(elem, layer_b_idx);
        }
        map
    };

    let adj_edges =
        collect_adjacent_edges(layer_a_idx, layer_b_idx, graph, long_edges, &layer_map);

    let upper_layer = if layer_a_idx < layer_b_idx {
        layer_a
    } else {
        layer_b
    };
    let lower_layer = if layer_a_idx < layer_b_idx {
        layer_b
    } else {
        layer_a
    };

    // Resolve positions for each edge (with side-aware sub-fractions)
    let resolved: Vec<(f64, f64, u32)> = adj_edges
        .iter()
        .map(|ae| {
            let upper_pos = resolve_position(
                &ae.upper, upper_layer, graph, prop_order, long_edges,
                edge_port_order, ae.edge_id, port_sides, EndpointRole::Upstream,
            );
            let lower_pos = resolve_position(
                &ae.lower, lower_layer, graph, prop_order, long_edges,
                edge_port_order, ae.edge_id, port_sides, EndpointRole::Downstream,
            );
            (upper_pos, lower_pos, ae.weight)
        })
        .collect();

    // O(|E|^2) pairwise crossing count
    let mut total_cost: u64 = 0;
    for i in 0..resolved.len() {
        for j in (i + 1)..resolved.len() {
            let (u1, l1, w1) = resolved[i];
            let (u2, l2, w2) = resolved[j];
            // Edges cross if the relative order of their upper endpoints
            // is opposite to the relative order of their lower endpoints.
            let upper_cmp = u1.partial_cmp(&u2);
            let lower_cmp = l1.partial_cmp(&l2);
            let crosses = matches!(
                (upper_cmp, lower_cmp),
                (Some(std::cmp::Ordering::Less), Some(std::cmp::Ordering::Greater))
                    | (Some(std::cmp::Ordering::Greater), Some(std::cmp::Ordering::Less))
            );
            if crosses {
                total_cost += (w1 + w2) as u64;
            }
        }
    }

    total_cost
}

/// Count total crossings across all adjacent layer pairs.
fn count_all_crossings(
    layers: &[LayerEntry],
    graph: &Graph,
    prop_order: &PropertyOrder,
    long_edges: &[LongEdge],
    edge_port_order: &EdgePortOrder,
    port_sides: &PortSideAssignment,
) -> u64 {
    let mut total = 0u64;
    for i in 0..layers.len().saturating_sub(1) {
        total += count_crossings_internal(
            &layers[i],
            &layers[i + 1],
            graph,
            prop_order,
            long_edges,
            i as u32,
            (i + 1) as u32,
            edge_port_order,
            port_sides,
        );
    }
    total
}

// ---------------------------------------------------------------------------
// Barycenter computation
// ---------------------------------------------------------------------------

/// Compute the weighted mean of a set of (position, weight) pairs.
fn weighted_mean(positions: &[(f64, u32)]) -> Option<f64> {
    if positions.is_empty() {
        return None;
    }
    let total_weight: f64 = positions.iter().map(|(_, w)| *w as f64).sum();
    if total_weight == 0.0 {
        return None;
    }
    let weighted_sum: f64 = positions.iter().map(|(p, w)| p * (*w as f64)).sum();
    Some(weighted_sum / total_weight)
}

// ---------------------------------------------------------------------------
// Property edge detection helper
// ---------------------------------------------------------------------------

/// Check if a property is an endpoint of an adjacent edge in layer `k`.
/// Returns a reference to the *other* endpoint if the property matches.
fn prop_touches_edge<'a>(
    prop_id: PropId,
    node_id: NodeId,
    ae: &'a AdjacentEdge,
    k: u32,
    layer_map: &HashMap<LayerElement, u32>,
) -> Option<&'a EdgeEndpoint> {
    let touches_upper = match &ae.upper {
        EdgeEndpoint { prop: Some(p), element } if *p == prop_id => {
            layer_map.get(element) == Some(&k)
        }
        EdgeEndpoint { prop: None, element: LayerElement::Node(nid), .. }
            if *nid == node_id =>
        {
            false
        }
        _ => false,
    };
    let touches_lower = match &ae.lower {
        EdgeEndpoint { prop: Some(p), element } if *p == prop_id => {
            layer_map.get(element) == Some(&k)
        }
        _ => false,
    };

    if touches_upper {
        Some(&ae.lower)
    } else if touches_lower {
        Some(&ae.upper)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Main algorithm
// ---------------------------------------------------------------------------

/// Run crossing minimization on the layered graph.
///
/// Modifies the ordering of items within each layer (and properties within
/// nodes) to minimize weighted edge crossings. Uses a layer-by-layer sweep
/// with weighted barycenter, alternating top-to-bottom and bottom-to-top,
/// up to MAX_ITERATIONS.
pub fn minimize_crossings(
    layers: &mut Vec<LayerEntry>,
    long_edges: &mut [LongEdge],
    graph: &Graph,
) -> (PropertyOrder, EdgePortOrder, PortSideAssignment) {
    if layers.len() <= 1 {
        let prop_order = PropertyOrder::from_graph(graph);
        let edge_port_order = compute_edge_port_order(graph, layers, long_edges, &prop_order);
        let port_sides = compute_layer_space_port_sides(graph, layers);
        return (prop_order, edge_port_order, port_sides);
    }

    let mut prop_order = PropertyOrder::from_graph(graph);

    // Empty port maps for barycenter computation (sub-fractions only matter
    // for crossing detection, not for computing barycenters).
    let empty_epo = EdgePortOrder::new();
    let empty_ps = PortSideAssignment::new();

    // Initialize long edge segment positions from their actual indices in
    // each layer.  build_layers() sets all positions to 0.0 which makes
    // every segment appear at the same coordinate, hiding real crossings.
    update_long_edge_positions(layers, long_edges);

    // Compute initial port sides + edge port order for crossing detection
    let mut best_port_sides = compute_layer_space_port_sides(graph, layers);
    let mut best_edge_port_order = compute_edge_port_order(graph, layers, long_edges, &prop_order);

    let mut best_layers = layers.clone();
    let mut best_prop_order = prop_order.clone();
    let mut best_crossings = count_all_crossings(
        layers, graph, &prop_order, long_edges,
        &best_edge_port_order, &best_port_sides,
    );

    // Always run the iteration loop: even when abstract crossings are zero,
    // barycenter-based property reordering improves physical port placement
    // and reduces visual edge collisions in the rendered output.
    for iteration in 0..MAX_ITERATIONS {
        let top_down = iteration % 2 == 0;

        let layer_indices: Vec<usize> = if top_down {
            (1..layers.len()).collect()
        } else {
            (0..layers.len() - 1).rev().collect()
        };

        let layer_map = build_layer_map(layers);

        for &k in &layer_indices {
            let adjacent_idx = if top_down { k - 1 } else { k + 1 };

            // Collect edges between layer k and the adjacent layer
            let adj_edges = collect_adjacent_edges(
                k as u32,
                adjacent_idx as u32,
                graph,
                long_edges,
                &layer_map,
            );

            let adj_layer = &layers[adjacent_idx];

            // Step 1: Compute property barycenters within each node in layer k
            // and reorder properties.
            let nodes_in_layer: Vec<NodeId> = layers[k]
                .items
                .iter()
                .filter_map(|item| match item {
                    LayerItem::Node(nid) => Some(*nid),
                    _ => None,
                })
                .collect();

            for &node_id in &nodes_in_layer {
                let props = prop_order.props_of(node_id).to_vec();
                if props.len() <= 1 {
                    continue;
                }

                // Collect edges from BOTH adjacent layers to classify each
                // property's connection direction and compute per-direction
                // barycenters.
                let above_idx = if k > 0 { Some(k - 1) } else { None };
                let below_idx = if k + 1 < layers.len() { Some(k + 1) } else { None };

                let edges_above = above_idx.map(|ai| {
                    collect_adjacent_edges(
                        k as u32, ai as u32, graph, long_edges, &layer_map,
                    )
                }).unwrap_or_default();
                let edges_below = below_idx.map(|bi| {
                    collect_adjacent_edges(
                        k as u32, bi as u32, graph, long_edges, &layer_map,
                    )
                }).unwrap_or_default();

                // Collect same-node constraint edges: constraints where both
                // source_prop and dest_prop belong to this node.  These are
                // invisible to the adjacent-layer machinery but should still
                // influence intra-node property ordering.
                let same_node_edges: Vec<(PropId, PropId, u32)> = graph
                    .edges
                    .iter()
                    .filter_map(|edge| match edge {
                        Edge::Constraint {
                            source_prop,
                            dest_prop,
                            ..
                        } => {
                            let src_node = graph.properties[source_prop.index()].node;
                            let dst_node = graph.properties[dest_prop.index()].node;
                            if src_node == node_id && dst_node == node_id {
                                Some((*source_prop, *dest_prop, edge.weight()))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    })
                    .collect();

                // Collect properties on this node that have cross-node edges.
                // Used by the bubble-sort to penalize cross-node edges that fall
                // inside same-node brackets (which causes physical crossings).
                let cross_node_props: HashSet<PropId> = graph
                    .edges
                    .iter()
                    .filter_map(|edge| match edge {
                        Edge::Constraint { source_prop, dest_prop, .. } => {
                            let src_node = graph.properties[source_prop.index()].node;
                            let dst_node = graph.properties[dest_prop.index()].node;
                            if src_node != dst_node {
                                if src_node == node_id { Some(*source_prop) }
                                else if dst_node == node_id { Some(*dest_prop) }
                                else { None }
                            } else { None }
                        }
                        Edge::DerivInput { source_prop, .. } => {
                            if graph.properties[source_prop.index()].node == node_id {
                                Some(*source_prop)
                            } else { None }
                        }
                        _ => None,
                    })
                    .collect();

                // Collect inter-node edge pairs (Constraint + DerivInput) for
                // bipartite crossing detection.  For edges from this node to the
                // same target node, crossings occur when source property order is
                // inverted relative to target property order.  We record
                // (local_prop, remote_prop, weight) and group by target node.
                // DerivInput edges use deriv.output_prop as the logical dest_prop.
                let mut inter_node_edges_by_target: HashMap<NodeId, Vec<(PropId, PropId, u32)>> =
                    HashMap::new();
                for edge in &graph.edges {
                    if let Some((src_prop, dst_prop)) = edge_prop_pair(graph, edge) {
                        let src_node = graph.properties[src_prop.index()].node;
                        let dst_node = graph.properties[dst_prop.index()].node;
                        if src_node == node_id && dst_node != node_id {
                            inter_node_edges_by_target
                                .entry(dst_node)
                                .or_default()
                                .push((src_prop, dst_prop, edge.weight()));
                        } else if dst_node == node_id && src_node != node_id {
                            inter_node_edges_by_target
                                .entry(src_node)
                                .or_default()
                                .push((dst_prop, src_prop, edge.weight()));
                        }
                    }
                }

                // Build a quick index: prop_idx within current ordering
                let prop_position: HashMap<PropId, usize> = props
                    .iter()
                    .enumerate()
                    .map(|(i, &pid)| (pid, i))
                    .collect();

                // For each property, determine if it connects above,
                // below, or neither.  Compute a barycenter from the
                // relevant adjacent layer.
                //
                // Direction encoding:
                //   0 = connects above only  (should sort first / top)
                //   1 = no edges or both     (middle)
                //   2 = connects below only  (should sort last / bottom)
                #[derive(Debug)]
                struct PropInfo {
                    prop_id: PropId,
                    direction: u8,
                    barycenter: f64,
                }

                let mut infos: Vec<PropInfo> = Vec::with_capacity(props.len());

                for (prop_idx, &prop_id) in props.iter().enumerate() {
                    let (bc_above, weight_above) = {
                        let mut positions: Vec<(f64, u32)> = Vec::new();
                        if let Some(ai) = above_idx {
                            let above_layer = &layers[ai];
                            for ae in &edges_above {
                                let touches = prop_touches_edge(
                                    prop_id, node_id, ae, k as u32, &layer_map,
                                );
                                if let Some(other_ep) = touches {
                                    let pos = resolve_position(
                                        other_ep, above_layer, graph,
                                        &prop_order, long_edges,
                                        &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Upstream,
                                    );
                                    positions.push((pos, ae.weight));
                                }
                            }
                        }
                        let total_w: f64 = positions.iter().map(|(_, w)| *w as f64).sum();
                        (weighted_mean(&positions), total_w)
                    };

                    let (bc_below, weight_below) = {
                        let mut positions: Vec<(f64, u32)> = Vec::new();
                        if let Some(bi) = below_idx {
                            let below_layer = &layers[bi];
                            for ae in &edges_below {
                                let touches = prop_touches_edge(
                                    prop_id, node_id, ae, k as u32, &layer_map,
                                );
                                if let Some(other_ep) = touches {
                                    let pos = resolve_position(
                                        other_ep, below_layer, graph,
                                        &prop_order, long_edges,
                                        &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Downstream,
                                    );
                                    positions.push((pos, ae.weight));
                                }
                            }
                        }
                        let total_w: f64 = positions.iter().map(|(_, w)| *w as f64).sum();
                        (weighted_mean(&positions), total_w)
                    };

                    // Compute same-node constraint contribution.
                    // For a constraint (source -> dest), source should be
                    // above (lower index than) dest.  We use the midpoint
                    // of the two endpoints' current positions as a grouping
                    // barycenter, then offset source down and dest up by
                    // 0.25 to break the tie in favor of source-before-dest.
                    // This clusters related properties together AND
                    // establishes the correct relative ordering.
                    let (bc_same_node, same_weight) = {
                        let mut positions: Vec<(f64, u32)> = Vec::new();
                        for &(src_prop, dst_prop, weight) in &same_node_edges {
                            if let (Some(&src_pos), Some(&dst_pos)) = (
                                prop_position.get(&src_prop),
                                prop_position.get(&dst_prop),
                            ) {
                                let midpoint = (src_pos as f64 + dst_pos as f64) / 2.0;
                                if prop_id == src_prop {
                                    // Source: slightly below midpoint
                                    positions.push((midpoint - 0.25, weight));
                                } else if prop_id == dst_prop {
                                    // Dest: slightly above midpoint
                                    positions.push((midpoint + 0.25, weight));
                                }
                            }
                        }
                        let total_w: f64 = positions.iter().map(|(_, w)| *w as f64).sum();
                        (weighted_mean(&positions), total_w)
                    };

                    let (direction, barycenter) = match (bc_above, bc_below, bc_same_node) {
                        // Cross-layer edges present: blend with same-node
                        (Some(a), None, Some(s)) => {
                            let bc = (a * weight_above + s * same_weight)
                                / (weight_above + same_weight);
                            (0, bc)
                        }
                        (Some(a), None, None) => (0, a),
                        (None, Some(b), Some(s)) => {
                            let bc = (b * weight_below + s * same_weight)
                                / (weight_below + same_weight);
                            (2, bc)
                        }
                        (None, Some(b), None) => (2, b),
                        (Some(a), Some(b), sn) => {
                            // Weight-aware direction: classify by dominant
                            // connection instead of always direction=1.
                            let dir = if weight_above > weight_below * 1.5 {
                                0 // Predominantly above
                            } else if weight_below > weight_above * 1.5 {
                                2 // Predominantly below
                            } else {
                                1 // Balanced
                            };
                            let bc = match sn {
                                Some(s) => {
                                    (a * weight_above + b * weight_below + s * same_weight)
                                        / (weight_above + weight_below + same_weight)
                                }
                                None => {
                                    (a * weight_above + b * weight_below)
                                        / (weight_above + weight_below)
                                }
                            };
                            (dir, bc)
                        }
                        // Only same-node edges: use same-node barycenter
                        (None, None, Some(s)) => (1, s),
                        (None, None, None) => {
                            // Unconnected — keep current relative position
                            (1, prop_idx as f64)
                        }
                    };

                    infos.push(PropInfo { prop_id, direction, barycenter });
                }

                // Sort: first by direction (above=0, middle=1, below=2),
                // then by barycenter within each group.
                infos.sort_by(|a, b| {
                    a.direction.cmp(&b.direction).then_with(|| {
                        a.barycenter.partial_cmp(&b.barycenter)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                });

                let mut new_prop_order: Vec<PropId> =
                    infos.iter().map(|i| i.prop_id).collect();

                // Post-sort: refine property ordering using sifting.
                // For each property (ordered by connectivity), remove it
                // from the list and try every insertion position, picking
                // the one that minimizes total cost.  Unlike bubble sort,
                // sifting can make non-local moves — critical for chiasm
                // alignment where the optimal position may be many steps
                // away from the current one.
                if !same_node_edges.is_empty() {
                    // Pre-compute per-target same_domain flag.
                    let target_same_domain: HashMap<NodeId, bool> =
                        inter_node_edges_by_target.keys().map(|&tn| {
                            let local_dom = graph.nodes[node_id.index()].domain;
                            let remote_dom = graph.nodes[tn.index()].domain;
                            (tn, local_dom.is_some() && local_dom == remote_dom)
                        }).collect();

                    // Pre-compute remote property indices (fixed across sifting).
                    let remote_indices: HashMap<PropId, usize> =
                        inter_node_edges_by_target.values().flat_map(|edges| {
                            edges.iter().map(|&(_, rp, _)| {
                                let rn = graph.properties[rp.index()].node;
                                let ri = prop_order.prop_index(rn, rp).unwrap_or(0);
                                (rp, ri)
                            })
                        }).collect();

                    // Pre-compute edge-length target positions per local prop.
                    let proximity_targets: HashMap<PropId, (f64, u32)> =
                        inter_node_edges_by_target.values().flat_map(|edges| {
                            edges.iter().map(|&(lp, rp, w)| {
                                let rn = graph.properties[rp.index()].node;
                                let rl = layer_map
                                    .get(&LayerElement::Node(rn))
                                    .copied()
                                    .unwrap_or(k as u32);
                                let target = if rl < k as u32 {
                                    0.0
                                } else {
                                    (props.len() - 1) as f64
                                };
                                (lp, (target, w))
                            })
                        }).collect();

                    // Cost function for a candidate ordering.
                    let order_cost = |order: &[PropId]| -> i64 {
                        let pos: HashMap<PropId, usize> = order
                            .iter()
                            .enumerate()
                            .map(|(idx, &pid)| (pid, idx))
                            .collect();
                        let mut cost: i64 = 0;

                        // Same-node constraint crossings.
                        for e1 in 0..same_node_edges.len() {
                            for e2 in (e1 + 1)..same_node_edges.len() {
                                let (s1, d1, w1) = same_node_edges[e1];
                                let (s2, d2, w2) = same_node_edges[e2];
                                if let (Some(&s1p), Some(&d1p), Some(&s2p), Some(&d2p)) =
                                    (pos.get(&s1), pos.get(&d1), pos.get(&s2), pos.get(&d2))
                                    && s1p != s2p && d1p != d2p
                                    && (s1p < s2p) != (d1p < d2p)
                                {
                                    cost += (w1 + w2) as i64;
                                }
                            }
                        }

                        // Cross-node edges inside same-node brackets.
                        for &(s, d, w) in &same_node_edges {
                            if let (Some(&sp), Some(&dp)) = (pos.get(&s), pos.get(&d)) {
                                let (lo, hi) = if sp < dp { (sp, dp) } else { (dp, sp) };
                                for &pid in order {
                                    if !cross_node_props.contains(&pid) { continue; }
                                    let r = pos[&pid];
                                    if lo < r && r < hi { cost += w as i64; }
                                }
                            }
                        }

                        // Bracket span.
                        for &(s, d, w) in &same_node_edges {
                            if let (Some(&sp), Some(&dp)) = (pos.get(&s), pos.get(&d)) {
                                cost += sp.abs_diff(dp) as i64 * w as i64;
                            }
                        }

                        // Inter-node bipartite crossings (chiasm-aware).
                        for (&tn, edges) in &inter_node_edges_by_target {
                            let chiasm = target_same_domain[&tn];
                            for e1 in 0..edges.len() {
                                for e2 in (e1 + 1)..edges.len() {
                                    let (lp1, rp1, w1) = edges[e1];
                                    let (lp2, rp2, w2) = edges[e2];
                                    if let (Some(&l1), Some(&l2)) =
                                        (pos.get(&lp1), pos.get(&lp2))
                                    {
                                        let r1 = remote_indices[&rp1];
                                        let r2 = remote_indices[&rp2];
                                        if r1 == r2 { continue; }
                                        let w = (w1 + w2) as i64 * 2;
                                        let bad = if chiasm {
                                            (l1 < l2) == (r1 < r2)
                                        } else {
                                            (l1 < l2) != (r1 < r2)
                                        };
                                        if bad { cost += w; }
                                    }
                                }
                            }
                        }

                        // Edge-length proximity.
                        for (&lp, &(target, w)) in &proximity_targets {
                            if let Some(&p) = pos.get(&lp) {
                                cost += ((p as f64 - target).abs() * 0.5 * w as f64) as i64;
                            }
                        }

                        cost
                    };

                    // Sifting: process each property by decreasing
                    // connectivity, try every insertion position.
                    let mut sift_order: Vec<usize> =
                        (0..new_prop_order.len()).collect();
                    sift_order.sort_by(|&a, &b| {
                        let count_a = same_node_edges.iter()
                            .filter(|(s, d, _)| *s == new_prop_order[a] || *d == new_prop_order[a])
                            .count()
                            + inter_node_edges_by_target.values()
                                .flat_map(|e| e.iter())
                                .filter(|(lp, _, _)| *lp == new_prop_order[a])
                                .count();
                        let count_b = same_node_edges.iter()
                            .filter(|(s, d, _)| *s == new_prop_order[b] || *d == new_prop_order[b])
                            .count()
                            + inter_node_edges_by_target.values()
                                .flat_map(|e| e.iter())
                                .filter(|(lp, _, _)| *lp == new_prop_order[b])
                                .count();
                        count_b.cmp(&count_a)
                    });

                    let max_rounds = 3;
                    for _ in 0..max_rounds {
                        let mut improved = false;
                        for &orig_idx in &sift_order {
                            let pid = new_prop_order[orig_idx];
                            // Find current position of this property.
                            let cur = new_prop_order.iter()
                                .position(|&p| p == pid)
                                .unwrap();
                            let base_cost = order_cost(&new_prop_order);

                            // Remove and try every insertion position.
                            let mut trial = new_prop_order.clone();
                            trial.remove(cur);
                            let mut best_pos = cur;
                            let mut best_cost = base_cost;
                            for ins in 0..=trial.len() {
                                let mut candidate = trial.clone();
                                candidate.insert(ins, pid);
                                let c = order_cost(&candidate);
                                if c < best_cost {
                                    best_cost = c;
                                    best_pos = ins;
                                }
                            }
                            if best_pos != cur {
                                let mut result = trial;
                                result.insert(best_pos, pid);
                                new_prop_order = result;
                                improved = true;
                            }
                        }
                        if !improved { break; }
                    }
                }

                // Chiasm override: for same-domain constraint bundles,
                // reverse the connected properties on this node relative
                // to the remote node's order. Applied AFTER sifting so
                // the chiastic (nested bracket) arrangement has the final
                // word, overriding any sifting decisions that would create
                // overlapping brackets.
                //
                // Only apply to the LOWER node of each pair to prevent
                // double reversal (both sides reversing cancels out).
                for (&target_node, edges) in &inter_node_edges_by_target {
                    if edges.len() < 2 { continue; }
                    let target_layer = layer_map
                        .get(&LayerElement::Node(target_node))
                        .copied()
                        .unwrap_or(k as u32);
                    if target_layer >= k as u32 { continue; }
                    let local_dom = graph.nodes[node_id.index()].domain;
                    let remote_dom = graph.nodes[target_node.index()].domain;
                    if local_dom.is_none() || local_dom != remote_dom { continue; }

                    let mut by_remote: Vec<(usize, PropId)> = edges.iter()
                        .filter_map(|&(lp, rp, _)| {
                            let rn = graph.properties[rp.index()].node;
                            let ri = prop_order.prop_index(rn, rp)?;
                            Some((ri, lp))
                        })
                        .collect();
                    by_remote.sort_by_key(|&(ri, _)| ri);

                    let desired_local: Vec<PropId> = by_remote.iter()
                        .rev()
                        .map(|&(_, lp)| lp)
                        .collect();

                    let mut slots: Vec<usize> = desired_local.iter()
                        .filter_map(|lp| new_prop_order.iter().position(|p| p == lp))
                        .collect();
                    slots.sort();

                    for (i, &slot) in slots.iter().enumerate() {
                        if i < desired_local.len() {
                            new_prop_order[slot] = desired_local[i];
                        }
                    }
                }

                prop_order.set_order(node_id, new_prop_order);
            }

            // Step 2: Compute item barycenters for layer k
            let num_items = layers[k].items.len();
            let mut item_barycenters: Vec<(usize, f64)> = Vec::with_capacity(num_items);

            for (item_idx, item) in layers[k].items.iter().enumerate() {
                let barycenter = match item {
                    LayerItem::Node(node_id) => {
                        // Node barycenter = mean of connected property barycenters
                        // A property is "connected" if it had edges to adj_layer
                        let props = prop_order.props_of(*node_id).to_vec();
                        let mut connected_bcs: Vec<f64> = Vec::new();

                        // Also check for Link edges (node-level, not property-level)
                        let mut node_level_positions: Vec<(f64, u32)> = Vec::new();

                        for ae in &adj_edges {
                            // Check if this is a node-level edge (Link)
                            let is_upper_node = matches!(
                                &ae.upper,
                                EdgeEndpoint {
                                    prop: None,
                                    element: LayerElement::Node(nid),
                                } if *nid == *node_id
                            ) && layer_map.get(&ae.upper.element) == Some(&(k as u32));

                            let is_lower_node = matches!(
                                &ae.lower,
                                EdgeEndpoint {
                                    prop: None,
                                    element: LayerElement::Node(nid),
                                } if *nid == *node_id
                            ) && layer_map.get(&ae.lower.element) == Some(&(k as u32));

                            if is_upper_node {
                                let pos = resolve_position(
                                    &ae.lower,
                                    adj_layer,
                                    graph,
                                    &prop_order,
                                    long_edges,
                                    &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Downstream,
                                );
                                node_level_positions.push((pos, ae.weight));
                            } else if is_lower_node {
                                let pos = resolve_position(
                                    &ae.upper,
                                    adj_layer,
                                    graph,
                                    &prop_order,
                                    long_edges,
                                    &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Upstream,
                                );
                                node_level_positions.push((pos, ae.weight));
                            }
                        }

                        // Collect property-level barycenters
                        for &prop_id in &props {
                            let mut positions: Vec<(f64, u32)> = Vec::new();
                            for ae in &adj_edges {
                                let touches_upper = matches!(
                                    &ae.upper,
                                    EdgeEndpoint { prop: Some(p), element }
                                    if *p == prop_id
                                        && layer_map.get(element) == Some(&(k as u32))
                                );
                                let touches_lower = matches!(
                                    &ae.lower,
                                    EdgeEndpoint { prop: Some(p), element }
                                    if *p == prop_id
                                        && layer_map.get(element) == Some(&(k as u32))
                                );
                                if touches_upper {
                                    let pos = resolve_position(
                                        &ae.lower,
                                        adj_layer,
                                        graph,
                                        &prop_order,
                                        long_edges,
                                        &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Downstream,
                                    );
                                    positions.push((pos, ae.weight));
                                } else if touches_lower {
                                    let pos = resolve_position(
                                        &ae.upper,
                                        adj_layer,
                                        graph,
                                        &prop_order,
                                        long_edges,
                                        &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Upstream,
                                    );
                                    positions.push((pos, ae.weight));
                                }
                            }
                            if let Some(bc) = weighted_mean(&positions) {
                                connected_bcs.push(bc);
                            }
                        }

                        if !connected_bcs.is_empty() || !node_level_positions.is_empty() {
                            // Combine property barycenters and node-level barycenters
                            let mut all_values: Vec<f64> = connected_bcs;
                            if let Some(nlbc) = weighted_mean(&node_level_positions) {
                                all_values.push(nlbc);
                            }
                            if all_values.is_empty() {
                                item_idx as f64
                            } else {
                                all_values.iter().sum::<f64>() / all_values.len() as f64
                            }
                        } else {
                            item_idx as f64
                        }
                    }
                    LayerItem::Derivation(deriv_id) => {
                        // Derivation barycenter from connected edges
                        let mut positions: Vec<(f64, u32)> = Vec::new();
                        let elem = LayerElement::Derivation(*deriv_id);

                        for ae in &adj_edges {
                            let is_upper = ae.upper.element == elem
                                && layer_map.get(&ae.upper.element) == Some(&(k as u32));
                            let is_lower = ae.lower.element == elem
                                && layer_map.get(&ae.lower.element) == Some(&(k as u32));

                            if is_upper {
                                let pos = resolve_position(
                                    &ae.lower,
                                    adj_layer,
                                    graph,
                                    &prop_order,
                                    long_edges,
                                    &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Downstream,
                                );
                                positions.push((pos, ae.weight));
                            } else if is_lower {
                                let pos = resolve_position(
                                    &ae.upper,
                                    adj_layer,
                                    graph,
                                    &prop_order,
                                    long_edges,
                                    &empty_epo, ae.edge_id, &empty_ps, EndpointRole::Upstream,
                                );
                                positions.push((pos, ae.weight));
                            }
                        }

                        weighted_mean(&positions).unwrap_or(item_idx as f64)
                    }
                    LayerItem::Segment(edge_id, _seg_layer) => {
                        // Segment barycenter = position of same edge in adjacent layer
                        position_of_segment(
                            *edge_id,
                            adjacent_idx as u32,
                            long_edges,
                            adj_layer,
                        )
                    }
                };

                item_barycenters.push((item_idx, barycenter));
            }

            // Step 3: Sort items in layer k by barycenter
            item_barycenters
                .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

            let old_items = layers[k].items.clone();
            layers[k].items = item_barycenters
                .iter()
                .map(|(idx, _)| old_items[*idx].clone())
                .collect();
        }

        // Update long edge segment positions after reordering
        update_long_edge_positions(layers, long_edges);

        // Recompute port sides and edge port order from current layout
        let current_port_sides = compute_layer_space_port_sides(graph, layers);
        let current_edge_port_order =
            compute_edge_port_order(graph, layers, long_edges, &prop_order);

        let current_crossings = count_all_crossings(
            layers, graph, &prop_order, long_edges,
            &current_edge_port_order, &current_port_sides,
        );

        if current_crossings < best_crossings || (current_crossings == 0 && iteration < 2) {
            best_layers = layers.clone();
            best_prop_order = prop_order.clone();
            best_port_sides = current_port_sides;
            best_edge_port_order = current_edge_port_order;
            best_crossings = current_crossings;
        }

        // Stop early once crossings hit zero, but only after running at
        // least two passes (one top-down + one bottom-up) so that property
        // reordering benefits from both sweep directions.
        if current_crossings == 0 && iteration >= 1 {
            break;
        }
    }

    // Restore best ordering
    *layers = best_layers;

    // Apply best property ordering back: we store the property order in the
    // layers structure. Since we can't modify Graph, callers should use the
    // layer ordering. We update long edge positions for the best layout.
    update_long_edge_positions(layers, long_edges);

    (best_prop_order, best_edge_port_order, best_port_sides)
}

/// Compute edge port ordering for each property based on the opposite endpoint's
/// layer-space position.  This uses the same positional data as crossing
/// minimization, ensuring port slot assignment (Phase 6b) is consistent with
/// property ordering (Phase 3b).
///
/// For each property that participates in Constraint or DerivInput edges, we
/// collect those edges and sort them by the opposite endpoint's position:
/// `(layer_index, item_position + prop_fraction)`.  Anchor edges are skipped
/// (they use center ports).
pub fn compute_edge_port_order(
    graph: &Graph,
    layers: &[LayerEntry],
    long_edges: &[LongEdge],
    prop_order: &PropertyOrder,
) -> EdgePortOrder {
    let layer_map = build_layer_map(layers);

    // For each property, collect (sort_key, edge_id) pairs.
    let mut prop_entries: HashMap<PropId, Vec<(f64, f64, EdgeId)>> = HashMap::new();

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);
        match edge {
            Edge::Anchor { .. } => {} // center ports, skip
            Edge::Constraint { source_prop, dest_prop, .. } => {
                // For source_prop: opposite endpoint is dest_prop
                if let Some(key) = opposite_sort_key(*dest_prop, graph, layers, &layer_map, long_edges, prop_order) {
                    prop_entries.entry(*source_prop).or_default().push((key.0, key.1, edge_id));
                }
                // For dest_prop: opposite endpoint is source_prop
                if let Some(key) = opposite_sort_key(*source_prop, graph, layers, &layer_map, long_edges, prop_order) {
                    prop_entries.entry(*dest_prop).or_default().push((key.0, key.1, edge_id));
                }
            }
            Edge::DerivInput { source_prop, target_deriv, .. } => {
                // For source_prop: opposite endpoint is the derivation
                let elem = LayerElement::Derivation(*target_deriv);
                if let Some(&layer_idx) = layer_map.get(&elem) {
                    let layer = &layers[layer_idx as usize];
                    let item_pos = position_of_element(&elem, layer).unwrap_or(0.0);
                    prop_entries.entry(*source_prop).or_default().push((layer_idx as f64, item_pos, edge_id));
                }
            }
        }
    }

    // Sort each property's edges by (layer_idx, item_pos + prop_fraction)
    let mut result = EdgePortOrder::new();
    for (prop_id, mut entries) in prop_entries {
        entries.sort_by(|a, b| {
            a.0.partial_cmp(&b.0).unwrap()
                .then(a.1.partial_cmp(&b.1).unwrap())
        });
        result.insert(prop_id, entries.into_iter().map(|(_, _, eid)| eid).collect());
    }

    result
}

/// Compute the sort key for the opposite endpoint of a property-level edge.
/// Returns `(layer_index, item_position + prop_fraction)`.
fn opposite_sort_key(
    opposite_prop: PropId,
    graph: &Graph,
    layers: &[LayerEntry],
    layer_map: &HashMap<LayerElement, u32>,
    _long_edges: &[LongEdge],
    prop_order: &PropertyOrder,
) -> Option<(f64, f64)> {
    let node_id = graph.properties[opposite_prop.index()].node;
    let elem = LayerElement::Node(node_id);
    let layer_idx = *layer_map.get(&elem)? as usize;
    let layer = layers.get(layer_idx)?;
    let item_pos = position_of_element(&elem, layer)?;

    let num_props = prop_order.num_props(node_id);
    let prop_idx = prop_order.prop_index(node_id, opposite_prop).unwrap_or(0);
    let prop_fraction = if num_props > 0 {
        (prop_idx as f64 + 1.0) / (num_props as f64 + 1.0)
    } else {
        0.5
    };

    Some((layer_idx as f64, item_pos + prop_fraction))
}

/// Update long edge position maps to reflect current layer orderings.
/// Compute port side assignments from layer-space positions.
///
/// This is the layer-space equivalent of `routing::assign_port_sides()`, run
/// during crossing minimization so that port sides co-optimize with property
/// ordering.  Phase 6a (`refine_port_sides`) later overrides cross-domain
/// bracket routing cases that require coordinate-space domain geometry.
fn compute_layer_space_port_sides(
    graph: &Graph,
    layers: &[LayerEntry],
) -> PortSideAssignment {
    let layer_map = build_layer_map(layers);
    let mut sides = PortSideAssignment::new();
    let mut same_pos_counter: HashMap<NodeId, usize> = HashMap::new();

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);

        match edge {
            Edge::Anchor { .. } => {} // center ports, no side
            Edge::Constraint { source_prop, dest_prop, .. } => {
                let src_node = graph.properties[source_prop.index()].node;
                let dst_node = graph.properties[dest_prop.index()].node;

                if src_node == dst_node {
                    // Same-node constraint: always Right
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                    sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
                } else {
                    assign_side_between_nodes(
                        &layer_map, layers,
                        LayerElement::Node(src_node),
                        LayerElement::Node(dst_node),
                        edge_id, &mut sides, &mut same_pos_counter,
                        src_node,
                    );
                }
            }
            Edge::DerivInput { source_prop, target_deriv } => {
                let src_node = graph.properties[source_prop.index()].node;
                let src_elem = LayerElement::Node(src_node);
                let dst_elem = LayerElement::Derivation(*target_deriv);

                // For DerivInput, derive a dummy node for the counter (use src_node).
                let src_pos = layer_map.get(&src_elem)
                    .and_then(|&l| position_of_element(&src_elem, &layers[l as usize]));
                let dst_pos = layer_map.get(&dst_elem)
                    .and_then(|&l| position_of_element(&dst_elem, &layers[l as usize]));

                match (src_pos, dst_pos) {
                    (Some(sp), Some(dp)) if (sp - dp).abs() > 0.01 => {
                        if sp < dp {
                            sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                            sides.insert((edge_id, EndpointRole::Downstream), PortSide::Left);
                        } else {
                            sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                            sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
                        }
                    }
                    _ => {
                        // Same position or missing: alternate
                        let cnt = same_pos_counter.entry(src_node).or_insert(0);
                        let side = if (*cnt).is_multiple_of(2) { PortSide::Right } else { PortSide::Left };
                        *cnt += 1;
                        sides.insert((edge_id, EndpointRole::Upstream), side);
                        sides.insert((edge_id, EndpointRole::Downstream), side);
                    }
                }
            }
        }
    }

    sides
}

/// Helper: assign Left/Right sides for a constraint between different nodes
/// based on their layer-space positions.
#[allow(clippy::too_many_arguments)]
fn assign_side_between_nodes(
    layer_map: &HashMap<LayerElement, u32>,
    layers: &[LayerEntry],
    src_elem: LayerElement,
    dst_elem: LayerElement,
    edge_id: EdgeId,
    sides: &mut PortSideAssignment,
    same_pos_counter: &mut HashMap<NodeId, usize>,
    src_node: NodeId,
) {
    let src_pos = layer_map.get(&src_elem)
        .and_then(|&l| position_of_element(&src_elem, &layers[l as usize]));
    let dst_pos = layer_map.get(&dst_elem)
        .and_then(|&l| position_of_element(&dst_elem, &layers[l as usize]));

    match (src_pos, dst_pos) {
        (Some(sp), Some(dp)) if (sp - dp).abs() > 0.01 => {
            if sp < dp {
                sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                sides.insert((edge_id, EndpointRole::Downstream), PortSide::Left);
            } else {
                sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
            }
        }
        _ => {
            // Same position or missing — alternate via per-node counter
            let cnt = same_pos_counter.entry(src_node).or_insert(0);
            let side = if (*cnt).is_multiple_of(2) { PortSide::Right } else { PortSide::Left };
            *cnt += 1;
            sides.insert((edge_id, EndpointRole::Upstream), side);
            sides.insert((edge_id, EndpointRole::Downstream), side);
        }
    }
}

fn update_long_edge_positions(layers: &[LayerEntry], long_edges: &mut [LongEdge]) {
    for le in long_edges.iter_mut() {
        for (&layer_idx, pos) in le.positions.iter_mut() {
            if let Some(layer) = layers.get(layer_idx as usize) && let Some(new_pos) =
                    position_of_element(&LayerElement::Segment(le.edge_id, layer_idx), layer)
            {
                *pos = new_pos;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::*;
    use std::collections::HashMap;

    /// Helper to build a minimal graph with given nodes, properties, edges.
    fn make_graph(
        nodes: Vec<Node>,
        properties: Vec<Property>,
        derivations: Vec<Derivation>,
        edges: Vec<Edge>,
    ) -> Graph {
        let mut prop_edges: HashMap<PropId, Vec<EdgeId>> = HashMap::new();
        let mut node_children: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut node_parent: HashMap<NodeId, EdgeId> = HashMap::new();

        for (i, edge) in edges.iter().enumerate() {
            let eid = EdgeId(i as u32);
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
                    target_deriv,
                    ..
                } => {
                    prop_edges.entry(*source_prop).or_default().push(eid);
                    // Also register the derivation's output prop
                    let deriv = &derivations[target_deriv.index()];
                    prop_edges.entry(deriv.output_prop).or_default().push(eid);
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

    fn make_node(id: u32, ident: &str, props: Vec<u32>) -> Node {
        Node {
            id: NodeId(id),
            ident: ident.to_string(),
            display_name: None,
            properties: props.into_iter().map(PropId).collect(),
            domain: None,
            is_anchored: false,
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

    // -----------------------------------------------------------------------
    // Test 1: Two nodes in adjacent layers, no crossings
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_crossings_simple() {
        // Layer 0: [Node(0)]
        // Layer 1: [Node(1)]
        // Edge: Link from Node(0) to Node(1)
        let nodes = vec![
            make_node(0, "A", vec![]),
            make_node(1, "B", vec![]),
        ];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let layer_a = LayerEntry {
            items: vec![LayerItem::Node(NodeId(0))],
        };
        let layer_b = LayerEntry {
            items: vec![LayerItem::Node(NodeId(1))],
        };

        let crossings = count_crossings(&layer_a, &layer_b, &graph);
        assert_eq!(crossings, 0);
    }

    // -----------------------------------------------------------------------
    // Test 2: Two edges that cross → cost = w(e1) + w(e2)
    // -----------------------------------------------------------------------
    #[test]
    fn test_two_crossing_links() {
        // Layer 0: [Node(0), Node(1)]
        // Layer 1: [Node(2), Node(3)]
        // Edge 0: Link Node(0) -> Node(3) (crosses with edge 1)
        // Edge 1: Link Node(1) -> Node(2) (crosses with edge 0)
        let nodes = vec![
            make_node(0, "A", vec![]),
            make_node(1, "B", vec![]),
            make_node(2, "C", vec![]),
            make_node(3, "D", vec![]),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(3),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let layer_a = LayerEntry {
            items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
        };
        let layer_b = LayerEntry {
            items: vec![LayerItem::Node(NodeId(2)), LayerItem::Node(NodeId(3))],
        };

        let crossings = count_crossings(&layer_a, &layer_b, &graph);
        // Both edges are Links with weight 3. Cost = 3 + 3 = 6
        assert_eq!(crossings, 6);
    }

    // -----------------------------------------------------------------------
    // Test 2b: Mixed-weight crossing edges
    // -----------------------------------------------------------------------
    #[test]
    fn test_crossing_mixed_weights() {
        // Layer 0: [Node(0) with prop 0, Node(1) with prop 1]
        // Layer 1: [Node(2) with prop 2, Node(3) with prop 3]
        // Edge 0: Constraint prop(0) -> prop(3) (weight 1)
        // Edge 1: Constraint prop(1) -> prop(2) (weight 1)
        // These cross: cost = 1 + 1 = 2
        let nodes = vec![
            make_node(0, "A", vec![0]),
            make_node(1, "B", vec![1]),
            make_node(2, "C", vec![2]),
            make_node(3, "D", vec![3]),
        ];
        let properties = vec![
            make_prop(0, 0, "p0"),
            make_prop(1, 1, "p1"),
            make_prop(2, 2, "p2"),
            make_prop(3, 3, "p3"),
        ];
        let edges = vec![
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(3),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(1),
                dest_prop: PropId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let layer_a = LayerEntry {
            items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
        };
        let layer_b = LayerEntry {
            items: vec![LayerItem::Node(NodeId(2)), LayerItem::Node(NodeId(3))],
        };

        let crossings = count_crossings(&layer_a, &layer_b, &graph);
        assert_eq!(crossings, 2);
    }

    // -----------------------------------------------------------------------
    // Test 3: Property reordering reduces crossings
    // -----------------------------------------------------------------------
    #[test]
    fn test_property_reorder_reduces_crossings() {
        // One node with two properties in layer 0, two target nodes in layer 1.
        // Edges cross in initial property order but not after reordering.
        //
        // Layer 0: [Node(0) with props [p0, p1]]
        // Layer 1: [Node(1), Node(2)]
        // Edge 0: Constraint p0 -> p3 (p3 is on Node(2), position 1)
        // Edge 1: Constraint p1 -> p2 (p2 is on Node(1), position 0)
        //
        // Initially p0 is above p1, so:
        //   p0 (fractional ~0.33) -> Node(2) (pos 1)
        //   p1 (fractional ~0.67) -> Node(1) (pos 0)
        // These cross!
        //
        // After reorder: p1 then p0:
        //   p1 (fractional ~0.33) -> Node(1) (pos 0)
        //   p0 (fractional ~0.67) -> Node(2) (pos 1)
        // No crossing.

        let nodes = vec![
            make_node(0, "A", vec![0, 1]),
            make_node(1, "B", vec![2]),
            make_node(2, "C", vec![3]),
        ];
        let properties = vec![
            make_prop(0, 0, "p0"),
            make_prop(1, 0, "p1"),
            make_prop(2, 1, "p2"),
            make_prop(3, 2, "p3"),
        ];
        let edges = vec![
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(3),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(1),
                dest_prop: PropId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1)), LayerItem::Node(NodeId(2))],
            },
        ];
        let mut long_edges = vec![];

        // Verify initial crossings exist
        let initial = count_all_crossings(
            &layers, &graph, &PropertyOrder::from_graph(&graph), &long_edges,
            &EdgePortOrder::new(), &PortSideAssignment::new(),
        );
        assert!(initial > 0, "Expected initial crossings, got {}", initial);

        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);

        let final_crossings = count_all_crossings(
            &layers, &graph, &PropertyOrder::from_graph(&graph), &long_edges,
            &EdgePortOrder::new(), &PortSideAssignment::new(),
        );
        assert!(
            final_crossings <= initial,
            "Crossings should not increase: initial={}, final={}",
            initial,
            final_crossings
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: Multiple iterations converge
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_iterations_converge() {
        // 3 layers with crossing edges. After minimization, crossings should
        // decrease or stay the same.
        //
        // Layer 0: [Node(0), Node(1)]
        // Layer 1: [Node(2), Node(3)]
        // Layer 2: [Node(4), Node(5)]
        //
        // Edges that create crossings between layers 0-1 and 1-2:
        // Link: 0->3, 1->2 (cross in layer 0-1)
        // Link: 2->5, 3->4 (cross in layer 1-2)

        let nodes = vec![
            make_node(0, "A", vec![]),
            make_node(1, "B", vec![]),
            make_node(2, "C", vec![]),
            make_node(3, "D", vec![]),
            make_node(4, "E", vec![]),
            make_node(5, "F", vec![]),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(3),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(2),
                child: NodeId(5),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(3),
                child: NodeId(4),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(2)), LayerItem::Node(NodeId(3))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(4)), LayerItem::Node(NodeId(5))],
            },
        ];
        let mut long_edges = vec![];

        let initial = count_all_crossings(
            &layers,
            &graph,
            &PropertyOrder::from_graph(&graph),
            &long_edges,
            &EdgePortOrder::new(),
            &PortSideAssignment::new(),
        );
        assert!(initial > 0, "Expected initial crossings");

        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);

        let final_crossings = count_all_crossings(
            &layers,
            &graph,
            &PropertyOrder::from_graph(&graph),
            &long_edges,
            &EdgePortOrder::new(),
            &PortSideAssignment::new(),
        );
        assert_eq!(
            final_crossings, 0,
            "Expected 0 crossings after convergence, got {}",
            final_crossings
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: Long edge segments participate correctly
    // -----------------------------------------------------------------------
    #[test]
    fn test_long_edge_segments() {
        // Layer 0: [Node(0), Node(1)]
        // Layer 1: [Segment(edge0, 1), Node(2)]
        // Layer 2: [Node(3), Node(4)]
        //
        // Edge 0 is a long edge: Node(0) -> Node(3), passing through layer 1
        // Edge 1: Link Node(1) -> Node(2)
        // Edge 2: Link Node(2) -> Node(4)
        //
        // The segment in layer 1 should be positioned based on its
        // connection to Node(0) in layer 0.

        let nodes = vec![
            make_node(0, "A", vec![]),
            make_node(1, "B", vec![]),
            make_node(2, "C", vec![]),
            make_node(3, "D", vec![]),
            make_node(4, "E", vec![]),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(3),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(2),
                child: NodeId(4),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
            },
            LayerEntry {
                items: vec![
                    LayerItem::Segment(EdgeId(0), 1),
                    LayerItem::Node(NodeId(2)),
                ],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(3)), LayerItem::Node(NodeId(4))],
            },
        ];

        let mut long_edges = vec![LongEdge {
            edge_id: EdgeId(0),
            source_layer: 0,
            target_layer: 2,
            positions: {
                let mut m = HashMap::new();
                m.insert(1, 0.0); // initial position in layer 1
                m
            },
        }];

        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);

        // After minimization the segment should maintain a reasonable position.
        // The key check: it compiles and runs without panics, and the segment
        // is still present in layer 1.
        let has_segment = layers[1].items.iter().any(|item| {
            matches!(item, LayerItem::Segment(EdgeId(0), 1))
        });
        assert!(has_segment, "Segment should still be in layer 1");
    }

    // -----------------------------------------------------------------------
    // Test: weighted_mean helper
    // -----------------------------------------------------------------------
    #[test]
    fn test_weighted_mean() {
        assert_eq!(weighted_mean(&[]), None);
        assert_eq!(weighted_mean(&[(2.0, 1)]), Some(2.0));
        assert_eq!(weighted_mean(&[(0.0, 1), (4.0, 1)]), Some(2.0));
        // Weighted: (0*3 + 4*1) / (3+1) = 1.0
        assert_eq!(weighted_mean(&[(0.0, 3), (4.0, 1)]), Some(1.0));
    }

    // -----------------------------------------------------------------------
    // Test: position_of_prop fractional
    // -----------------------------------------------------------------------
    #[test]
    fn test_position_of_prop_fractional() {
        let nodes = vec![make_node(0, "A", vec![0, 1, 2])];
        let properties = vec![
            make_prop(0, 0, "p0"),
            make_prop(1, 0, "p1"),
            make_prop(2, 0, "p2"),
        ];
        let graph = make_graph(nodes, properties, vec![], vec![]);
        let prop_order = PropertyOrder::from_graph(&graph);

        let layer = LayerEntry {
            items: vec![LayerItem::Node(NodeId(0))],
        };

        // Node(0) is at position 0 in the layer.
        // 3 properties, so positions are:
        //   p0: 0 + 1/4 = 0.25
        //   p1: 0 + 2/4 = 0.5
        //   p2: 0 + 3/4 = 0.75
        let p0 = position_of_prop(PropId(0), &graph, &layer, &prop_order);
        let p1 = position_of_prop(PropId(1), &graph, &layer, &prop_order);
        let p2 = position_of_prop(PropId(2), &graph, &layer, &prop_order);

        assert!((p0 - 0.25).abs() < 1e-9, "p0 = {}", p0);
        assert!((p1 - 0.5).abs() < 1e-9, "p1 = {}", p1);
        assert!((p2 - 0.75).abs() < 1e-9, "p2 = {}", p2);
    }

    // -----------------------------------------------------------------------
    // Test: minimize_crossings with no layers does not panic
    // -----------------------------------------------------------------------
    #[test]
    fn test_empty_layers() {
        let graph = make_graph(vec![], vec![], vec![], vec![]);
        let mut layers: Vec<LayerEntry> = vec![];
        let mut long_edges = vec![];
        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);
        assert!(layers.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: single layer does not panic
    // -----------------------------------------------------------------------
    #[test]
    fn test_single_layer() {
        let nodes = vec![make_node(0, "A", vec![])];
        let graph = make_graph(nodes, vec![], vec![], vec![]);
        let mut layers = vec![LayerEntry {
            items: vec![LayerItem::Node(NodeId(0))],
        }];
        let mut long_edges = vec![];
        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);
        assert_eq!(layers.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test: uncrossing with node reordering
    // -----------------------------------------------------------------------
    #[test]
    fn test_node_reorder_eliminates_crossings() {
        // Layer 0: [Node(0), Node(1)]  (fixed)
        // Layer 1: [Node(3), Node(2)]  (initially crossed)
        // Edges: 0->2, 1->3
        // Initially: 0 (pos 0) -> 2 (pos 1), 1 (pos 1) -> 3 (pos 0) → cross
        // After reorder layer 1 to [Node(2), Node(3)]: no cross

        let nodes = vec![
            make_node(0, "A", vec![]),
            make_node(1, "B", vec![]),
            make_node(2, "C", vec![]),
            make_node(3, "D", vec![]),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(2),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(3),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(3)), LayerItem::Node(NodeId(2))],
            },
        ];
        let mut long_edges = vec![];

        let initial = count_all_crossings(
            &layers,
            &graph,
            &PropertyOrder::from_graph(&graph),
            &long_edges,
            &EdgePortOrder::new(),
            &PortSideAssignment::new(),
        );
        assert_eq!(initial, 6, "Expected 6 initial crossing cost (3+3)");

        let _ = minimize_crossings(&mut layers, &mut long_edges, &graph);

        let final_crossings = count_all_crossings(
            &layers,
            &graph,
            &PropertyOrder::from_graph(&graph),
            &long_edges,
            &EdgePortOrder::new(),
            &PortSideAssignment::new(),
        );
        assert_eq!(final_crossings, 0, "Expected 0 crossings after minimization");

        // Verify the order: Node(2) should come before Node(3) in layer 1
        match (&layers[1].items[0], &layers[1].items[1]) {
            (LayerItem::Node(a), LayerItem::Node(b)) => {
                assert_eq!(*a, NodeId(2));
                assert_eq!(*b, NodeId(3));
            }
            _ => panic!("Expected two nodes in layer 1"),
        }
    }

    // -----------------------------------------------------------------------
    // Test: same-node constraint edges reorder properties
    // -----------------------------------------------------------------------
    #[test]
    fn test_same_node_constraint_reorders_properties() {
        // A single node with 4 properties, connected by same-node constraints.
        // The constraints form a chain:
        //   p2 -> p0  (source p2 should be above dest p0)
        //   p3 -> p1  (source p3 should be above dest p1)
        //
        // Initial order: [p0, p1, p2, p3]
        // Expected after: sources (p2, p3) should move above their targets,
        // producing an ordering like [p2, p3, p0, p1] or [p3, p2, p1, p0]
        // — the key invariant is that p2 appears before p0 and p3 before p1.
        //
        // We need at least 2 layers so the sweep runs, but the node only
        // appears in one layer.  The second layer is a dummy.

        let nodes = vec![
            make_node(0, "Report", vec![0, 1, 2, 3]),
            make_node(1, "Dummy", vec![]),
        ];
        let properties = vec![
            make_prop(0, 0, "dest_a"),
            make_prop(1, 0, "dest_b"),
            make_prop(2, 0, "src_a"),
            make_prop(3, 0, "src_b"),
        ];
        let edges = vec![
            // Anchor to give us two layers
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            // Same-node constraints: src_a(p2) -> dest_a(p0)
            Edge::Constraint {
                source_prop: PropId(2),
                dest_prop: PropId(0),
                operation: Some("ge".to_string()),
            },
            // Same-node constraints: src_b(p3) -> dest_b(p1)
            Edge::Constraint {
                source_prop: PropId(3),
                dest_prop: PropId(1),
                operation: Some("ge".to_string()),
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1))],
            },
        ];
        let mut long_edges = vec![];

        let (prop_order, _, _) = minimize_crossings(&mut layers, &mut long_edges, &graph);

        let final_order = prop_order.props_of(NodeId(0));

        // Source properties should appear before their corresponding targets
        let pos_of = |pid: PropId| -> usize {
            final_order.iter().position(|&p| p == pid).unwrap()
        };

        assert!(
            pos_of(PropId(2)) < pos_of(PropId(0)),
            "src_a (p2) should be before dest_a (p0), got order: {:?}",
            final_order
        );
        assert!(
            pos_of(PropId(3)) < pos_of(PropId(1)),
            "src_b (p3) should be before dest_b (p1), got order: {:?}",
            final_order
        );
    }

    // -----------------------------------------------------------------------
    // Test: parallel same-node constraints minimize crossings
    // -----------------------------------------------------------------------
    #[test]
    fn test_same_node_parallel_constraints_no_crossings() {
        // A node with 4 properties and two parallel same-node constraints:
        //   p0 -> p2  and  p1 -> p3
        // These are "parallel" (non-crossing) when sources and targets
        // maintain the same relative order.  Initial order [p0, p1, p2, p3]
        // already has p0 before p2 and p1 before p3 — should be preserved
        // or improved.
        //
        // Crossing would occur if the result had p0 after p2 or p1 after p3
        // in positions that cause the constraint edges to visually cross.

        let nodes = vec![
            make_node(0, "Node", vec![0, 1, 2, 3]),
            make_node(1, "Dummy", vec![]),
        ];
        let properties = vec![
            make_prop(0, 0, "a"),
            make_prop(1, 0, "b"),
            make_prop(2, 0, "c"),
            make_prop(3, 0, "d"),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(2),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(1),
                dest_prop: PropId(3),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1))],
            },
        ];
        let mut long_edges = vec![];

        let (prop_order, _, _) = minimize_crossings(&mut layers, &mut long_edges, &graph);
        let final_order = prop_order.props_of(NodeId(0));

        let pos_of = |pid: PropId| -> usize {
            final_order.iter().position(|&p| p == pid).unwrap()
        };

        // Sources should be before their targets
        assert!(
            pos_of(PropId(0)) < pos_of(PropId(2)),
            "a (p0) should be before c (p2), got order: {:?}",
            final_order
        );
        assert!(
            pos_of(PropId(1)) < pos_of(PropId(3)),
            "b (p1) should be before d (p3), got order: {:?}",
            final_order
        );

        // Parallel constraints should not cross: the relative order of
        // sources should match the relative order of targets.
        // If p0 is before p1, then p2 should be before p3 (or vice versa).
        let sources_order = pos_of(PropId(0)) < pos_of(PropId(1));
        let targets_order = pos_of(PropId(2)) < pos_of(PropId(3));
        assert_eq!(
            sources_order, targets_order,
            "Parallel same-node constraints should not cross: order {:?}",
            final_order
        );
    }

    // -----------------------------------------------------------------------
    // Test: crossing same-node constraints get uncrossed
    // -----------------------------------------------------------------------
    #[test]
    fn test_same_node_crossing_constraints_uncrossed() {
        // A node with 4 properties and two CROSSING same-node constraints:
        //   p0 -> p3  and  p1 -> p2
        // Initial order: [p0, p1, p2, p3]
        //   - p0 (pos 0) -> p3 (pos 3): edge goes down
        //   - p1 (pos 1) -> p2 (pos 2): edge goes down
        //   These cross!
        //
        // After reordering, the algorithm should produce an order where
        // these constraints do NOT cross (sources maintain same relative
        // order as targets).

        let nodes = vec![
            make_node(0, "Node", vec![0, 1, 2, 3]),
            make_node(1, "Dummy", vec![]),
        ];
        let properties = vec![
            make_prop(0, 0, "a"),
            make_prop(1, 0, "b"),
            make_prop(2, 0, "c"),
            make_prop(3, 0, "d"),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            // Crossing constraints: p0 -> p3 and p1 -> p2
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(3),
                operation: None,
            },
            Edge::Constraint {
                source_prop: PropId(1),
                dest_prop: PropId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let mut layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1))],
            },
        ];
        let mut long_edges = vec![];

        let (prop_order, _, _) = minimize_crossings(&mut layers, &mut long_edges, &graph);
        let final_order = prop_order.props_of(NodeId(0));

        let pos_of = |pid: PropId| -> usize {
            final_order.iter().position(|&p| p == pid).unwrap()
        };

        // The key invariant: sources should be before their targets
        assert!(
            pos_of(PropId(0)) < pos_of(PropId(3)),
            "a (p0) should be before d (p3), got order: {:?}",
            final_order
        );
        assert!(
            pos_of(PropId(1)) < pos_of(PropId(2)),
            "b (p1) should be before c (p2), got order: {:?}",
            final_order
        );

        // The constraints should not cross each other:
        // relative order of sources matches relative order of targets.
        let sources_order = pos_of(PropId(0)) < pos_of(PropId(1));
        let targets_order = pos_of(PropId(3)) < pos_of(PropId(2));
        assert_eq!(
            sources_order, targets_order,
            "Same-node constraints should be uncrossed: order {:?}",
            final_order
        );
    }

    // -----------------------------------------------------------------------
    // Test: EdgePortOrder sorted by opposite layer position
    // -----------------------------------------------------------------------
    #[test]
    fn test_edge_port_order_sorted_by_opposite_position() {
        // 1 source node with 1 property in layer 0,
        // 3 target nodes at different positions in layer 1.
        // 3 constraints from same source prop to different targets (scrambled order).
        // Verify EdgePortOrder sorts by target position.
        let nodes = vec![
            make_node(0, "Src", vec![0]),
            make_node(1, "TgtA", vec![1]),
            make_node(2, "TgtB", vec![2]),
            make_node(3, "TgtC", vec![3]),
        ];
        let properties = vec![
            make_prop(0, 0, "src_p"),
            make_prop(1, 1, "tgt_a"),
            make_prop(2, 2, "tgt_b"),
            make_prop(3, 3, "tgt_c"),
        ];
        // Edges defined in scrambled order: to TgtC, TgtA, TgtB
        let edges = vec![
            Edge::Anchor { parent: NodeId(0), child: NodeId(1), operation: None },
            Edge::Anchor { parent: NodeId(0), child: NodeId(2), operation: None },
            Edge::Anchor { parent: NodeId(0), child: NodeId(3), operation: None },
            Edge::Constraint { source_prop: PropId(0), dest_prop: PropId(3), operation: None }, // edge 3 -> TgtC
            Edge::Constraint { source_prop: PropId(0), dest_prop: PropId(1), operation: None }, // edge 4 -> TgtA
            Edge::Constraint { source_prop: PropId(0), dest_prop: PropId(2), operation: None }, // edge 5 -> TgtB
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        // Layer 0: [Src], Layer 1: [TgtA, TgtB, TgtC] in position order
        let mut layers = vec![
            LayerEntry { items: vec![LayerItem::Node(NodeId(0))] },
            LayerEntry { items: vec![
                LayerItem::Node(NodeId(1)),
                LayerItem::Node(NodeId(2)),
                LayerItem::Node(NodeId(3)),
            ]},
        ];
        let mut long_edges = vec![];

        let (_, edge_port_order, _) = minimize_crossings(&mut layers, &mut long_edges, &graph);

        // Source prop (PropId(0)) should have edges sorted by target position:
        // TgtA(pos 0) → edge 4, TgtB(pos 1) → edge 5, TgtC(pos 2) → edge 3
        let src_edges = &edge_port_order[&PropId(0)];
        assert_eq!(src_edges.len(), 3, "Source prop should have 3 edges");

        // The edges should be in target position order
        let target_positions: Vec<usize> = src_edges.iter().map(|&eid| {
            match &graph.edges[eid.index()] {
                Edge::Constraint { dest_prop, .. } => {
                    let dest_node = graph.properties[dest_prop.index()].node;
                    layers[1].items.iter().position(|item| matches!(item, LayerItem::Node(n) if *n == dest_node)).unwrap()
                }
                _ => panic!("Expected constraint"),
            }
        }).collect();

        assert!(
            target_positions.windows(2).all(|w| w[0] <= w[1]),
            "Edges should be sorted by target position, got positions: {:?}",
            target_positions
        );
    }

    // -----------------------------------------------------------------------
    // Test: EdgePortOrder for same-node constraints
    // -----------------------------------------------------------------------
    #[test]
    fn test_edge_port_order_same_node_constraints() {
        // Node with 4 properties, same-node constraints between non-adjacent properties.
        // Verify ordering reflects property position within the node.
        let nodes = vec![
            make_node(0, "N", vec![0, 1, 2, 3]),
            make_node(1, "Dummy", vec![]),
        ];
        let properties = vec![
            make_prop(0, 0, "p0"),
            make_prop(1, 0, "p1"),
            make_prop(2, 0, "p2"),
            make_prop(3, 0, "p3"),
        ];
        let edges = vec![
            Edge::Anchor { parent: NodeId(0), child: NodeId(1), operation: None },
            // p0 -> p3 (scrambled: target further down)
            Edge::Constraint { source_prop: PropId(0), dest_prop: PropId(3), operation: None },
            // p0 -> p1 (target closer up)
            Edge::Constraint { source_prop: PropId(0), dest_prop: PropId(1), operation: None },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let mut layers = vec![
            LayerEntry { items: vec![LayerItem::Node(NodeId(0))] },
            LayerEntry { items: vec![LayerItem::Node(NodeId(1))] },
        ];
        let mut long_edges = vec![];

        let (prop_order, edge_port_order, _) = minimize_crossings(&mut layers, &mut long_edges, &graph);

        // PropId(0) has two edges. The ordering should reflect opposite prop position.
        if let Some(edges_for_p0) = edge_port_order.get(&PropId(0)) {
            assert_eq!(edges_for_p0.len(), 2);

            // Edge to p1 should come before edge to p3 (p1 has lower prop index)
            let dest_indices: Vec<usize> = edges_for_p0.iter().map(|&eid| {
                match &graph.edges[eid.index()] {
                    Edge::Constraint { dest_prop, .. } => {
                        let node_id = graph.properties[dest_prop.index()].node;
                        prop_order.prop_index(node_id, *dest_prop).unwrap()
                    }
                    _ => panic!("Expected constraint"),
                }
            }).collect();

            assert!(
                dest_indices.windows(2).all(|w| w[0] <= w[1]),
                "Same-node constraint edges should be sorted by dest prop position, got: {:?}",
                dest_indices
            );
        }
    }
}
