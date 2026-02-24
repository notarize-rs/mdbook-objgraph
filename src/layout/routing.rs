/// Orthogonal channel-based edge routing (DESIGN.md §4.2.6).

use crate::model::types::{DomainId, Edge, EdgeId, Graph, NodeId, PropId};

use super::{
    layout_endpoints, DerivLayout, DomainLayout, EdgeLabel, EdgePath, EndpointRole,
    LayoutEndpoint, NodeLayout, PortSide, CHANNEL_GAP, CORRIDOR_PAD, STUB_LENGTH,
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
        // Center the group: offset = (i - (n-1)/2) * CORRIDOR_PAD
        // For a new occupant being added at index `occupant_index`:
        let _offset = (n as f64 - 0.0) * CORRIDOR_PAD;
        // Simple scheme: first occupant at center, subsequent ones offset alternately
        if n == 0 {
            self.position
        } else if n % 2 == 1 {
            self.position + ((n as f64 + 1.0) / 2.0).ceil() * CORRIDOR_PAD
        } else {
            self.position - (n as f64 / 2.0) * CORRIDOR_PAD
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
// Corridor model (LAYOUT.md §4.2.6)
// ---------------------------------------------------------------------------

/// A vertical channel within a corridor, tracking occupant vertical extents
/// so that edges sharing a channel don't overlap vertically.
#[derive(Debug, Clone)]
pub struct CorridorChannel {
    pub x: f64,
    pub occupants: Vec<(EdgeId, f64, f64)>, // (edge, y_start, y_end)
}

impl CorridorChannel {
    /// Check whether a new vertical extent overlaps any existing occupant.
    fn overlaps(&self, y_start: f64, y_end: f64) -> bool {
        let (lo, hi) = if y_start <= y_end {
            (y_start, y_end)
        } else {
            (y_end, y_start)
        };
        self.occupants.iter().any(|&(_, os, oe)| {
            let (olo, ohi) = if os <= oe { (os, oe) } else { (oe, os) };
            !(hi < olo || lo > ohi)
        })
    }

    /// Reserve this channel for an edge and return the channel x.
    fn reserve(&mut self, edge_id: EdgeId, y_start: f64, y_end: f64) -> f64 {
        self.occupants.push((edge_id, y_start, y_end));
        self.x
    }
}

/// A corridor — a fixed-width zone between node edges and domain boundaries.
#[derive(Debug, Clone)]
pub struct Corridor {
    pub x_start: f64,
    pub x_end: f64,
    pub channels: Vec<CorridorChannel>,
    /// Domain IDs this corridor belongs to. Empty for inter-domain corridors.
    /// Multiple IDs when same-column domains share the same corridor x-range.
    pub domain_ids: Vec<DomainId>,
}

impl Corridor {
    /// Allocate a channel with no vertical overlap for the given edge extent.
    /// If all existing channels overlap, create a new one.
    fn allocate_channel(&mut self, edge_id: EdgeId, y_start: f64, y_end: f64) -> f64 {
        // Try existing channels (non-overlapping extents can share).
        for ch in &mut self.channels {
            if !ch.overlaps(y_start, y_end) {
                return ch.reserve(edge_id, y_start, y_end);
            }
        }
        // All channels overlap — create a new one with CHANNEL_GAP spacing.
        let new_x = if self.channels.is_empty() {
            self.x_start + CORRIDOR_PAD
        } else {
            self.channels.last().unwrap().x + CHANNEL_GAP
        };
        let mut ch = CorridorChannel {
            x: new_x,
            occupants: Vec::new(),
        };
        let x = ch.reserve(edge_id, y_start, y_end);
        self.channels.push(ch);
        x
    }

    /// The center x of this corridor.
    fn center_x(&self) -> f64 {
        (self.x_start + self.x_end) / 2.0
    }
}

/// Build corridors from domain bounding boxes.
///
/// Creates three kinds of corridors:
/// 1. Per-domain intra-domain corridors (left/right edges of each domain).
/// 2. Inter-column gap corridors between columns of domains.
/// 3. Outer corridors on the left and right edges of the entire layout.
///
/// The outer and inter-column corridors have empty `domain_ids` and are used
/// exclusively by cross-domain edges.
fn build_corridors(domain_layouts: &[DomainLayout]) -> Vec<Corridor> {
    let mut corridors = Vec::new();

    // Sort domains by x for inter-domain corridor detection.
    let mut sorted_domains: Vec<&DomainLayout> = domain_layouts.iter().collect();
    sorted_domains.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());

    for dl in &sorted_domains {
        let corridor_width = CORRIDOR_PAD * 2.0;

        let left_x_start = dl.x;
        let left_x_end = dl.x + corridor_width;
        let right_x_start = dl.x + dl.width - corridor_width;
        let right_x_end = dl.x + dl.width;

        // Merge with existing corridor at same x-range, or create new.
        merge_or_create_corridor(&mut corridors, left_x_start, left_x_end, dl.id);
        merge_or_create_corridor(&mut corridors, right_x_start, right_x_end, dl.id);
    }

    // Inter-column gap corridors between adjacent domains at different x-ranges.
    let mut seen_gaps: Vec<(f64, f64)> = Vec::new();
    for w in sorted_domains.windows(2) {
        let d1_right = w[0].x + w[0].width;
        let d2_left = w[1].x;
        let gap = d2_left - d1_right;
        if gap > 0.5 {
            // Avoid duplicate gap corridors for domains at same x.
            let key = (d1_right, d2_left);
            if seen_gaps.iter().any(|(a, b)| (a - key.0).abs() < 0.5 && (b - key.1).abs() < 0.5) {
                continue;
            }
            seen_gaps.push(key);
            corridors.push(Corridor {
                x_start: d1_right,
                x_end: d2_left,
                channels: vec![CorridorChannel {
                    x: d1_right + CORRIDOR_PAD,
                    occupants: Vec::new(),
                }],
                domain_ids: vec![],
            });
        }
    }

    // Outer corridors: left edge (x=0 to leftmost domain) and right edge
    // (rightmost domain to rightmost domain + outer width).
    if let (Some(leftmost), Some(rightmost)) = (sorted_domains.first(), sorted_domains.last()) {
        let left_edge = leftmost.x;
        if left_edge > CORRIDOR_PAD + 0.5 {
            // There's space for an outer left corridor.
            corridors.push(Corridor {
                x_start: 0.0,
                x_end: left_edge,
                channels: vec![CorridorChannel {
                    x: CORRIDOR_PAD,
                    occupants: Vec::new(),
                }],
                domain_ids: vec![],
            });
        }

        let right_edge = rightmost.x + rightmost.width;
        let outer_width = CORRIDOR_PAD * 2.0;
        corridors.push(Corridor {
            x_start: right_edge,
            x_end: right_edge + outer_width,
            channels: vec![CorridorChannel {
                x: right_edge + CORRIDOR_PAD,
                occupants: Vec::new(),
            }],
            domain_ids: vec![],
        });
    }

    corridors
}

/// Merge a domain into an existing corridor at the same x-range, or create one.
fn merge_or_create_corridor(
    corridors: &mut Vec<Corridor>,
    x_start: f64,
    x_end: f64,
    domain_id: DomainId,
) {
    // Check if a corridor already exists at this x-range.
    for c in corridors.iter_mut() {
        if (c.x_start - x_start).abs() < 0.5 && (c.x_end - x_end).abs() < 0.5 {
            if !c.domain_ids.contains(&domain_id) {
                c.domain_ids.push(domain_id);
            }
            return;
        }
    }
    // Create new corridor.
    let center_x = (x_start + x_end) / 2.0;
    corridors.push(Corridor {
        x_start,
        x_end,
        channels: vec![CorridorChannel {
            x: center_x,
            occupants: Vec::new(),
        }],
        domain_ids: vec![domain_id],
    });
}

/// Find the index of the nearest corridor in the direction the port faces.
///
/// Returns `None` if no matching corridor is found.
fn find_best_corridor_idx(
    port_x: f64,
    port_side: PortSide,
    corridors: &[Corridor],
    edge_domain: Option<DomainId>,
) -> Option<usize> {
    let domain_matches = |c: &Corridor| -> bool {
        match edge_domain {
            Some(did) => c.domain_ids.contains(&did),
            None => c.domain_ids.is_empty(),
        }
    };

    match port_side {
        PortSide::Left => corridors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.center_x() <= port_x && domain_matches(c))
            .min_by(|(_, a), (_, b)| {
                let da = (a.center_x() - port_x).abs();
                let db = (b.center_x() - port_x).abs();
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i),
        PortSide::Right => corridors
            .iter()
            .enumerate()
            .filter(|(_, c)| c.center_x() >= port_x && domain_matches(c))
            .min_by(|(_, a), (_, b)| {
                let da = (a.center_x() - port_x).abs();
                let db = (b.center_x() - port_x).abs();
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i),
    }
}

/// Find the nearest corridor in the direction the port faces and allocate a channel.
///
/// `edge_domain` constrains corridor selection: `Some(did)` means prefer only
/// corridors belonging to that domain (for intra-domain edges); `None` means
/// any corridor is acceptable (for cross-domain edges).
fn find_corridor_channel(
    port_x: f64,
    port_side: PortSide,
    corridors: &mut Vec<Corridor>,
    edge_id: EdgeId,
    y_start: f64,
    y_end: f64,
    edge_domain: Option<DomainId>,
) -> f64 {
    match find_best_corridor_idx(port_x, port_side, corridors, edge_domain) {
        Some(idx) => corridors[idx].allocate_channel(edge_id, y_start, y_end),
        None => {
            // Fallback: no corridor found, use offset from port.
            match port_side {
                PortSide::Left => port_x - CORRIDOR_PAD,
                PortSide::Right => port_x + CORRIDOR_PAD,
            }
        }
    }
}

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
    let mut same_col_counter: usize = 0;

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
            // Self-loop: route through right corridor (matching mockup).
            sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
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
                // Same center x: alternate between left and right corridors.
                let side = if same_col_counter % 2 == 0 {
                    PortSide::Right
                } else {
                    PortSide::Left
                };
                same_col_counter += 1;
                sides.insert((edge_id, EndpointRole::Upstream), side);
                sides.insert((edge_id, EndpointRole::Downstream), side);
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

// ---------------------------------------------------------------------------
// Port position computation
// ---------------------------------------------------------------------------

/// Tracks per-(PropId, PortSide) connection counts and assignment indices
/// for distributed port placement.
struct PortDistributor {
    /// Total connections per (PropId, PortSide).
    counts: std::collections::HashMap<(PropId, PortSide), usize>,
    /// Next assignment index per (PropId, PortSide).
    next_index: std::collections::HashMap<(PropId, PortSide), usize>,
}

impl PortDistributor {
    /// Build a distributor by counting all property-side connections from edges.
    fn new(graph: &Graph, port_sides: &PortSideAssignment) -> Self {
        let mut counts: std::collections::HashMap<(PropId, PortSide), usize> =
            std::collections::HashMap::new();

        for (idx, edge) in graph.edges.iter().enumerate() {
            let edge_id = EdgeId(idx as u32);
            match edge {
                Edge::Constraint { source_prop, dest_prop, .. } => {
                    if let Some(&side) = port_sides.get(&(edge_id, EndpointRole::Upstream)) {
                        *counts.entry((*source_prop, side)).or_insert(0) += 1;
                    }
                    if let Some(&side) = port_sides.get(&(edge_id, EndpointRole::Downstream)) {
                        *counts.entry((*dest_prop, side)).or_insert(0) += 1;
                    }
                }
                Edge::DerivInput { source_prop, .. } => {
                    if let Some(&side) = port_sides.get(&(edge_id, EndpointRole::Upstream)) {
                        *counts.entry((*source_prop, side)).or_insert(0) += 1;
                    }
                }
                Edge::Anchor { .. } => {} // anchors use center ports
            }
        }

        PortDistributor {
            counts,
            next_index: std::collections::HashMap::new(),
        }
    }

    /// Get the distributed y for a property-side connection, advancing the index.
    fn next_y(&mut self, nl: &NodeLayout, prop_idx: usize, prop_id: PropId, side: PortSide) -> f64 {
        let total = self.counts.get(&(prop_id, side)).copied().unwrap_or(1);
        let index = self.next_index.entry((prop_id, side)).or_insert(0);
        let current = *index;
        *index += 1;
        nl.distributed_port_y(prop_idx, current, total)
    }
}

/// Compute the (x, y) port position and optional side for an edge endpoint.
///
/// Returns `(x, y, Option<PortSide>)`. The side is `None` for link center ports.
/// When `distributor` is provided, property ports use distributed y placement.
fn port_position(
    graph: &Graph,
    edge_id: EdgeId,
    role: EndpointRole,
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    port_sides: &PortSideAssignment,
    distributor: &mut PortDistributor,
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
                    Some((nl.anchor_port_x(), nl.anchor_port_bottom_y(), None))
                }
                EndpointRole::Downstream => {
                    // Target is child node, top center.
                    let nl = find_node_layout(node_layouts, *child)?;
                    Some((nl.anchor_port_x(), nl.anchor_port_top_y(), None))
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
                    let y = match side {
                        Some(s) => distributor.next_y(nl, prop_idx, *source_prop, s),
                        None => nl.port_y(prop_idx),
                    };
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
                    let y = match side {
                        Some(s) => distributor.next_y(nl, prop_idx, *dest_prop, s),
                        None => nl.port_y(prop_idx),
                    };
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
                    let y = match side {
                        Some(s) => distributor.next_y(nl, prop_idx, *source_prop, s),
                        None => nl.port_y(prop_idx),
                    };
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

/// Route a single edge using corridor-based vertical channels.
fn route_single_edge(
    edge_id: EdgeId,
    src_x: f64,
    src_y: f64,
    src_side: Option<PortSide>,
    tgt_x: f64,
    tgt_y: f64,
    tgt_side: Option<PortSide>,
    h_channels: &mut [Channel],
    corridors: &mut Vec<Corridor>,
    src_domain: Option<DomainId>,
    tgt_domain: Option<DomainId>,
) -> Vec<Segment> {
    // Case 1: Center-port edges (Anchors) — no side assignment.
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
            let h_y = if let Some(hi) = find_h_channel_between(h_channels, src_y, tgt_y) {
                h_channels[hi].reserve(edge_id)
            } else {
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

    // Case 2a: Intra-column bracket — both same side, same x → H-V-H through corridor.
    if src_side == tgt_side && src_side.is_some() && (src_x - tgt_x).abs() < 0.5 {
        let side = src_side.unwrap();
        let v_x = find_corridor_channel(src_x, side, corridors, edge_id, src_y, tgt_y, src_domain);
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

    // Case 2b: Cross-corridor routing.
    //
    // When both endpoints route to the same corridor (e.g., both sides of a
    // cross-domain edge face the gap corridor), use a single vertical channel
    // for an H-V-H (3-segment) route. Otherwise, use H-V-H-V-H (5-segment).
    if let (Some(src_s), Some(tgt_s)) = (src_side, tgt_side) {
        let src_corr = find_best_corridor_idx(src_x, src_s, corridors, src_domain);
        let tgt_corr = find_best_corridor_idx(tgt_x, tgt_s, corridors, tgt_domain);
        if let (Some(si), Some(ti)) = (src_corr, tgt_corr) && si == ti {
            // Same corridor — single channel, H-V-H route.
            let v_x = corridors[si].allocate_channel(edge_id, src_y, tgt_y);
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
    }

    // Different corridors — H-V-H-V-H route.
    let h_y = if let Some(hi) = find_h_channel_between(h_channels, src_y, tgt_y) {
        h_channels[hi].reserve(edge_id)
    } else {
        (src_y + tgt_y) / 2.0
    };

    let v1_x = match src_side {
        Some(side) => find_corridor_channel(src_x, side, corridors, edge_id, src_y, h_y, src_domain),
        None => src_x,
    };

    let v2_x = match tgt_side {
        Some(side) => find_corridor_channel(tgt_x, side, corridors, edge_id, h_y, tgt_y, tgt_domain),
        None => tgt_x,
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
// Arrowhead clearance
// ---------------------------------------------------------------------------

/// Shorten the last segment of a route by `ARROWHEAD_SIZE` so the arrowhead tip
/// lands exactly at the target boundary (markers use refX="0").
fn shorten_route_for_arrowhead(route: &mut Route) {
    let amount = super::ARROWHEAD_SIZE;
    loop {
        let seg = match route.segments.last_mut() {
            Some(s) => s,
            None => return,
        };
        let len = seg.length();
        if len > amount + 0.001 {
            // Shorten the last segment by `amount`.
            match seg {
                Segment::Horizontal { x_end, x_start, .. } => {
                    if *x_end >= *x_start {
                        *x_end -= amount;
                    } else {
                        *x_end += amount;
                    }
                }
                Segment::Vertical { y_end, y_start, .. } => {
                    if *y_end >= *y_start {
                        *y_end -= amount;
                    } else {
                        *y_end += amount;
                    }
                }
            }
            return;
        }
        // Segment is too short; remove it entirely and try the next one.
        route.segments.pop();
    }
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

/// Route all edges using orthogonal corridor-based routing.
///
/// Edges are routed in priority order (anchors first, then deriv inputs, then
/// constraints). Each route reserves corridor channels so later edges are offset.
pub fn route_all_edges(
    graph: &Graph,
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    domain_layouts: &[DomainLayout],
    port_sides: &PortSideAssignment,
) -> Vec<Route> {
    let mut h_channels = build_h_channels(node_layouts, deriv_layouts);
    let mut corridors = build_corridors(domain_layouts);
    let mut distributor = PortDistributor::new(graph, port_sides);

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
            &mut distributor,
        );
        let tgt = port_position(
            graph,
            edge_id,
            EndpointRole::Downstream,
            node_layouts,
            deriv_layouts,
            port_sides,
            &mut distributor,
        );

        let (src_x, src_y, src_side) = match src {
            Some(s) => s,
            None => continue,
        };
        let (tgt_x, tgt_y, tgt_side) = match tgt {
            Some(t) => t,
            None => continue,
        };

        // Determine the domain affinity for corridor selection.
        //
        // Intra-domain edges: both endpoints use the same domain's corridors.
        // Cross-domain edges: always use (None, None) to select the inter-column
        // gap corridor. Port side assignment already directs same-column
        // cross-domain edges toward the gap corridor side.
        let (src_domain, tgt_domain) = {
            let edge = &graph.edges[idx];
            let (src_nid, tgt_nid) = match edge {
                Edge::Anchor { parent, child, .. } => (Some(*parent), Some(*child)),
                Edge::Constraint { source_prop, dest_prop, .. } => {
                    (Some(prop_node(graph, *source_prop)), Some(prop_node(graph, *dest_prop)))
                }
                Edge::DerivInput { source_prop, .. } => {
                    (Some(prop_node(graph, *source_prop)), None)
                }
            };
            let sd = src_nid.and_then(|n| graph.nodes[n.index()].domain);
            let td = tgt_nid.and_then(|n| graph.nodes[n.index()].domain);
            if sd == td {
                // Intra-domain: use own domain corridors.
                (sd, td)
            } else {
                // Cross-domain: always use the inter-column gap corridor.
                (None, None)
            }
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
            &mut corridors,
            src_domain,
            tgt_domain,
        );

        let mut route = Route { edge_id, segments };
        shorten_route_for_arrowhead(&mut route);
        routes.push(route);
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

/// Returns the label position for a route: the midpoint of the first vertical
/// segment, offset 4px horizontally. Falls back to arc-length midpoint if no
/// vertical segment exists. Returns `(x, y, anchor)`.
pub fn route_label_position(route: &Route) -> (f64, f64, &'static str) {
    // Find first vertical segment.
    for seg in &route.segments {
        if let Segment::Vertical { x, y_start, y_end } = seg {
            let mid_y = (y_start + y_end) / 2.0;
            // Determine which side to offset based on horizontal context.
            // If there's a preceding horizontal segment going right-to-left (x_end < x_start),
            // the label goes to the right of the channel; otherwise to the left.
            let offset_right = route.segments.first().map_or(true, |first| {
                match first {
                    Segment::Horizontal { x_start, x_end, .. } => x_end > x_start,
                    _ => true,
                }
            });
            if offset_right {
                return (*x + 4.0, mid_y, "start");
            } else {
                return (*x - 4.0, mid_y, "end");
            }
        }
    }
    // Fallback: arc-length midpoint.
    let total: f64 = route.segments.iter().map(|s| s.length()).sum();
    if total < 1e-9 {
        let (x, y) = route.segments.first().map(|s| s.start()).unwrap_or((0.0, 0.0));
        return (x, y, "middle");
    }
    let mut remaining = total / 2.0;
    for seg in &route.segments {
        let len = seg.length();
        if remaining <= len {
            let frac = remaining / len;
            let (sx, sy) = seg.start();
            let (ex, ey) = seg.end();
            return (sx + (ex - sx) * frac, sy + (ey - sy) * frac, "middle");
        }
        remaining -= len;
    }
    let (x, y) = route.segments.last().map(|s| s.end()).unwrap_or((0.0, 0.0));
    (x, y, "middle")
}

/// Convert a Route to an EdgePath.  If `label_text` is Some, an EdgeLabel is
/// placed along the first vertical corridor segment, offset 4px horizontally.
pub fn route_to_edge_path(route: &Route, label_text: Option<String>) -> EdgePath {
    let label = label_text.map(|text| {
        let (x, y, anchor) = route_label_position(route);
        EdgeLabel { text, x, y, anchor }
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
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: "B".into(),
                display_name: None,
                properties: vec![PropId(1)],
                domain: None,
                is_anchored: false,
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

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &[], &port_sides);

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
                // y_end = top of B (100) minus ARROWHEAD_SIZE (6) = 94
                assert!(
                    (y_end - 94.0).abs() < 0.1,
                    "y_end should be 94.0 (arrowhead clearance), got {}",
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

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &[], &port_sides);

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

        let routes = route_all_edges(&graph, &node_layouts, &deriv_layouts, &[], &port_sides);

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

        // Third segment: vertical to child top center minus arrowhead clearance.
        match &link_route.segments[2] {
            Segment::Vertical { x, y_end, .. } => {
                assert!((x - 240.0).abs() < 0.1); // B center x
                assert!((y_end - 94.0).abs() < 0.1); // B top y (100) - ARROWHEAD_SIZE (6)
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

        // Same center x: first same-column edge gets Right (alternating).
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Upstream)],
            PortSide::Right
        );
        assert_eq!(
            sides[&(EdgeId(1), EndpointRole::Downstream)],
            PortSide::Right
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
            is_anchored: true,
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

        // Self-loop: both Right (route through right corridor).
        assert_eq!(
            sides[&(EdgeId(0), EndpointRole::Upstream)],
            PortSide::Right
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
    fn test_corridors_from_domains() {
        use crate::layout::DomainLayout;
        use crate::model::types::DomainId;

        let domain_layouts = vec![
            DomainLayout {
                id: DomainId(0),
                display_name: "D0".into(),
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 200.0,
            },
            DomainLayout {
                id: DomainId(1),
                display_name: "D1".into(),
                x: 120.0,
                y: 0.0,
                width: 100.0,
                height: 200.0,
            },
        ];

        let corridors = build_corridors(&domain_layouts);

        // 2 domains at different x-ranges → 2 intra-domain corridors each +
        // 1 inter-column gap + 1 outer right corridor = 6 corridors.
        // (No outer left corridor since D0 starts at x=0.)
        assert_eq!(corridors.len(), 6, "Expected 6 corridors, got {}", corridors.len());

        // D0 left corridor: x_start=0, center channel at CORRIDOR_PAD=8.
        assert!((corridors[0].channels[0].x - 8.0).abs() < 0.1);
        // D0 right corridor: x_end=100, center channel at 100-8=92.
        assert!((corridors[1].channels[0].x - 92.0).abs() < 0.1);
        // Inter-column gap corridor: between x=100 and x=120, first channel at
        // x_start + CORRIDOR_PAD = 108.
        let gap_idx = corridors.iter().position(|c| c.x_start > 99.0 && c.x_end < 121.0 && c.domain_ids.is_empty()).unwrap();
        assert!((corridors[gap_idx].channels[0].x - 108.0).abs() < 0.1);
        // Outer right corridor: x_start=220, first channel at 220+8=228.
        let outer_right_idx = corridors.iter().position(|c| c.x_start > 219.0 && c.domain_ids.is_empty() && c.x_start != corridors[gap_idx].x_start).unwrap();
        assert!((corridors[outer_right_idx].channels[0].x - 228.0).abs() < 0.1);

        // Verify domain_ids assignments.
        assert_eq!(corridors[0].domain_ids, vec![DomainId(0)]); // D0 left
        assert_eq!(corridors[1].domain_ids, vec![DomainId(0)]); // D0 right
        assert_eq!(corridors[2].domain_ids, vec![DomainId(1)]); // D1 left
        assert_eq!(corridors[3].domain_ids, vec![DomainId(1)]); // D1 right
        assert!(corridors[gap_idx].domain_ids.is_empty());         // inter-column gap
        assert!(corridors[outer_right_idx].domain_ids.is_empty()); // outer right
    }

    // -----------------------------------------------------------------------
    // Test: Cross-domain routing uses inter-domain gap corridor
    // -----------------------------------------------------------------------

    #[test]
    fn test_cross_domain_routing_uses_gap_corridor() {
        // Two domains: D0 at x~0..100, D1 at x~120..220.
        // Source node A in D0, target node B in D1.
        // Cross-domain constraint A::p0 -> B::p1 should use D0's right corridor
        // for v1_x and D1's left corridor for v2_x.
        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: "A".into(),
                display_name: None,
                properties: vec![PropId(0)],
                domain: Some(DomainId(0)),
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: "B".into(),
                display_name: None,
                properties: vec![PropId(1)],
                domain: Some(DomainId(1)),
                is_anchored: false,
                is_selected: false,
            },
        ];

        let properties = vec![
            Property {
                id: PropId(0),
                node: NodeId(0),
                name: "p0".into(),
                critical: true,
                constrained: false,
            },
            Property {
                id: PropId(1),
                node: NodeId(1),
                name: "p1".into(),
                critical: true,
                constrained: false,
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

        let domains = vec![
            Domain {
                id: DomainId(0),
                display_name: "D0".into(),
                members: vec![NodeId(0)],
            },
            Domain {
                id: DomainId(1),
                display_name: "D1".into(),
                members: vec![NodeId(1)],
            },
        ];

        let graph = Graph {
            nodes,
            properties,
            derivations: vec![],
            edges,
            domains,
            prop_edges,
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        };

        // Node A in D0 zone, node B in D1 zone.
        // Domain padding (lr_pad) = DOMAIN_PADDING + CORRIDOR_PAD*2 = 10 + 16 = 26px per side.
        // D0: x=0, nodes at x=26, domain width = 80 + 2*26 = 132, right edge = 132
        // Gap: 132..148 = 16px inter-domain corridor
        // D1: x=148, nodes at x=174, domain width = 132, right edge = 280
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 26.0, // inside D0 (domain x=0 + lr_pad=26)
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 174.0, // inside D1 (domain x=148 + lr_pad=26)
                y: 100.0,
                width: 80.0,
                height: 52.0,
            },
        ];

        let domain_layouts = vec![
            DomainLayout {
                id: DomainId(0),
                display_name: "D0".into(),
                x: 0.0,
                y: -10.0,
                width: 132.0, // 80 + 2*26
                height: 72.0,
            },
            DomainLayout {
                id: DomainId(1),
                display_name: "D1".into(),
                x: 148.0, // gap: 132..148 = 16px inter-domain corridor
                y: 80.0,
                width: 132.0,
                height: 72.0,
            },
        ];

        let deriv_layouts: Vec<DerivLayout> = vec![];
        let port_sides = assign_port_sides(&graph, &node_layouts, &deriv_layouts);

        let routes = route_all_edges(
            &graph,
            &node_layouts,
            &deriv_layouts,
            &domain_layouts,
            &port_sides,
        );

        assert_eq!(routes.len(), 1);
        let route = &routes[0];

        // The route should be H-V-H (3 segments) or H-V-H-V-H (5 segments).
        // Key: vertical segments should be in the inter-domain gap corridor,
        // NOT in the domain-specific corridors.

        // Find vertical segments.
        let verticals: Vec<&Segment> = route
            .segments
            .iter()
            .filter(|s| matches!(s, Segment::Vertical { .. }))
            .collect();

        assert!(
            !verticals.is_empty(),
            "Route should have at least one vertical segment"
        );

        // Inter-domain corridor: x_start=132, x_end=148, first channel at x_start + CORRIDOR_PAD = 140
        let inter_domain_first_channel = 140.0;
        // D0 right corridor center: 124
        let d0_right_corridor = 124.0;
        // D1 left corridor center: 156
        let d1_left_corridor = 156.0;

        // All vertical segments should be in the inter-domain corridor,
        // not in domain-specific corridors.
        for v in &verticals {
            let v_x = match v {
                Segment::Vertical { x, .. } => *x,
                _ => unreachable!(),
            };
            let dist_to_gap = (v_x - inter_domain_first_channel).abs();
            let dist_to_d0 = (v_x - d0_right_corridor).abs();
            let dist_to_d1 = (v_x - d1_left_corridor).abs();
            assert!(
                dist_to_gap <= dist_to_d0 && dist_to_gap <= dist_to_d1,
                "Vertical at x={} should be in gap corridor (~{}), not D0 ({}) or D1 ({})",
                v_x,
                inter_domain_first_channel,
                d0_right_corridor,
                d1_left_corridor,
            );
        }
    }
}
