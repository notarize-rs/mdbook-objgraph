//! Orthogonal channel-based edge routing (DESIGN.md §4.2.6).
//!
//! # Edge Routing Architecture
//!
//! All edge routing is centralized in this module. The pipeline is:
//!
//! 1. **Port side assignment** (`refine_port_sides`): Determines which side
//!    (Left or Right) each property-level port exits the node. Uses coordinate-
//!    space geometry to override the layer-space assignments from crossing
//!    minimization.
//!
//! 2. **Corridor construction** (`build_corridors`): Creates vertical channel
//!    regions alongside domains and in inter-column gaps. Three types:
//!    - **Intra-domain corridors**: Left/right edges of each domain. Tagged
//!      with `domain_ids`. Used by intra-domain bracket edges.
//!    - **Inter-column gap corridors**: Between adjacent domain columns.
//!      Empty `domain_ids`. Used by cross-column edges.
//!    - **Outer corridors**: Left/right edges of the entire layout. Empty
//!      `domain_ids`. Used by same-column cross-domain edges.
//!
//! 3. **Edge routing** (`route_all_edges` → `route_single_edge`): Routes each
//!    edge through corridors using H-V-H or H-V-H-V-H segment patterns.
//!
//! # Corridor Selection Rules — DO NOT BREAK
//!
//! The `edge_domain` parameter controls corridor selection:
//! - `Some(domain_id)` → **intra-domain edge**: only corridors tagged with
//!   that domain ID.
//! - `None` → **cross-domain edge**: only corridors with empty `domain_ids`
//!   (gap or outer corridors).
//!
//! **Invariant: Cross-domain edges must NEVER route through intra-domain
//! corridors.** This is enforced by the `find_best_corridor_idx` filter and
//! verified by the `inter_domain_edges_in_intra_corridors` quality check.
//!
//! When no corridor is found in the preferred direction, `find_corridor_channel`
//! tries the opposite direction before falling back to an untracked offset.
//! The opposite-direction fallback prevents cross-domain edges from landing
//! inside intra-domain corridors when no outer corridor exists on the
//! preferred side.
//!
//! # Outer Corridor Guarantee
//!
//! Outer corridors are ALWAYS created on both sides of the layout. When the
//! leftmost domain is flush with x=0, the outer-left corridor extends into
//! negative x space. The SVG `content_offset_x` mechanism compensates for
//! negative-x content, just as it does for edge labels.
//!
//! The outer-left corridor uses `grows_left: true` so that new channels are
//! allocated in the -x direction (away from the domain boundary). Without
//! this, channel expansion would grow into the intra-domain corridor zone.
//!
//! # Channel Collision Prevention
//!
//! Each corridor tracks channel occupants with vertical extents. The
//! `allocate_channel` method checks for vertical overlap before reusing a
//! channel, and also checks for crossing-aware blocking by outer channels.
//! The blind-offset fallback in `find_corridor_channel` does NOT track
//! occupants and will cause collisions — it should never be reached if
//! outer corridors are properly created.
//!
//! # Fan-Out Channel Ordering — DO NOT BREAK
//!
//! When a single source property (e.g., `System Clock::current_time`) fans
//! out to many targets through the same corridor, the initial channel
//! allocation assigns inner channels to edges with smaller vertical midpoints.
//! This creates crossings: each outer edge's horizontal stub passes through
//! inner edges' vertical segments.
//!
//! `fix_fanout_channel_order` reverses the assignment for cross-domain edges:
//! the edge entering from the **lowest source y** gets the **outermost**
//! corridor channel. This nests the horizontal stubs properly — the lowest-y
//! stub is longest but no inner vertical has started yet at that y level.
//!
//! **Key constraints:**
//! - Only applies to **cross-domain** constraint edges. Intra-domain brackets
//!   are handled separately by `fix_bracket_nesting_channels`.
//! - Edges are sub-grouped by **approach direction** (src_x > corridor_x vs
//!   src_x < corridor_x). The corridor allocator can reuse a channel for
//!   edges approaching from opposite sides; mixing them in a fan-out group
//!   would swap channels between non-overlapping occupants, causing collisions.
//! - Runs BEFORE `fix_bracket_nesting_channels`. The two passes are compatible:
//!   bracket nesting only swaps adjacent channels within same-node pairs,
//!   preserving the overall fan-out ordering.

use crate::model::types::{DomainId, Edge, EdgeId, Graph, NodeId, PropId};

