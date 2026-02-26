pub mod coordinate;
pub mod crossing;
pub mod domain;
pub mod layer_assign;
pub mod long_edge;
pub mod quality;
pub mod routing;

use crate::model::types::{
    DerivId, DomainId, Edge, EdgeId, Graph, NodeId, PropId,
};

use long_edge::{LayerEntry, LayerItem};

// ---------------------------------------------------------------------------
// Sizing constants (LAYOUT.md §4.2.4)
// ---------------------------------------------------------------------------

/// Grid base unit; all spacing values are multiples of this.
pub const BASE_UNIT: f64 = 4.0;
/// Node header height (pad 12 + cap-height 8 + pad 12).
pub const HEADER_HEIGHT: f64 = 32.0;
/// Each property row height.
pub const ROW_HEIGHT: f64 = 20.0;
/// Horizontal padding inside nodes (left/right).
pub const CONTENT_PAD: f64 = 12.0;
/// Problem indicator dot radius.
pub const DOT_RADIUS: f64 = 2.0;
pub const PORT_RADIUS: f64 = 4.0;
/// Vertical gap between nodes in the same column.
pub const INTER_NODE_GAP: f64 = 28.0;
/// Minimum horizontal gap between nodes in the same layer.
pub const NODE_H_SPACING: f64 = 40.0;
/// Vertical gap between node layers.
pub const LAYER_V_SPACING: f64 = 48.0;
/// Vertical gap for derivation layers.
pub const DERIV_V_SPACING: f64 = 24.0;
/// Domain title area height (pad 12 + cap-height 8 + pad 12).
pub const DOMAIN_TITLE_HEIGHT: f64 = 32.0;
/// Extra domain padding beyond corridor space.
/// Per the design spec, corridors (CORRIDOR_PAD * 2 per side) ARE the
/// domain-to-node gap — no additional padding is specified.
pub const DOMAIN_PADDING: f64 = 0.0;
/// Padding from corridor edge to channel center.
pub const CORRIDOR_PAD: f64 = 8.0;
/// Padding between adjacent channels in a corridor.
pub const CHANNEL_GAP: f64 = 4.0;
/// Parallel edge offset in shared channels.
pub const EDGE_SPACING: f64 = 8.0;
/// Cross-domain constraint stub length (dotted line near destination port).
pub const STUB_LENGTH: f64 = 10.0;
/// All arrowheads are 6×6; path endpoint offset by this amount.
pub const ARROWHEAD_SIZE: f64 = 6.0;
/// Derivation pill height (matches row height).
pub const PILL_HEIGHT: f64 = 20.0;
/// Horizontal padding inside derivation pill (left/right).
pub const PILL_CONTENT_PAD: f64 = 12.0;
/// Character width estimate for monospace text.
pub const CHAR_WIDTH: f64 = 5.5;
/// Character width factor for proportional (sans-serif) label text.
/// Average character width ≈ font_size × this factor.  Tuned to match
/// common system sans-serif fonts (Inter, Segoe UI, DejaVu Sans) which
/// render wider than the previous 0.55 estimate.
pub const LABEL_CHAR_WIDTH_FACTOR: f64 = 0.65;
/// Extra horizontal padding added to each side of label bounding boxes when
/// computing the SVG canvas dimensions.  This compensates for the inherent
/// inaccuracy of the character-counting text width estimate -- proportional
/// fonts can render wider than `len * font_size * LABEL_CHAR_WIDTH_FACTOR`,
/// particularly for labels containing wide characters (m, w, _).
pub const LABEL_OVERFLOW_PAD: f64 = 8.0;
/// Global margin around the entire SVG.
pub const GLOBAL_MARGIN: f64 = 20.0;

// ---------------------------------------------------------------------------
// Font-size constants (LAYOUT.md §4.2.4)
// ---------------------------------------------------------------------------

/// Node title text.
pub const TITLE_FONT_SIZE: f64 = 12.0;
/// Property name text (monospace).
pub const PROP_FONT_SIZE: f64 = 10.0;
/// Domain label text.
pub const DOMAIN_FONT_SIZE: f64 = 10.0;
/// Anchor edge label text.
pub const ANCHOR_LABEL_SIZE: f64 = 8.0;
/// Constraint edge label text.
pub const CONSTRAINT_LABEL_SIZE: f64 = 6.0;
/// Derivation pill label text (monospace).
pub const PILL_FONT_SIZE: f64 = 8.0;

// ---------------------------------------------------------------------------
// Layout result types (DESIGN.md §5.5)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LayoutResult {
    pub nodes: Vec<NodeLayout>,
    pub derivations: Vec<DerivLayout>,
    pub domains: Vec<DomainLayout>,
    pub anchors: Vec<EdgePath>,
    pub intra_domain_constraints: Vec<EdgePath>,
    pub cross_domain_constraints: Vec<CrossDomainPaths>,
    pub cross_domain_deriv_chains: Vec<DerivChain>,
    pub property_order: crossing::PropertyOrder,
    pub width: f64,
    pub height: f64,
    /// Extra horizontal offset added to the SVG translate to accommodate edge
    /// labels that extend past the left edge of the content area.
    pub content_offset_x: f64,
}

