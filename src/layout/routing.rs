/// Orthogonal channel-based edge routing (DESIGN.md §4.2.6).

use crate::model::types::{Edge, EdgeId, Graph, NodeId, PropId};

use super::{
    layout_endpoints, DerivLayout, EdgeLabel, EdgePath, EndpointRole, LayoutEndpoint, NodeLayout,
    PortSide, EDGE_SPACING, STUB_LENGTH,
};

// ---------------------------------------------------------------------------
// Route / Segment types
// ---------------------------------------------------------------------------

/// A segment in an orthogonal edge route.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Horizontal { y: f64, x_start: f64, x_end: f64 },
    Vertical { x: f64, y_start: f64, y_end: f64 },
}

impl Segment {
    /// Length of this segment (always non-negative).
    fn length(&self) -> f64 {
        match self {
            Segment::Horizontal { x_start, x_end, .. } => (x_end - x_start).abs(),
            Segment::Vertical { y_start, y_end, .. } => (y_end - y_start).abs(),
        }
    }

    /// Starting point of this segment.
    fn start(&self) -> (f64, f64) {
        match self {
            Segment::Horizontal { y, x_start, .. } => (*x_start, *y),
            Segment::Vertical { x, y_start, .. } => (*x, *y_start),
        }
    }

    /// Ending point of this segment.
    fn end(&self) -> (f64, f64) {
        match self {
            Segment::Horizontal { y, x_end, .. } => (*x_end, *y),
            Segment::Vertical { x, y_end, .. } => (*x, *y_end),
        }
    }
}

/// A complete route for one edge.
#[derive(Debug, Clone)]
pub struct Route {
    pub edge_id: EdgeId,
    pub segments: Vec<Segment>,
}

/// An axis for a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// A routing channel -- a corridor between/around obstacles.
#[derive(Debug, Clone)]
pub struct Channel {
    pub axis: Axis,
    pub position: f64,
    pub range: (f64, f64),
    pub occupants: Vec<EdgeId>,
}

impl Channel {
    /// The effective position for a new occupant, accounting for already-reserved edges.
    /// Occupants are spaced symmetrically around the channel center.
    fn offset_position(&self, occupant_index: usize) -> f64 {
        let n = occupant_index;
        // Center the group: offset = (i - (n-1)/2) * EDGE_SPACING
        // For a new occupant being added at index `occupant_index`:
        let _offset = (n as f64 - 0.0) * EDGE_SPACING;
        // Simple scheme: first occupant at center, subsequent ones offset alternately
        if n == 0 {
            self.position
        } else if n % 2 == 1 {
            self.position + ((n as f64 + 1.0) / 2.0).ceil() * EDGE_SPACING
        } else {
            self.position - (n as f64 / 2.0) * EDGE_SPACING
        }
    }

    /// Reserve this channel for an edge and return the assigned position.
    fn reserve(&mut self, edge_id: EdgeId) -> f64 {
        let idx = self.occupants.len();
        let pos = self.offset_position(idx);
        self.occupants.push(edge_id);
        pos
    }
}

/// The port side assignments for all edge endpoints.
pub type PortSideAssignment = std::collections::HashMap<(EdgeId, EndpointRole), PortSide>;

// ---------------------------------------------------------------------------
// Port side assignment
// ---------------------------------------------------------------------------

/// Resolve a property to its owning node.
fn prop_node(graph: &Graph, prop_id: PropId) -> NodeId {
    graph.properties[prop_id.index()].node
}

/// Find the NodeLayout for a given NodeId.
fn find_node_layout<'a>(node_layouts: &'a [NodeLayout], node_id: NodeId) -> Option<&'a NodeLayout> {
    node_layouts.iter().find(|nl| nl.id == node_id)
}

/// Find the DerivLayout for a given DerivId.
fn find_deriv_layout<'a>(
    deriv_layouts: &'a [DerivLayout],
    deriv_id: crate::model::types::DerivId,
) -> Option<&'a DerivLayout> {
    deriv_layouts.iter().find(|dl| dl.id == deriv_id)
}

