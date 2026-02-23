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
pub const DOMAIN_PADDING: f64 = 10.0;
/// Padding from corridor edge to channel center.
pub const CORRIDOR_PAD: f64 = 8.0;
/// Padding between adjacent channels in a corridor.
pub const CHANNEL_GAP: f64 = 4.0;
/// Parallel edge offset in shared channels.
pub const EDGE_SPACING: f64 = 8.0;
/// Cross-domain constraint stub arrow length.
pub const STUB_LENGTH: f64 = 20.0;
/// All arrowheads are 6×6; path endpoint offset by this amount.
pub const ARROWHEAD_SIZE: f64 = 6.0;
/// Derivation pill height (matches row height).
pub const PILL_HEIGHT: f64 = 20.0;
/// Horizontal padding inside derivation pill (left/right).
pub const PILL_CONTENT_PAD: f64 = 12.0;
/// Character width estimate for monospace text.
pub const CHAR_WIDTH: f64 = 5.5;
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
    pub width: f64,
    pub height: f64,
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
}

#[derive(Debug, Clone)]
pub struct DerivChain {
    pub deriv_id: DerivId,
    pub participants: Vec<NodeId>,
    pub full_paths: Vec<EdgePath>,
    pub stub_paths: Vec<EdgePath>,
}

#[derive(Debug, Clone)]
pub struct CrossDomainPaths {
    pub participants: Vec<NodeId>,
    pub full_path: EdgePath,
    pub stub_paths: Vec<EdgePath>,
}

// ---------------------------------------------------------------------------
// Port side assignment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointRole {
    Upstream,
    Downstream,
}

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

/// Compute the uniform width for all members of a domain.
fn domain_node_width(graph: &Graph, domain_id: DomainId) -> f64 {
    graph
        .domains
        .iter()
        .find(|d| d.id == domain_id)
        .map(|d| {
            d.members
                .iter()
                .map(|&nid| single_node_content_width(graph, nid))
                .fold(0.0_f64, f64::max)
                .max(CONTENT_PAD * 4.0)
        })
        .unwrap_or(CONTENT_PAD * 4.0)
}

/// Returns the display width for a node.
///
/// Nodes within a domain share a uniform width (the max of all domain members).
/// Top-level nodes (no domain) use their individual content-driven width.
pub fn node_width(graph: &Graph, node_id: NodeId) -> f64 {
    let node = &graph.nodes[node_id.index()];
    match node.domain {
        Some(did) => domain_node_width(graph, did),
        None => single_node_content_width(graph, node_id).max(CONTENT_PAD * 4.0),
    }
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
// Main layout entry point
// ---------------------------------------------------------------------------

/// Run the full layout pipeline on a validated graph.
pub fn layout(graph: &Graph) -> Result<LayoutResult, crate::ObgraphError> {
    // Phase 2: Layer assignment (network simplex with typed layers)
    let assignment = layer_assign::network_simplex(graph)?;

    // Phase 3a: Build layers with long edge segments
    let (mut layers, mut long_edges) = long_edge::build_layers(&assignment, graph);

    // Phase 3b: Crossing minimization
    crossing::minimize_crossings(&mut layers, &mut long_edges, graph);

    // Phase 4: Coordinate assignment (Brandes-Köpf)
    let (mut node_layouts, deriv_layouts) =
        coordinate::assign_coordinates(&layers, &long_edges, &assignment, graph);

    // Phase 4b: Tree centering — re-center each parent over its intra-domain
    // link-tree children for clean columnar alignment.
    tree_center_nodes(&mut node_layouts, graph, &layers);

    // Phase 5: Domain bounding boxes
    let mut domain_layouts = domain::compute_domain_bounds(graph, &node_layouts);

    // Phase 5b: Columnar domain layout — assign domains to columns with
    // dedicated gap corridors for cross-domain edge routing.
    domain::columnar_layout(&mut node_layouts, &mut domain_layouts, graph);

    // Phase 5c: Enforce vertical ordering for cross-domain anchor hierarchy.
    domain::separate_domains_vertically(&mut node_layouts, &mut domain_layouts, graph);

    // Phase 6a: Port side assignment
    let port_sides = routing::assign_port_sides(graph, &node_layouts, &deriv_layouts);

    // Phase 6b: Edge routing (corridor-based)
    let routes = routing::route_all_edges(
        graph,
        &node_layouts,
        &deriv_layouts,
        &domain_layouts,
        &port_sides,
    );

    // Classify edges into anchors, derivation edges, and constraints
    let mut anchors = Vec::new();
    let mut intra_domain_constraints = Vec::new();
    let mut cross_domain_constraints: Vec<CrossDomainPaths> = Vec::new();

    for route in &routes {
        let edge = &graph.edges[route.edge_id.index()];
        let label_text = edge_operation(edge);
        let label = label_text.map(|text| {
            let (x, y, anchor) = routing::route_label_position(route);
            EdgeLabel { text, x, y, anchor }
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
            Edge::DerivInput { .. } => {
                // Derivation input edges are now part of DerivChain structs
                // (built by Stream D). For now, include them in intra-domain constraints
                // so they still render.
                intra_domain_constraints.push(edge_path);
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
                    // Generate stub routes (no label on stubs)
                    let stub_route = routing::generate_stub(route);
                    let stub_svg = routing::route_to_svg_path(&stub_route);
                    let stub_path = EdgePath {
                        edge_id: route.edge_id,
                        svg_path: stub_svg,
                        label: None,
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

    // Compute overall dimensions
    let (width, height) = compute_dimensions(&node_layouts, &deriv_layouts, &domain_layouts);

    Ok(LayoutResult {
        nodes: node_layouts,
        derivations: deriv_layouts,
        domains: domain_layouts,
        anchors,
        intra_domain_constraints,
        cross_domain_constraints,
        cross_domain_deriv_chains: Vec::new(),
        width,
        height,
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

/// Compute the overall SVG dimensions from all layout elements.
fn compute_dimensions(
    node_layouts: &[NodeLayout],
    deriv_layouts: &[DerivLayout],
    domain_layouts: &[DomainLayout],
) -> (f64, f64) {
    let mut max_x = 0.0_f64;
    let mut max_y = 0.0_f64;

    for nl in node_layouts {
        max_x = max_x.max(nl.x + nl.width);
        max_y = max_y.max(nl.y + nl.height);
    }
    for dl in deriv_layouts {
        max_x = max_x.max(dl.x + dl.width);
        max_y = max_y.max(dl.y + dl.height);
    }
    for dl in domain_layouts {
        max_x = max_x.max(dl.x + dl.width);
        max_y = max_y.max(dl.y + dl.height);
    }

    (max_x + GLOBAL_MARGIN * 2.0, max_y + GLOBAL_MARGIN * 2.0)
}