#[derive(Debug, Clone)]
pub struct NodeLayout {
    pub id: NodeId,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NodeLayout {
    /// Port x-coordinate on the left edge.
    pub fn port_left_x(&self) -> f64 {
        self.x
    }

    /// Port x-coordinate on the right edge.
    pub fn port_right_x(&self) -> f64 {
        self.x + self.width
    }

    /// Port y-coordinate for a property at the given index (single connection).
    pub fn port_y(&self, prop_index: usize) -> f64 {
        self.y + HEADER_HEIGHT + prop_index as f64 * ROW_HEIGHT + ROW_HEIGHT / 2.0
    }

    /// Port y-coordinate with distributed placement when a property side has
    /// multiple connections.  Divides the row into `total + 1` equal segments
    /// and places the `index`-th port (0-based) at segment boundary `index + 1`.
    /// The result is rounded to the nearest even pixel for grid alignment.
    pub fn distributed_port_y(&self, prop_index: usize, index: usize, total: usize) -> f64 {
        if total <= 1 {
            return self.port_y(prop_index);
        }
        let base_y = self.y + HEADER_HEIGHT + prop_index as f64 * ROW_HEIGHT;
        let segment = ROW_HEIGHT / (total as f64 + 1.0);
        let y = base_y + segment * (index as f64 + 1.0);
        (y / 2.0).round() * 2.0
    }

    /// Anchor port x-coordinate (center of node).
    pub fn anchor_port_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    /// Anchor port y-coordinate at top edge (for incoming anchor from parent).
    pub fn anchor_port_top_y(&self) -> f64 {
        self.y
    }

    /// Anchor port y-coordinate at bottom edge (for outgoing anchor to child).
    pub fn anchor_port_bottom_y(&self) -> f64 {
        self.y + self.height
    }
}

#[derive(Debug, Clone)]
pub struct DerivLayout {
    pub id: DerivId,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct DomainLayout {
    pub id: DomainId,
    pub display_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct EdgePath {
    pub edge_id: EdgeId,
    pub svg_path: String,
    /// Optional operation label and its rendering position.
    pub label: Option<EdgeLabel>,
}

#[derive(Debug, Clone)]
pub struct EdgeLabel {
    pub text: String,
    pub x: f64,
    pub y: f64,
    /// "start", "middle", or "end"
    pub anchor: &'static str,
    /// Font size in pixels (needed for bounding box estimation).
    pub font_size: f64,
}

impl EdgeLabel {
    /// Estimate the rendered pixel width of the label text.
    pub fn estimate_text_width(&self) -> f64 {
        self.text.len() as f64 * self.font_size * LABEL_CHAR_WIDTH_FACTOR
    }

    /// Returns the (left_x, right_x) bounding box of the label in layout
    /// coordinates, based on text-anchor and estimated text width.
    pub fn bounding_x(&self) -> (f64, f64) {
        let w = self.estimate_text_width();
        match self.anchor {
            "start" => (self.x, self.x + w),
            "end" => (self.x - w, self.x),
            // "middle"
            _ => (self.x - w / 2.0, self.x + w / 2.0),
        }
    }

    /// Full AABB as (left_x, top_y, width, height).
    /// SVG text y is the baseline; ascent ≈ font_size above it.
    pub fn bounding_box(&self) -> (f64, f64, f64, f64) {
        let (left_x, right_x) = self.bounding_x();
        (left_x, self.y - self.font_size, right_x - left_x, self.font_size)
    }

    /// Clamp the label position so that its bounding box stays within
    /// `[0, max_x] x [0, max_y]` in content coordinates.
    pub fn clamp_to_content_area(&mut self, max_x: f64, max_y: f64) {
        let (left, right) = self.bounding_x();
        // Clamp horizontal: shift x so left >= 0 and right <= max_x.
        if left < 0.0 {
            self.x -= left; // shift right by the amount of left overflow
        } else if right > max_x {
            self.x -= right - max_x; // shift left by the amount of right overflow
        }
        // Clamp vertical: top of bounding box = y - font_size, bottom = y.
        let top = self.y - self.font_size;
        if top < 0.0 {
            self.y -= top; // shift down
        } else if self.y > max_y {
            self.y = max_y; // shift up so baseline is at max_y
        }
    }
}

#[derive(Debug, Clone)]
pub struct DerivChain {
    pub deriv_id: DerivId,
    pub participants: Vec<NodeId>,
    pub full_paths: Vec<EdgePath>,
    pub stub_paths: Vec<StubPath>,
}

/// A short dotted stub near the destination port of a cross-domain constraint.
#[derive(Debug, Clone)]
pub struct StubPath {
    pub edge_id: EdgeId,
    /// SVG path for the dotted stub (short segment near the destination port).
    pub dotted_svg: String,
}

#[derive(Debug, Clone)]
pub struct CrossDomainPaths {
    pub participants: Vec<NodeId>,
    pub full_path: EdgePath,
    pub stub_paths: Vec<StubPath>,
}

// ---------------------------------------------------------------------------
// Port side assignment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortSide {
    Left,
    Right,
}

impl PortSide {
    pub fn opposite(self) -> Self {
        match self {
            PortSide::Left => PortSide::Right,
            PortSide::Right => PortSide::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointRole {
    Upstream,
    Downstream,
}

/// The port side assignments for all edge endpoints.
pub type PortSideAssignment = std::collections::HashMap<(EdgeId, EndpointRole), PortSide>;

// ---------------------------------------------------------------------------
// Node / derivation dimension helpers
// ---------------------------------------------------------------------------

/// Compute the minimum width required by a single node (private helper).
fn single_node_content_width(graph: &Graph, node_id: NodeId) -> f64 {
    let node = &graph.nodes[node_id.index()];
    let label_width = node.label().len() as f64 * CHAR_WIDTH;
    let max_prop_width = node
        .properties
        .iter()
        .map(|&pid| graph.properties[pid.index()].name.len() as f64 * CHAR_WIDTH)
        .fold(0.0_f64, f64::max);
    f64::max(label_width, max_prop_width) + CONTENT_PAD * 2.0
}

/// Returns the display width for a node.
///
/// All nodes in the graph share a uniform width (the max content width across
/// all nodes). This produces a clean, aligned appearance.
pub fn node_width(graph: &Graph, _node_id: NodeId) -> f64 {
    graph
        .nodes
        .iter()
        .map(|n| single_node_content_width(graph, n.id))
        .fold(CONTENT_PAD * 4.0, f64::max)
}

/// Compute the height of a node from the graph model.
pub fn node_height(graph: &Graph, node_id: NodeId) -> f64 {
    let node = &graph.nodes[node_id.index()];
    HEADER_HEIGHT + node.properties.len() as f64 * ROW_HEIGHT
}

/// Compute the width of a derivation node.
pub fn deriv_width(graph: &Graph, deriv_id: DerivId) -> f64 {
    let deriv = &graph.derivations[deriv_id.index()];
    deriv.operation.len() as f64 * CHAR_WIDTH + PILL_CONTENT_PAD * 2.0
}

/// Compute the height of a derivation pill.
pub fn deriv_height() -> f64 {
    PILL_HEIGHT
}

// ---------------------------------------------------------------------------
// Layout endpoint mapping (DESIGN.md §4.2.6)
// ---------------------------------------------------------------------------

/// Identifies which graph element is the upstream (source, higher layer)
/// and downstream (target, lower layer) endpoint of an edge.
#[derive(Debug, Clone, Copy)]
pub enum LayoutEndpoint {
    Node(NodeId),
    Prop(PropId),
    Deriv(DerivId),
}

/// Returns (upstream, downstream) layout endpoints for an edge.
pub fn layout_endpoints(edge: &Edge) -> (LayoutEndpoint, LayoutEndpoint) {
    match edge {
        Edge::Anchor { parent, child, .. } => {
            (LayoutEndpoint::Node(*parent), LayoutEndpoint::Node(*child))
        }
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            (LayoutEndpoint::Prop(*source_prop), LayoutEndpoint::Prop(*dest_prop))
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
            ..
        } => {
            (LayoutEndpoint::Prop(*source_prop), LayoutEndpoint::Deriv(*target_deriv))
        }
    }
}

// ---------------------------------------------------------------------------
// Tree centering post-processor (DESIGN.md §4.2.4)
// ---------------------------------------------------------------------------

/// Post-process node X coordinates so that each parent is centered over its
/// link-tree children, giving a clean columnar appearance.
///
/// Steps:
///   1. Bottom-up: for each node with children, set its center X to the mean
///      of its children's center X values.
///   2. Left-to-right spacing sweep per layer to enforce NODE_H_SPACING.
///   3. Re-normalize so that the minimum X across all nodes is ≥ 0.
fn tree_center_nodes(
    node_layouts: &mut [NodeLayout],
    graph: &Graph,
    layers: &[LayerEntry],
) {
    // Bottom-up pass: center each parent over its link-tree children.
    for layer in layers.iter().rev() {
        for item in &layer.items {
            if let LayerItem::Node(nid) = item {
                // Only consider children in the same domain (intra-domain links).
                // Cross-domain links should not pull a parent out of its domain.
                let parent_domain = graph.nodes[nid.index()].domain;
                let children: Vec<NodeId> = graph
                    .node_children
                    .get(nid)
                    .iter()
                    .flat_map(|eids| eids.iter())
                    .filter_map(|&eid| match &graph.edges[eid.index()] {
                        Edge::Anchor { child, .. }
                            if graph.nodes[child.index()].domain == parent_domain =>
                        {
                            Some(*child)
                        }
                        _ => None,
                    })
                    .collect();

                if children.is_empty() {
                    continue;
                }

                let mut centers: Vec<f64> = children
                    .iter()
                    .map(|&cid| {
                        let cw = node_width(graph, cid);
                        node_layouts[cid.index()].x + cw / 2.0
                    })
                    .collect();
                centers.sort_by(|a, b| a.partial_cmp(b).unwrap());

                let mean_center = (centers[0] + centers[centers.len() - 1]) / 2.0;
                let nw = node_width(graph, *nid);
                node_layouts[nid.index()].x = mean_center - nw / 2.0;
            }
        }
    }

    // Left-to-right spacing sweep within each layer.
    for layer in layers {
        let mut node_ids: Vec<NodeId> = layer
            .items
            .iter()
            .filter_map(|item| {
                if let LayerItem::Node(nid) = item {
                    Some(*nid)
                } else {
                    None
                }
            })
            .collect();

        node_ids.sort_by(|a, b| {
            node_layouts[a.index()]
                .x
                .partial_cmp(&node_layouts[b.index()].x)
                .unwrap()
        });

        for i in 1..node_ids.len() {
            let prev_w = node_width(graph, node_ids[i - 1]);
            let prev_right = node_layouts[node_ids[i - 1].index()].x + prev_w;
            let needed = prev_right + NODE_H_SPACING;
            if node_layouts[node_ids[i].index()].x < needed {
                node_layouts[node_ids[i].index()].x = needed;
            }
        }
    }

    // Re-normalize: shift all nodes so min X = 0.
    let min_x = node_layouts
        .iter()
        .map(|nl| nl.x)
        .fold(f64::INFINITY, f64::min);
    if min_x.is_finite() && min_x < -1e-9 {
        for nl in node_layouts.iter_mut() {
            nl.x -= min_x;
        }
    }
}

// ---------------------------------------------------------------------------
// Derivation re-centering post-processor
// ---------------------------------------------------------------------------

/// Re-center derivation pills horizontally over their connected input nodes
/// after all node positioning is finalized.
///
/// Derivations are initially positioned by Brandes-Kopf (Phase 4), but
/// subsequent phases (tree centering 4b, columnar layout 5b, vertical
/// compaction 5c) shift node x-positions without updating derivation
/// positions.  This pass recomputes each derivation's x-coordinate to be
/// centered on the mean x-center of its input source nodes.
///
/// Vertical positioning is handled by `separate_column_elements_vertically`
/// (Phase 5c) which places cross-domain derivations as column elements with
/// proper gap spacing relative to domains.  We also adjust the y-coordinate
/// here: if both connected layers (input nodes above, output node below) are
/// available, place the derivation at the vertical midpoint -- but only when
/// that midpoint does not fall inside any domain bounding box.
fn recenter_derivations(
    deriv_layouts: &mut [DerivLayout],
    node_layouts: &[NodeLayout],
    domain_layouts: &[DomainLayout],
    graph: &Graph,
) {
    for deriv in &graph.derivations {
        let dl = &deriv_layouts[deriv.id.index()];
        let dw = dl.width;
        let dh = dl.height;

        // Collect x-centers and bottom y of input source nodes.
        let mut input_x_centers: Vec<f64> = Vec::new();
        let mut input_bottom_y = f64::NEG_INFINITY;
        for &input_prop in &deriv.inputs {
            let src_node = graph.properties[input_prop.index()].node;
            let nl = &node_layouts[src_node.index()];
            input_x_centers.push(nl.x + nl.width / 2.0);
            input_bottom_y = input_bottom_y.max(nl.y + nl.height);
        }

        // Output node top y.
        let out_node = graph.properties[deriv.output_prop.index()].node;
        let out_nl = &node_layouts[out_node.index()];
        let output_top_y = out_nl.y;

        // --- X centering ---
        if !input_x_centers.is_empty() {
            let mean_x: f64 =
                input_x_centers.iter().sum::<f64>() / input_x_centers.len() as f64;
            deriv_layouts[deriv.id.index()].x = mean_x - dw / 2.0;
        }

        // --- Y adjustment ---
        // Try the midpoint between input node bottoms and output node top.
        // Only use it if the resulting pill rectangle does not intersect
        // any domain bounding box.
        if input_bottom_y.is_finite() {
            let mid_y = (input_bottom_y + output_top_y) / 2.0;
            let candidate_y = mid_y - dh / 2.0;
            let candidate_x = deriv_layouts[deriv.id.index()].x;

            let overlaps_domain = domain_layouts.iter().any(|dom| {
                // AABB overlap test.
                candidate_x < dom.x + dom.width
                    && candidate_x + dw > dom.x
                    && candidate_y < dom.y + dom.height
                    && candidate_y + dh > dom.y
            });

            if !overlaps_domain {
                deriv_layouts[deriv.id.index()].y = candidate_y;
            }
            // Otherwise keep the y from vertical compaction (Phase 5c).
        }
    }
}

// ---------------------------------------------------------------------------
// Label collision helpers
// ---------------------------------------------------------------------------

/// A line segment as two endpoints: ((x1,y1), (x2,y2)).
type LineSeg = ((f64, f64), (f64, f64));

/// AABB overlap test for two (x, y, w, h) rectangles.
fn aabbs_overlap(a: &(f64, f64, f64, f64), b: &(f64, f64, f64, f64)) -> bool {
    let (ax, ay, aw, ah) = *a;
    let (bx, by, bw, bh) = *b;
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

/// Test if a line segment ((x1,y1),(x2,y2)) intersects an AABB (x,y,w,h).
fn segment_intersects_aabb(
    seg: &LineSeg,
    aabb: &(f64, f64, f64, f64),
) -> bool {
    let ((x1, y1), (x2, y2)) = *seg;
    let (ax, ay, aw, ah) = *aabb;
    // Bounding box quick reject.
    let seg_min_x = x1.min(x2);
    let seg_max_x = x1.max(x2);
    let seg_min_y = y1.min(y2);
    let seg_max_y = y1.max(y2);
    if seg_max_x < ax || seg_min_x > ax + aw || seg_max_y < ay || seg_min_y > ay + ah {
        return false;
    }
    // For orthogonal segments (our case), the BB test is sufficient:
    // a horizontal or vertical segment whose BB overlaps the AABB does intersect.
    true
}

/// Pick the best label candidate position by minimizing collisions.
///
/// Scores each candidate against:
/// - Already-placed labels (Label-Label overlap: +3 penalty)
/// - Domain/node obstacle AABBs (Label-Domain/Node overlap: +1 penalty each)
/// - Other edge segments (Edge-Label overlap: +2 penalty each, max 4)
#[allow(clippy::too_many_arguments)]
fn pick_best_label_candidate(
    candidates: &[(f64, f64, &'static str)],
    text: &str,
    font_size: f64,
    placed_labels: &[(f64, f64, f64, f64)],
    obstacle_aabbs: &[(f64, f64, f64, f64)],
    node_aabbs: &[(f64, f64, f64, f64)],
    edge_segments: &[Vec<LineSeg>],
    own_route_idx: usize,
) -> (f64, f64, &'static str) {
    if candidates.len() <= 1 {
        return candidates.first().copied().unwrap_or((0.0, 0.0, "middle"));
    }

    let text_width = text.len() as f64 * font_size * LABEL_CHAR_WIDTH_FACTOR;

    // Generate node-clearing candidates: for each primary candidate that would
    // be occluded by a node, create a shifted version just outside the node.
    let mut all_candidates: Vec<(f64, f64, &'static str)> = candidates.to_vec();
    for &(cx, cy, anchor) in candidates {
        let (left, _right) = match anchor {
            "start" => (cx, cx + text_width),
            "end" => (cx - text_width, cx),
            _ => (cx - text_width / 2.0, cx + text_width / 2.0),
        };
        let bb = (left, cy - font_size, text_width.max(0.01), font_size);
        for node_bb in node_aabbs {
            let frac = overlap_fraction(node_bb, &bb);
            if frac > 0.3 {
                let node_right = node_bb.0 + node_bb.2;
                let node_left = node_bb.0;
                // Candidate shifted right: label starts just past node right edge.
                let shifted_right_x = node_right + 2.0;
                all_candidates.push((shifted_right_x, cy, "start"));
                // Candidate shifted left: label ends just before node left edge.
                let shifted_left_x = node_left - 2.0;
                all_candidates.push((shifted_left_x, cy, "end"));
            }
        }
    }

    let mut best = all_candidates[0];
    let mut best_score = u32::MAX;

    for &(cx, cy, anchor) in &all_candidates {
        // Compute this candidate's bounding box.
        let (left, right) = match anchor {
            "start" => (cx, cx + text_width),
            "end" => (cx - text_width, cx),
            _ => (cx - text_width / 2.0, cx + text_width / 2.0),
        };
        let bb = (left, cy - font_size, right - left, font_size);

        let mut score: u32 = 0;

        // Node occlusion penalty (very high — label becomes unreadable).
        // Check each node: if >50% of label area is hidden behind node, heavy penalty.
        for node_bb in node_aabbs {
            let frac = overlap_fraction(node_bb, &bb);
            if frac > 0.5 {
                score += 100;
            } else if frac > 0.01 {
                score += 10;
            }
        }

        // Label-Label collisions (high penalty).
        for placed in placed_labels {
            if aabbs_overlap(&bb, placed) {
                score += 3;
            }
        }

        // Label-Domain collisions (moderate penalty).
        for obs in obstacle_aabbs {
            if aabbs_overlap(&bb, obs) {
                score += 1;
            }
        }

        // Edge-Label collisions (moderate penalty, capped to limit cost).
        let mut edge_hits = 0u32;
        for (idx, segs) in edge_segments.iter().enumerate() {
            if idx == own_route_idx {
                continue;
            }
            if edge_hits >= 4 {
                break;
            }
            for seg in segs {
                if segment_intersects_aabb(seg, &bb) {
                    edge_hits += 1;
                    break;
                }
            }
        }
        score += edge_hits * 2;

        if score < best_score {
            best_score = score;
            best = (cx, cy, anchor);
        }

        // Perfect score — stop early.
        if score == 0 {
            break;
        }
    }

    best
}

/// Fraction of `other` that is overlapped by `self` (both as tuples `(x, y, w, h)`).
fn overlap_fraction(a: &(f64, f64, f64, f64), b: &(f64, f64, f64, f64)) -> f64 {
    let b_area = b.2 * b.3;
    if b_area <= 0.0 {
        return 0.0;
    }
    let ix = (a.0 + a.2).min(b.0 + b.2) - a.0.max(b.0);
    let iy = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    (ix * iy) / b_area
}

// ---------------------------------------------------------------------------
// Main layout entry point
// ---------------------------------------------------------------------------

/// Run the full layout pipeline on a validated graph.
pub fn layout(graph: &Graph) -> Result<LayoutResult, crate::ObgraphError> {
    // Phase 2: Layer assignment (compound graph layering for domain contiguity)
    let assignment = layer_assign::compound_network_simplex(graph)?;

    // Phase 3a: Build layers with long edge segments
    let (mut layers, mut long_edges) = long_edge::build_layers(&assignment, graph);

    // Phase 3b: Crossing minimization (also computes edge port ordering + port sides)
    let (prop_order, _edge_port_order, _layer_port_sides) =
        crossing::minimize_crossings(&mut layers, &mut long_edges, graph);

    // Phase 4: Coordinate assignment (Brandes-Köpf)
    let (mut node_layouts, mut deriv_layouts) =
        coordinate::assign_coordinates(&layers, &long_edges, &assignment, graph);

    // Phase 4b: Tree centering — re-center each parent over its intra-domain
    // link-tree children for clean columnar alignment.
    tree_center_nodes(&mut node_layouts, graph, &layers);

    // Phase 5: Domain bounding boxes
    let mut domain_layouts = domain::compute_domain_bounds(graph, &node_layouts);

    // Phase 5b: Columnar domain layout — assign domains to columns with
    // dedicated gap corridors for cross-domain edge routing.
    domain::columnar_layout(&mut node_layouts, &mut domain_layouts, graph);

    // Phase 5c: Compact vertical separation — place domains, free nodes, and
    // cross-domain derivations with tight inter-element gaps.
    domain::separate_column_elements_vertically(
        &mut node_layouts,
        &mut deriv_layouts,
        &mut domain_layouts,
        graph,
    );

    // Phase 5d: Re-center derivations over their (now-shifted) input nodes.
    // Phases 4b, 5b, and 5c move nodes without updating derivation positions.
    // This pass recomputes derivation x (centered on input nodes) and
    // conditionally adjusts y (midpoint between layers, avoiding domains).
    recenter_derivations(&mut deriv_layouts, &node_layouts, &domain_layouts, graph);

    // Phase 5e: Normalize — shift all elements so that the minimum x and y are >= 0.
    // This must happen before edge routing so that SVG path coordinates match the
    // final node/domain positions.
    normalize_positions(&mut node_layouts, &mut deriv_layouts, &mut domain_layouts);

    // Phase 6a: Assign port sides from coordinate-space geometry.
    // Layer-space port sides from Phase 3b feed the sweep's crossing detection
    // but are not used here — coordinate-space positions are more accurate for
    // physical routing decisions.
    let port_sides = routing::refine_port_sides(
        graph, &node_layouts, &deriv_layouts, &domain_layouts,
        &PortSideAssignment::new(), &prop_order,
    );

    // Phase 6a2: Expand intra-domain corridors where many bracket edges overlap.
    // This must happen after port side assignment (6a) so we know which side
    // each edge exits, and before routing (6b) so corridors are wide enough.
    domain::expand_corridors_for_edges(
        &mut node_layouts,
        &mut domain_layouts,
        graph,
        &port_sides,
    );

    // Phase 6b: Edge routing (corridor-based, with coordinate-space port ordering)
    let routes = routing::route_all_edges(
        graph,
        &node_layouts,
        &deriv_layouts,
        &domain_layouts,
        &port_sides,
        &prop_order,
    );

    // Classify edges into anchors, derivation edges, and constraints.
    //
    // Labels are placed using a collision-aware candidate selection:
    // each edge generates multiple candidate label positions and the
    // one with the fewest overlaps against already-placed labels,
    // domain boundaries, and other edge segments is chosen.
    let mut anchors = Vec::new();
    let mut intra_domain_constraints = Vec::new();
    let mut cross_domain_constraints: Vec<CrossDomainPaths> = Vec::new();

    // Collect obstacle AABBs: domain bounding boxes (moderate penalty for overlaps).
    let obstacle_aabbs: Vec<(f64, f64, f64, f64)> = domain_layouts
        .iter()
        .map(|dl| (dl.x, dl.y, dl.width, dl.height))
        .collect();

    // Node AABBs — labels hidden behind nodes are serious errors.
    let node_aabbs: Vec<(f64, f64, f64, f64)> = node_layouts
        .iter()
        .map(|nl| (nl.x, nl.y, nl.width, nl.height))
        .collect();

    // Already-placed label bounding boxes for collision detection.
    let mut placed_label_bbs: Vec<(f64, f64, f64, f64)> = Vec::new();

    // Collect DerivInput edge paths by derivation for post-loop classification.
    let mut deriv_edge_paths: std::collections::HashMap<DerivId, Vec<(EdgePath, usize)>> =
        std::collections::HashMap::new();

    // Pre-parse edge segments for edge-label collision checking.
    let edge_segments: Vec<Vec<LineSeg>> = routes
        .iter()
        .map(|route| {
            route
                .segments
                .iter()
                .map(|seg| (seg.start(), seg.end()))
                .collect()
        })
        .collect();

    for (route_idx, route) in routes.iter().enumerate() {
        let edge = &graph.edges[route.edge_id.index()];
        let label_text = edge_operation(edge);
        let label_font_size = match edge {
            Edge::Anchor { .. } => ANCHOR_LABEL_SIZE,
            Edge::Constraint { .. } | Edge::DerivInput { .. } => CONSTRAINT_LABEL_SIZE,
        };
        let label = label_text.map(|text| {
            let candidates = routing::route_label_candidates(route);
            let (x, y, anchor) = pick_best_label_candidate(
                &candidates,
                &text,
                label_font_size,
                &placed_label_bbs,
                &obstacle_aabbs,
                &node_aabbs,
                &edge_segments,
                route_idx,
            );
            let lbl = EdgeLabel {
                text,
                x,
                y,
                anchor,
                font_size: label_font_size,
            };
            placed_label_bbs.push(lbl.bounding_box());
            lbl
        });
        let svg_path = routing::route_to_svg_path(route);
        let edge_path = EdgePath {
            edge_id: route.edge_id,
            svg_path,
            label,
        };

        match edge {
            Edge::Anchor { .. } => {
                anchors.push(edge_path);
            }
            Edge::DerivInput { target_deriv, .. } => {
                // Collect by derivation; classified after the loop.
                deriv_edge_paths
                    .entry(*target_deriv)
                    .or_default()
                    .push((edge_path, route_idx));
            }
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => {
                let src_node = graph.properties[source_prop.index()].node;
                let dst_node = graph.properties[dest_prop.index()].node;
                let is_cross_domain = is_cross_domain_constraint(graph, src_node, dst_node);

                if is_cross_domain {
                    // Generate short dotted stub near the destination port
                    let stub_route = routing::generate_stub(route);
                    let dotted_svg = routing::route_to_svg_path(&stub_route);
                    let stub_path = StubPath {
                        edge_id: route.edge_id,
                        dotted_svg,
                    };

                    cross_domain_constraints.push(CrossDomainPaths {
                        participants: vec![src_node, dst_node],
                        full_path: edge_path,
                        stub_paths: vec![stub_path],
                    });
                } else {
                    intra_domain_constraints.push(edge_path);
                }
            }
        }
    }

    // Classify DerivInput edges: intra-domain derivations are always-visible,
    // cross-domain derivations get show/hide with stubs (DerivChain).
    let mut cross_domain_deriv_chains: Vec<DerivChain> = Vec::new();
    for (deriv_id, edge_entries) in deriv_edge_paths {
        if is_deriv_cross_domain(graph, deriv_id) {
            let deriv = &graph.derivations[deriv_id.index()];
            let mut participants: Vec<NodeId> = deriv
                .inputs
                .iter()
                .map(|&pid| graph.properties[pid.index()].node)
                .collect();
            let output_node = graph.properties[deriv.output_prop.index()].node;
            participants.push(output_node);
            participants.sort_unstable();
            participants.dedup();

            let mut full_paths = Vec::new();
            let mut stub_paths = Vec::new();
            for (ep, ri) in edge_entries {
                let stub_route = routing::generate_stub(&routes[ri]);
                let dotted_svg = routing::route_to_svg_path(&stub_route);
                stub_paths.push(StubPath {
                    edge_id: ep.edge_id,
                    dotted_svg,
                });
                full_paths.push(ep);
            }

            cross_domain_deriv_chains.push(DerivChain {
                deriv_id,
                participants,
                full_paths,
                stub_paths,
            });
        } else {
            for (ep, _ri) in edge_entries {
                intra_domain_constraints.push(ep);
            }
        }
    }

    // Phase 7: Clamp edge label positions so they stay within the content
    // bounding box.  Labels placed near the canvas edge (especially the left
    // edge at x ≈ 0) can extend beyond the content area; clamping prevents
    // the quality check from flagging them as outside the canvas.
    //
    // We subtract LABEL_OVERFLOW_PAD from the max bounds to leave extra room
    // for font-estimation error — the actual rendered text can be wider than
    // our character-counting estimate, and this margin keeps labels safely
    // inside the final canvas.
    {
        let mut content_max_x = 0.0_f64;
        let mut content_max_y = 0.0_f64;
        for nl in &node_layouts {
            content_max_x = content_max_x.max(nl.x + nl.width);
            content_max_y = content_max_y.max(nl.y + nl.height);
        }
        for dl in &deriv_layouts {
            content_max_x = content_max_x.max(dl.x + dl.width);
            content_max_y = content_max_y.max(dl.y + dl.height);
        }
        for dl in &domain_layouts {
            content_max_x = content_max_x.max(dl.x + dl.width);
            content_max_y = content_max_y.max(dl.y + dl.height);
        }

        let clamp_max_x = content_max_x - LABEL_OVERFLOW_PAD;
        let clamp_max_y = content_max_y;

        for ep in anchors.iter_mut()
            .chain(intra_domain_constraints.iter_mut())
        {
            if let Some(ref mut label) = ep.label {
                label.clamp_to_content_area(clamp_max_x, clamp_max_y);
            }
        }
        for cdp in cross_domain_constraints.iter_mut() {
            if let Some(ref mut label) = cdp.full_path.label {
                label.clamp_to_content_area(clamp_max_x, clamp_max_y);
            }
        }
    }

    // Collect all edge labels for dimension computation.
    let all_labels: Vec<&EdgeLabel> = anchors
        .iter()
        .chain(intra_domain_constraints.iter())
        .filter_map(|ep| ep.label.as_ref())
        .chain(
            cross_domain_constraints
                .iter()
                .filter_map(|cdp| cdp.full_path.label.as_ref()),
        )
        .collect();

    // Compute overall dimensions, accounting for label overflow.
    let (width, height, content_offset_x) =
        compute_dimensions(&node_layouts, &deriv_layouts, &domain_layouts, &all_labels);

    Ok(LayoutResult {
        nodes: node_layouts,
        derivations: deriv_layouts,
        domains: domain_layouts,
        anchors,
        intra_domain_constraints,
        cross_domain_constraints,
        cross_domain_deriv_chains,
        property_order: prop_order,
        width,
        height,
        content_offset_x,
    })
}

/// Extract the operation label text from an edge, if present.
fn edge_operation(edge: &Edge) -> Option<String> {
    match edge {
        Edge::Anchor { operation, .. } => operation.clone(),
        Edge::Constraint { operation, .. } => operation.clone(),
        Edge::DerivInput { .. } => None,
    }
}

/// A constraint is cross-domain if its endpoints are in different domains
/// or either is top-level (no domain).
fn is_cross_domain_constraint(graph: &Graph, src_node: NodeId, dst_node: NodeId) -> bool {
    let src_domain = graph.nodes[src_node.index()].domain;
    let dst_domain = graph.nodes[dst_node.index()].domain;
    match (src_domain, dst_domain) {
        (Some(a), Some(b)) => a != b,
        _ => true, // one or both are top-level
    }
}

/// Returns true if a derivation's inputs span multiple domains (or any
/// input/output is domain-less).  Cross-domain derivations route through
/// inter-domain gap corridors and use show/hide with stubs.
pub(crate) fn is_deriv_cross_domain(graph: &Graph, deriv_id: DerivId) -> bool {
    let deriv = &graph.derivations[deriv_id.index()];
    let output_domain =
        graph.nodes[graph.properties[deriv.output_prop.index()].node.index()].domain;
    let mut all_doms: Vec<Option<DomainId>> = deriv
        .inputs
        .iter()
        .map(|&pid| graph.nodes[graph.properties[pid.index()].node.index()].domain)
        .collect();
    all_doms.push(output_domain);
    !(all_doms.iter().all(|d| *d == all_doms[0]) && all_doms[0].is_some())
}

/// Shift all layout elements so that the minimum x and y are >= 0.
/// This is needed because domain title areas can extend above the first node,
/// producing negative y coordinates.
fn normalize_positions(
    node_layouts: &mut [NodeLayout],
    deriv_layouts: &mut [DerivLayout],
    domain_layouts: &mut [DomainLayout],
) {
    let min_x = node_layouts
        .iter()
        .map(|nl| nl.x)
        .chain(deriv_layouts.iter().map(|dl| dl.x))
        .chain(domain_layouts.iter().map(|dl| dl.x))
        .fold(f64::INFINITY, f64::min);
    let min_y = node_layouts
        .iter()
        .map(|nl| nl.y)
        .chain(deriv_layouts.iter().map(|dl| dl.y))
        .chain(domain_layouts.iter().map(|dl| dl.y))
        .fold(f64::INFINITY, f64::min);

    let shift_x = if min_x.is_finite() && min_x < 0.0 { -min_x } else { 0.0 };
    let shift_y = if min_y.is_finite() && min_y < 0.0 { -min_y } else { 0.0 };

    if shift_x > 0.0 || shift_y > 0.0 {
        for nl in node_layouts.iter_mut() {
            nl.x += shift_x;
            nl.y += shift_y;
        }
        for dl in deriv_layouts.iter_mut() {
            dl.x += shift_x;
            dl.y += shift_y;
        }
        for dl in domain_layouts.iter_mut() {
            dl.x += shift_x;
            dl.y += shift_y;
        }
    }
}

/// Compute the overall SVG dimensions from all layout elements, accounting for
/// edge labels that may extend beyond the content bounding box.
///
/// Returns `(width, height, content_offset_x)` where `content_offset_x` is the
/// extra horizontal shift needed in the SVG translate to accommodate labels that
/// extend past the left edge of the content area.
fn compute_dimensions(
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    domain_layouts: &[DomainLayout],
    labels: &[&EdgeLabel],
) -> (f64, f64, f64) {
    // Content bounding box (nodes, derivations, domains).
    let mut content_min_x = 0.0_f64;
    let mut content_max_x = 0.0_f64;
    let mut max_y = 0.0_f64;

    for nl in node_layouts {
        content_min_x = content_min_x.min(nl.x);
        content_max_x = content_max_x.max(nl.x + nl.width);
        max_y = max_y.max(nl.y + nl.height);
    }
    for dl in deriv_layouts {
        content_min_x = content_min_x.min(dl.x);
        content_max_x = content_max_x.max(dl.x + dl.width);
        max_y = max_y.max(dl.y + dl.height);
    }
    for dl in domain_layouts {
        content_min_x = content_min_x.min(dl.x);
        content_max_x = content_max_x.max(dl.x + dl.width);
        max_y = max_y.max(dl.y + dl.height);
    }

    // Compute the horizontal extent of all edge labels, padded to
    // compensate for text-width estimation error (LABEL_OVERFLOW_PAD on
    // each side of every label bounding box).
    let mut label_min_x = content_min_x;
    let mut label_max_x = content_max_x;
    for lbl in labels {
        let (left, right) = lbl.bounding_x();
        label_min_x = label_min_x.min(left - LABEL_OVERFLOW_PAD);
        label_max_x = label_max_x.max(right + LABEL_OVERFLOW_PAD);
    }

    // If any content (labels, nodes, domains) extends past the left edge
    // (x < 0), compute offset to shift the content right.
    let min_x = label_min_x.min(content_min_x);
    let content_offset_x = if min_x < 0.0 { -min_x } else { 0.0 };

    // Total width accounts for: global margin on each side, content width,
    // plus any extra space needed for labels on either side.
    let effective_max_x = label_max_x.max(content_max_x);
    let width = effective_max_x + content_offset_x + GLOBAL_MARGIN * 2.0;
    let height = max_y + GLOBAL_MARGIN * 2.0;

    (width, height, content_offset_x)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_bounding_x_start_anchor() {
        let lbl = EdgeLabel {
            text: "hello".into(),
            x: 100.0,
            y: 50.0,
            anchor: "start",
            font_size: 10.0,
        };
        let (left, right) = lbl.bounding_x();
        assert!((left - 100.0).abs() < 1e-9);
        // 5 chars * 10px * 0.65 = 32.5
        assert!((right - 132.5).abs() < 1e-9);
    }

    #[test]
    fn label_bounding_x_end_anchor() {
        let lbl = EdgeLabel {
            text: "hello".into(),
            x: 30.0,
            y: 50.0,
            anchor: "end",
            font_size: 10.0,
        };
        let (left, right) = lbl.bounding_x();
        // 5 * 10 * 0.65 = 32.5 → left = 30 - 32.5 = -2.5
        assert!((left - (-2.5)).abs() < 1e-9);
        assert!((right - 30.0).abs() < 1e-9);
    }

    #[test]
    fn label_bounding_x_middle_anchor() {
        let lbl = EdgeLabel {
            text: "test".into(),
            x: 50.0,
            y: 50.0,
            anchor: "middle",
            font_size: 8.0,
        };
        let (left, right) = lbl.bounding_x();
        // 4 * 8 * 0.65 = 20.8 → half = 10.4
        assert!((left - 39.6).abs() < 1e-9);
        assert!((right - 60.4).abs() < 1e-9);
    }

    #[test]
    fn dimensions_no_labels_unchanged() {
        let nodes = vec![NodeLayout {
            id: NodeId(0),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        }];
        let (w, h, offset) = compute_dimensions(&nodes, &[], &[], &[]);
        // width = 100 + 2*20 = 140, height = 50 + 40 = 90, no offset
        assert!((w - 140.0).abs() < 1e-9);
        assert!((h - 90.0).abs() < 1e-9);
        assert!((offset).abs() < 1e-9);
    }

    #[test]
    fn dimensions_label_overflows_left() {
        let nodes = vec![NodeLayout {
            id: NodeId(0),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        }];
        // Label at x=10, text-anchor="end", text width = 5*8*0.65 = 26
        // Left edge = 10 - 26 = -16
        let lbl = EdgeLabel {
            text: "hello".into(),
            x: 10.0,
            y: 25.0,
            anchor: "end",
            font_size: 8.0,
        };
        let labels = vec![&lbl];
        let (w, h, offset) = compute_dimensions(&nodes, &[], &[], &labels);
        // Label bounding_x = (-16, 10), padded = (-24, 18)
        // content_offset_x = 24 (to compensate for -24 padded left overflow)
        assert!((offset - 24.0).abs() < 1e-9);
        // width = max(100, 100) + 24 + 40 = 164
        assert!((w - 164.0).abs() < 1e-9);
        assert!((h - 90.0).abs() < 1e-9);
    }

    #[test]
    fn dimensions_label_overflows_right() {
        let nodes = vec![NodeLayout {
            id: NodeId(0),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        }];
        // Label at x=90, text-anchor="start", text width = 10*6*0.65 = 39
        // Right edge = 90 + 39 = 129 (exceeds content_max_x of 100)
        let lbl = EdgeLabel {
            text: "0123456789".into(),
            x: 90.0,
            y: 25.0,
            anchor: "start",
            font_size: 6.0,
        };
        let labels = vec![&lbl];
        let (w, _h, offset) = compute_dimensions(&nodes, &[], &[], &labels);
        // Label bounding_x = (90, 129), padded right = 137
        // No left overflow (padded left = 82 > 0), so offset = 0
        assert!((offset).abs() < 1e-9);
        // width = max(137, 100) + 0 + 40 = 177
        assert!((w - 177.0).abs() < 1e-9);
    }

    #[test]
    fn dimensions_label_overflows_both_sides() {
        let nodes = vec![NodeLayout {
            id: NodeId(0),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        }];
        // Left-overflow label: x=5, anchor="end", 10 chars * 6px * 0.65 = 39
        // Left edge = 5 - 39 = -34
        let lbl_left = EdgeLabel {
            text: "0123456789".into(),
            x: 5.0,
            y: 25.0,
            anchor: "end",
            font_size: 6.0,
        };
        // Right-overflow label: x=90, anchor="start", 10 chars * 6px * 0.65 = 39
        // Right edge = 90 + 39 = 129
        let lbl_right = EdgeLabel {
            text: "0123456789".into(),
            x: 90.0,
            y: 25.0,
            anchor: "start",
            font_size: 6.0,
        };
        let labels = vec![&lbl_left, &lbl_right];
        let (w, _h, offset) = compute_dimensions(&nodes, &[], &[], &labels);
        // Left label padded: (-42, 13), right label padded: (82, 137)
        // content_offset_x = 42
        assert!((offset - 42.0).abs() < 1e-9);
        // width = max(137, 100) + 42 + 40 = 219
        assert!((w - 219.0).abs() < 1e-9);
    }
}