/// Assign port sides for all edges based on relative node positions.
///
/// Links use center top/bottom ports so they get no side assignment.
/// Constraints and DerivInputs get left/right side assignments based on
/// the relative horizontal positions of their endpoint nodes.
pub fn assign_port_sides(
    graph: &Graph,
    node_layouts: &[NodeLayout],
    _deriv_layouts: &[DerivLayout],
) -> PortSideAssignment {
    let mut sides = PortSideAssignment::new();

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);

        // Links use center ports; no side assignment needed.
        if edge.is_anchor() {
            continue;
        }

        let (upstream, downstream) = layout_endpoints(edge);

        // Determine the source and target nodes for side computation.
        let src_node_id = match upstream {
            LayoutEndpoint::Prop(pid) => prop_node(graph, pid),
            LayoutEndpoint::Node(nid) => nid,
            LayoutEndpoint::Deriv(_) => continue, // shouldn't happen for upstream
        };

        let tgt_node_id = match downstream {
            LayoutEndpoint::Prop(pid) => prop_node(graph, pid),
            LayoutEndpoint::Node(nid) => nid,
            LayoutEndpoint::Deriv(did) => {
                // For DerivInput: downstream is a derivation.
                // We assign the upstream (source prop) side based on relative position.
                let src_nl = match find_node_layout(node_layouts, src_node_id) {
                    Some(nl) => nl,
                    None => continue,
                };
                let dl = match find_deriv_layout(_deriv_layouts, did) {
                    Some(dl) => dl,
                    None => continue,
                };
                let src_cx = src_nl.x + src_nl.width / 2.0;
                let tgt_cx = dl.x + dl.width / 2.0;

                if src_cx < tgt_cx {
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                } else if src_cx > tgt_cx {
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                } else {
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                }
                // Downstream of a DerivInput connects to derivation center; no side.
                continue;
            }
        };

        let src_nl = match find_node_layout(node_layouts, src_node_id) {
            Some(nl) => nl,
            None => continue,
        };
        let tgt_nl = match find_node_layout(node_layouts, tgt_node_id) {
            Some(nl) => nl,
            None => continue,
        };

        if src_node_id == tgt_node_id {
            // Self-loop: exit right, enter left.
            sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
            sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
        } else {
            let src_cx = src_nl.x + src_nl.width / 2.0;
            let tgt_cx = tgt_nl.x + tgt_nl.width / 2.0;

            if src_cx < tgt_cx {
                sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                sides.insert((edge_id, EndpointRole::Downstream), PortSide::Left);
            } else if src_cx > tgt_cx {
                sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
            } else {
                // Same center x: use left side for a clean intra-column U-shape.
                sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                sides.insert((edge_id, EndpointRole::Downstream), PortSide::Left);
            }
        }
    }

    sides
}

// ---------------------------------------------------------------------------
// Channel construction
// ---------------------------------------------------------------------------

/// Build horizontal channels between consecutive layers.
///
/// Each horizontal channel sits midway between the bottom of one layer's tallest
/// node and the top of the next layer's topmost node. We detect layers by finding
/// clusters of nodes at similar y positions.
fn build_h_channels(node_layouts: &[NodeLayout], deriv_layouts: &[DerivLayout]) -> Vec<Channel> {
    // Gather all distinct y-bands (top, bottom) for nodes and derivations.
    let mut bands: Vec<(f64, f64)> = Vec::new(); // (top_y, bottom_y) per element

    for nl in node_layouts {
        bands.push((nl.y, nl.y + nl.height));
    }
    for dl in deriv_layouts {
        bands.push((dl.y, dl.y + dl.height));
    }

    if bands.is_empty() {
        return Vec::new();
    }

    // Sort by top y to identify layers.
    bands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Group into layers: elements whose top y differs by less than 1.0 are same layer.
    let mut layers: Vec<(f64, f64)> = Vec::new(); // (min_top, max_bottom) per layer
    let mut cur_min_top = bands[0].0;
    let mut cur_max_bottom = bands[0].1;

    for &(top, bottom) in &bands[1..] {
        if (top - cur_min_top).abs() < 1.0 {
            // Same layer.
            cur_max_bottom = cur_max_bottom.max(bottom);
        } else {
            layers.push((cur_min_top, cur_max_bottom));
            cur_min_top = top;
            cur_max_bottom = bottom;
        }
    }
    layers.push((cur_min_top, cur_max_bottom));

    // Create one horizontal channel between each pair of consecutive layers.
    let mut channels = Vec::new();
    for i in 0..layers.len().saturating_sub(1) {
        let bottom_of_upper = layers[i].1;
        let top_of_lower = layers[i + 1].0;
        let mid_y = (bottom_of_upper + top_of_lower) / 2.0;

        // Determine the full x range.
        let x_min = node_layouts
            .iter()
            .map(|nl| nl.x)
            .chain(deriv_layouts.iter().map(|dl| dl.x))
            .fold(f64::INFINITY, f64::min)
            - 50.0;
        let x_max = node_layouts
            .iter()
            .map(|nl| nl.x + nl.width)
            .chain(deriv_layouts.iter().map(|dl| dl.x + dl.width))
            .fold(f64::NEG_INFINITY, f64::max)
            + 50.0;

        channels.push(Channel {
            axis: Axis::Horizontal,
            position: mid_y,
            range: (x_min, x_max),
            occupants: Vec::new(),
        });
    }

    channels
}