use super::{
    layout_endpoints, DomainLayout, EdgeLabel, EdgePath, EndpointRole,
    LayoutEndpoint, NodeLayout, PortSide, PortSideAssignment, CHANNEL_GAP, CORRIDOR_PAD,
    STUB_LENGTH,
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
    pub(super) fn start(&self) -> (f64, f64) {
        match self {
            Segment::Horizontal { y, x_start, .. } => (*x_start, *y),
            Segment::Vertical { x, y_start, .. } => (*x, *y_start),
        }
    }

    /// Ending point of this segment.
    pub(super) fn end(&self) -> (f64, f64) {
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

// PortSideAssignment type is defined in super (mod.rs) so crossing.rs can also produce it.

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
    /// When true, new channels are allocated to the LEFT (-x) of existing ones.
    /// Used for the outer-left corridor where channels must grow away from the
    /// adjacent domain boundary, not toward it.
    pub grows_left: bool,
}

impl Corridor {
    /// Allocate a channel with no vertical overlap for the given edge extent.
    /// If all existing channels overlap, create a new one.
    ///
    /// Crossing-aware: when considering reusing an inner channel `i`, checks
    /// whether any outer channel `j > i` has an occupant whose vertical extent
    /// includes `y_start` (the horizontal entry point). If so, the new edge's
    /// horizontal entry segment would cross that outer occupant's vertical
    /// segment, so channel `i` is skipped.
    fn allocate_channel(&mut self, edge_id: EdgeId, y_start: f64, y_end: f64) -> f64 {
        let entry_y = y_start.min(y_end);
        let exit_y = y_start.max(y_end);

        // Try existing channels (non-overlapping extents can share).
        let n = self.channels.len();
        for i in 0..n {
            if !self.channels[i].overlaps(y_start, y_end) {
                // Check whether any outer channel (j > i) has an occupant
                // whose vertical extent includes the entry point y_start.
                // If so, the horizontal segment from source to this inner
                // channel would cross that outer occupant's vertical segment.
                let outer_blocks = (i + 1..n).any(|j| {
                    self.channels[j].occupants.iter().any(|&(_, os, oe)| {
                        let (olo, ohi) = if os <= oe { (os, oe) } else { (oe, os) };
                        // The outer occupant blocks reuse if its vertical extent
                        // overlaps with any part of the new edge's entry range.
                        // The entry range is the y_start horizontal segment, which
                        // occurs at a single y value. But the vertical segment of
                        // the outer occupant spans [olo, ohi]. If y_start (or y_end)
                        // falls within that span, crossing occurs.
                        (olo < entry_y + 0.5 && ohi > entry_y - 0.5)
                            || (olo < exit_y + 0.5 && ohi > exit_y - 0.5)
                    })
                });
                if !outer_blocks {
                    return self.channels[i].reserve(edge_id, y_start, y_end);
                }
            }
        }
        // All channels overlap or would cause crossings — create a new one.
        let new_x = if self.channels.is_empty() {
            self.x_start + CORRIDOR_PAD
        } else if self.grows_left {
            self.channels.last().unwrap().x - CHANNEL_GAP
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
fn build_corridors(
    domain_layouts: &[DomainLayout],
    node_layouts: &[NodeLayout],
    graph: &Graph,
) -> Vec<Corridor> {
    let mut corridors = Vec::new();

    // Sort domains by x for inter-domain corridor detection.
    let mut sorted_domains: Vec<&DomainLayout> = domain_layouts.iter().collect();
    sorted_domains.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());

    for dl in &sorted_domains {
        // Compute corridor width from the actual distance between the domain
        // edge and the nearest member node. This handles domains that were
        // expanded by `expand_corridors_for_edges` to fit more bracket edges.
        let members: Vec<&NodeLayout> = graph
            .domains
            .iter()
            .find(|d| d.id == dl.id)
            .into_iter()
            .flat_map(|d| d.members.iter())
            .map(|nid| &node_layouts[nid.index()])
            .collect();

        let (left_corridor_width, right_corridor_width) = if members.is_empty() {
            (CORRIDOR_PAD * 2.0, CORRIDOR_PAD * 2.0)
        } else {
            let (min_node_x, _, max_node_right, _) = super::node_bounds(&members).unwrap();
            let left_w = (min_node_x - dl.x).max(CORRIDOR_PAD * 2.0);
            let right_w = ((dl.x + dl.width) - max_node_right).max(CORRIDOR_PAD * 2.0);
            (left_w, right_w)
        };

        let left_x_start = dl.x;
        let left_x_end = dl.x + left_corridor_width;
        let right_x_start = dl.x + dl.width - right_corridor_width;
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
                grows_left: false,
            });
        }
    }

    // Outer corridors: left edge (before leftmost domain) and right edge
    // (after rightmost domain).  These are inter-domain corridors used by
    // cross-domain same-column edges that route away from the inter-column
    // gap.  ALWAYS created — if the leftmost domain is flush with x=0,
    // the corridor extends into negative x (the SVG content_offset_x
    // mechanism will shift the viewport to compensate, just as it does
    // for edge labels that extend past the left boundary).
    if let (Some(leftmost), Some(rightmost)) = (sorted_domains.first(), sorted_domains.last()) {
        let left_edge = leftmost.x;
        let outer_width = CORRIDOR_PAD * 2.0;
        let x_start = (left_edge - outer_width).min(0.0);
        corridors.push(Corridor {
            x_start,
            x_end: left_edge,
            channels: vec![CorridorChannel {
                x: left_edge - CORRIDOR_PAD,
                occupants: Vec::new(),
            }],
            domain_ids: vec![],
            grows_left: true,
        });

        let right_edge = rightmost.x + rightmost.width;
        corridors.push(Corridor {
            x_start: right_edge,
            x_end: right_edge + outer_width,
            channels: vec![CorridorChannel {
                x: right_edge + CORRIDOR_PAD,
                occupants: Vec::new(),
            }],
            domain_ids: vec![],
            grows_left: false,
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
        grows_left: false,
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

/// Register an edge as an occupant of a specific corridor channel (identified
/// by its x-coordinate) so that subsequent channel allocations see the
/// occupancy.  Used by bundle routing to keep the corridor model consistent
/// after bypassing `find_corridor_channel`.
/// Find the nearest corridor in the direction the port faces and allocate a channel.
///
/// `edge_domain` constrains corridor selection: `Some(did)` means prefer only
/// corridors belonging to that domain (for intra-domain edges); `None` means
/// any corridor is acceptable (for cross-domain edges).
fn find_corridor_channel(
    port_x: f64,
    port_side: PortSide,
    corridors: &mut [Corridor],
    edge_id: EdgeId,
    y_start: f64,
    y_end: f64,
    edge_domain: Option<DomainId>,
) -> f64 {
    if let Some(idx) = find_best_corridor_idx(port_x, port_side, corridors, edge_domain) {
        return corridors[idx].allocate_channel(edge_id, y_start, y_end);
    }

    // No corridor in the preferred direction.  Try the opposite direction
    // rather than using a blind offset that could land inside an intra-domain
    // corridor.  This is critical for cross-domain edges (edge_domain=None)
    // where the outer corridor may only exist on one side.
    let opposite = match port_side {
        PortSide::Left => PortSide::Right,
        PortSide::Right => PortSide::Left,
    };
    if let Some(idx) = find_best_corridor_idx(port_x, opposite, corridors, edge_domain) {
        return corridors[idx].allocate_channel(edge_id, y_start, y_end);
    }

    // Last resort: offset from port (should not happen with outer corridors).
    match port_side {
        PortSide::Left => port_x - CORRIDOR_PAD,
        PortSide::Right => port_x + CORRIDOR_PAD,
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
fn find_node_layout(node_layouts: &[NodeLayout], node_id: NodeId) -> Option<&NodeLayout> {
    node_layouts.iter().find(|nl| nl.id == node_id)
}

/// For an intra-domain constraint, pick the corridor side that minimizes crossings.
///
/// Uses the same heuristic as `same_column_outer_side`: route away from the
/// inter-column gap (the side with no neighbor domains). Falls back to Right.
fn intra_domain_corridor_side(
    graph: &Graph,
    node_id: NodeId,
    domain_layouts: &[DomainLayout],
) -> PortSide {
    let domain_id = match graph.nodes[node_id.index()].domain {
        Some(d) => d,
        None => return PortSide::Right,
    };
    let dl = match domain_layouts.iter().find(|dl| dl.id == domain_id) {
        Some(d) => d,
        None => return PortSide::Right,
    };
    let col_center = dl.x + dl.width / 2.0;
    let has_left = domain_layouts.iter().any(|other| {
        other.id != domain_id && other.x + other.width < col_center
    });
    let has_right = domain_layouts.iter().any(|other| {
        other.id != domain_id && other.x > col_center
    });
    match (has_left, has_right) {
        (true, false) => PortSide::Right,   // Gap is left, route right.
        (false, true) => PortSide::Left,    // Gap is right, route left.
        _ => PortSide::Right,               // Default: right corridor.
    }
}

/// Determine if two nodes' domains are in the same column and which side is "outer".
///
/// Two domains are in the same column if their x-ranges overlap significantly.
/// The "outer" side is the side away from the column gap (i.e., away from other columns).
/// Returns `Some(PortSide)` if both nodes are in same-column different domains,
/// `None` if they're in different columns or same domain.
fn same_column_outer_side(
    graph: &Graph,
    src_node_id: NodeId,
    tgt_node_id: NodeId,
    domain_layouts: &[DomainLayout],
) -> Option<PortSide> {
    let src_domain = graph.nodes[src_node_id.index()].domain?;
    let tgt_domain = graph.nodes[tgt_node_id.index()].domain?;

    // Same domain = not cross-domain; handled elsewhere.
    if src_domain == tgt_domain {
        return None;
    }

    let src_dl = domain_layouts.iter().find(|dl| dl.id == src_domain)?;
    let tgt_dl = domain_layouts.iter().find(|dl| dl.id == tgt_domain)?;

    // Check if domains overlap in x (same column).
    let src_left = src_dl.x;
    let src_right = src_dl.x + src_dl.width;
    let tgt_left = tgt_dl.x;
    let tgt_right = tgt_dl.x + tgt_dl.width;

    let overlap = src_right.min(tgt_right) - src_left.max(tgt_left);
    let min_width = src_dl.width.min(tgt_dl.width);
    if overlap < min_width * 0.5 {
        // Not same column (less than 50% overlap).
        return None;
    }

    // Same column — determine which side is "outer" (away from other columns).
    // Find the leftmost and rightmost domain x extents across all domains to
    // determine if this column is on the left or right of the layout.
    let col_left = src_left.min(tgt_left);
    let col_right = src_right.max(tgt_right);
    let col_center = (col_left + col_right) / 2.0;

    // Check if there are other domains to the left or right.
    let has_left_neighbor = domain_layouts.iter().any(|dl| {
        dl.id != src_domain && dl.id != tgt_domain
            && dl.x + dl.width < col_center
    });
    let has_right_neighbor = domain_layouts.iter().any(|dl| {
        dl.id != src_domain && dl.id != tgt_domain
            && dl.x > col_center
    });

    // Route away from the inter-column gap.
    let proposed_side = match (has_left_neighbor, has_right_neighbor) {
        (true, false) => PortSide::Right,   // Gap is left, route right.
        (false, true) => PortSide::Left,    // Gap is right, route left.
        (true, true) => PortSide::Right,    // Both sides have neighbors; prefer right.
        (false, false) => PortSide::Right,  // No neighbors; default to right.
    };

    // When the source domain extends beyond the target domain on the proposed
    // outer side, the inter-domain corridor between them is positioned at the
    // target domain's boundary — "behind" the source node's port. Bracket
    // routing through that corridor is impossible, so fall back to normal
    // cross-column routing through the inter-column gap corridor.
    let src_overhangs = match proposed_side {
        PortSide::Right => src_right > tgt_right + 1.0,
        PortSide::Left => src_left < tgt_left - 1.0,
    };
    if src_overhangs {
        return None;
    }

    Some(proposed_side)
}

/// Determine the port side for a cross-node constraint based on coordinate-space
/// geometry. Encapsulates the cascade: same_column_outer_side → center-x
/// comparison → intra_domain_corridor_side fallback.
fn determine_constraint_side(
    graph: &Graph,
    src_node: NodeId,
    dst_node: NodeId,
    node_layouts: &[NodeLayout],
    domain_layouts: &[DomainLayout],
) -> PortSide {
    // Cross-domain same-column: bracket routing through outer corridor.
    if let Some(outer_side) = same_column_outer_side(graph, src_node, dst_node, domain_layouts) {
        return outer_side;
    }

    // Use coordinate-space horizontal positions.
    let src_nl = find_node_layout(node_layouts, src_node);
    let tgt_nl = find_node_layout(node_layouts, dst_node);
    match (src_nl, tgt_nl) {
        (Some(src_nl), Some(tgt_nl)) => {
            let src_cx = src_nl.x + src_nl.width / 2.0;
            let tgt_cx = tgt_nl.x + tgt_nl.width / 2.0;
            if src_cx < tgt_cx {
                PortSide::Right
            } else if src_cx > tgt_cx {
                PortSide::Left
            } else {
                // Same center x: use domain geometry.
                intra_domain_corridor_side(graph, src_node, domain_layouts)
            }
        }
        _ => PortSide::Right,
    }
}

/// Refine port side assignments from layer-space using coordinate-space geometry.
///
/// Takes the layer-space `PortSideAssignment` produced by crossing minimization
/// and overrides entries that require coordinate-space information:
///   - Same-node constraints: grouped into contiguous bracket ladders,
///     each group routed to the opposite side from cross-node constraints.
///   - Cross-domain same-column edges: use bracket routing through the outer corridor.
///
/// All other assignments (same-layer different-position constraints) are
/// preserved from the layer-space computation, which was co-optimized with
/// property ordering during the sweep.
pub fn refine_port_sides(
    graph: &Graph,
    node_layouts: &[NodeLayout],
    domain_layouts: &[DomainLayout],
    layer_sides: &PortSideAssignment,
    prop_order: &super::crossing::PropertyOrder,
) -> PortSideAssignment {
    use std::collections::HashMap;

    let mut sides = layer_sides.clone();

    // Pre-compute same-column bracket edge counts per (source_node, outer_side)
    // so we can split overloaded corridors across both sides.
    let mut bracket_counts: HashMap<(NodeId, PortSide), Vec<(EdgeId, NodeId)>> =
        HashMap::new();
    for (idx, edge) in graph.edges.iter().enumerate() {
        if let Edge::Constraint { source_prop, dest_prop, .. } = edge {
            let src_node = prop_node(graph, *source_prop);
            let dst_node = prop_node(graph, *dest_prop);
            if src_node != dst_node
                && let Some(outer_side) =
                    same_column_outer_side(graph, src_node, dst_node, domain_layouts)
            {
                bracket_counts
                    .entry((src_node, outer_side))
                    .or_default()
                    .push((EdgeId(idx as u32), dst_node));
            }
        }
    }

    // For overloaded bracket corridors (>2 edges from same source), build a set
    // of edge IDs that should route through the opposite (inner) corridor.
    // We alternate by destination node: edges to the first target go outer,
    // edges to the second target go inner, etc.
    let mut bracket_flip: std::collections::HashSet<EdgeId> = std::collections::HashSet::new();
    for ((_src_node, _outer_side), edges) in &bracket_counts {
        if edges.len() <= 2 {
            continue;
        }
        let mut seen_dst: Vec<NodeId> = Vec::new();
        for &(_, dst) in edges {
            if !seen_dst.contains(&dst) {
                seen_dst.push(dst);
            }
        }
        for (i, dst_node) in seen_dst.iter().enumerate() {
            if i % 2 == 1 {
                for &(eid, dst) in edges {
                    if dst == *dst_node {
                        bracket_flip.insert(eid);
                    }
                }
            }
        }
    }

    // ── Pre-compute cross-node dominant side per node ────────────────
    // For each node, determine which side cross-node constraints predominantly
    // use. Same-node constraints should route on the opposite side to avoid
    // mixing corridors.
    let cross_node_dominant: HashMap<NodeId, PortSide> = {
        let mut counts: HashMap<NodeId, (usize, usize)> = HashMap::new(); // (left, right)
        for edge in &graph.edges {
            if let Edge::Constraint { source_prop, dest_prop, .. } = edge {
                let src = prop_node(graph, *source_prop);
                let dst = prop_node(graph, *dest_prop);
                if src != dst {
                    let side = determine_constraint_side(
                        graph, src, dst, node_layouts, domain_layouts,
                    );
                    let e = counts.entry(src).or_default();
                    match side {
                        PortSide::Left => e.0 += 1,
                        PortSide::Right => e.1 += 1,
                    }
                    // For the destination node, the edge arrives from the opposite
                    // direction.
                    let e2 = counts.entry(dst).or_default();
                    match side {
                        PortSide::Left => e2.0 += 1,
                        PortSide::Right => e2.1 += 1,
                    }
                }
            }
        }
        counts
            .into_iter()
            .map(|(nid, (l, r))| {
                (nid, if l >= r { PortSide::Left } else { PortSide::Right })
            })
            .collect()
    };

    // ── Pre-compute same-node group sides ────────────────────────────
    // Group same-node constraints into contiguous bracket ladders using
    // property order. Each group gets a consistent side based on data
    // (opposite of cross-node dominant side).
    let same_node_group_sides: HashMap<EdgeId, PortSide> = {
        let mut per_node: HashMap<NodeId, Vec<(EdgeId, usize, usize)>> = HashMap::new();
        for (idx, edge) in graph.edges.iter().enumerate() {
            if let Edge::Constraint { source_prop, dest_prop, .. } = edge {
                let src_node = prop_node(graph, *source_prop);
                let dst_node = prop_node(graph, *dest_prop);
                if src_node == dst_node {
                    let si = prop_order.prop_index(src_node, *source_prop).unwrap_or(0);
                    let di = prop_order.prop_index(src_node, *dest_prop).unwrap_or(0);
                    let (lo, hi) = if si < di { (si, di) } else { (di, si) };
                    per_node
                        .entry(src_node)
                        .or_default()
                        .push((EdgeId(idx as u32), lo, hi));
                }
            }
        }

        let mut result = HashMap::new();
        for (node_id, mut brackets) in per_node {
            brackets.sort_by_key(|&(_, lo, _)| lo);

            // Group into contiguous ladders.
            let mut groups: Vec<Vec<EdgeId>> = Vec::new();
            let mut current_group = vec![brackets[0].0];
            let mut current_end = brackets[0].2;
            for &(eid, lo, hi) in &brackets[1..] {
                if lo <= current_end + 1 {
                    current_group.push(eid);
                    current_end = current_end.max(hi);
                } else {
                    groups.push(std::mem::take(&mut current_group));
                    current_group.push(eid);
                    current_end = hi;
                }
            }
            groups.push(current_group);

            // Choose side per group: opposite of cross-node dominant side.
            // When no cross-node constraints exist, use intra-domain corridor side.
            let cross_side = cross_node_dominant.get(&node_id).copied();
            for group in &groups {
                let side = match cross_side {
                    Some(s) => match s {
                        PortSide::Left => PortSide::Right,
                        PortSide::Right => PortSide::Left,
                    },
                    None => intra_domain_corridor_side(graph, node_id, domain_layouts),
                };
                for &eid in group {
                    result.insert(eid, side);
                }
            }
        }
        result
    };

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);

        match edge {
            Edge::Anchor { .. } => {} // center ports, no side
            Edge::Constraint { source_prop, dest_prop, .. } => {
                let src_node = prop_node(graph, *source_prop);
                let dst_node = prop_node(graph, *dest_prop);

                // Same-node constraints: use pre-computed group-aware side.
                if src_node == dst_node {
                    let side = same_node_group_sides
                        .get(&edge_id)
                        .copied()
                        .unwrap_or(PortSide::Right);
                    sides.insert((edge_id, EndpointRole::Upstream), side);
                    sides.insert((edge_id, EndpointRole::Downstream), side);
                    continue;
                }

                // Cross-domain same-column: bracket routing through outer corridor,
                // with overloaded corridors split across both sides.
                if let Some(outer_side) =
                    same_column_outer_side(graph, src_node, dst_node, domain_layouts)
                {
                    let side = if bracket_flip.contains(&edge_id) {
                        match outer_side {
                            PortSide::Left => PortSide::Right,
                            PortSide::Right => PortSide::Left,
                        }
                    } else {
                        outer_side
                    };
                    sides.insert((edge_id, EndpointRole::Upstream), side);
                    sides.insert((edge_id, EndpointRole::Downstream), side);
                    continue;
                }

                // Different-node constraints: override with coordinate-space
                // horizontal positions, which are more accurate than layer-space
                // indices for determining exit/entry direction.
                let src_nl = match find_node_layout(node_layouts, src_node) {
                    Some(nl) => nl,
                    None => continue,
                };
                let tgt_nl = match find_node_layout(node_layouts, dst_node) {
                    Some(nl) => nl,
                    None => continue,
                };
                let src_cx = src_nl.x + src_nl.width / 2.0;
                let tgt_cx = tgt_nl.x + tgt_nl.width / 2.0;

                if src_cx < tgt_cx {
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Right);
                    sides.insert((edge_id, EndpointRole::Downstream), PortSide::Left);
                } else if src_cx > tgt_cx {
                    sides.insert((edge_id, EndpointRole::Upstream), PortSide::Left);
                    sides.insert((edge_id, EndpointRole::Downstream), PortSide::Right);
                } else {
                    // Same center x: data-driven side selection.
                    // For same-domain, use intra-domain corridor side.
                    // For cross-domain, also use domain geometry (no alternation).
                    let side = intra_domain_corridor_side(graph, src_node, domain_layouts);
                    sides.insert((edge_id, EndpointRole::Upstream), side);
                    sides.insert((edge_id, EndpointRole::Downstream), side);
                }
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
fn build_h_channels(node_layouts: &[NodeLayout]) -> Vec<Channel> {
    // Gather all distinct y-bands (top, bottom) for nodes.
    let mut bands: Vec<(f64, f64)> = Vec::new(); // (top_y, bottom_y) per element

    for nl in node_layouts {
        bands.push((nl.y, nl.y + nl.height));
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
            .fold(f64::INFINITY, f64::min)
            - 50.0;
        let x_max = node_layouts
            .iter()
            .map(|nl| nl.x + nl.width)
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

/// Pre-computed port slot assignments for distributed port placement.
///
/// Slots are assigned by sorting each property's edges by the physical Y
/// coordinate of the opposite endpoint, so that the topmost port slot
/// connects to the topmost destination — minimizing crossing at the property.
struct PortDistributor {
    /// Pre-computed slot index for each (EdgeId, PropId, PortSide).
    slots: std::collections::HashMap<(EdgeId, PropId, PortSide), usize>,
    /// Total connections per (PropId, PortSide).
    counts: std::collections::HashMap<(PropId, PortSide), usize>,
}

impl PortDistributor {
    /// Build a distributor using coordinate-space vertical positions for slot ordering.
    fn new(
        graph: &Graph,
        port_sides: &PortSideAssignment,
        node_layouts: &[NodeLayout],
        prop_order: &super::crossing::PropertyOrder,
    ) -> Self {
        // Count total connections per (PropId, PortSide).
        let mut counts: std::collections::HashMap<(PropId, PortSide), usize> =
            std::collections::HashMap::new();

        // Collect (edge_id, role, opposite_y) per (PropId, PortSide).
        let mut prop_side_edges: std::collections::HashMap<
            (PropId, PortSide),
            Vec<(EdgeId, f64)>,
        > = std::collections::HashMap::new();

        for (idx, edge) in graph.edges.iter().enumerate() {
            let edge_id = EdgeId(idx as u32);
            match edge {
                Edge::Constraint { source_prop, dest_prop, .. } => {
                    if let Some(&side) = port_sides.get(&(edge_id, EndpointRole::Upstream)) {
                        *counts.entry((*source_prop, side)).or_insert(0) += 1;
                        // Opposite endpoint for upstream is the downstream (dest) prop.
                        let opp_y = opposite_y(graph, edge, EndpointRole::Upstream,
                            node_layouts, prop_order);
                        prop_side_edges.entry((*source_prop, side))
                            .or_default().push((edge_id, opp_y));
                    }
                    if let Some(&side) = port_sides.get(&(edge_id, EndpointRole::Downstream)) {
                        *counts.entry((*dest_prop, side)).or_insert(0) += 1;
                        let opp_y = opposite_y(graph, edge, EndpointRole::Downstream,
                            node_layouts, prop_order);
                        prop_side_edges.entry((*dest_prop, side))
                            .or_default().push((edge_id, opp_y));
                    }
                }
                Edge::Anchor { .. } => {} // anchors use center ports
            }
        }

        // Sort each property-side group by opposite Y and assign slot indices.
        let mut slots = std::collections::HashMap::new();
        for ((prop_id, side), mut edges) in prop_side_edges {
            edges.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (slot, (edge_id, _)) in edges.iter().enumerate() {
                slots.insert((*edge_id, prop_id, side), slot);
            }
        }

        PortDistributor { slots, counts }
    }

    /// Get the distributed y for a property-side connection using pre-computed slot.
    fn port_y(&self, nl: &NodeLayout, prop_idx: usize, edge_id: EdgeId, prop_id: PropId, side: PortSide) -> f64 {
        let total = self.counts.get(&(prop_id, side)).copied().unwrap_or(1);
        let slot = self.slots.get(&(edge_id, prop_id, side)).copied().unwrap_or(0);
        nl.distributed_port_y(prop_idx, slot, total)
    }
}

/// Pre-computed port slot assignments for derivation pill top/bottom ports.
///
/// Distributes x-positions across the flat portion of the pill's top or
/// bottom edge.  Slots are sorted by the opposite endpoint's x-coordinate
/// to minimize crossing of the short vertical stubs.
struct PillPortDistributor {
    /// Pre-computed slot index for each (EdgeId, NodeId, is_top).
    slots: std::collections::HashMap<(EdgeId, NodeId, bool), usize>,
    /// Total connections per (NodeId, is_top).
    counts: std::collections::HashMap<(NodeId, bool), usize>,
}

impl PillPortDistributor {
    fn new(graph: &Graph, node_layouts: &[NodeLayout]) -> Self {
        let mut counts: std::collections::HashMap<(NodeId, bool), usize> =
            std::collections::HashMap::new();
        let mut groups: std::collections::HashMap<(NodeId, bool), Vec<(EdgeId, f64)>> =
            std::collections::HashMap::new();

        for (idx, edge) in graph.edges.iter().enumerate() {
            let edge_id = EdgeId(idx as u32);
            if let Edge::Constraint { .. } = edge {
                let (src_nid, dst_nid) = graph.edge_nodes(edge);

                // Pill is source → bottom port (is_top = false).
                if graph.nodes[src_nid.index()].is_derivation() {
                    let key = (src_nid, false);
                    *counts.entry(key).or_insert(0) += 1;
                    let opp_x = node_layouts[dst_nid.index()].x
                        + node_layouts[dst_nid.index()].width / 2.0;
                    groups.entry(key).or_default().push((edge_id, opp_x));
                }

                // Pill is target → top port (is_top = true).
                if graph.nodes[dst_nid.index()].is_derivation() {
                    let key = (dst_nid, true);
                    *counts.entry(key).or_insert(0) += 1;
                    let opp_x = node_layouts[src_nid.index()].x
                        + node_layouts[src_nid.index()].width / 2.0;
                    groups.entry(key).or_default().push((edge_id, opp_x));
                }
            }
        }

        // Sort each group by opposite x and assign slot indices.
        let mut slots = std::collections::HashMap::new();
        for ((nid, is_top), mut edges) in groups {
            edges.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (slot, (edge_id, _)) in edges.iter().enumerate() {
                slots.insert((*edge_id, nid, is_top), slot);
            }
        }

        PillPortDistributor { slots, counts }
    }

    /// Get the distributed x for a pill top/bottom port.
    fn port_x(&self, nl: &NodeLayout, edge_id: EdgeId, node_id: NodeId, is_top: bool) -> f64 {
        let total = self.counts.get(&(node_id, is_top)).copied().unwrap_or(1);
        let slot = self.slots.get(&(edge_id, node_id, is_top)).copied().unwrap_or(0);
        nl.pill_port_distributed_x(slot, total)
    }
}

/// Get the physical Y coordinate of the opposite endpoint for port slot ordering.
fn opposite_y(
    graph: &Graph,
    edge: &Edge,
    my_role: EndpointRole,
    node_layouts: &[NodeLayout],
    prop_order: &super::crossing::PropertyOrder,
) -> f64 {
    let (upstream, downstream) = layout_endpoints(edge);
    let opp = match my_role {
        EndpointRole::Upstream => &downstream,
        EndpointRole::Downstream => &upstream,
    };
    match opp {
        LayoutEndpoint::Node(nid) => {
            find_node_layout(node_layouts, *nid)
                .map(|nl| nl.y + nl.height / 2.0)
                .unwrap_or(0.0)
        }
        LayoutEndpoint::Prop(pid) => {
            let nid = graph.properties[pid.index()].node;
            find_node_layout(node_layouts, nid)
                .and_then(|nl| {
                    if graph.nodes[nid.index()].is_derivation() {
                        Some(nl.pill_center_y())
                    } else {
                        let idx = prop_order.prop_index(nid, *pid)?;
                        Some(nl.port_y(idx))
                    }
                })
                .unwrap_or(0.0)
        }
    }
}

/// Compute the (x, y) port position and optional side for an edge endpoint.
///
/// Returns `(x, y, Option<PortSide>)`. The side is `None` for link center ports.
/// When `distributor` is provided, property ports use distributed y placement.
#[allow(clippy::too_many_arguments)]
fn port_position(
    graph: &Graph,
    edge_id: EdgeId,
    role: EndpointRole,
    node_layouts: &[NodeLayout],
    port_sides: &PortSideAssignment,
    distributor: &PortDistributor,
    pill_distributor: &PillPortDistributor,
    prop_order: &super::crossing::PropertyOrder,
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
                    let is_pill = graph.nodes[node_id.index()].is_derivation();

                    if is_pill {
                        // Pill as source → bottom port, distributed x.
                        // Use the same side as regular edges so routing
                        // follows the identical H-V-H / H-V-H-V-H code path.
                        let x = pill_distributor.port_x(nl, edge_id, node_id, false);
                        let y = nl.pill_port_bottom_y();
                        Some((x, y, side))
                    } else {
                        let prop_idx = prop_order.prop_index(node_id, *source_prop)?;
                        let x = match side {
                            Some(PortSide::Left) => nl.port_left_x(),
                            Some(PortSide::Right) | None => nl.port_right_x(),
                        };
                        let y = match side {
                            Some(s) => distributor.port_y(nl, prop_idx, edge_id, *source_prop, s),
                            None => nl.port_y(prop_idx),
                        };
                        Some((x, y, side))
                    }
                }
                EndpointRole::Downstream => {
                    let node_id = prop_node(graph, *dest_prop);
                    let nl = find_node_layout(node_layouts, node_id)?;
                    let is_pill = graph.nodes[node_id.index()].is_derivation();

                    if is_pill {
                        // Pill as target → top port, distributed x.
                        let x = pill_distributor.port_x(nl, edge_id, node_id, true);
                        let y = nl.pill_port_top_y();
                        Some((x, y, side))
                    } else {
                        let prop_idx = prop_order.prop_index(node_id, *dest_prop)?;
                        let x = match side {
                            Some(PortSide::Left) => nl.port_left_x(),
                            Some(PortSide::Right) | None => nl.port_right_x(),
                        };
                        let y = match side {
                            Some(s) => distributor.port_y(nl, prop_idx, edge_id, *dest_prop, s),
                            None => nl.port_y(prop_idx),
                        };
                        Some((x, y, side))
                    }
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
#[allow(clippy::too_many_arguments)]
fn route_single_edge(
    edge_id: EdgeId,
    src_x: f64,
    src_y: f64,
    src_side: Option<PortSide>,
    tgt_x: f64,
    tgt_y: f64,
    tgt_side: Option<PortSide>,
    h_channels: &mut [Channel],
    corridors: &mut [Corridor],
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

    // Case 2: All constraint edges — unified H-V-H-V-H routing.
    //
    // Every constraint edge uses the same 5-segment pattern:
    //   H(src_port → v1) - V(v1: src_y → h_y) - H(h_y: v1 → v2) - V(v2: h_y → tgt_y) - H(v2 → tgt_port)
    //
    // Degenerate segments (zero length) are removed by collapse_zero_length,
    // which naturally produces H-V-H (3 segments) when v1==v2, or even
    // a single V when src_x==v1==v2==tgt_x.
    // Both sides must be Some for constraint edges — anchors return in Case 1.
    // unwrap with a clear message rather than silently defaulting.
    let src_s = src_side.expect("constraint edge missing src_side");
    let tgt_s = tgt_side.expect("constraint edge missing tgt_side");

    // Probe corridors (non-mutating) to decide if both endpoints share one.
    let src_corr = find_best_corridor_idx(src_x, src_s, corridors, src_domain);
    let tgt_corr = find_best_corridor_idx(tgt_x, tgt_s, corridors, tgt_domain);
    let same_corridor = matches!((src_corr, tgt_corr), (Some(si), Some(ti)) if si == ti);

    if same_corridor {
        // Single-corridor H-V-H: allocate one channel spanning src→tgt.
        let ci = src_corr.unwrap();
        let v_x = corridors[ci].allocate_channel(edge_id, src_y, tgt_y);
        collapse_zero_length(vec![
            Segment::Horizontal { y: src_y, x_start: src_x, x_end: v_x },
            Segment::Vertical { x: v_x, y_start: src_y, y_end: tgt_y },
            Segment::Horizontal { y: tgt_y, x_start: v_x, x_end: tgt_x },
        ])
    } else {
        // Two-corridor H-V-H-V-H: need a horizontal transfer channel.
        let h_y = if let Some(hi) = find_h_channel_between(h_channels, src_y, tgt_y) {
            h_channels[hi].reserve(edge_id)
        } else {
            (src_y + tgt_y) / 2.0
        };
        let v1_x = find_corridor_channel(src_x, src_s, corridors, edge_id, src_y, h_y, src_domain);
        let v2_x = find_corridor_channel(tgt_x, tgt_s, corridors, edge_id, h_y, tgt_y, tgt_domain);
        collapse_zero_length(vec![
            Segment::Horizontal { y: src_y, x_start: src_x, x_end: v1_x },
            Segment::Vertical { x: v1_x, y_start: src_y, y_end: h_y },
            Segment::Horizontal { y: h_y, x_start: v1_x, x_end: v2_x },
            Segment::Vertical { x: v2_x, y_start: h_y, y_end: tgt_y },
            Segment::Horizontal { y: tgt_y, x_start: v2_x, x_end: tgt_x },
        ])
    }
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
    if route.segments.is_empty() {
        return;
    }

    let last_len = route.segments.last().unwrap().length();

    if last_len >= amount + 0.001 {
        // Last segment is long enough — shorten it directly.
        let seg = route.segments.last_mut().unwrap();
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
    } else if route.segments.len() >= 2 {
        // Last segment is shorter than the arrowhead.  Absorb it entirely
        // and take the excess from the penultimate segment, then re-attach
        // a minimal-length final segment so the arrowhead marker still
        // points in the original direction.
        let overflow = amount - last_len;
        let final_seg = route.segments.pop().unwrap();

        // Shorten the (now-last) penultimate segment by the overflow.
        if let Some(pen) = route.segments.last_mut() {
            match pen {
                Segment::Horizontal { x_end, x_start, .. } => {
                    if *x_end >= *x_start {
                        *x_end -= overflow;
                    } else {
                        *x_end += overflow;
                    }
                }
                Segment::Vertical { y_end, y_start, .. } => {
                    if *y_end >= *y_start {
                        *y_end -= overflow;
                    } else {
                        *y_end += overflow;
                    }
                }
            }
        }

        // Re-attach the final segment, starting from the new endpoint of
        // the penultimate segment with near-zero length (preserves arrow direction).
        let new_start = route.segments.last().map(|s| s.end()).unwrap_or((0.0, 0.0));
        let reattached = match final_seg {
            Segment::Horizontal { x_start, x_end, .. } => {
                let dir = if x_end >= x_start { 1.0 } else { -1.0 };
                Segment::Horizontal {
                    y: new_start.1,
                    x_start: new_start.0,
                    x_end: new_start.0 + dir * 0.01,
                }
            }
            Segment::Vertical { y_start, y_end, .. } => {
                let dir = if y_end >= y_start { 1.0 } else { -1.0 };
                Segment::Vertical {
                    x: new_start.0,
                    y_start: new_start.1,
                    y_end: new_start.1 + dir * 0.01,
                }
            }
        };
        route.segments.push(reattached);
    }
    // If only one segment and it's too short, leave as-is.
}

// ---------------------------------------------------------------------------
// Edge priority for routing order
// ---------------------------------------------------------------------------

/// Routing priority: lower number = routed first = gets best channels.
fn edge_priority(edge: &Edge) -> u32 {
    match edge {
        Edge::Anchor { .. } => 0,
        Edge::Constraint { .. } => 1,
    }
}


/// Compute the vertical midpoint of an edge's endpoints in coordinate space.
/// Used for topology-aware corridor channel allocation: edges are sorted by
/// (priority, vertical_midpoint) so higher edges get channels first.
fn edge_vertical_midpoint(
    graph: &Graph,
    edge: &Edge,
    node_layouts: &[NodeLayout],
    prop_order: &super::crossing::PropertyOrder,
) -> f64 {
    let endpoint_y = |ep: &LayoutEndpoint| -> Option<f64> {
        match ep {
            LayoutEndpoint::Node(nid) => {
                let nl = find_node_layout(node_layouts, *nid)?;
                Some(nl.y + nl.height / 2.0)
            }
            LayoutEndpoint::Prop(pid) => {
                let nid = graph.properties[pid.index()].node;
                let nl = find_node_layout(node_layouts, nid)?;
                if graph.nodes[nid.index()].is_derivation() {
                    Some(nl.pill_center_y())
                } else {
                    let idx = prop_order.prop_index(nid, *pid)?;
                    Some(nl.port_y(idx))
                }
            }
        }
    };

    let (upstream, downstream) = layout_endpoints(edge);
    let y1 = endpoint_y(&upstream).unwrap_or(0.0);
    let y2 = endpoint_y(&downstream).unwrap_or(0.0);
    (y1 + y2) / 2.0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Route all edges using orthogonal corridor-based routing.
///
/// Edges are routed in priority order (anchors first, then constraints),
/// with topology-aware secondary sorting by vertical midpoint so corridor
/// channels are allocated top-to-bottom.
pub fn route_all_edges(
    graph: &Graph,
    node_layouts: &[NodeLayout],
    domain_layouts: &[DomainLayout],
    port_sides: &PortSideAssignment,
    prop_order: &super::crossing::PropertyOrder,
) -> Vec<Route> {
    let mut h_channels = build_h_channels(node_layouts);
    let mut corridors = build_corridors(domain_layouts, node_layouts, graph);

    let distributor = PortDistributor::new(
        graph, port_sides, node_layouts, prop_order,
    );
    let pill_distributor = PillPortDistributor::new(graph, node_layouts);

    // Build a priority-sorted list of edge indices with topology-aware secondary sort.
    let mut edge_indices: Vec<usize> = (0..graph.edges.len()).collect();
    edge_indices.sort_by(|&a, &b| {
        let pa = edge_priority(&graph.edges[a]);
        let pb = edge_priority(&graph.edges[b]);
        pa.cmp(&pb).then_with(|| {
            let ma = edge_vertical_midpoint(graph, &graph.edges[a], node_layouts, prop_order);
            let mb = edge_vertical_midpoint(graph, &graph.edges[b], node_layouts, prop_order);
            ma.partial_cmp(&mb).unwrap_or(std::cmp::Ordering::Equal)
        })
    });


    let mut routes = Vec::new();

    for idx in edge_indices {
        let edge_id = EdgeId(idx as u32);

        let src = port_position(
            graph,
            edge_id,
            EndpointRole::Upstream,
            node_layouts,
            port_sides,
            &distributor,
            &pill_distributor,
            prop_order,
        );
        let tgt = port_position(
            graph,
            edge_id,
            EndpointRole::Downstream,
            node_layouts,
            port_sides,
            &distributor,
            &pill_distributor,
            prop_order,
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
        // Cross-domain constraint edges: use (None, None) to select the
        // inter-column gap corridor.
        let (src_domain, tgt_domain) = {
            let edge = &graph.edges[idx];
            let (src_nid, tgt_nid) = graph.edge_nodes(edge);
            let sd = graph.nodes[src_nid.index()].domain;
            let td = graph.nodes[tgt_nid.index()].domain;
            if sd == td { (sd, td) } else { (None, None) }
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

    // Post-route channel reassignment, in two phases:
    //
    // Phase 1: Fan-out ordering. When multiple H-V-H bracket routes share
    // the same corridor, order channels so that edges entering from lower y
    // (nearer source ports) get outermost channels. This ensures horizontal
    // stubs nest properly — no outer stub crosses an inner vertical.
    //
    // Phase 2: Bracket nesting. For same-node-pair brackets, swap channels
    // to minimize within-pair crossings. This runs after fan-out ordering
    // and only adjusts adjacent channels within a pair.
    fix_fanout_channel_order(graph, &mut routes);
    fix_bracket_nesting_channels(graph, &mut routes);

    routes
}

/// Fix fan-out channel ordering for H-V-H bracket routes through shared corridors.
///
/// When multiple edges from the same source region fan out through a single
/// corridor, the initial channel allocation (by routing order) assigns inner
/// channels to edges with smaller vertical midpoints. This creates crossings:
/// each outer edge's horizontal stub must pass through all inner edges'
/// vertical segments.
///
/// The fix reverses the assignment: edges entering from lower source y get
/// the **outermost** corridor channel. This ensures horizontal stubs nest
/// properly — the lowest-y stub is longest but passes over no verticals,
/// and each higher-y stub is shorter and contained within the outer stubs.
///
/// # Grouping
///
/// Routes are grouped by corridor region: edges whose vertical segment x
/// values cluster within `CHANNEL_GAP * 3` are in the same corridor.
/// Within each corridor cluster, channels are reassigned by ascending src_y.
///
/// # Interaction with `fix_bracket_nesting_channels`
///
/// This function runs BEFORE bracket nesting. Bracket nesting only swaps
/// adjacent channels within same-node pairs, preserving the overall fan-out
/// ordering. The two passes are compatible because same-node pairs have
/// adjacent source y values, so their relative order within the fan-out
/// doesn't affect cross-pair crossings.
fn fix_fanout_channel_order(graph: &Graph, routes: &mut [Route]) {
    struct FanoutEntry {
        route_idx: usize,
        corridor_x: f64,
        src_y: f64,
        src_x: f64,
        approaches_from_right: bool, // src_x > corridor_x
    }

    let mut entries: Vec<FanoutEntry> = Vec::new();

    for (ri, route) in routes.iter().enumerate() {
        let edge = &graph.edges[route.edge_id.index()];
        // Only cross-domain constraint edges benefit from fan-out reordering.
        // Intra-domain brackets are handled by fix_bracket_nesting_channels.
        let is_cross_domain = match edge {
            Edge::Constraint { .. } => {
                let (sn, dn) = graph.edge_nodes(edge);
                graph.nodes[sn.index()].domain != graph.nodes[dn.index()].domain
            }
            _ => false,
        };
        if !is_cross_domain { continue; }
        if route.segments.len() != 3 { continue; }

        let (src_x, src_y, corridor_x) = match (&route.segments[0], &route.segments[1]) {
            (
                Segment::Horizontal { y, x_start, .. },
                Segment::Vertical { x, .. },
            ) => (*x_start, *y, *x),
            _ => continue,
        };

        let approaches_from_right = src_x > corridor_x;
        entries.push(FanoutEntry { route_idx: ri, corridor_x, src_y, src_x, approaches_from_right });
    }

    if entries.len() < 2 { return; }

    // Cluster entries by corridor region AND approach direction.
    // Two edges share a fan-out group only if they use the same corridor
    // AND approach from the same side. The corridor allocator can reuse
    // a channel for edges approaching from opposite sides (non-overlapping
    // horizontal stubs), so mixing sides would create false channel swaps.
    //
    // Step 1: Sort by corridor_x, cluster adjacent entries within threshold.
    // Step 2: Sub-group each cluster by approach direction.
    entries.sort_by(|a, b| a.corridor_x.partial_cmp(&b.corridor_x).unwrap_or(std::cmp::Ordering::Equal));

    let threshold = CHANNEL_GAP * 3.0;
    let mut raw_clusters: Vec<Vec<usize>> = Vec::new();
    let mut current_cluster: Vec<usize> = vec![0];

    for i in 1..entries.len() {
        if (entries[i].corridor_x - entries[i - 1].corridor_x).abs() <= threshold {
            current_cluster.push(i);
        } else {
            raw_clusters.push(std::mem::take(&mut current_cluster));
            current_cluster.push(i);
        }
    }
    raw_clusters.push(current_cluster);

    // Sub-group by approach direction.
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for raw in &raw_clusters {
        let mut from_left: Vec<usize> = Vec::new();
        let mut from_right: Vec<usize> = Vec::new();
        for &ci in raw {
            if entries[ci].approaches_from_right {
                from_right.push(ci);
            } else {
                from_left.push(ci);
            }
        }
        if from_left.len() >= 2 { clusters.push(from_left); }
        if from_right.len() >= 2 { clusters.push(from_right); }
    }

    for cluster_indices in &clusters {
        // Determine corridor direction: channels grow away from source.
        let first = &entries[cluster_indices[0]];
        let is_right_corridor = first.corridor_x > first.src_x;

        // Collect current corridor_x values and sort by distance from source
        // (innermost first). If fewer unique x-values than edges (multiple
        // edges share a channel), expand with additional channels so every
        // edge gets a unique channel.
        let ref_x = first.src_x;
        let mut xs: Vec<f64> = cluster_indices.iter().map(|&ci| entries[ci].corridor_x).collect();
        xs.sort_by(|a, b| {
            let da = (*a - ref_x).abs();
            let db = (*b - ref_x).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        // Ensure unique channels: if duplicates exist, expand outward.
        xs.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        while xs.len() < cluster_indices.len() {
            let last = *xs.last().unwrap();
            xs.push(if is_right_corridor { last + CHANNEL_GAP } else { last - CHANNEL_GAP });
        }

        // Sort cluster entries by src_y ascending (lowest enters first → outermost).
        let mut sorted_indices: Vec<usize> = cluster_indices.clone();
        sorted_indices.sort_by(|&a, &b| {
            entries[a].src_y.partial_cmp(&entries[b].src_y).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Assign channels: entry with lowest src_y gets outermost (xs[last]),
        // entry with highest src_y gets innermost (xs[0]).
        let n = sorted_indices.len();
        for (rank, &ci) in sorted_indices.iter().enumerate() {
            let new_x = xs[n - 1 - rank]; // reverse: lowest src_y → outermost
            let old_x = entries[ci].corridor_x;
            if (new_x - old_x).abs() < 0.01 { continue; } // no change

            let route = &mut routes[entries[ci].route_idx];

            // Update the vertical segment x.
            if let Segment::Vertical { x, .. } = &mut route.segments[1] {
                *x = new_x;
            }
            // Update the horizontal segments' corridor-side endpoints.
            update_corridor_endpoint(&mut route.segments[0], old_x, new_x, is_right_corridor);
            update_corridor_endpoint(&mut route.segments[2], old_x, new_x, is_right_corridor);
        }
    }
}

/// Update the corridor-side endpoint of a horizontal segment.
fn update_corridor_endpoint(seg: &mut Segment, old_x: f64, new_x: f64, _is_right: bool) {
    if let Segment::Horizontal { x_start, x_end, .. } = seg {
        if (*x_start - old_x).abs() < (*x_end - old_x).abs() {
            *x_start = new_x;
        } else {
            *x_end = new_x;
        }
    }
}

/// Fix bracket nesting by reassigning corridor channel x-coordinates.
///
/// For same-node-pair constraint bundles routed as brackets (H-V-H pattern)
/// through the same corridor, the inner bracket (shortest vertical span)
/// should use the inner channel (closest to the node), and the outer bracket
/// (widest span) should use the outer channel.  The initial routing order
/// may assign channels arbitrarily; this post-processing step swaps the
/// corridor x-coordinates to achieve correct nesting.
fn fix_bracket_nesting_channels(graph: &Graph, routes: &mut [Route]) {
    use std::collections::HashMap;

    // Identify bracket routes: H-V-H pattern with 3 segments.
    // Extract (edge_id, corridor_x, span) for each bracket.
    struct BracketInfo {
        route_idx: usize,
        corridor_x: f64,
        src_y: f64,
        dst_y: f64,
        dst_x: f64,  // destination node x (from seg2's non-corridor endpoint)
    }

    // Group bracket routes by (node_pair, corridor_x_sign).
    // corridor_x_sign distinguishes left vs right corridors.
    let mut bundles: HashMap<(u32, u32, bool), Vec<BracketInfo>> = HashMap::new();

    for (ri, route) in routes.iter().enumerate() {
        let edge = &graph.edges[route.edge_id.index()];
        let (src_node, dst_node) = match edge {
            Edge::Constraint { source_prop, dest_prop, .. } => {
                (prop_node(graph, *source_prop), prop_node(graph, *dest_prop))
            }
            _ => continue,
        };
        if src_node == dst_node { continue; }

        // Check for H-V-H bracket pattern (3 segments).
        if route.segments.len() != 3 { continue; }
        let corridor_x = match &route.segments[1] {
            Segment::Vertical { x, .. } => *x,
            _ => continue,
        };
        let (src_y, tgt_y) = match (&route.segments[0], &route.segments[2]) {
            (Segment::Horizontal { y: sy, .. }, Segment::Horizontal { y: ty, .. }) => (*sy, *ty),
            _ => continue,
        };

        let (lo, hi) = if src_node.0 <= dst_node.0 {
            (src_node.0, dst_node.0)
        } else {
            (dst_node.0, src_node.0)
        };
        // Group by node pair and whether the corridor is on the left or
        // right side (using the first segment's horizontal direction).
        let src_x = match &route.segments[0] {
            Segment::Horizontal { x_start, .. } => *x_start,
            _ => continue,
        };
        let is_right = corridor_x > src_x;

        // Determine dest_x: the non-corridor endpoint of seg2.
        let dst_x = match &route.segments[2] {
            Segment::Horizontal { x_start, x_end, .. } => {
                if (*x_start - corridor_x).abs() < (*x_end - corridor_x).abs() {
                    *x_end  // x_start is corridor-side, x_end is dest-side
                } else {
                    *x_start
                }
            }
            _ => continue,
        };

        bundles.entry((lo, hi, is_right)).or_default().push(BracketInfo {
            route_idx: ri,
            corridor_x,
            src_y,
            dst_y: tgt_y,
            dst_x,
        });
    }

    // For each bundle with ≥2 brackets, reassign corridor x-coordinates
    // so brackets nest properly (no source-side horizontal crosses an
    // inner vertical).  The edge whose vertical starts earliest (smallest
    // min_y) must be outermost — its horizontal enters above all inner
    // verticals.
    for bundle in bundles.values_mut() {
        if bundle.len() < 2 { continue; }

        // Collect current x-coordinates sorted by distance from node
        // (innermost first).
        let mut xs: Vec<f64> = bundle.iter().map(|b| b.corridor_x).collect();
        // Sort by absolute distance from the first segment's start x
        // (which is the node edge). Inner channels are closer.
        let node_x = match &routes[bundle[0].route_idx].segments[0] {
            Segment::Horizontal { x_start, .. } => *x_start,
            _ => continue,
        };
        xs.sort_by(|a, b| {
            let da = (*a - node_x).abs();
            let db = (*b - node_x).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });

        // For each pair of edges, determine the assignment (inner vs outer)
        // that minimizes geometric crossings.  An outer edge's horizontal
        // segments cross the inner edge's vertical when the horizontal's y
        // falls within the inner's vertical span AND the horizontal passes
        // through the inner channel's x.
        //
        // Source-side: the outer horizontal always passes through inner_x
        //   (it goes from source past inner to outer).
        // Dest-side: the outer horizontal passes through inner_x only when
        //   inner_x is between outer_x and dest_x (same-side corridor).
        //
        // For bundles > 2 edges, fall back to min_y descending sort.
        // Count how many crossings a given inner/outer assignment produces.
        let count_crossings = |inner_src_y: f64, inner_dst_y: f64, inner_dst_x: f64,
                                outer_src_y: f64, outer_dst_y: f64, outer_dst_x: f64,
                                inner_x: f64, outer_x: f64| -> usize {
            let inner_lo = inner_src_y.min(inner_dst_y);
            let inner_hi = inner_src_y.max(inner_dst_y);
            let mut n = 0;
            // Source-side: outer horizontal at outer_src_y passes through inner_x.
            if outer_src_y > inner_lo - 0.5 && outer_src_y < inner_hi + 0.5 { n += 1; }
            // Dest-side: outer horizontal passes through inner_x only when
            // inner_x is between outer_x and dest_x (same-side corridor).
            let dest_passes_inner = (inner_x - outer_x) * (inner_x - outer_dst_x) < 0.0;
            if dest_passes_inner &&
               outer_dst_y > inner_lo - 0.5 && outer_dst_y < inner_hi + 0.5 { n += 1; }
            let _ = inner_dst_x; // used only for symmetry with outer_dst_x
            n
        };

        if bundle.len() == 2 {
            let (a, b) = (&bundle[0], &bundle[1]);
            // xs[0] = innermost, xs[1] = outermost
            let ca = count_crossings(a.src_y, a.dst_y, a.dst_x, b.src_y, b.dst_y, b.dst_x, xs[0], xs[1]);
            let cb = count_crossings(b.src_y, b.dst_y, b.dst_x, a.src_y, a.dst_y, a.dst_x, xs[0], xs[1]);
            if cb < ca {
                bundle.swap(0, 1);
            } else if ca == cb && ca > 0 {
                // Tied with crossings — try swapping source y values.
                // This changes which edge enters the corridor first, often
                // breaking the tie for same-side corridor brackets.
                let (sa, sb) = (a.src_y, b.src_y);
                let ca_s = count_crossings(a.src_y, a.dst_y, a.dst_x, sb, b.dst_y, b.dst_x, xs[0], xs[1]);
                let _ = ca_s; // baseline with no src swap
                // Try: a gets b's src_y, b gets a's src_y
                let ca2 = count_crossings(sb, a.dst_y, a.dst_x, sa, b.dst_y, b.dst_x, xs[0], xs[1]);
                let cb2 = count_crossings(sa, b.dst_y, b.dst_x, sb, a.dst_y, a.dst_x, xs[0], xs[1]);
                if ca2 == 0 || cb2 == 0 {
                    // Source y swap helps — apply it to the routes.
                    // Extract y values, swap, and write back (avoids double borrow).
                    let ri_a = bundle[0].route_idx;
                    let ri_b = bundle[1].route_idx;
                    let ya_h = match &routes[ri_a].segments[0] {
                        Segment::Horizontal { y, .. } => *y,
                        _ => continue,
                    };
                    let yb_h = match &routes[ri_b].segments[0] {
                        Segment::Horizontal { y, .. } => *y,
                        _ => continue,
                    };
                    if let Segment::Horizontal { y, .. } = &mut routes[ri_a].segments[0] { *y = yb_h; }
                    if let Segment::Horizontal { y, .. } = &mut routes[ri_b].segments[0] { *y = ya_h; }
                    // Also update the vertical segment y_start to match.
                    let ya_v = match &routes[ri_a].segments[1] {
                        Segment::Vertical { y_start, .. } => *y_start,
                        _ => continue,
                    };
                    let yb_v = match &routes[ri_b].segments[1] {
                        Segment::Vertical { y_start, .. } => *y_start,
                        _ => continue,
                    };
                    if let Segment::Vertical { y_start, .. } = &mut routes[ri_a].segments[1] { *y_start = yb_v; }
                    if let Segment::Vertical { y_start, .. } = &mut routes[ri_b].segments[1] { *y_start = ya_v; }
                    // Update bundle src_y for subsequent assignment.
                    bundle[0] = BracketInfo { src_y: sb, ..bundle[0] };
                    bundle[1] = BracketInfo { src_y: sa, ..bundle[1] };
                    // Pick the crossing-free assignment.
                    if cb2 < ca2 {
                        bundle.swap(0, 1);
                    }
                } else {
                    // Swap didn't help — fall back to min_y tiebreak.
                    let a_min = bundle[0].src_y.min(bundle[0].dst_y);
                    let b_min = bundle[1].src_y.min(bundle[1].dst_y);
                    if b_min > a_min {
                        bundle.swap(0, 1);
                    }
                }
            } else if ca == cb {
                // Tied at 0 — use min_y descending tiebreak.
                let a_min = bundle[0].src_y.min(bundle[0].dst_y);
                let b_min = bundle[1].src_y.min(bundle[1].dst_y);
                if b_min > a_min {
                    bundle.swap(0, 1);
                }
            }
        } else {
            // Fall back to min_y descending for larger bundles.
            bundle.sort_by(|a, b| {
                let a_min = a.src_y.min(a.dst_y);
                let b_min = b.src_y.min(b.dst_y);
                b_min.partial_cmp(&a_min).unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        // Assign xs[i] to bundle[i]: shortest span → innermost x.
        for (i, info) in bundle.iter().enumerate() {
            let new_x = xs[i];
            let old_x = info.corridor_x;
            let route = &mut routes[info.route_idx];
            // Update the vertical segment x.
            if let Segment::Vertical { x, .. } = &mut route.segments[1] {
                *x = new_x;
            }
            // Update the horizontal segments' corridor-side endpoints.
            // The corridor-side endpoint is the one matching the old corridor_x.
            if let Segment::Horizontal { x_start, x_end, .. } = &mut route.segments[0] {
                if (*x_start - old_x).abs() < (*x_end - old_x).abs() {
                    *x_start = new_x;
                } else {
                    *x_end = new_x;
                }
            }
            if let Segment::Horizontal { x_start, x_end, .. } = &mut route.segments[2] {
                if (*x_start - old_x).abs() < (*x_end - old_x).abs() {
                    *x_start = new_x;
                } else {
                    *x_end = new_x;
                }
            }
        }
    }
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

/// Generate a short dotted stub from the last STUB_LENGTH pixels of a route.
///
/// Walks backwards along the route's segments from the destination end,
/// consuming up to STUB_LENGTH total distance. Returns a single `Route`
/// representing the dotted stub near the destination port.
pub fn generate_stub(route: &Route) -> Route {
    let mut remaining = STUB_LENGTH;
    let mut tail_segments: Vec<Segment> = Vec::new();

    for seg in route.segments.iter().rev() {
        let len = seg.length();

        if len <= remaining + 0.001 {
            tail_segments.push(seg.clone());
            remaining -= len;
            if remaining < 0.001 {
                break;
            }
        } else {
            // Truncate this segment from the destination end: keep only the
            // last `remaining` pixels.
            let fraction = remaining / len;
            match seg {
                Segment::Horizontal { y, x_start, x_end } => {
                    let new_x_start = x_start + (x_end - x_start) * (1.0 - fraction);
                    tail_segments.push(Segment::Horizontal {
                        y: *y,
                        x_start: new_x_start,
                        x_end: *x_end,
                    });
                }
                Segment::Vertical { x, y_start, y_end } => {
                    let new_y_start = y_start + (y_end - y_start) * (1.0 - fraction);
                    tail_segments.push(Segment::Vertical {
                        x: *x,
                        y_start: new_y_start,
                        y_end: *y_end,
                    });
                }
            }
            break;
        }
    }

    // Reverse to restore source-to-destination order.
    tail_segments.reverse();

    Route {
        edge_id: route.edge_id,
        segments: tail_segments,
    }
}

/// Generate candidate label positions for a route, ordered by preference.
///
/// Returns up to 4 candidates.  The caller selects the one with fewest
/// collisions against already-placed labels and layout obstacles.
///
/// Candidate strategies:
///
/// 1. **Middle horizontal segment** (5-segment H-V-H-V-H routes or V-H-V
///    anchor routes): inter-layer space with few obstacles.
///
/// 2. **First horizontal segment** of an H-V-H bracket route: near the
///    source port, each edge at a different y → natural separation.
///
/// 3. **Vertical segment at 25%**: near the source junction, away from the
///    corridor midpoint where other edges are densest.
///
/// 4. **Vertical segment at 50%**: original midpoint position (fallback).
pub fn route_label_candidates(route: &Route) -> Vec<(f64, f64, &'static str)> {
    let mut candidates = Vec::new();
    let n = route.segments.len();

    // Candidate A: 5-segment route — middle horizontal segment (index 2).
    if n == 5
        && let Segment::Horizontal { y, x_start, x_end } = &route.segments[2]
    {
        let mid_x = (x_start + x_end) / 2.0;
        candidates.push((mid_x, *y - 4.0, "middle"));
    }

    // Candidate B: V-H-V anchor route — middle horizontal segment (index 1).
    if n == 3
        && let (
            Segment::Vertical { .. },
            Segment::Horizontal { y, x_start, x_end },
            Segment::Vertical { .. },
        ) = (&route.segments[0], &route.segments[1], &route.segments[2])
    {
        let mid_x = (x_start + x_end) / 2.0;
        candidates.push((mid_x, *y - 4.0, "middle"));
    }

    // Candidate C: first horizontal segment midpoint (bracket or 5-seg routes).
    if (n == 3 || n == 5)
        && let Segment::Horizontal { y, x_start, x_end } = &route.segments[0]
        && (x_end - x_start).abs() > 1.0
    {
        let mid_x = (x_start + x_end) / 2.0;
        candidates.push((mid_x, *y - 4.0, "middle"));
    }

    // Candidate D: last horizontal segment midpoint.
    if n >= 3
        && let Segment::Horizontal { y, x_start, x_end } = route.segments.last().unwrap()
        && (x_end - x_start).abs() > 1.0
    {
        let mid_x = (x_start + x_end) / 2.0;
        let pos = (mid_x, *y - 4.0, "middle");
        if !candidates.iter().any(|c| (c.0 - pos.0).abs() < 1.0 && (c.1 - pos.1).abs() < 1.0) {
            candidates.push(pos);
        }
    }

    // Candidate E/F/G/H: Vertical segment at 25% and 50%, on both sides.
    //
    // For bracket routes (H-V-H) the vertical runs outside the node column.
    // Placing the label on the *outward* side (away from nodes) can push it
    // past the domain boundary, while placing it on the *inward* side (toward
    // the inter-node gap) keeps it in free space.  We generate candidates on
    // both sides so the collision-aware scorer can pick the best one.
    for (i, seg) in route.segments.iter().enumerate() {
        if let Segment::Vertical { x, y_start, y_end } = seg {
            let offset_right = if i > 0 {
                match &route.segments[i - 1] {
                    Segment::Horizontal { x_start, x_end, .. } => x_end > x_start,
                    _ => true,
                }
            } else {
                true
            };
            let (out_x, out_anchor, in_x, in_anchor) = if offset_right {
                (*x + 4.0, "start", *x - 4.0, "end")
            } else {
                (*x - 4.0, "end", *x + 4.0, "start")
            };

            // 25% position (near source junction) — inward side first (preferred).
            let y_25 = y_start + (y_end - y_start) * 0.25;
            candidates.push((in_x, y_25, in_anchor));

            // 25% position — outward side.
            candidates.push((out_x, y_25, out_anchor));

            // 50% position (midpoint) — inward side first.
            let y_50 = (y_start + y_end) / 2.0;
            candidates.push((in_x, y_50, in_anchor));

            // 50% position — outward side.
            candidates.push((out_x, y_50, out_anchor));

            // 75% position (near target junction) — inward side first.
            let y_75 = y_start + (y_end - y_start) * 0.75;
            candidates.push((in_x, y_75, in_anchor));

            // 75% position — outward side.
            candidates.push((out_x, y_75, out_anchor));

            break; // Only use first vertical segment.
        }
    }

    // Fallback: arc-length midpoint.
    if candidates.is_empty() {
        let total: f64 = route.segments.iter().map(|s| s.length()).sum();
        if total < 1e-9 {
            let (x, y) = route.segments.first().map(|s| s.start()).unwrap_or((0.0, 0.0));
            candidates.push((x, y, "middle"));
        } else {
            let mut remaining = total / 2.0;
            for seg in &route.segments {
                let len = seg.length();
                if remaining <= len {
                    let frac = remaining / len;
                    let (sx, sy) = seg.start();
                    let (ex, ey) = seg.end();
                    candidates.push((sx + (ex - sx) * frac, sy + (ey - sy) * frac, "middle"));
                    break;
                }
                remaining -= len;
            }
            if candidates.is_empty() {
                let (x, y) = route.segments.last().map(|s| s.end()).unwrap_or((0.0, 0.0));
                candidates.push((x, y, "middle"));
            }
        }
    }

    candidates
}

/// Returns the label position for a route (simple version without collision
/// avoidance).  Picks the first candidate from `route_label_candidates`.
pub fn route_label_position(route: &Route) -> (f64, f64, &'static str) {
    route_label_candidates(route)
        .into_iter()
        .next()
        .unwrap_or((0.0, 0.0, "middle"))
}

/// Convert a Route to an EdgePath.  If `label_text` is Some, an EdgeLabel is
/// placed along the first vertical corridor segment, offset 4px horizontally.
/// `font_size` specifies the label font size for bounding-box estimation.
pub fn route_to_edge_path(route: &Route, label_text: Option<String>, font_size: f64) -> EdgePath {
    let label = label_text.map(|text| {
        let (x, y, anchor) = route_label_position(route);
        EdgeLabel { text, x, y, anchor, font_size }
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
                ident: Some("A".into()),
                display_name: None,
                properties: vec![PropId(0)],
                domain: None,
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("B".into()),
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
        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let port_sides = refine_port_sides(&graph, &node_layouts, &[], &PortSideAssignment::new(), &prop_order);
        let routes = route_all_edges(&graph, &node_layouts, &[], &port_sides, &prop_order);

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
        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let port_sides = refine_port_sides(&graph, &node_layouts, &[], &PortSideAssignment::new(), &prop_order);
        let routes = route_all_edges(&graph, &node_layouts, &[], &port_sides, &prop_order);

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
        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let port_sides = refine_port_sides(&graph, &node_layouts, &[], &PortSideAssignment::new(), &prop_order);
        let routes = route_all_edges(&graph, &node_layouts, &[], &port_sides, &prop_order);

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

        // Simulate layer-space port side assignment: A is left of B in layer,
        // so upstream=Right (exit toward B), downstream=Left (enter from A).
        let mut layer_sides = PortSideAssignment::new();
        layer_sides.insert((EdgeId(1), EndpointRole::Upstream), PortSide::Right);
        layer_sides.insert((EdgeId(1), EndpointRole::Downstream), PortSide::Left);

        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let sides = refine_port_sides(&graph, &node_layouts, &[], &layer_sides, &prop_order);

        // Edge 0 is a Link: no side assignments.
        assert!(!sides.contains_key(&(EdgeId(0), EndpointRole::Upstream)));
        assert!(!sides.contains_key(&(EdgeId(0), EndpointRole::Downstream)));

        // Edge 1 is a Constraint from A.prop_a -> B.prop_b
        // Layer-space assigned Right/Left; no cross-domain override.
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
        // Simulate layer-space: same-position nodes get alternating sides.
        // First edge gets Right.
        let mut layer_sides = PortSideAssignment::new();
        layer_sides.insert((EdgeId(1), EndpointRole::Upstream), PortSide::Right);
        layer_sides.insert((EdgeId(1), EndpointRole::Downstream), PortSide::Right);

        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let sides = refine_port_sides(&graph, &node_layouts, &[], &layer_sides, &prop_order);

        // Same center x: layer-space assigned Right; no cross-domain override.
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
            ident: Some("A".into()),
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

        // Simulate layer-space: same-node constraint → Right.
        let mut layer_sides = PortSideAssignment::new();
        layer_sides.insert((EdgeId(0), EndpointRole::Upstream), PortSide::Right);
        layer_sides.insert((EdgeId(0), EndpointRole::Downstream), PortSide::Right);

        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let sides = refine_port_sides(&graph, &node_layouts, &[], &layer_sides, &prop_order);

        // Self-loop: both Right (from layer-space, preserved by refinement).
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
    fn test_stub_generation_from_destination_end() {
        // Route: 100px vertical. Stub should be the last STUB_LENGTH pixels
        // (from y=90 to y=100 with STUB_LENGTH=10).
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
                assert!(
                    (y_start - (100.0 - STUB_LENGTH)).abs() < 0.01,
                    "Stub start should be at y={}, got {}",
                    100.0 - STUB_LENGTH,
                    y_start
                );
                assert!(
                    (y_end - 100.0).abs() < 0.01,
                    "Stub end should be at destination y=100, got {}",
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
                y_end: 5.0,
            }],
        };

        let stub = generate_stub(&route);

        assert_eq!(stub.segments.len(), 1);
        match &stub.segments[0] {
            Segment::Vertical { y_start, y_end, .. } => {
                assert!((y_start - 0.0).abs() < 0.01);
                assert!((y_end - 5.0).abs() < 0.01);
            }
            other => panic!("Expected Vertical, got {:?}", other),
        }
    }

    #[test]
    fn test_stub_generation_multi_segment() {
        // Route with two segments: short vertical then long horizontal.
        // Stub extracts the last STUB_LENGTH pixels from the destination end.
        let seg1_len = STUB_LENGTH / 2.0; // 5px vertical
        let horiz_len = STUB_LENGTH * 3.0; // 30px horizontal
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
                    x_end: 40.0 + horiz_len,
                },
            ],
        };

        let stub = generate_stub(&route);

        // The last STUB_LENGTH (10px) is entirely within the 30px horizontal
        // segment. So the stub is a horizontal segment from x=(40+30-10)=60
        // to x=(40+30)=70.
        let horiz_end = 40.0 + horiz_len;
        let stub_start = horiz_end - STUB_LENGTH;

        assert_eq!(stub.segments.len(), 1);
        match &stub.segments[0] {
            Segment::Horizontal { y, x_start, x_end } => {
                assert!((y - seg1_len).abs() < 0.01);
                assert!(
                    (x_start - stub_start).abs() < 0.01,
                    "Stub x_start should be {}, got {}",
                    stub_start,
                    x_start
                );
                assert!(
                    (x_end - horiz_end).abs() < 0.01,
                    "Stub x_end should be at destination {}, got {}",
                    horiz_end,
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

        let edge_path = route_to_edge_path(&route, None, 8.0);
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

        let h_channels = build_h_channels(&node_layouts);

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

        // Create a minimal graph with two empty domains (no member nodes).
        let graph = Graph {
            nodes: vec![],
            properties: vec![],

            edges: vec![],
            domains: vec![
                crate::model::types::Domain {
                    id: DomainId(0),
                    display_name: "D0".into(),
                    members: vec![],
                },
                crate::model::types::Domain {
                    id: DomainId(1),
                    display_name: "D1".into(),
                    members: vec![],
                },
            ],
            prop_edges: HashMap::new(),
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        };

        let corridors = build_corridors(&domain_layouts, &[], &graph);

        // 2 domains at different x-ranges → 2 intra-domain corridors each +
        // 1 inter-column gap + 1 outer left + 1 outer right = 7 corridors.
        assert_eq!(corridors.len(), 7, "Expected 7 corridors, got {}", corridors.len());

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
                ident: Some("A".into()),
                display_name: None,
                properties: vec![PropId(0)],
                domain: Some(DomainId(0)),
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("B".into()),
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

            edges,
            domains,
            prop_edges,
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        };

        // Node A in D0 zone, node B in D1 zone.
        // Domain padding (lr_pad) = DOMAIN_PADDING + CORRIDOR_PAD*2 = 0 + 16 = 16px per side.
        // D0: x=0, nodes at x=16, domain width = 80 + 2*16 = 112, right edge = 112
        // Gap: 112..128 = 16px inter-domain corridor
        // D1: x=128, nodes at x=144, domain width = 112, right edge = 240
        let node_layouts = vec![
            NodeLayout {
                id: NodeId(0),
                x: 16.0, // inside D0 (domain x=0 + lr_pad=16)
                y: 0.0,
                width: 80.0,
                height: 52.0,
            },
            NodeLayout {
                id: NodeId(1),
                x: 144.0, // inside D1 (domain x=128 + lr_pad=16)
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
                width: 112.0, // 80 + 2*16
                height: 72.0,
            },
            DomainLayout {
                id: DomainId(1),
                display_name: "D1".into(),
                x: 128.0, // gap: 112..128 = 16px inter-domain corridor
                y: 80.0,
                width: 112.0,
                height: 72.0,
            },
        ];


        // Simulate layer-space: A is left of B → Upstream=Right, Downstream=Left.
        let mut layer_sides = PortSideAssignment::new();
        layer_sides.insert((EdgeId(0), EndpointRole::Upstream), PortSide::Right);
        layer_sides.insert((EdgeId(0), EndpointRole::Downstream), PortSide::Left);

        let prop_order = crate::layout::crossing::PropertyOrder::from_graph(&graph);
        let port_sides = refine_port_sides(&graph, &node_layouts, &domain_layouts, &layer_sides, &prop_order);

        let routes = route_all_edges(
            &graph,
            &node_layouts,
            &domain_layouts,
            &port_sides,
            &prop_order,
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

        // Inter-domain corridor: x_start=112, x_end=128, first channel at x_start + CORRIDOR_PAD = 120
        let inter_domain_first_channel = 120.0;
        // D0 right corridor center: 104 (node_right=96, domain_right=112, center=96+8)
        let d0_right_corridor = 104.0;
        // D1 left corridor center: 136 (domain_left=128, node_left=144, center=128+8)
        let d1_left_corridor = 136.0;

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
