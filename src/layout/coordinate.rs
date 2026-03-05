//! Brandes-Kopf coordinate assignment (DESIGN.md 4.2.4).
//!
//! Three phases:
//!   A. Vertical alignment (4 directional passes)
//!   B. Horizontal compaction (per alignment)
//!   C. Balancing (median of 4 candidates)
//!
//! Plus y-coordinate assignment based on layer types and spacing constants.

use std::collections::HashMap;

use crate::model::types::{Graph, NodeId};

use super::layer_assign::LayerAssignment;
use super::long_edge::{LayerEntry, LayerItem, LongEdge};
use super::{
    node_height, node_width, NodeLayout,
    INTER_NODE_GAP, NODE_H_SPACING,
};

// ---------------------------------------------------------------------------
// Unified element ID
// ---------------------------------------------------------------------------

/// A unified identifier for any element that participates in layer ordering.
/// Nodes and long-edge segments are flattened into a single index space so
/// Brandes-Kopf can treat them uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ElemId(u32);

/// Mapping tables between ElemId and the domain-specific identifiers.
struct ElemMap {
    /// ElemId -> what kind of element it is
    kinds: Vec<ElemKind>,
    /// NodeId -> ElemId
    node_to_elem: HashMap<NodeId, ElemId>,
    /// (EdgeId, layer) -> ElemId for long-edge segments
    #[allow(dead_code)]
    seg_to_elem: HashMap<(crate::model::types::EdgeId, u32), ElemId>,
    /// Total number of elements
    count: usize,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum ElemKind {
    Node(NodeId),
    Segment(crate::model::types::EdgeId, u32),
}

/// Build the unified element map from the layer entries.
fn build_elem_map(layers: &[LayerEntry]) -> ElemMap {
    let mut kinds = Vec::new();
    let mut node_to_elem = HashMap::new();
    let mut seg_to_elem = HashMap::new();

    for layer in layers {
        for item in &layer.items {
            let id = ElemId(kinds.len() as u32);
            match item {
                LayerItem::Node(nid) => {
                    node_to_elem.entry(*nid).or_insert_with(|| {
                        kinds.push(ElemKind::Node(*nid));
                        id
                    });
                }
                LayerItem::Segment(eid, layer_idx) => {
                    seg_to_elem.entry((*eid, *layer_idx)).or_insert_with(|| {
                        kinds.push(ElemKind::Segment(*eid, *layer_idx));
                        id
                    });
                }
            }
        }
    }

    let count = kinds.len();
    ElemMap {
        kinds,
        node_to_elem,
        seg_to_elem,
        count,
    }
}

/// Get the ElemId for a LayerItem.
fn item_to_elem(item: &LayerItem, emap: &ElemMap) -> ElemId {
    match item {
        LayerItem::Node(nid) => emap.node_to_elem[nid],
        LayerItem::Segment(eid, layer_idx) => emap.seg_to_elem[&(*eid, *layer_idx)],
    }
}

/// Width of an element for spacing computations.
fn elem_width(elem: ElemId, emap: &ElemMap, graph: &Graph) -> f64 {
    match emap.kinds[elem.0 as usize] {
        ElemKind::Node(nid) => node_width(graph, nid),
        // Long-edge segments are treated as zero-width points for spacing.
        ElemKind::Segment(_, _) => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Layer position lookup tables
// ---------------------------------------------------------------------------

/// For each ElemId: which layer it is in and its position within that layer.
struct PosTable {
    layer_of: Vec<u32>,
    pos_in_layer: Vec<u32>,
}

fn build_pos_table(layers: &[LayerEntry], emap: &ElemMap) -> PosTable {
    let n = emap.count;
    let mut layer_of = vec![0u32; n];
    let mut pos_in_layer = vec![0u32; n];

    for (li, layer) in layers.iter().enumerate() {
        for (pi, item) in layer.items.iter().enumerate() {
            let e = item_to_elem(item, emap);
            layer_of[e.0 as usize] = li as u32;
            pos_in_layer[e.0 as usize] = pi as u32;
        }
    }

    PosTable {
        layer_of,
        pos_in_layer,
    }
}

// ---------------------------------------------------------------------------
// The four alignment directions
// ---------------------------------------------------------------------------

/// The four alignment directions used by Brandes-Kopf.
#[derive(Debug, Clone, Copy)]
pub enum AlignDirection {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
}

impl AlignDirection {
    fn is_upper(self) -> bool {
        matches!(self, AlignDirection::UpperLeft | AlignDirection::UpperRight)
    }

    fn is_left(self) -> bool {
        matches!(self, AlignDirection::UpperLeft | AlignDirection::LowerLeft)
    }
}

// ---------------------------------------------------------------------------
// Adjacency: find neighbors of an element in a fixed adjacent layer
// ---------------------------------------------------------------------------

/// Build adjacency information from the layer structure.
/// For each (layer_index, position) pair, find which elements in the adjacent
/// layer are connected to it.
///
/// We derive adjacency from the layer ordering: two elements in consecutive
/// layers are adjacent if they belong to the same long edge chain, or if
/// a graph edge connects them.
///
/// For simplicity we build a map: (ElemId) -> Vec<ElemId> of elements in the
/// "other" layer that are connected, for each direction.
#[allow(clippy::collapsible_if)]
fn build_adjacency(
    layers: &[LayerEntry],
    _long_edges: &[LongEdge],
    graph: &Graph,
    emap: &ElemMap,
) -> HashMap<ElemId, Vec<ElemId>> {
    let mut adj: HashMap<ElemId, Vec<ElemId>> = HashMap::new();

    // For each pair of adjacent layers, discover edges between them.
    for li in 0..layers.len().saturating_sub(1) {
        let upper = &layers[li];
        let lower = &layers[li + 1];

        // Build quick lookup: which ElemIds are in the lower layer.
        let lower_set: HashMap<ElemId, usize> = lower
            .items
            .iter()
            .enumerate()
            .map(|(pos, item)| (item_to_elem(item, emap), pos))
            .collect();
        let upper_set: HashMap<ElemId, usize> = upper
            .items
            .iter()
            .enumerate()
            .map(|(pos, item)| (item_to_elem(item, emap), pos))
            .collect();

        // Collect pairs to add as bidirectional adjacency edges.
        let mut pairs: Vec<(ElemId, ElemId)> = Vec::new();

        // Helper: test whether both endpoints are in this upper/lower pair.
        let in_pair = |a: ElemId, b: ElemId| -> bool {
            (upper_set.contains_key(&a) && lower_set.contains_key(&b))
                || (upper_set.contains_key(&b) && lower_set.contains_key(&a))
        };

        // 1) Long-edge segments: consecutive layers with the same EdgeId.
        for u_item in &upper.items {
            if let LayerItem::Segment(eid, _) = u_item {
                for l_item in &lower.items {
                    if let LayerItem::Segment(eid2, _) = l_item {
                        if eid == eid2 {
                            let eu = item_to_elem(u_item, emap);
                            let el = item_to_elem(l_item, emap);
                            pairs.push((eu, el));
                        }
                    }
                }
            }
        }

        // 2) Graph edges that connect elements in adjacent layers.
        for edge in &graph.edges {
            let (src_node, dst_node) = graph.edge_nodes(edge);
            if let (Some(&es), Some(&ed)) = (
                emap.node_to_elem.get(&src_node),
                emap.node_to_elem.get(&dst_node),
            ) {
                if in_pair(es, ed) { pairs.push((es, ed)); }
            }
        }

        // Apply collected pairs as bidirectional adjacency.
        for (a, b) in pairs {
            adj.entry(a).or_default().push(b);
            adj.entry(b).or_default().push(a);
        }
    }

    // Deduplicate adjacency lists.
    for list in adj.values_mut() {
        list.sort_by_key(|e| e.0);
        list.dedup();
    }

    adj
}

/// Given an element, return its neighbors in a specific adjacent layer,
/// sorted by their position in that layer.
fn neighbors_in_layer(
    elem: ElemId,
    target_layer: u32,
    adj: &HashMap<ElemId, Vec<ElemId>>,
    pos_table: &PosTable,
) -> Vec<ElemId> {
    let empty = Vec::new();
    let neighbors = adj.get(&elem).unwrap_or(&empty);
    let mut result: Vec<ElemId> = neighbors
        .iter()
        .copied()
        .filter(|&n| pos_table.layer_of[n.0 as usize] == target_layer)
        .collect();
    result.sort_by_key(|&n| pos_table.pos_in_layer[n.0 as usize]);
    result
}

// ---------------------------------------------------------------------------
// Phase A: Vertical Alignment
// ---------------------------------------------------------------------------

struct Alignment {
    root: Vec<ElemId>,
    align: Vec<ElemId>,
}

fn vertical_alignment(
    layers: &[LayerEntry],
    adj: &HashMap<ElemId, Vec<ElemId>>,
    emap: &ElemMap,
    pos_table: &PosTable,
    direction: AlignDirection,
) -> Alignment {
    let n = emap.count;
    let mut root: Vec<ElemId> = (0..n).map(|i| ElemId(i as u32)).collect();
    let mut align: Vec<ElemId> = (0..n).map(|i| ElemId(i as u32)).collect();

    // Determine scan order of layers.
    let layer_indices: Vec<usize> = if direction.is_upper() {
        // Scan from top (layer 0) to bottom.
        (1..layers.len()).collect()
    } else {
        // Scan from bottom to top.
        (0..layers.len().saturating_sub(1)).rev().collect()
    };

    for &li in &layer_indices {
        // The "fixed" layer is the one we look at for neighbors.
        let fixed_layer = if direction.is_upper() {
            li as u32 - 1
        } else {
            li as u32 + 1
        };

        let mut r: i64 = -1; // Rightmost aligned position so far.

        // Iterate through items in this layer in left-to-right or right-to-left order.
        let items: Vec<&LayerItem> = if direction.is_left() {
            layers[li].items.iter().collect()
        } else {
            layers[li].items.iter().rev().collect()
        };

        for item in &items {
            let v = item_to_elem(item, emap);
            let neighbors = neighbors_in_layer(v, fixed_layer, adj, pos_table);

            if neighbors.is_empty() {
                continue;
            }

            // Compute median indices.
            let medians = median_neighbors(&neighbors, direction);

            for m in medians {
                if align[m.0 as usize] == m {
                    let pos = pos_table.pos_in_layer[m.0 as usize] as i64;
                    let check = if direction.is_left() {
                        pos > r
                    } else {
                        // For right-to-left, we track the leftmost position,
                        // so we check pos < r (or r == -1 for init).
                        r == -1 || pos < r
                    };
                    if check {
                        align[m.0 as usize] = v;
                        root[v.0 as usize] = root[m.0 as usize];
                        align[v.0 as usize] = root[v.0 as usize];
                        r = pos;
                    }
                }
            }
        }
    }

    Alignment { root, align }
}

/// Return the median neighbor(s). For even count, returns the two middle ones;
/// for odd count, returns just the single median.
fn median_neighbors(neighbors: &[ElemId], direction: AlignDirection) -> Vec<ElemId> {
    if neighbors.is_empty() {
        return Vec::new();
    }
    let len = neighbors.len();
    if len == 1 {
        return vec![neighbors[0]];
    }
    let mid = (len - 1) / 2;
    if len % 2 == 1 {
        vec![neighbors[mid]]
    } else {
        // Two medians; the order depends on direction.
        if direction.is_left() {
            vec![neighbors[mid], neighbors[mid + 1]]
        } else {
            vec![neighbors[mid + 1], neighbors[mid]]
        }
    }
}

// ---------------------------------------------------------------------------
// Phase B: Horizontal Compaction
// ---------------------------------------------------------------------------

/// Minimum separation between two adjacent elements.
fn min_separation(left: ElemId, right: ElemId, emap: &ElemMap, graph: &Graph) -> f64 {
    let w_left = elem_width(left, emap, graph);
    let w_right = elem_width(right, emap, graph);
    w_left / 2.0 + NODE_H_SPACING + w_right / 2.0
}

fn horizontal_compaction(
    layers: &[LayerEntry],
    alignment: &Alignment,
    emap: &ElemMap,
    pos_table: &PosTable,
    graph: &Graph,
) -> Vec<f64> {
    let n = emap.count;
    let mut sink: Vec<ElemId> = (0..n).map(|i| ElemId(i as u32)).collect();
    let mut shift: Vec<f64> = vec![f64::INFINITY; n];
    let mut x: Vec<Option<f64>> = vec![None; n];

    // Build a layer-position-to-ElemId lookup for finding left neighbors.
    let mut layer_items: Vec<Vec<ElemId>> = vec![Vec::new(); layers.len()];
    for (li, layer) in layers.iter().enumerate() {
        layer_items[li] = layer
            .items
            .iter()
            .map(|item| item_to_elem(item, emap))
            .collect();
    }

    // Process each root block.
    for layer in layers {
        for item in &layer.items {
            let v = item_to_elem(item, emap);
            if alignment.root[v.0 as usize] == v {
                place_block(
                    v, alignment, emap, pos_table, graph, &layer_items, &mut sink,
                    &mut shift, &mut x,
                );
            }
        }
    }

    // Second pass: compute final x from root and shift.
    let mut result = vec![0.0f64; n];
    for layer in layers {
        for item in &layer.items {
            let v = item_to_elem(item, emap);
            let r = alignment.root[v.0 as usize];
            result[v.0 as usize] = x[r.0 as usize].unwrap_or(0.0);
            let s = sink[r.0 as usize];
            if shift[s.0 as usize] < f64::INFINITY {
                result[v.0 as usize] += shift[s.0 as usize];
            }
        }
    }

    result
}

#[allow(clippy::too_many_arguments)]
fn place_block(
    v: ElemId,
    alignment: &Alignment,
    emap: &ElemMap,
    pos_table: &PosTable,
    graph: &Graph,
    layer_items: &[Vec<ElemId>],
    sink: &mut [ElemId],
    shift: &mut [f64],
    x: &mut [Option<f64>],
) {
    if x[v.0 as usize].is_some() {
        return;
    }
    x[v.0 as usize] = Some(0.0);

    let mut w = v;
    let max_chain = emap.count + 1;
    let mut steps = 0;
    loop {
        let layer = pos_table.layer_of[w.0 as usize] as usize;
        let pos = pos_table.pos_in_layer[w.0 as usize] as usize;

        // Check for left neighbor.
        if pos > 0 && pos < layer_items[layer].len() {
            let pred = layer_items[layer][pos - 1];
            let pred_root = alignment.root[pred.0 as usize];
            place_block(
                pred_root, alignment, emap, pos_table, graph, layer_items, sink, shift, x,
            );

            if sink[v.0 as usize] == v {
                sink[v.0 as usize] = sink[pred_root.0 as usize];
            }

            let sep = min_separation(pred, w, emap, graph);

            if sink[v.0 as usize] != sink[pred_root.0 as usize] {
                let current_shift = x[v.0 as usize].unwrap() - x[pred_root.0 as usize].unwrap()
                    - sep;
                let s_idx = sink[pred_root.0 as usize].0 as usize;
                shift[s_idx] = shift[s_idx].min(current_shift);
            } else {
                let new_x = x[pred_root.0 as usize].unwrap() + sep;
                x[v.0 as usize] = Some(x[v.0 as usize].unwrap().max(new_x));
            }
        }

        w = alignment.align[w.0 as usize];
        steps += 1;
        if w == v || steps >= max_chain {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Phase C: Balancing
// ---------------------------------------------------------------------------

fn balanced_x_coordinates(
    layers: &[LayerEntry],
    adj: &HashMap<ElemId, Vec<ElemId>>,
    emap: &ElemMap,
    pos_table: &PosTable,
    graph: &Graph,
) -> Vec<f64> {
    let n = emap.count;

    let directions = [
        AlignDirection::UpperLeft,
        AlignDirection::UpperRight,
        AlignDirection::LowerLeft,
        AlignDirection::LowerRight,
    ];

    let mut all_x: Vec<Vec<f64>> = Vec::new();

    for &dir in &directions {
        let alignment = vertical_alignment(layers, adj, emap, pos_table, dir);
        let xs = horizontal_compaction(layers, &alignment, emap, pos_table, graph);
        all_x.push(xs);
    }

    // Normalize each so min = 0.
    for xs in &mut all_x {
        let min_val = xs.iter().copied().fold(f64::INFINITY, f64::min);
        if min_val.is_finite() {
            for x in xs.iter_mut() {
                *x -= min_val;
            }
        }
    }

    // Final: median of 4 candidates (average of two middle values).
    let mut result = vec![0.0f64; n];
    for i in 0..n {
        let mut vals = [all_x[0][i], all_x[1][i], all_x[2][i], all_x[3][i]];
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        result[i] = (vals[1] + vals[2]) / 2.0;
    }

    result
}

// ---------------------------------------------------------------------------
// Y-coordinate assignment
// ---------------------------------------------------------------------------

/// Assign y-coordinates based on layer assignment (DESIGN.md 4.2.4).
///
/// Returns a map from layer index to its y-offset.
pub fn assign_y_coordinates(layers: &[LayerEntry], graph: &Graph) -> HashMap<u32, f64> {
    let mut y_map: HashMap<u32, f64> = HashMap::new();
    let mut y_offset: f64 = 0.0;

    for (li, layer) in layers.iter().enumerate() {
        if layer.items.is_empty() {
            continue;
        }

        // Layers containing only long-edge segments are virtual routing layers;
        // they carry no visual content and should not contribute vertical space.
        let is_segment_only = layer.items.iter().all(|item| {
            matches!(item, LayerItem::Segment(_, _))
        });
        if is_segment_only {
            // Record the y for segment routing but don't advance the offset.
            y_map.insert(li as u32, y_offset);
            continue;
        }

        y_map.insert(li as u32, y_offset);

        // Find max height among items in this layer.
        let max_height = layer
            .items
            .iter()
            .map(|item| match item {
                LayerItem::Node(nid) => node_height(graph, *nid),
                LayerItem::Segment(_, _) => 0.0,
            })
            .fold(0.0_f64, f64::max);
        // Use INTER_NODE_GAP for the vertical gap between layers.
        // The initial LAYER_V_SPACING (48px) is a theoretical maximum;
        // the mockup and vertical compaction target INTER_NODE_GAP (28px)
        // between all adjacent elements.
        y_offset += INTER_NODE_GAP + max_height;
    }

    y_map
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the Brandes-Kopf coordinate assignment algorithm.
///
/// Produces x- and y-coordinates for all nodes.
pub fn assign_coordinates(
    layers: &[LayerEntry],
    long_edges: &[LongEdge],
    _assignment: &LayerAssignment,
    graph: &Graph,
) -> Vec<NodeLayout> {
    let emap = build_elem_map(layers);
    let pos_table = build_pos_table(layers, &emap);
    let adj = build_adjacency(layers, long_edges, graph, &emap);

    // Phase A+B+C: balanced x-coordinates.
    let x_coords = balanced_x_coordinates(layers, &adj, &emap, &pos_table, graph);

    // Y-coordinates.
    let y_map = assign_y_coordinates(layers, graph);

    // Build output layouts.
    let mut node_layouts: Vec<NodeLayout> = Vec::with_capacity(graph.nodes.len());

    // Initialize with defaults so we can index by id.
    for node in &graph.nodes {
        node_layouts.push(NodeLayout {
            id: node.id,
            x: 0.0,
            y: 0.0,
            width: node_width(graph, node.id),
            height: node_height(graph, node.id),
        });
    }

    // Assign positions from x_coords and y_map.
    for (li, layer) in layers.iter().enumerate() {
        let y = y_map.get(&(li as u32)).copied().unwrap_or(0.0);
        for item in &layer.items {
            let elem = item_to_elem(item, &emap);
            let x = x_coords[elem.0 as usize];
            match item {
                LayerItem::Node(nid) => {
                    let w = node_width(graph, *nid);
                    let nl = &mut node_layouts[nid.index()];
                    // x from Brandes-Kopf is center position; convert to top-left.
                    nl.x = x - w / 2.0;
                    nl.y = y;
                }
                LayerItem::Segment(_, _) => {
                    // Segments don't produce layout output.
                }
            }
        }
    }

    // Normalize so that the minimum x across all layouts is 0.
    let min_x = node_layouts
        .iter()
        .map(|nl| nl.x)
        .fold(f64::INFINITY, f64::min);
    if min_x.is_finite() && min_x.abs() > 1e-9 {
        for nl in &mut node_layouts {
            nl.x -= min_x;
        }
    }

    node_layouts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{HEADER_HEIGHT, INTER_NODE_GAP, ROW_HEIGHT};
    use crate::model::types::*;
    use std::collections::HashMap;

    /// Helper: build a minimal graph with the given number of nodes, each with
    /// the specified number of properties.
    fn make_graph(node_specs: &[(&str, usize)]) -> Graph {
        let mut nodes = Vec::new();
        let mut properties = Vec::new();

        for (i, &(name, prop_count)) in node_specs.iter().enumerate() {
            let nid = NodeId(i as u32);
            let mut prop_ids = Vec::new();
            for j in 0..prop_count {
                let pid = PropId(properties.len() as u32);
                properties.push(Property {
                    id: pid,
                    node: nid,
                    name: format!("prop{}", j),
                    critical: true, constrained: false,
                });
                prop_ids.push(pid);
            }
            nodes.push(Node {
                id: nid,
                ident: Some(name.to_string()),
                display_name: None,
                properties: prop_ids,
                domain: None,
                is_anchored: i == 0,
                is_selected: false,
            });
        }

        Graph {
            nodes,
            properties,
            edges: Vec::new(),
            domains: Vec::new(),
            prop_edges: HashMap::new(),
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        }
    }

    /// Helper: build a graph with edges.
    fn make_graph_with_edges(
        node_specs: &[(&str, usize)],
        link_edges: &[(u32, u32)],
    ) -> Graph {
        let mut g = make_graph(node_specs);
        let mut node_children: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut node_parent: HashMap<NodeId, EdgeId> = HashMap::new();

        for (i, &(parent_idx, child_idx)) in link_edges.iter().enumerate() {
            let eid = EdgeId(i as u32);
            let parent = NodeId(parent_idx);
            let child = NodeId(child_idx);
            g.edges.push(Edge::Anchor {
                parent,
                child,
                operation: None,
            });
            node_children.entry(parent).or_default().push(eid);
            node_parent.insert(child, eid);
        }

        g.node_children = node_children;
        g.node_parent = node_parent;
        g
    }

    fn make_layer_assignment(node_layers: &[(u32, u32)], num_layers: u32) -> LayerAssignment {
        let mut nl = HashMap::new();
        for &(nid, layer) in node_layers {
            nl.insert(NodeId(nid), layer);
        }
        LayerAssignment {
            node_layers: nl,
            num_layers,
            meta_order: Vec::new(),
        }
    }

    // Test 1: Single node placed at x=0
    #[test]
    fn test_single_node_at_origin() {
        let graph = make_graph(&[("A", 2)]);
        let layers = vec![LayerEntry {
            items: vec![LayerItem::Node(NodeId(0))],
        }];
        let long_edges = vec![];
        let assignment = make_layer_assignment(&[(0, 0)], 1);

        let node_layouts =
            assign_coordinates(&layers, &long_edges, &assignment, &graph);

        assert_eq!(node_layouts.len(), 1);
        // The x coordinate of the single node should place it at 0 (center = 0,
        // so x = -width/2). Since there's only one node, it's at x = -width/2.
        // After balancing, min is normalized to 0, so center = width/2,
        // meaning x (top-left) = 0.
        let nl = &node_layouts[0];
        assert!(
            nl.x.abs() < 1e-6,
            "Single node should be at x=0, got x={}",
            nl.x
        );
        assert!(
            nl.y.abs() < 1e-6,
            "Single node should be at y=0, got y={}",
            nl.y
        );
    }

    // Test 2: Two nodes in same layer separated by min_separation
    #[test]
    fn test_two_nodes_same_layer_separation() {
        let graph = make_graph(&[("A", 1), ("B", 1)]);
        let layers = vec![LayerEntry {
            items: vec![LayerItem::Node(NodeId(0)), LayerItem::Node(NodeId(1))],
        }];
        let long_edges = vec![];
        let assignment = make_layer_assignment(&[(0, 0), (1, 0)], 1);

        let node_layouts =
            assign_coordinates(&layers, &long_edges, &assignment, &graph);

        let left = &node_layouts[0];
        let right = &node_layouts[1];

        // The gap between them should be at least NODE_H_SPACING.
        let left_right_edge = left.x + left.width;
        let gap = right.x - left_right_edge;
        assert!(
            gap >= NODE_H_SPACING - 1e-6,
            "Gap between nodes should be >= NODE_H_SPACING ({}), got {}",
            NODE_H_SPACING,
            gap
        );
    }

    // Test 3: Two connected nodes in adjacent layers are vertically aligned
    #[test]
    fn test_two_connected_nodes_aligned() {
        let graph = make_graph_with_edges(&[("A", 1), ("B", 1)], &[(0, 1)]);
        let layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1))],
            },
        ];
        let long_edges = vec![];
        let assignment = make_layer_assignment(&[(0, 0), (1, 1)], 2);

        let node_layouts =
            assign_coordinates(&layers, &long_edges, &assignment, &graph);

        let a = &node_layouts[0];
        let b = &node_layouts[1];

        // Both are the only items in their layers, so both should have x near 0.
        // After normalization, both centers should be equal (aligned).
        let a_center = a.x + a.width / 2.0;
        let b_center = b.x + b.width / 2.0;
        assert!(
            (a_center - b_center).abs() < 1e-6,
            "Connected nodes should be vertically aligned: A center={}, B center={}",
            a_center,
            b_center
        );
    }

    // Test 4: Y-coordinates match layer spacing rules
    #[test]
    fn test_y_coordinates_layer_spacing() {
        // Two node layers.
        let graph = make_graph(&[("A", 2), ("B", 1)]);
        let layers = vec![
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(0))],
            },
            LayerEntry {
                items: vec![LayerItem::Node(NodeId(1))],
            },
        ];

        let y_map = assign_y_coordinates(&layers, &graph);

        let y0 = y_map[&0];
        let y1 = y_map[&1];

        // Layer 0 has node A with 2 props: height = HEADER_HEIGHT + 2*ROW_HEIGHT.
        let expected_height_0 = HEADER_HEIGHT + 2.0 * ROW_HEIGHT;
        let expected_y1 = y0 + INTER_NODE_GAP + expected_height_0;

        assert!(
            (y0).abs() < 1e-6,
            "First layer y should be 0, got {}",
            y0
        );
        assert!(
            (y1 - expected_y1).abs() < 1e-6,
            "Second layer y should be {}, got {}",
            expected_y1,
            y1
        );
    }
}