/// Build vertical channels between nodes within the same layer and at the edges.
///
/// Each vertical channel sits midway between adjacent nodes (sorted by x)
/// within the same visual layer. Additional channels are placed to the left of
/// the leftmost node and to the right of the rightmost node.
fn build_v_channels(node_layouts: &[NodeLayout], deriv_layouts: &[DerivLayout]) -> Vec<Channel> {
    // Gather all elements as (x, width, y, height).
    let mut elements: Vec<(f64, f64, f64, f64)> = Vec::new();
    for nl in node_layouts {
        elements.push((nl.x, nl.width, nl.y, nl.height));
    }
    for dl in deriv_layouts {
        elements.push((dl.x, dl.width, dl.y, dl.height));
    }

    if elements.is_empty() {
        return Vec::new();
    }

    // Group by similar y (layer detection).
    elements.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap().then(a.0.partial_cmp(&b.0).unwrap()));

    let mut channels = Vec::new();
    let mut layer_start = 0;

    while layer_start < elements.len() {
        let layer_y = elements[layer_start].2;
        let mut layer_end = layer_start;
        while layer_end < elements.len() && (elements[layer_end].2 - layer_y).abs() < 1.0 {
            layer_end += 1;
        }

        let layer = &elements[layer_start..layer_end];

        // Compute the y range for this layer.
        let y_min = layer
            .iter()
            .map(|e| e.2)
            .fold(f64::INFINITY, f64::min)
            - 50.0;
        let y_max = layer
            .iter()
            .map(|e| e.2 + e.3)
            .fold(f64::NEG_INFINITY, f64::max)
            + 50.0;

        // Sort by x within the layer.
        let mut sorted: Vec<_> = layer.to_vec();
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        // Channel to the left of the leftmost node.
        if let Some(first) = sorted.first() {
            let left_x = first.0 - super::NODE_H_SPACING / 2.0;
            channels.push(Channel {
                axis: Axis::Vertical,
                position: left_x,
                range: (y_min, y_max),
                occupants: Vec::new(),
            });
        }

        // Channels between consecutive nodes.
        for w in sorted.windows(2) {
            let right_of_left = w[0].0 + w[0].1;
            let left_of_right = w[1].0;
            let mid_x = (right_of_left + left_of_right) / 2.0;

            channels.push(Channel {
                axis: Axis::Vertical,
                position: mid_x,
                range: (y_min, y_max),
                occupants: Vec::new(),
            });
        }

        // Channel to the right of the rightmost node.
        if let Some(last) = sorted.last() {
            let right_x = last.0 + last.1 + super::NODE_H_SPACING / 2.0;
            channels.push(Channel {
                axis: Axis::Vertical,
                position: right_x,
                range: (y_min, y_max),
                occupants: Vec::new(),
            });
        }

        layer_start = layer_end;
    }

    channels
}

// ---------------------------------------------------------------------------
// Port position computation
// ---------------------------------------------------------------------------

/// Compute the (x, y) port position and optional side for an edge endpoint.
///
/// Returns `(x, y, Option<PortSide>)`. The side is `None` for link center ports.
fn port_position(
    graph: &Graph,
    edge_id: EdgeId,
    role: EndpointRole,
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    port_sides: &PortSideAssignment,
) -> Option<(f64, f64, Option<PortSide>)> {
    let edge = &graph.edges[edge_id.index()];
    let (_upstream, _downstream) = layout_endpoints(edge);

    match edge {
        Edge::Anchor { parent, child, .. } => {
            // Links use center top/bottom ports.
            match role {
                EndpointRole::Upstream => {
                    // Source is parent node, bottom center.
                    let nl = find_node_layout(node_layouts, *parent)?;
                    Some((nl.link_port_x(), nl.link_port_bottom_y(), None))
                }
                EndpointRole::Downstream => {
                    // Target is child node, top center.
                    let nl = find_node_layout(node_layouts, *child)?;
                    Some((nl.link_port_x(), nl.link_port_top_y(), None))
                }
            }
        }
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let side = port_sides.get(&(edge_id, role)).copied();
            match role {
                EndpointRole::Upstream => {
                    let node_id = prop_node(graph, *source_prop);
                    let nl = find_node_layout(node_layouts, node_id)?;
                    let node = &graph.nodes[node_id.index()];
                    let prop_idx = node
                        .properties
                        .iter()
                        .position(|&pid| pid == *source_prop)?;
                    let x = match side {
                        Some(PortSide::Left) => nl.port_left_x(),
                        Some(PortSide::Right) | None => nl.port_right_x(),
                    };
                    let y = nl.port_y(prop_idx);
                    Some((x, y, side))
                }
                EndpointRole::Downstream => {
                    let node_id = prop_node(graph, *dest_prop);
                    let nl = find_node_layout(node_layouts, node_id)?;
                    let node = &graph.nodes[node_id.index()];
                    let prop_idx = node
                        .properties
                        .iter()
                        .position(|&pid| pid == *dest_prop)?;
                    let x = match side {
                        Some(PortSide::Left) => nl.port_left_x(),
                        Some(PortSide::Right) | None => nl.port_right_x(),
                    };
                    let y = nl.port_y(prop_idx);
                    Some((x, y, side))
                }
            }
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
        } => {
            let side = port_sides.get(&(edge_id, role)).copied();
            match role {
                EndpointRole::Upstream => {
                    let node_id = prop_node(graph, *source_prop);
                    let nl = find_node_layout(node_layouts, node_id)?;
                    let node = &graph.nodes[node_id.index()];
                    let prop_idx = node
                        .properties
                        .iter()
                        .position(|&pid| pid == *source_prop)?;
                    let x = match side {
                        Some(PortSide::Left) => nl.port_left_x(),
                        Some(PortSide::Right) | None => nl.port_right_x(),
                    };
                    let y = nl.port_y(prop_idx);
                    Some((x, y, side))
                }
                EndpointRole::Downstream => {
                    // Downstream of DerivInput connects to derivation center.
                    let dl = find_deriv_layout(deriv_layouts, *target_deriv)?;
                    let x = dl.x + dl.width / 2.0;
                    let y = dl.y;
                    Some((x, y, None))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Single edge routing
// ---------------------------------------------------------------------------

/// Find the nearest vertical channel in the given direction from `from_x`.
fn find_nearest_v_channel(
    v_channels: &[Channel],
    from_x: f64,
    side: Option<PortSide>,
) -> Option<usize> {
    match side {
        Some(PortSide::Left) => {
            // Search leftward: find channel with position <= from_x, closest.
            v_channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.position <= from_x)
                .min_by(|(_, a), (_, b)| {
                    let da = (a.position - from_x).abs();
                    let db = (b.position - from_x).abs();
                    da.partial_cmp(&db).unwrap()
                })
                .map(|(i, _)| i)
        }
        Some(PortSide::Right) => {
            // Search rightward: find channel with position >= from_x, closest.
            v_channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.position >= from_x)
                .min_by(|(_, a), (_, b)| {
                    let da = (a.position - from_x).abs();
                    let db = (b.position - from_x).abs();
                    da.partial_cmp(&db).unwrap()
                })
                .map(|(i, _)| i)
        }
        None => {
            // No side preference: find nearest channel.
            v_channels
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    let da = (a.position - from_x).abs();
                    let db = (b.position - from_x).abs();
                    da.partial_cmp(&db).unwrap()
                })
                .map(|(i, _)| i)
        }
    }
}

/// Find a horizontal channel between two y-positions.
fn find_h_channel_between(h_channels: &[Channel], src_y: f64, tgt_y: f64) -> Option<usize> {
    let (min_y, max_y) = if src_y < tgt_y {
        (src_y, tgt_y)
    } else {
        (tgt_y, src_y)
    };

    h_channels
        .iter()
        .enumerate()
        .filter(|(_, ch)| ch.position > min_y && ch.position < max_y)
        .min_by(|(_, a), (_, b)| {
            // Prefer the channel closest to the midpoint.
            let mid = (min_y + max_y) / 2.0;
            let da = (a.position - mid).abs();
            let db = (b.position - mid).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
}

/// Route a single edge. Returns the segments for the route.
fn route_single_edge(
    edge_id: EdgeId,
    src_x: f64,
    src_y: f64,
    src_side: Option<PortSide>,
    tgt_x: f64,
    tgt_y: f64,
    tgt_side: Option<PortSide>,
    h_channels: &mut Vec<Channel>,
    v_channels: &mut Vec<Channel>,
) -> Vec<Segment> {
    // Case 1: Center-port edges (Links) -- no side assignment
    if src_side.is_none() && tgt_side.is_none() {
        if (src_x - tgt_x).abs() < 0.5 {
            // Straight vertical drop.
            return vec![Segment::Vertical {
                x: src_x,
                y_start: src_y,
                y_end: tgt_y,
            }];
        } else {
            // V-H-V: three segments.
            // Find a horizontal channel between src and tgt.
            let h_y = if let Some(hi) = find_h_channel_between(h_channels, src_y, tgt_y) {
                h_channels[hi].reserve(edge_id)
            } else {
                // Fallback: midpoint.
                (src_y + tgt_y) / 2.0
            };

            return vec![
                Segment::Vertical {
                    x: src_x,
                    y_start: src_y,
                    y_end: h_y,
                },
                Segment::Horizontal {
                    y: h_y,
                    x_start: src_x,
                    x_end: tgt_x,
                },
                Segment::Vertical {
                    x: tgt_x,
                    y_start: h_y,
                    y_end: tgt_y,
                },
            ];
        }
    }

    // Case 2a: Intra-column constraint — both Left ports, same X → clean H-V-H bracket.
    if src_side == Some(PortSide::Left)
        && tgt_side == Some(PortSide::Left)
        && (src_x - tgt_x).abs() < 0.5
    {
        let v_x = if let Some(vi) = find_nearest_v_channel(v_channels, src_x, src_side) {
            v_channels[vi].reserve(edge_id)
        } else {
            src_x - super::NODE_H_SPACING / 2.0
        };
        return collapse_zero_length(vec![
            Segment::Horizontal {
                y: src_y,
                x_start: src_x,
                x_end: v_x,
            },
            Segment::Vertical {
                x: v_x,
                y_start: src_y,
                y_end: tgt_y,
            },
            Segment::Horizontal {
                y: tgt_y,
                x_start: v_x,
                x_end: tgt_x,
            },
        ]);
    }

    // Case 2: Property-port edges -- up to 5 segments (H-V-H-V-H)
    let v1_x = if let Some(vi) = find_nearest_v_channel(v_channels, src_x, src_side) {
        v_channels[vi].reserve(edge_id)
    } else {
        // Fallback: offset from src based on side.
        match src_side {
            Some(PortSide::Left) => src_x - super::NODE_H_SPACING / 2.0,
            _ => src_x + super::NODE_H_SPACING / 2.0,
        }
    };

    let h_y = if let Some(hi) = find_h_channel_between(h_channels, src_y, tgt_y) {
        h_channels[hi].reserve(edge_id)
    } else {
        // Fallback: midpoint.
        (src_y + tgt_y) / 2.0
    };

    let v2_x = if tgt_side.is_some() {
        if let Some(vi) = find_nearest_v_channel(v_channels, tgt_x, tgt_side) {
            v_channels[vi].reserve(edge_id)
        } else {
            match tgt_side {
                Some(PortSide::Left) => tgt_x - super::NODE_H_SPACING / 2.0,
                _ => tgt_x + super::NODE_H_SPACING / 2.0,
            }
        }
    } else {
        // Target has no side (e.g., derivation center): go straight to tgt_x.
        tgt_x
    };

    let segments = vec![
        Segment::Horizontal {
            y: src_y,
            x_start: src_x,
            x_end: v1_x,
        },
        Segment::Vertical {
            x: v1_x,
            y_start: src_y,
            y_end: h_y,
        },
        Segment::Horizontal {
            y: h_y,
            x_start: v1_x,
            x_end: v2_x,
        },
        Segment::Vertical {
            x: v2_x,
            y_start: h_y,
            y_end: tgt_y,
        },
        Segment::Horizontal {
            y: tgt_y,
            x_start: v2_x,
            x_end: tgt_x,
        },
    ];

    collapse_zero_length(segments)
}

/// Remove segments with zero length.
fn collapse_zero_length(segments: Vec<Segment>) -> Vec<Segment> {
    segments
        .into_iter()
        .filter(|seg| seg.length() > 0.001) // tolerance for floating-point
        .collect()
}

// ---------------------------------------------------------------------------
// Edge priority for routing order
// ---------------------------------------------------------------------------

/// Routing priority: lower number = routed first = gets best channels.
fn edge_priority(edge: &Edge) -> u32 {
    match edge {
        Edge::Anchor { .. } => 0,
        Edge::DerivInput { .. } => 1,
        Edge::Constraint { .. } => 2,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Route all edges using orthogonal channel-based routing.
///
/// Edges are routed in priority order (links first, then deriv inputs, then
/// constraints). Each route reserves channels so later edges are offset.
pub fn route_all_edges(
    graph: &Graph,
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    port_sides: &PortSideAssignment,
) -> Vec<Route> {
    let mut h_channels = build_h_channels(node_layouts, deriv_layouts);
    let mut v_channels = build_v_channels(node_layouts, deriv_layouts);

    // Build a priority-sorted list of edge indices.
    let mut edge_indices: Vec<usize> = (0..graph.edges.len()).collect();
    edge_indices.sort_by_key(|&i| edge_priority(&graph.edges[i]));

    let mut routes = Vec::new();

    for idx in edge_indices {
        let edge_id = EdgeId(idx as u32);

        let src = port_position(
            graph,
            edge_id,
            EndpointRole::Upstream,
            node_layouts,
            deriv_layouts,
            port_sides,
        );
        let tgt = port_position(
            graph,
            edge_id,
            EndpointRole::Downstream,
            node_layouts,
            deriv_layouts,
            port_sides,
        );

        let (src_x, src_y, src_side) = match src {
            Some(s) => s,
            None => continue,
        };
        let (tgt_x, tgt_y, tgt_side) = match tgt {
            Some(t) => t,
            None => continue,
        };

        let segments = route_single_edge(
            edge_id,
            src_x,
            src_y,
            src_side,
            tgt_x,
            tgt_y,
            tgt_side,
            &mut h_channels,
            &mut v_channels,
        );

        routes.push(Route { edge_id, segments });
    }

    routes
}

/// Convert a Route to an SVG path string.
///
/// The first segment starts with `M` (moveto), subsequent segments use `L` (lineto).
pub fn route_to_svg_path(route: &Route) -> String {
    if route.segments.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();

    for (i, seg) in route.segments.iter().enumerate() {
        let (sx, sy) = seg.start();
        let (ex, ey) = seg.end();

        if i == 0 {
            parts.push(format!("M{},{}", sx, sy));
        }
        parts.push(format!("L{},{}", ex, ey));
    }

    parts.join(" ")
}

/// Generate a stub route from the first STUB_LENGTH pixels of a full route.
///
/// Walks along the route's segments consuming up to STUB_LENGTH total distance,
/// truncating the last segment as needed.
pub fn generate_stub(route: &Route) -> Route {
    let mut remaining = STUB_LENGTH;
    let mut stub_segments = Vec::new();

    for seg in &route.segments {
        let len = seg.length();

        if len <= remaining + 0.001 {
            // Include the full segment.
            stub_segments.push(seg.clone());
            remaining -= len;
            if remaining < 0.001 {
                break;
            }
        } else {
            // Truncate this segment.
            let fraction = remaining / len;
            match seg {
                Segment::Horizontal { y, x_start, x_end } => {
                    let new_x_end = x_start + (x_end - x_start) * fraction;
                    stub_segments.push(Segment::Horizontal {
                        y: *y,
                        x_start: *x_start,
                        x_end: new_x_end,
                    });
                }
                Segment::Vertical { x, y_start, y_end } => {
                    let new_y_end = y_start + (y_end - y_start) * fraction;
                    stub_segments.push(Segment::Vertical {
                        x: *x,
                        y_start: *y_start,
                        y_end: new_y_end,
                    });
                }
            }
            break;
        }
    }

    Route {
        edge_id: route.edge_id,
        segments: stub_segments,
    }
}

/// Returns the point at the midpoint (by arc length) of a route as (x, y).
pub fn route_midpoint(route: &Route) -> (f64, f64) {
    let total: f64 = route.segments.iter().map(|s| s.length()).sum();
    if total < 1e-9 {
        return route.segments.first().map(|s| s.start()).unwrap_or((0.0, 0.0));
    }
    let mut remaining = total / 2.0;
    for seg in &route.segments {
        let len = seg.length();
        if remaining <= len {
            let frac = remaining / len;
            let (sx, sy) = seg.start();
            let (ex, ey) = seg.end();
            return (sx + (ex - sx) * frac, sy + (ey - sy) * frac);
        }
        remaining -= len;
    }
    route.segments.last().map(|s| s.end()).unwrap_or((0.0, 0.0))
}

/// Convert a Route to an EdgePath.  If `label_text` is Some, an EdgeLabel is
/// placed at the arc-length midpoint of the route, offset 2px above.
pub fn route_to_edge_path(route: &Route, label_text: Option<String>) -> EdgePath {
    let label = label_text.map(|text| {
        let (x, y) = route_midpoint(route);
        EdgeLabel { text, x, y: y - 2.0, anchor: "middle" }
    });
    EdgePath {
        edge_id: route.edge_id,
        svg_path: route_to_svg_path(route),
        label,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::model::types::*;

    /// Helper: create a minimal graph with two nodes, two properties, one link, one constraint.
    fn test_graph() -> Graph {
        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: "A".into(),
                display_name: None,
                properties: vec![PropId(0)],
                domain: None,
                is_root: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: "B".into(),
                display_name: None,
                properties: vec![PropId(1)],
                domain: None,
                is_root: false,
                is_selected: false,
            },
        ];

        let properties = vec![
            Property {
                id: PropId(0),
                node: NodeId(0),
                name: "prop_a".into(),
                critical: true, constrained: false,
            },
            Property {
                id: PropId(1),
                node: NodeId(1),
                name: "prop_b".into(),
                critical: true, constrained: false,
            },
        ];

        let edges = vec![
            // Edge 0: Link A -> B
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            // Edge 1: Constraint prop_a -> prop_b
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(1),
                operation: None,
            },
        ];

        let mut prop_edges = HashMap::new();
        prop_edges.insert(PropId(0), vec![EdgeId(1)]);
        prop_edges.insert(PropId(1), vec![EdgeId(1)]);

        let mut node_children = HashMap::new();
        node_children.insert(NodeId(0), vec![EdgeId(0)]);

        let mut node_parent = HashMap::new();
        node_parent.insert(NodeId(1), EdgeId(0));

        Graph {
            nodes,
            properties,
            derivations: vec![],
            edges,
            domains: vec![],
            prop_edges,
            node_children,
            node_parent,
        }
    }

    fn test_node_layouts() -> Vec<NodeLayout> {
        vec![
            // Node A: at (0, 0), width=80, height=52 (header 28 + 1 prop * 24)
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            // Node B: at (120, 100), width=80, height=52
            NodeLayout {
                id: NodeId(1),
                x: 120.0,
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ]
    }

    // -----------------------------------------------------------------------
    // Test 1: Straight vertical link
    // -----------------------------------------------------------------------

    #[test]
    fn test_straight_vertical_link() {
        let graph = test_graph();

        // Place B directly below A, same x.
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 0.0,
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ];
        let deriv_layouts: Vec<DerivLayout> = vec![];
        let port_sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &port_sides);

        // Find the link route (edge 0).
        let link_route = routes.iter().find(|r| r.edge_id == EdgeId(0)).unwrap();

        // Should be a single vertical segment.
        assert_eq!(link_route.segments.len(), 1);
        match &link_route.segments[0] {
            Segment::Vertical { x, y_start, y_end } => {
                // Link port x = node.x + width/2 = 0 + 40 = 40
                assert!((x - 40.0).abs() < 0.1, "x should be 40.0, got {}", x);
                // y_start = bottom of A = 0 + 52 = 52
                assert!(
                    (y_start - 52.0).abs() < 0.1,
                    "y_start should be 52.0, got {}",
                    y_start
                );
                // y_end = top of B = 100
                assert!(
                    (y_end - 100.0).abs() < 0.1,
                    "y_end should be 100.0, got {}",
                    y_end
                );
            }
            other => panic!("Expected Vertical segment, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 2: L-shaped constraint route
    // -----------------------------------------------------------------------

    #[test]
    fn test_l_shaped_constraint_route() {
        // Create a graph where the constraint should produce a route
        // that has at least horizontal and vertical components.
        let graph = test_graph();
        let node_layouts = test_node_layouts();
        let deriv_layouts: Vec<DerivLayout> = vec![];
        let port_sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &port_sides);

        // Find the constraint route (edge 1).
        let constraint_route = routes.iter().find(|r| r.edge_id == EdgeId(1)).unwrap();

        // Should have multiple segments (the property-port routing creates H-V-H-V-H).
        assert!(
            constraint_route.segments.len() >= 2,
            "Constraint route should have >= 2 segments, got {}",
            constraint_route.segments.len()
        );

        // Verify all segments are either horizontal or vertical (orthogonal).
        for seg in &constraint_route.segments {
            match seg {
                Segment::Horizontal { .. } | Segment::Vertical { .. } => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test 3: Z-shaped route with horizontal channel
    // -----------------------------------------------------------------------

    #[test]
    fn test_z_shaped_route_link() {
        let graph = test_graph();

        // Place B offset to the right and below A.
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 200.0,
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ];
        let deriv_layouts: Vec<DerivLayout> = vec![];
        let port_sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &port_sides);

        // Find the link route (edge 0).
        let link_route = routes.iter().find(|r| r.edge_id == EdgeId(0)).unwrap();

        // Should be V-H-V (3 segments): Z shape.
        assert_eq!(
            link_route.segments.len(),
            3,
            "Z-shaped link should have 3 segments, got {:?}",
            link_route.segments
        );

        // First segment: vertical from parent bottom center.
        match &link_route.segments[0] {
            Segment::Vertical { x, y_start, .. } => {
                assert!((x - 40.0).abs() < 0.1); // A center x
                assert!((y_start - 52.0).abs() < 0.1); // A bottom y
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }

        // Second segment: horizontal.
        match &link_route.segments[1] {
            Segment::Horizontal { x_start, x_end, .. } => {
                assert!((x_start - 40.0).abs() < 0.1); // from A center
                assert!((x_end - 240.0).abs() < 0.1); // to B center (200 + 80/2)
            }
            other => panic!("Expected Horizontal, got {:?}", other),
        }

        // Third segment: vertical to child top center.
        match &link_route.segments[2] {
            Segment::Vertical { x, y_end, .. } => {
                assert!((x - 240.0).abs() < 0.1); // B center x
                assert!((y_end - 100.0).abs() < 0.1); // B top y
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 4: Port side assignment
    // -----------------------------------------------------------------------

    #[test]
    fn test_port_side_assignment_left_right() {
        let graph = test_graph();
        let node_layouts = test_node_layouts(); // A at x=0, B at x=120

        let deriv_layouts: Vec<DerivLayout> = vec![];
        let sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        // Edge 0 is a Link: no side assignments.
        assert!(!sides.contains_key(&(EdgeId(0), EndpointRole::Upstream)));
        assert!(!sides.contains_key(&(EdgeId(0), EndpointRole::Downstream)));

        // Edge 1 is a Constraint from A.prop_a -> B.prop_b
        // A center = 0 + 40 = 40, B center = 120 + 40 = 160
        // src_cx (40) < tgt_cx (160), so:
        //   Upstream = Right, Downstream = Left
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Upstream)],
            PortSide::Right,
            "Source node is to the left, so upstream port should be Right"
        );
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Downstream)],
            PortSide::Left,
            "Target node is to the right, so downstream port should be Left"
        );
    }

    #[test]
    fn test_port_side_assignment_same_center() {
        let graph = test_graph();

        // Both nodes at the same x (same center).
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 0.0,
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ];
        let deriv_layouts: Vec<DerivLayout> = vec![];
        let sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        // Same center x: both Left (intra-column bracket routing).
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Upstream)],
            PortSide::Left
        );
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Downstream)],
            PortSide::Left
        );
    }

    #[test]
    fn test_port_side_assignment_self_loop() {
        // Create a constraint from a property on A back to another property on A.
        let nodes = vec![Node {
            id: NodeId(0),
            ident: "A".into(),
            display_name: None,
            properties: vec![PropId(0), PropId(1)],
            domain: None,
            is_root: true,
            is_selected: false,
        }];

        let properties = vec![
            Property {
                id: PropId(0),
                node: NodeId(0),
                name: "p1".into(),
                critical: true, constrained: false,
            },
            Property {
                id: PropId(1),
                node: NodeId(0),
                name: "p2".into(),
                critical: true, constrained: false,
            },
        ];

        let edges = vec![Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        }];

        let mut prop_edges = HashMap::new();
        prop_edges.insert(PropId(0), vec![EdgeId(0)]);
        prop_edges.insert(PropId(1), vec![EdgeId(0)]);

        let graph = Graph {
            nodes,
            properties,
            derivations: vec![],
            edges,
            domains: vec![],
            prop_edges,
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        };

        let node_layouts = vec![NodeLayout {
            id: NodeId(0),
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 76.0, // header + 2 props
        }];

        let deriv_layouts: Vec<DerivLayout> = vec![];
        let sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        // Self-loop: Upstream=Left, Downstream=Right.
        assert_eq!(
            sides[&(EdgeId(0), EndpointRole::Upstream)],
            PortSide::Left
        );
        assert_eq!(
            sides[&(EdgeId(0), EndpointRole::Downstream)],
            PortSide::Right
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: SVG path generation
    // -----------------------------------------------------------------------

    #[test]
    fn test_svg_path_generation() {
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![
                Segment::Vertical {
                    x: 10.0,
                    y_start: 20.0,
                    y_end: 50.0,
                },
                Segment::Horizontal {
                    y: 50.0,
                    x_start: 10.0,
                    x_end: 30.0,
                },
            ],
        };

        let path = route_to_svg_path(&route);
        assert_eq!(path, "M10,20 L10,50 L30,50");
    }

    #[test]
    fn test_svg_path_generation_single_segment() {
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![Segment::Vertical {
                x: 40.0,
                y_start: 52.0,
                y_end: 100.0,
            }],
        };

        let path = route_to_svg_path(&route);
        assert_eq!(path, "M40,52 L40,100");
    }

    #[test]
    fn test_svg_path_generation_empty() {
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![],
        };

        let path = route_to_svg_path(&route);
        assert_eq!(path, "");
    }

    // -----------------------------------------------------------------------
    // Test 6: Stub generation
    // -----------------------------------------------------------------------

    #[test]
    fn test_stub_generation_truncates() {
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![Segment::Vertical {
                x: 40.0,
                y_start: 0.0,
                y_end: 100.0,
            }],
        };

        let stub = generate_stub(&route);

        assert_eq!(stub.segments.len(), 1);
        match &stub.segments[0] {
            Segment::Vertical { x, y_start, y_end } => {
                assert!((x - 40.0).abs() < 0.01);
                assert!((y_start - 0.0).abs() < 0.01);
                assert!(
                    (y_end - STUB_LENGTH).abs() < 0.01,
                    "Stub should truncate to STUB_LENGTH={}, got {}",
                    STUB_LENGTH,
                    y_end
                );
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_generation_short_route() {
        // A route shorter than STUB_LENGTH should be preserved entirely.
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![Segment::Vertical {
                x: 40.0,
                y_start: 0.0,
                y_end: 10.0,
            }],
        };

        let stub = generate_stub(&route);

        assert_eq!(stub.segments.len(), 1);
        match &stub.segments[0] {
            Segment::Vertical { y_end, .. } => {
                assert!((y_end - 10.0).abs() < 0.01);
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_generation_multi_segment() {
        // Two segments: first is STUB_LENGTH/2 px, second is STUB_LENGTH*3 px.
        // Stub should take all of the first and STUB_LENGTH/2 px of the second.
        let seg1_len = STUB_LENGTH / 2.0;
        let route = Route {
            edge_id: EdgeId(0),
            segments: vec![
                Segment::Vertical {
                    x: 40.0,
                    y_start: 0.0,
                    y_end: seg1_len,
                },
                Segment::Horizontal {
                    y: seg1_len,
                    x_start: 40.0,
                    x_end: 40.0 + STUB_LENGTH * 3.0,
                },
            ],
        };

        let stub = generate_stub(&route);

        assert_eq!(stub.segments.len(), 2);

        // First segment: full seg1_len px vertical.
        match &stub.segments[0] {
            Segment::Vertical { y_start, y_end, .. } => {
                assert!((y_start - 0.0).abs() < 0.01);
                assert!((y_end - seg1_len).abs() < 0.01);
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }

        // Second segment: truncated horizontal, STUB_LENGTH/2 px out of STUB_LENGTH*3.
        let expected_x_end = 40.0 + (STUB_LENGTH - seg1_len);
        match &stub.segments[1] {
            Segment::Horizontal {
                y,
                x_start,
                x_end,
            } => {
                assert!((y - seg1_len).abs() < 0.01);
                assert!((x_start - 40.0).abs() < 0.01);
                assert!(
                    (x_end - expected_x_end).abs() < 0.01,
                    "Should go {}px into the long segment, got x_end={}",
                    STUB_LENGTH - seg1_len,
                    x_end
                );
            }
            other => panic!("Expected Horizontal, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test: collapse zero-length segments
    // -----------------------------------------------------------------------

    #[test]
    fn test_collapse_zero_length_segments() {
        let segments = vec![
            Segment::Horizontal {
                y: 10.0,
                x_start: 5.0,
                x_end: 5.0,
            }, // zero length
            Segment::Vertical {
                x: 5.0,
                y_start: 10.0,
                y_end: 30.0,
            }, // 20 length
            Segment::Horizontal {
                y: 30.0,
                x_start: 5.0,
                x_end: 5.0,
            }, // zero length
        ];

        let collapsed = collapse_zero_length(segments);
        assert_eq!(collapsed.len(), 1);
        match &collapsed[0] {
            Segment::Vertical {
                x, y_start, y_end, ..
            } => {
                assert!((x - 5.0).abs() < 0.01);
                assert!((y_start - 10.0).abs() < 0.01);
                assert!((y_end - 30.0).abs() < 0.01);
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test: route_to_edge_path convenience wrapper
    // -----------------------------------------------------------------------

    #[test]
    fn test_route_to_edge_path() {
        let route = Route {
            edge_id: EdgeId(42),
            segments: vec![Segment::Vertical {
                x: 10.0,
                y_start: 0.0,
                y_end: 50.0,
            }],
        };

        let edge_path = route_to_edge_path(&route, None);
        assert_eq!(edge_path.edge_id, EdgeId(42));
        assert_eq!(edge_path.svg_path, "M10,0 L10,50");
    }

    // -----------------------------------------------------------------------
    // Test: channel construction
    // -----------------------------------------------------------------------

    #[test]
    fn test_h_channels_between_layers() {
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 0.0,
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ];

        let h_channels = build_h_channels(&node_layouts, &[]);

        // Should have exactly one horizontal channel between the two layers.
        assert_eq!(h_channels.len(), 1);
        // Position should be midway between bottom of layer 0 (52) and top of layer 1 (100).
        assert!(
            (h_channels[0].position - 76.0).abs() < 0.1,
            "Channel position should be 76.0, got {}",
            h_channels[0].position
        );
    }

    #[test]
    fn test_v_channels_between_nodes() {
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 120.0,
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
        ];

        let v_channels = build_v_channels(&node_layouts, &[]);

        // Should have 3 vertical channels: left of A, between A and B, right of B.
        assert_eq!(v_channels.len(), 3);

        // Middle channel: between right edge of A (80) and left edge of B (120).
        let mid_chan = &v_channels[1];
        assert!(
            (mid_chan.position - 100.0).abs() < 0.1,
            "Mid channel should be at 100.0, got {}",
            mid_chan.position
        );
    }
}
