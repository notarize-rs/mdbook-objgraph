//! Post-layout quality analysis.
//!
//! Computes measurable metrics from a `LayoutResult` to detect overlaps,
//! collisions, and other layout defects. Used as a test gate and diagnostic
//! tool during layout algorithm iteration.

use crate::model::types::{DerivId, DomainId, Edge, EdgeId, Graph, NodeId};

use super::{
    DerivLayout, DomainLayout, EdgeLabel, EdgePath, LayoutResult, NodeLayout, StubPath,
    ARROWHEAD_SIZE, CORRIDOR_PAD, DOMAIN_PADDING, DOMAIN_TITLE_HEIGHT, NODE_H_SPACING,
};

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// A quality report for a laid-out graph.
#[derive(Debug)]
pub struct QualityReport {
    // ── Existing fields ──────────────────────────────────────────────
    pub node_overlaps: Vec<(NodeId, NodeId)>,
    pub domain_overlaps: Vec<(DomainId, DomainId)>,
    pub nodes_outside_domain: Vec<(NodeId, DomainId)>,
    pub node_edge_overlaps: Vec<(NodeId, EdgeId)>,
    pub edge_crossings: usize,
    pub min_node_gap: f64,
    pub aspect_ratio: f64,
    pub total_edge_length: f64,
    pub node_width_delta: f64,
    pub max_parent_misalignment: f64,
    pub mean_constraint_segments: f64,
    pub derivs_inside_domains: Vec<(DerivId, DomainId)>,
    pub free_nodes_inside_domains: Vec<(NodeId, DomainId)>,
    pub domain_contiguity_violations: Vec<(DomainId, NodeId)>,
    pub inter_domain_edges_in_intra_corridors: Vec<(EdgeId, DomainId)>,
    pub crossing_pairs: Vec<(EdgeId, EdgeId)>,
    pub channel_collisions: Vec<(EdgeId, EdgeId)>,
    pub total_height: f64,
    pub total_width: f64,
    pub column_heights: Vec<f64>,
    pub column_height_imbalance: f64,

    // ── Collision matrix: Node ───────────────────────────────────────
    pub node_deriv_overlaps: Vec<(NodeId, DerivId)>,
    pub label_node_overlaps: Vec<(EdgeId, NodeId)>,
    pub arrowhead_node_overlaps: Vec<(EdgeId, NodeId)>,
    pub stub_node_overlaps: Vec<(EdgeId, NodeId)>,

    // ── Collision matrix: Domain ─────────────────────────────────────
    pub edge_domain_boundary_crossings: Vec<(EdgeId, DomainId)>,
    pub label_domain_overlaps: Vec<(EdgeId, DomainId)>,
    pub arrowhead_domain_overlaps: Vec<(EdgeId, DomainId)>,
    pub stub_domain_overlaps: Vec<(EdgeId, DomainId)>,

    // ── Collision matrix: Derivation ─────────────────────────────────
    pub deriv_deriv_overlaps: Vec<(DerivId, DerivId)>,
    pub edge_deriv_overlaps: Vec<(EdgeId, DerivId)>,
    pub label_deriv_overlaps: Vec<(EdgeId, DerivId)>,
    pub arrowhead_deriv_overlaps: Vec<(EdgeId, DerivId)>,
    pub stub_deriv_overlaps: Vec<(EdgeId, DerivId)>,

    // ── Collision matrix: Edge ↔ other edge sub-elements ─────────────
    pub edge_label_overlaps: Vec<(EdgeId, EdgeId)>,
    pub edge_arrowhead_overlaps: Vec<(EdgeId, EdgeId)>,
    pub edge_stub_overlaps: Vec<(EdgeId, EdgeId)>,
    pub edge_domain_title_overlaps: Vec<(EdgeId, DomainId)>,

    // ── Collision matrix: Label ──────────────────────────────────────
    pub label_label_overlaps: Vec<(EdgeId, EdgeId)>,
    pub label_arrowhead_overlaps: Vec<(EdgeId, EdgeId)>,
    pub label_stub_overlaps: Vec<(EdgeId, EdgeId)>,
    pub label_domain_title_overlaps: Vec<(EdgeId, DomainId)>,

    // ── Collision matrix: Arrowhead ──────────────────────────────────
    pub arrowhead_arrowhead_overlaps: Vec<(EdgeId, EdgeId)>,
    pub arrowhead_stub_overlaps: Vec<(EdgeId, EdgeId)>,
    pub arrowhead_domain_title_overlaps: Vec<(EdgeId, DomainId)>,

    // ── Collision matrix: Stub ───────────────────────────────────────
    pub stub_stub_overlaps: Vec<(EdgeId, EdgeId)>,
    pub stub_domain_title_overlaps: Vec<(EdgeId, DomainId)>,

    // ── Collision matrix: Domain title ───────────────────────────────
    pub domain_title_title_overlaps: Vec<(DomainId, DomainId)>,

    // ── Occlusion / hidden elements ──────────────────────────────────
    pub edges_hidden_under_nodes: Vec<(EdgeId, NodeId)>,
    pub labels_hidden_under_nodes: Vec<(EdgeId, NodeId)>,
    pub arrowheads_hidden_under_nodes: Vec<(EdgeId, NodeId)>,
    pub stubs_hidden_under_nodes: Vec<(EdgeId, NodeId)>,
    /// Connected edges with segments hidden behind their own endpoint node.
    /// Each entry is (edge_id, node_id, hidden_px, total_px) — the edge connects
    /// to the node but a significant portion of its path is inside the node AABB.
    pub connected_edge_occlusion: Vec<(EdgeId, NodeId, f64, f64)>,

    // ── Canvas overflow (elements outside canvas bounds) ─────────────
    /// Nodes partially or fully outside the canvas.
    pub nodes_outside_canvas: Vec<(NodeId, f64)>,
    /// Domains partially or fully outside the canvas.
    pub domains_outside_canvas: Vec<(DomainId, f64)>,
    /// Derivation pills partially or fully outside the canvas.
    pub derivs_outside_canvas: Vec<(DerivId, f64)>,
    /// Edge segments with portions outside the canvas. (edge_id, overflow_px)
    pub edges_outside_canvas: Vec<(EdgeId, f64)>,
    /// Edge labels partially or fully outside the canvas. (edge_id, overflow_px)
    pub labels_outside_canvas: Vec<(EdgeId, f64)>,
    /// Arrowheads partially or fully outside the canvas. (edge_id, overflow_px)
    pub arrowheads_outside_canvas: Vec<(EdgeId, f64)>,
    /// Stubs with portions outside the canvas. (edge_id, overflow_px)
    pub stubs_outside_canvas: Vec<(EdgeId, f64)>,

    // ── Domain corridor correctness ──────────────────────────────────
    pub intra_edges_in_wrong_corridor: Vec<(EdgeId, DomainId)>,

    // ── Layout symmetry / balance ────────────────────────────────────
    pub visual_balance: f64,
    pub max_column_centering_error: f64,
    pub domain_size_cv: f64,

    // ── Edge routing quality ─────────────────────────────────────────
    pub port_side_balance: f64,
    pub edge_length_cv: f64,
    pub segment_complexity_distribution: [usize; 3],
    pub routing_direction_balance: f64,
}

impl QualityReport {
    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    pub fn has_warnings(&self) -> bool {
        self.warning_count() > 0
    }

    /// Human-readable summary grouped by category.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Quality Report: {} errors, {} warnings",
            self.error_count(),
            self.warning_count()
        ));

        // ── Existing metrics ─────────────────────────────────────────
        lines.push(format!("  Node-node overlaps:    {}", self.node_overlaps.len()));
        lines.push(format!("  Domain overlaps:       {}", self.domain_overlaps.len()));
        lines.push(format!("  Nodes outside domain:  {}", self.nodes_outside_domain.len()));
        lines.push(format!("  Node-edge overlaps:    {}", self.node_edge_overlaps.len()));
        lines.push(format!("  Edge-edge crossings:   {}", self.edge_crossings));
        lines.push(format!("  Min node gap:          {:.1}px (threshold: {:.1}px)", self.min_node_gap, NODE_H_SPACING));
        lines.push(format!("  Aspect ratio:          {:.2}", self.aspect_ratio));
        lines.push(format!("  Total edge length:     {:.0}px", self.total_edge_length));
        lines.push(format!("  Node width delta:      {:.1}px", self.node_width_delta));
        lines.push(format!("  Max parent misalign:   {:.1}px", self.max_parent_misalignment));
        lines.push(format!("  Mean constraint segs:  {:.1}", self.mean_constraint_segments));
        lines.push(format!("  Dimensions:            {:.0}w x {:.0}h px", self.total_width, self.total_height));
        lines.push(format!("  Column heights:        {:?}", self.column_heights.iter().map(|h| format!("{:.0}", h)).collect::<Vec<_>>()));
        lines.push(format!("  Column height imbal:   {:.0}px", self.column_height_imbalance));

        // ── Collision matrix ─────────────────────────────────────────
        lines.push(String::new());
        lines.push("  Collision matrix:".to_string());
        let collision_fields: &[(&str, usize)] = &[
            ("Node↔Deriv", self.node_deriv_overlaps.len()),
            ("Label↔Node", self.label_node_overlaps.len()),
            ("Arrow↔Node", self.arrowhead_node_overlaps.len()),
            ("Stub↔Node", self.stub_node_overlaps.len()),
            ("Edge↔DomBorder", self.edge_domain_boundary_crossings.len()),
            ("Label↔Domain", self.label_domain_overlaps.len()),
            ("Arrow↔Domain", self.arrowhead_domain_overlaps.len()),
            ("Stub↔Domain", self.stub_domain_overlaps.len()),
            ("Deriv↔Deriv", self.deriv_deriv_overlaps.len()),
            ("Edge↔Deriv", self.edge_deriv_overlaps.len()),
            ("Label↔Deriv", self.label_deriv_overlaps.len()),
            ("Arrow↔Deriv", self.arrowhead_deriv_overlaps.len()),
            ("Stub↔Deriv", self.stub_deriv_overlaps.len()),
            ("Edge↔Label", self.edge_label_overlaps.len()),
            ("Edge↔Arrow", self.edge_arrowhead_overlaps.len()),
            ("Edge↔Stub", self.edge_stub_overlaps.len()),
            ("Edge↔DomTitle", self.edge_domain_title_overlaps.len()),
            ("Label↔Label", self.label_label_overlaps.len()),
            ("Label↔Arrow", self.label_arrowhead_overlaps.len()),
            ("Label↔Stub", self.label_stub_overlaps.len()),
            ("Label↔DomTitle", self.label_domain_title_overlaps.len()),
            ("Arrow↔Arrow", self.arrowhead_arrowhead_overlaps.len()),
            ("Arrow↔Stub", self.arrowhead_stub_overlaps.len()),
            ("Arrow↔DomTitle", self.arrowhead_domain_title_overlaps.len()),
            ("Stub↔Stub", self.stub_stub_overlaps.len()),
            ("Stub↔DomTitle", self.stub_domain_title_overlaps.len()),
            ("DomTitle↔DomTitle", self.domain_title_title_overlaps.len()),
        ];
        for (name, count) in collision_fields {
            if *count > 0 {
                lines.push(format!("    {:<20} {}", name, count));
            }
        }
        let total_collisions: usize = collision_fields.iter().map(|(_, c)| c).sum();
        lines.push(format!("    Total new collisions: {}", total_collisions));

        // ── Occlusion ────────────────────────────────────────────────
        lines.push(String::new());
        lines.push("  Occlusion (hidden elements):".to_string());
        lines.push(format!("    Edges hidden under nodes:  {}", self.edges_hidden_under_nodes.len()));
        lines.push(format!("    Labels hidden under nodes: {}", self.labels_hidden_under_nodes.len()));
        lines.push(format!("    Arrows hidden under nodes: {}", self.arrowheads_hidden_under_nodes.len()));
        lines.push(format!("    Stubs hidden under nodes:  {}", self.stubs_hidden_under_nodes.len()));
        lines.push(format!("    Connected-edge occlusion:  {}", self.connected_edge_occlusion.len()));
        if !self.connected_edge_occlusion.is_empty() {
            for &(eid, nid, hidden, total) in &self.connected_edge_occlusion {
                lines.push(format!(
                    "      edge {} behind node {}: {:.0}/{:.0}px ({:.0}%)",
                    eid.0, nid.0, hidden, total, hidden / total * 100.0
                ));
            }
        }

        // ── Canvas overflow ────────────────────────────────────────────
        lines.push(String::new());
        lines.push("  Canvas overflow:".to_string());
        let overflow_fields: &[(&str, usize)] = &[
            ("Nodes", self.nodes_outside_canvas.len()),
            ("Domains", self.domains_outside_canvas.len()),
            ("Derivations", self.derivs_outside_canvas.len()),
            ("Edges", self.edges_outside_canvas.len()),
            ("Labels", self.labels_outside_canvas.len()),
            ("Arrowheads", self.arrowheads_outside_canvas.len()),
            ("Stubs", self.stubs_outside_canvas.len()),
        ];
        let total_overflow: usize = overflow_fields.iter().map(|(_, c)| c).sum();
        if total_overflow == 0 {
            lines.push("    (none)".to_string());
        } else {
            for (name, count) in overflow_fields {
                if *count > 0 {
                    lines.push(format!("    {:<20} {}", name, count));
                }
            }
            lines.push(format!("    Total overflows:     {}", total_overflow));
        }

        // ── Domain corridor correctness ──────────────────────────────
        lines.push(String::new());
        lines.push("  Domain corridor correctness:".to_string());
        lines.push(format!("    Inter-domain in intra-corridor: {}", self.inter_domain_edges_in_intra_corridors.len()));
        lines.push(format!("    Intra-edge in wrong corridor:   {}", self.intra_edges_in_wrong_corridor.len()));

        // ── Layout symmetry ──────────────────────────────────────────
        lines.push(String::new());
        lines.push("  Layout symmetry:".to_string());
        lines.push(format!("    Visual balance:          {:.3} (0 = centered)", self.visual_balance));
        lines.push(format!("    Column centering error:  {:.1}px (0 = aligned)", self.max_column_centering_error));
        lines.push(format!("    Domain size CV:          {:.3} (0 = uniform)", self.domain_size_cv));

        // ── Edge routing quality ─────────────────────────────────────
        lines.push(String::new());
        lines.push("  Edge routing quality:".to_string());
        lines.push(format!("    Port side balance:       {:.3} (1.0 = balanced)", self.port_side_balance));
        lines.push(format!("    Edge length CV:          {:.3} (0 = uniform)", self.edge_length_cv));
        lines.push(format!(
            "    Segment complexity:      [simple={}, bracket={}, spaghetti={}]",
            self.segment_complexity_distribution[0],
            self.segment_complexity_distribution[1],
            self.segment_complexity_distribution[2],
        ));
        lines.push(format!("    Routing dir balance:     {:.3} (1.0 = symmetric)", self.routing_direction_balance));

        // ── Error detail lines ───────────────────────────────────────
        let has_detail = self.error_count() > 0;
        if has_detail {
            lines.push(String::new());
            lines.push("  Error details:".to_string());
        }
        for &(a, b) in &self.node_overlaps {
            lines.push(format!("    ERROR: Node {} overlaps Node {}", a.0, b.0));
        }
        for &(a, b) in &self.domain_overlaps {
            lines.push(format!("    ERROR: Domain {} overlaps Domain {}", a.0, b.0));
        }
        for &(n, d) in &self.nodes_outside_domain {
            lines.push(format!("    ERROR: Node {} outside domain {}", n.0, d.0));
        }
        for &(d, dom) in &self.derivs_inside_domains {
            lines.push(format!("    ERROR: Deriv {} inside domain {}", d.0, dom.0));
        }
        for &(n, dom) in &self.free_nodes_inside_domains {
            lines.push(format!("    ERROR: Free node {} inside domain {}", n.0, dom.0));
        }
        for &(dom, n) in &self.domain_contiguity_violations {
            lines.push(format!("    ERROR: Domain {} contiguity violated by node {}", dom.0, n.0));
        }
        for &(eid, did) in &self.inter_domain_edges_in_intra_corridors {
            lines.push(format!("    ERROR: Inter-domain edge {} in intra-corridor of domain {}", eid.0, did.0));
        }
        for &(a, b) in &self.channel_collisions {
            lines.push(format!("    ERROR: Channel collision edge {} vs edge {}", a.0, b.0));
        }
        for &(n, d) in &self.node_deriv_overlaps {
            lines.push(format!("    ERROR: Node {} overlaps deriv {}", n.0, d.0));
        }
        for &(d1, d2) in &self.deriv_deriv_overlaps {
            lines.push(format!("    ERROR: Deriv {} overlaps deriv {}", d1.0, d2.0));
        }
        for &(eid, did) in &self.intra_edges_in_wrong_corridor {
            lines.push(format!("    ERROR: Intra-domain edge {} in wrong corridor (domain {})", eid.0, did.0));
        }

        lines.join("\n")
    }

    pub fn error_count(&self) -> usize {
        self.node_overlaps.len()
            + self.domain_overlaps.len()
            + self.nodes_outside_domain.len()
            + self.derivs_inside_domains.len()
            + self.free_nodes_inside_domains.len()
            + self.domain_contiguity_violations.len()
            + self.inter_domain_edges_in_intra_corridors.len()
            + self.channel_collisions.len()
            + self.node_deriv_overlaps.len()
            + self.deriv_deriv_overlaps.len()
            + self.intra_edges_in_wrong_corridor.len()
    }

    pub fn warning_count(&self) -> usize {
        let tight_gap = if self.min_node_gap < NODE_H_SPACING - 1.0 { 1 } else { 0 };
        self.node_edge_overlaps.len()
            + tight_gap
            + self.label_node_overlaps.len()
            + self.arrowhead_node_overlaps.len()
            + self.stub_node_overlaps.len()
            + self.edge_domain_boundary_crossings.len()
            + self.label_domain_overlaps.len()
            + self.arrowhead_domain_overlaps.len()
            + self.stub_domain_overlaps.len()
            + self.edge_deriv_overlaps.len()
            + self.label_deriv_overlaps.len()
            + self.arrowhead_deriv_overlaps.len()
            + self.stub_deriv_overlaps.len()
            + self.edge_label_overlaps.len()
            + self.edge_arrowhead_overlaps.len()
            + self.edge_stub_overlaps.len()
            + self.edge_domain_title_overlaps.len()
            + self.label_label_overlaps.len()
            + self.label_arrowhead_overlaps.len()
            + self.label_stub_overlaps.len()
            + self.label_domain_title_overlaps.len()
            + self.arrowhead_arrowhead_overlaps.len()
            + self.arrowhead_stub_overlaps.len()
            + self.arrowhead_domain_title_overlaps.len()
            + self.stub_stub_overlaps.len()
            + self.stub_domain_title_overlaps.len()
            + self.domain_title_title_overlaps.len()
            + self.edges_hidden_under_nodes.len()
            + self.labels_hidden_under_nodes.len()
            + self.arrowheads_hidden_under_nodes.len()
            + self.stubs_hidden_under_nodes.len()
            + self.connected_edge_occlusion.len()
            + self.nodes_outside_canvas.len()
            + self.domains_outside_canvas.len()
            + self.derivs_outside_canvas.len()
            + self.edges_outside_canvas.len()
            + self.labels_outside_canvas.len()
            + self.arrowheads_outside_canvas.len()
            + self.stubs_outside_canvas.len()
    }
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

/// Analyze a layout for quality issues.
pub fn analyze(graph: &Graph, layout: &LayoutResult) -> QualityReport {
    let node_overlaps = find_node_overlaps(&layout.nodes);
    let domain_overlaps = find_domain_overlaps(&layout.domains);
    let nodes_outside_domain = find_nodes_outside_domain(graph, &layout.nodes, &layout.domains);
    let min_node_gap = compute_min_node_gap(&layout.nodes);
    let aspect_ratio = if layout.height > 0.0 {
        layout.width / layout.height
    } else {
        1.0
    };

    // Collect all edge paths for geometric analysis.
    let all_paths: Vec<&EdgePath> = layout
        .anchors
        .iter()
        .chain(layout.intra_domain_constraints.iter())
        .chain(
            layout
                .cross_domain_constraints
                .iter()
                .map(|c| &c.full_path),
        )
        .collect();

    // Also collect deriv chain full paths.
    let deriv_chain_paths: Vec<&EdgePath> = layout
        .cross_domain_deriv_chains
        .iter()
        .flat_map(|dc| dc.full_paths.iter())
        .collect();

    // All paths including deriv chains (for comprehensive analysis).
    let all_paths_with_derivs: Vec<&EdgePath> = all_paths
        .iter()
        .copied()
        .chain(deriv_chain_paths.iter().copied())
        .collect();

    let parsed: Vec<(EdgeId, Vec<LineSeg>)> = all_paths_with_derivs
        .iter()
        .map(|ep| (ep.edge_id, parse_svg_path(&ep.svg_path)))
        .collect();

    let node_edge_overlaps = find_node_edge_overlaps(graph, &layout.nodes, &parsed);
    let crossing_pairs = find_crossing_pairs(&parsed);
    let edge_crossings = count_segment_crossings(&parsed);
    let total_edge_length: f64 = parsed
        .iter()
        .flat_map(|(_, segs)| segs.iter())
        .map(|s| s.length())
        .sum();

    let node_width_delta = compute_node_width_delta(&layout.nodes);
    let max_parent_misalignment = compute_max_parent_misalignment(graph, &layout.nodes);
    let mean_constraint_segments =
        compute_mean_constraint_segments(&layout.intra_domain_constraints);
    let derivs_inside_domains =
        find_derivs_inside_domains(graph, &layout.derivations, &layout.domains);
    let free_nodes_inside_domains =
        find_free_nodes_inside_domains(graph, &layout.nodes, &layout.domains);
    let domain_contiguity_violations =
        find_domain_contiguity_violations(graph, &layout.nodes, &layout.domains);
    let inter_domain_edges_in_intra_corridors =
        find_inter_domain_edges_in_intra_corridors(graph, &layout.domains, &layout.nodes, &parsed);
    let channel_collisions = find_channel_collisions(graph, &parsed);

    let total_height = layout.height;
    let total_width = layout.width;
    let column_heights = compute_column_heights(&layout.domains);
    let column_height_imbalance = if column_heights.len() >= 2 {
        let max_h = column_heights.iter().copied().fold(0.0_f64, f64::max);
        let min_h = column_heights.iter().copied().fold(f64::INFINITY, f64::min);
        max_h - min_h
    } else {
        0.0
    };

    // ── Collect labels, arrowheads, stubs, domain titles ─────────────

    let all_labels: Vec<(EdgeId, &EdgeLabel)> = all_paths_with_derivs
        .iter()
        .filter_map(|ep| ep.label.as_ref().map(|l| (ep.edge_id, l)))
        .collect();

    let all_arrowheads: Vec<(EdgeId, Aabb)> = parsed
        .iter()
        .filter_map(|(eid, segs)| arrowhead_aabb(segs).map(|a| (*eid, a)))
        .collect();

    let all_stubs: Vec<(EdgeId, Vec<LineSeg>)> = layout
        .cross_domain_constraints
        .iter()
        .flat_map(|c| c.stub_paths.iter())
        .chain(
            layout
                .cross_domain_deriv_chains
                .iter()
                .flat_map(|dc| dc.stub_paths.iter()),
        )
        .map(|sp| (sp.edge_id, parse_stub_segments(sp)))
        .collect();

    let domain_title_aabbs: Vec<(DomainId, Aabb)> = layout
        .domains
        .iter()
        .map(|dl| (dl.id, Aabb::from_domain_title(dl)))
        .collect();

    // ── New collision detectors ──────────────────────────────────────

    let node_deriv_overlaps = find_node_deriv_overlaps(&layout.nodes, &layout.derivations);
    let label_node_overlaps =
        find_label_node_overlaps(graph, &layout.nodes, &all_labels);
    let arrowhead_node_overlaps =
        find_arrowhead_node_overlaps(graph, &layout.nodes, &all_arrowheads);
    let stub_node_overlaps =
        find_stub_node_overlaps(graph, &layout.nodes, &all_stubs);

    let edge_domain_boundary_crossings =
        find_edge_domain_boundary_crossings(graph, &layout.domains, &parsed);
    let label_domain_overlaps = find_label_domain_overlaps(&layout.domains, &all_labels);
    let arrowhead_domain_overlaps =
        find_arrowhead_domain_overlaps(graph, &layout.domains, &all_arrowheads);
    let stub_domain_overlaps =
        find_stub_domain_overlaps(graph, &layout.domains, &all_stubs);

    let deriv_deriv_overlaps = find_deriv_deriv_overlaps(&layout.derivations);
    let edge_deriv_overlaps =
        find_edge_deriv_overlaps(graph, &layout.derivations, &parsed);
    let label_deriv_overlaps =
        find_label_deriv_overlaps(&layout.derivations, &all_labels);
    let arrowhead_deriv_overlaps =
        find_arrowhead_deriv_overlaps(graph, &layout.derivations, &all_arrowheads);
    let stub_deriv_overlaps =
        find_stub_deriv_overlaps(graph, &layout.derivations, &all_stubs);

    let edge_label_overlaps = find_edge_label_overlaps(&parsed, &all_labels);
    let edge_arrowhead_overlaps = find_edge_arrowhead_overlaps(&parsed, &all_arrowheads);
    let edge_stub_overlaps = find_edge_stub_overlaps(&parsed, &all_stubs);
    let edge_domain_title_overlaps =
        find_edge_domain_title_overlaps(&parsed, &domain_title_aabbs);

    let label_label_overlaps = find_label_label_overlaps(&all_labels);
    let label_arrowhead_overlaps = find_label_arrowhead_overlaps(&all_labels, &all_arrowheads);
    let label_stub_overlaps = find_label_stub_overlaps(&all_labels, &all_stubs);
    let label_domain_title_overlaps =
        find_label_domain_title_overlaps(&all_labels, &domain_title_aabbs);

    let arrowhead_arrowhead_overlaps = find_arrowhead_arrowhead_overlaps(&all_arrowheads);
    let arrowhead_stub_overlaps = find_arrowhead_stub_overlaps(&all_arrowheads, &all_stubs);
    let arrowhead_domain_title_overlaps =
        find_arrowhead_domain_title_overlaps(&all_arrowheads, &domain_title_aabbs);

    let stub_stub_overlaps = find_stub_stub_overlaps(&all_stubs);
    let stub_domain_title_overlaps =
        find_stub_domain_title_overlaps(&all_stubs, &domain_title_aabbs);

    let domain_title_title_overlaps = find_domain_title_title_overlaps(&domain_title_aabbs);

    // ── Occlusion ────────────────────────────────────────────────────

    let edges_hidden_under_nodes =
        find_edges_hidden_under_nodes(graph, &layout.nodes, &parsed);
    let labels_hidden_under_nodes =
        find_labels_hidden_under_nodes(&layout.nodes, &all_labels);
    let arrowheads_hidden_under_nodes =
        find_arrowheads_hidden_under_nodes(graph, &layout.nodes, &all_arrowheads);
    let stubs_hidden_under_nodes =
        find_stubs_hidden_under_nodes(graph, &layout.nodes, &all_stubs);
    let connected_edge_occlusion =
        find_connected_edge_occlusion(graph, &layout.nodes, &parsed);

    // ── Canvas overflow ────────────────────────────────────────────────

    let nodes_outside_canvas = find_nodes_outside_canvas(&layout.nodes, total_width, total_height);
    let domains_outside_canvas =
        find_domains_outside_canvas(&layout.domains, total_width, total_height);
    let derivs_outside_canvas =
        find_derivs_outside_canvas(&layout.derivations, total_width, total_height);
    let edges_outside_canvas = find_edges_outside_canvas(&parsed, total_width, total_height);
    let labels_outside_canvas =
        find_labels_outside_canvas(&all_labels, total_width, total_height);
    let arrowheads_outside_canvas =
        find_arrowheads_outside_canvas(&all_arrowheads, total_width, total_height);
    let stubs_outside_canvas = find_stubs_outside_canvas(&all_stubs, total_width, total_height);

    // ── Domain corridor correctness ──────────────────────────────────

    let intra_edges_in_wrong_corridor =
        find_intra_edges_in_wrong_corridor(graph, &layout.domains, &parsed);

    // ── Layout symmetry ──────────────────────────────────────────────

    let visual_balance = compute_visual_balance(&layout.nodes, &layout.domains, total_width, total_height);
    let max_column_centering_error = compute_max_column_centering_error(&layout.domains);
    let domain_size_cv = compute_domain_size_cv(&layout.domains);

    // ── Edge routing quality ─────────────────────────────────────────

    let port_side_balance = compute_port_side_balance(&parsed);
    let edge_length_cv = compute_edge_length_cv(&parsed);
    let segment_complexity_distribution = compute_segment_complexity_distribution(&parsed);
    let routing_direction_balance = compute_routing_direction_balance(&parsed);

    QualityReport {
        node_overlaps,
        domain_overlaps,
        nodes_outside_domain,
        node_edge_overlaps,
        edge_crossings,
        min_node_gap,
        aspect_ratio,
        total_edge_length,
        node_width_delta,
        max_parent_misalignment,
        mean_constraint_segments,
        derivs_inside_domains,
        free_nodes_inside_domains,
        domain_contiguity_violations,
        inter_domain_edges_in_intra_corridors,
        crossing_pairs,
        channel_collisions,
        total_height,
        total_width,
        column_heights,
        column_height_imbalance,
        node_deriv_overlaps,
        label_node_overlaps,
        arrowhead_node_overlaps,
        stub_node_overlaps,
        edge_domain_boundary_crossings,
        label_domain_overlaps,
        arrowhead_domain_overlaps,
        stub_domain_overlaps,
        deriv_deriv_overlaps,
        edge_deriv_overlaps,
        label_deriv_overlaps,
        arrowhead_deriv_overlaps,
        stub_deriv_overlaps,
        edge_label_overlaps,
        edge_arrowhead_overlaps,
        edge_stub_overlaps,
        edge_domain_title_overlaps,
        label_label_overlaps,
        label_arrowhead_overlaps,
        label_stub_overlaps,
        label_domain_title_overlaps,
        arrowhead_arrowhead_overlaps,
        arrowhead_stub_overlaps,
        arrowhead_domain_title_overlaps,
        stub_stub_overlaps,
        stub_domain_title_overlaps,
        domain_title_title_overlaps,
        edges_hidden_under_nodes,
        labels_hidden_under_nodes,
        arrowheads_hidden_under_nodes,
        stubs_hidden_under_nodes,
        connected_edge_occlusion,
        nodes_outside_canvas,
        domains_outside_canvas,
        derivs_outside_canvas,
        edges_outside_canvas,
        labels_outside_canvas,
        arrowheads_outside_canvas,
        stubs_outside_canvas,
        intra_edges_in_wrong_corridor,
        visual_balance,
        max_column_centering_error,
        domain_size_cv,
        port_side_balance,
        edge_length_cv,
        segment_complexity_distribution,
        routing_direction_balance,
    }
}

// ---------------------------------------------------------------------------
// Geometry primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct LineSeg {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

impl LineSeg {
    fn length(&self) -> f64 {
        let dx = self.x2 - self.x1;
        let dy = self.y2 - self.y1;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy)]
struct Aabb {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Aabb {
    fn from_node(nl: &NodeLayout) -> Self {
        Self {
            x: nl.x,
            y: nl.y,
            w: nl.width,
            h: nl.height,
        }
    }

    fn from_domain(dl: &DomainLayout) -> Self {
        Self {
            x: dl.x,
            y: dl.y,
            w: dl.width,
            h: dl.height,
        }
    }

    fn from_deriv(dl: &DerivLayout) -> Self {
        Self {
            x: dl.x,
            y: dl.y,
            w: dl.width,
            h: dl.height,
        }
    }

    fn intersects(&self, other: &Aabb) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }

    fn contains(&self, other: &Aabb) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.w <= self.x + self.w
            && other.y + other.h <= self.y + self.h
    }

    /// Shrink the box by a small epsilon to avoid false positives from
    /// edges that exactly touch a node boundary.
    fn shrunk(&self, margin: f64) -> Aabb {
        Aabb {
            x: self.x + margin,
            y: self.y + margin,
            w: (self.w - 2.0 * margin).max(0.0),
            h: (self.h - 2.0 * margin).max(0.0),
        }
    }

    fn from_label(label: &EdgeLabel) -> Self {
        let (x, y, w, h) = label.bounding_box();
        Self { x, y, w, h }
    }

    /// Conservative AABB for the domain title zone (full domain width,
    /// DOMAIN_TITLE_HEIGHT tall).
    fn from_domain_title(dl: &DomainLayout) -> Self {
        Self {
            x: dl.x,
            y: dl.y,
            w: dl.width,
            h: DOMAIN_TITLE_HEIGHT,
        }
    }

    fn contains_point(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Test if a line segment intersects an AABB.
fn segment_intersects_aabb(seg: &LineSeg, aabb: &Aabb) -> bool {
    // Quick reject: if both endpoints are outside the same side, no intersection.
    let (xmin, xmax) = (aabb.x, aabb.x + aabb.w);
    let (ymin, ymax) = (aabb.y, aabb.y + aabb.h);

    // Check if either endpoint is inside the box.
    let p1_inside = seg.x1 >= xmin && seg.x1 <= xmax && seg.y1 >= ymin && seg.y1 <= ymax;
    let p2_inside = seg.x2 >= xmin && seg.x2 <= xmax && seg.y2 >= ymin && seg.y2 <= ymax;
    if p1_inside || p2_inside {
        return true;
    }

    // For orthogonal segments (horizontal/vertical), simplified test.
    let dx = (seg.x2 - seg.x1).abs();
    let dy = (seg.y2 - seg.y1).abs();

    if dy < 1e-6 {
        // Horizontal segment.
        let y = seg.y1;
        if y < ymin || y > ymax {
            return false;
        }
        let (sx_min, sx_max) = if seg.x1 < seg.x2 {
            (seg.x1, seg.x2)
        } else {
            (seg.x2, seg.x1)
        };
        sx_min < xmax && sx_max > xmin
    } else if dx < 1e-6 {
        // Vertical segment.
        let x = seg.x1;
        if x < xmin || x > xmax {
            return false;
        }
        let (sy_min, sy_max) = if seg.y1 < seg.y2 {
            (seg.y1, seg.y2)
        } else {
            (seg.y2, seg.y1)
        };
        sy_min < ymax && sy_max > ymin
    } else {
        // General case (shouldn't happen for orthogonal routing, but handle it).
        // Use parametric line-box intersection.
        let mut tmin = 0.0_f64;
        let mut tmax = 1.0_f64;
        let dir_x = seg.x2 - seg.x1;
        let dir_y = seg.y2 - seg.y1;

        for &(orig, dir, lo, hi) in &[
            (seg.x1, dir_x, xmin, xmax),
            (seg.y1, dir_y, ymin, ymax),
        ] {
            if dir.abs() < 1e-12 {
                if orig < lo || orig > hi {
                    return false;
                }
            } else {
                let inv_d = 1.0 / dir;
                let mut t1 = (lo - orig) * inv_d;
                let mut t2 = (hi - orig) * inv_d;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tmin = tmin.max(t1);
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return false;
                }
            }
        }
        true
    }
}

/// Test if two line segments cross (proper intersection, not touching endpoints).
fn segments_cross(a: &LineSeg, b: &LineSeg) -> bool {
    fn cross2d(ox: f64, oy: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
        (ax - ox) * (by - oy) - (ay - oy) * (bx - ox)
    }

    let d1 = cross2d(a.x1, a.y1, a.x2, a.y2, b.x1, b.y1);
    let d2 = cross2d(a.x1, a.y1, a.x2, a.y2, b.x2, b.y2);
    let d3 = cross2d(b.x1, b.y1, b.x2, b.y2, a.x1, a.y1);
    let d4 = cross2d(b.x1, b.y1, b.x2, b.y2, a.x2, a.y2);

    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return true;
    }
    false
}

/// True if both endpoints of the segment are inside the AABB.
fn segment_fully_inside_aabb(seg: &LineSeg, aabb: &Aabb) -> bool {
    aabb.contains_point(seg.x1, seg.y1) && aabb.contains_point(seg.x2, seg.y2)
}

/// True if the segment intersects the AABB but is NOT fully contained inside
/// it — i.e., the segment crosses the AABB border.
fn segment_crosses_aabb_border(seg: &LineSeg, aabb: &Aabb) -> bool {
    segment_intersects_aabb(seg, aabb) && !segment_fully_inside_aabb(seg, aabb)
}

/// Returns true if the edge is a DerivInput targeting this derivation.
fn edge_connects_to_deriv(graph: &Graph, edge_id: EdgeId, deriv_id: DerivId) -> bool {
    match &graph.edges[edge_id.index()] {
        Edge::DerivInput { target_deriv, .. } => *target_deriv == deriv_id,
        _ => false,
    }
}

/// Compute a 6×6 AABB for the arrowhead at the end of an edge's last segment.
/// Returns None if the edge has no segments.
fn arrowhead_aabb(segs: &[LineSeg]) -> Option<Aabb> {
    let last = segs.last()?;
    // Arrowhead is a 6×6 marker at the endpoint of the last segment,
    // extending in the direction the segment travels.
    let (tip_x, tip_y) = (last.x2, last.y2);
    let s = ARROWHEAD_SIZE;
    // Place a 6×6 box centered on the tip.
    Some(Aabb {
        x: tip_x - s / 2.0,
        y: tip_y - s / 2.0,
        w: s,
        h: s,
    })
}

/// Parse a StubPath's SVG into line segments.
fn parse_stub_segments(stub: &StubPath) -> Vec<LineSeg> {
    parse_svg_path(&stub.dotted_svg)
}

/// Extract source and target NodeIds for any edge type.
fn edge_endpoint_nodes(graph: &Graph, edge_id: EdgeId) -> (Option<NodeId>, Option<NodeId>) {
    match &graph.edges[edge_id.index()] {
        Edge::Anchor { parent, child, .. } => (Some(*parent), Some(*child)),
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let dst_node = graph.properties[dest_prop.index()].node;
            (Some(src_node), Some(dst_node))
        }
        Edge::DerivInput {
            source_prop,
            target_deriv,
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let deriv = &graph.derivations[target_deriv.index()];
            let out_node = graph.properties[deriv.output_prop.index()].node;
            (Some(src_node), Some(out_node))
        }
    }
}

/// Extract domain membership of an edge's endpoints.
fn edge_endpoint_domains(graph: &Graph, edge_id: EdgeId) -> (Option<DomainId>, Option<DomainId>) {
    let (src_nid, tgt_nid) = edge_endpoint_nodes(graph, edge_id);
    let sd = src_nid.and_then(|n| graph.nodes[n.index()].domain);
    let td = tgt_nid.and_then(|n| graph.nodes[n.index()].domain);
    (sd, td)
}

// ---------------------------------------------------------------------------
// SVG path parsing
// ---------------------------------------------------------------------------

/// Parse a simple SVG path (M/L commands only) into line segments.
fn parse_svg_path(d: &str) -> Vec<LineSeg> {
    let mut segments = Vec::new();
    let mut points: Vec<(f64, f64)> = Vec::new();

    for token in d.split_whitespace() {
        let cmd = token.chars().next().unwrap_or(' ');
        let rest = if cmd == 'M' || cmd == 'L' {
            &token[1..]
        } else {
            token
        };

        if let Some((xs, ys)) = rest.split_once(',')
            && let Ok(x) = xs.parse::<f64>()
            && let Ok(y) = ys.parse::<f64>()
        {
            points.push((x, y));
        }
    }

    for w in points.windows(2) {
        segments.push(LineSeg {
            x1: w[0].0,
            y1: w[0].1,
            x2: w[1].0,
            y2: w[1].1,
        });
    }

    segments
}

// ---------------------------------------------------------------------------
// Metric computations
// ---------------------------------------------------------------------------

fn find_node_overlaps(nodes: &[NodeLayout]) -> Vec<(NodeId, NodeId)> {
    let mut overlaps = Vec::new();
    for i in 0..nodes.len() {
        let a = Aabb::from_node(&nodes[i]);
        for j in (i + 1)..nodes.len() {
            let b = Aabb::from_node(&nodes[j]);
            if a.intersects(&b) {
                overlaps.push((nodes[i].id, nodes[j].id));
            }
        }
    }
    overlaps
}

fn find_domain_overlaps(domains: &[DomainLayout]) -> Vec<(DomainId, DomainId)> {
    let mut overlaps = Vec::new();
    for i in 0..domains.len() {
        let a = Aabb::from_domain(&domains[i]);
        for j in (i + 1)..domains.len() {
            let b = Aabb::from_domain(&domains[j]);
            if a.intersects(&b) {
                overlaps.push((domains[i].id, domains[j].id));
            }
        }
    }
    overlaps
}

fn find_nodes_outside_domain(
    graph: &Graph,
    nodes: &[NodeLayout],
    domains: &[DomainLayout],
) -> Vec<(NodeId, DomainId)> {
    let mut violations = Vec::new();
    for domain in &graph.domains {
        let dl = domains.iter().find(|d| d.id == domain.id);
        let dl = match dl {
            Some(d) => d,
            None => continue,
        };
        let domain_box = Aabb::from_domain(dl);
        for &nid in &domain.members {
            if let Some(nl) = nodes.iter().find(|n| n.id == nid) {
                let node_box = Aabb::from_node(nl);
                if !domain_box.contains(&node_box) {
                    violations.push((nid, domain.id));
                }
            }
        }
    }
    violations
}

/// Find cross-domain derivation pills that overlap a domain they don't belong to.
fn find_derivs_inside_domains(
    graph: &Graph,
    derivs: &[DerivLayout],
    domains: &[DomainLayout],
) -> Vec<(DerivId, DomainId)> {
    let mut violations = Vec::new();
    for deriv in &graph.derivations {
        // Collect input/output domains.
        let mut involved: std::collections::HashSet<DomainId> = std::collections::HashSet::new();
        for &pid in &deriv.inputs {
            if let Some(did) = graph.nodes[graph.properties[pid.index()].node.index()].domain {
                involved.insert(did);
            }
        }
        if let Some(did) = graph.nodes[graph.properties[deriv.output_prop.index()].node.index()].domain {
            involved.insert(did);
        }
        let is_cross_domain = involved.len() > 1
            || deriv.inputs.iter().any(|&pid| {
                graph.nodes[graph.properties[pid.index()].node.index()].domain.is_none()
            });
        if !is_cross_domain {
            continue;
        }
        let dl = &derivs[deriv.id.index()];
        let deriv_box = Aabb::from_deriv(dl);
        for domain_dl in domains {
            let domain_box = Aabb::from_domain(domain_dl);
            if deriv_box.intersects(&domain_box) {
                violations.push((deriv.id, domain_dl.id));
            }
        }
    }
    violations
}

/// Find domain-less nodes that overlap any domain rect.
fn find_free_nodes_inside_domains(
    graph: &Graph,
    nodes: &[NodeLayout],
    domains: &[DomainLayout],
) -> Vec<(NodeId, DomainId)> {
    let mut violations = Vec::new();
    for node in &graph.nodes {
        if node.domain.is_some() {
            continue; // Not a free node.
        }
        if let Some(nl) = nodes.iter().find(|n| n.id == node.id) {
            let node_box = Aabb::from_node(nl);
            for dl in domains {
                let domain_box = Aabb::from_domain(dl);
                if node_box.intersects(&domain_box) {
                    violations.push((node.id, dl.id));
                }
            }
        }
    }
    violations
}

/// Find domains whose vertical contiguity is violated by a foreign node.
/// A violation occurs when a node NOT in the domain has a y-range that falls
/// between the domain's topmost and bottommost member nodes.
fn find_domain_contiguity_violations(
    graph: &Graph,
    nodes: &[NodeLayout],
    domains: &[DomainLayout],
) -> Vec<(DomainId, NodeId)> {
    let mut violations = Vec::new();
    for domain in &graph.domains {
        // Compute the y-range of member nodes.
        let member_set: std::collections::HashSet<NodeId> =
            domain.members.iter().copied().collect();
        let min_y = domain
            .members
            .iter()
            .filter_map(|&nid| nodes.iter().find(|n| n.id == nid))
            .map(|nl| nl.y)
            .fold(f64::INFINITY, f64::min);
        let max_y = domain
            .members
            .iter()
            .filter_map(|&nid| nodes.iter().find(|n| n.id == nid))
            .map(|nl| nl.y + nl.height)
            .fold(f64::NEG_INFINITY, f64::max);
        if !min_y.is_finite() || !max_y.is_finite() {
            continue;
        }
        // Find the x-range of this domain for column check.
        let dl = match domains.iter().find(|d| d.id == domain.id) {
            Some(d) => d,
            None => continue,
        };
        // Check every non-member node.
        for nl in nodes {
            if member_set.contains(&nl.id) {
                continue;
            }
            // Only check nodes in the same column (x-range overlaps domain).
            let overlaps_x = nl.x < dl.x + dl.width && nl.x + nl.width > dl.x;
            if !overlaps_x {
                continue;
            }
            // Check if node's y-range falls within the domain's member y-range.
            let node_top = nl.y;
            let node_bottom = nl.y + nl.height;
            if node_bottom > min_y && node_top < max_y {
                violations.push((domain.id, nl.id));
            }
        }
    }
    violations
}

fn find_node_edge_overlaps(
    graph: &Graph,
    nodes: &[NodeLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(NodeId, EdgeId)> {
    let mut overlaps = Vec::new();

    for nl in nodes {
        // Shrink node box slightly to avoid false positives from edges
        // that start or end exactly on the node boundary.
        let node_box = Aabb::from_node(nl).shrunk(2.0);
        if node_box.w <= 0.0 || node_box.h <= 0.0 {
            continue;
        }

        for &(edge_id, ref segs) in edges {
            // Skip edges that are connected to this node (they naturally touch it).
            if edge_connects_to_node(graph, edge_id, nl.id) {
                continue;
            }

            let hits = segs.iter().any(|seg| segment_intersects_aabb(seg, &node_box));
            if hits {
                overlaps.push((nl.id, edge_id));
            }
        }
    }
    overlaps
}

/// Check if an edge has this node as one of its endpoints.
fn edge_connects_to_node(graph: &Graph, edge_id: EdgeId, node_id: NodeId) -> bool {
    let edge = &graph.edges[edge_id.index()];
    match edge {
        crate::model::types::Edge::Anchor { parent, child, .. } => {
            *parent == node_id || *child == node_id
        }
        crate::model::types::Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            let dst_node = graph.properties[dest_prop.index()].node;
            src_node == node_id || dst_node == node_id
        }
        crate::model::types::Edge::DerivInput {
            source_prop,
            target_deriv: _,
        } => {
            let src_node = graph.properties[source_prop.index()].node;
            // Derivation nodes don't have a NodeId, so only check source.
            src_node == node_id
        }
    }
}

fn count_segment_crossings(edges: &[(EdgeId, Vec<LineSeg>)]) -> usize {
    let mut count = 0;
    for i in 0..edges.len() {
        for j in (i + 1)..edges.len() {
            for seg_a in &edges[i].1 {
                for seg_b in &edges[j].1 {
                    if segments_cross(seg_a, seg_b) {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

fn find_crossing_pairs(edges: &[(EdgeId, Vec<LineSeg>)]) -> Vec<(EdgeId, EdgeId)> {
    let mut pairs = Vec::new();
    for i in 0..edges.len() {
        for j in (i + 1)..edges.len() {
            let mut crosses = false;
            for seg_a in &edges[i].1 {
                for seg_b in &edges[j].1 {
                    if segments_cross(seg_a, seg_b) {
                        crosses = true;
                    }
                }
            }
            if crosses {
                pairs.push((edges[i].0, edges[j].0));
            }
        }
    }
    pairs
}

fn compute_node_width_delta(nodes: &[NodeLayout]) -> f64 {
    if nodes.is_empty() {
        return 0.0;
    }
    let max_w = nodes.iter().map(|n| n.width).fold(f64::NEG_INFINITY, f64::max);
    let min_w = nodes.iter().map(|n| n.width).fold(f64::INFINITY, f64::min);
    max_w - min_w
}

fn compute_max_parent_misalignment(graph: &Graph, nodes: &[NodeLayout]) -> f64 {
    use std::collections::HashMap;

    // Build parent -> children center-X list.
    let mut parent_to_child_centers: HashMap<NodeId, Vec<f64>> = HashMap::new();
    for edge in &graph.edges {
        if let Edge::Anchor { parent, child, .. } = edge
            && let Some(child_nl) = nodes.iter().find(|n| n.id == *child)
        {
            let cx = child_nl.x + child_nl.width / 2.0;
            parent_to_child_centers.entry(*parent).or_default().push(cx);
        }
    }

    let mut max_offset = 0.0_f64;
    for (parent_id, child_centers) in &parent_to_child_centers {
        let parent_nl = match nodes.iter().find(|n| n.id == *parent_id) {
            Some(n) => n,
            None => continue,
        };
        // Midpoint of the leftmost and rightmost child centers.
        let min_cx = child_centers.iter().copied().fold(f64::INFINITY, f64::min);
        let max_cx = child_centers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let midpoint = (min_cx + max_cx) / 2.0;
        let parent_cx = parent_nl.x + parent_nl.width / 2.0;
        max_offset = max_offset.max((parent_cx - midpoint).abs());
    }
    max_offset
}

fn compute_mean_constraint_segments(constraints: &[EdgePath]) -> f64 {
    if constraints.is_empty() {
        return 0.0;
    }
    let total_segs: usize = constraints
        .iter()
        .map(|ep| parse_svg_path(&ep.svg_path).len())
        .sum();
    total_segs as f64 / constraints.len() as f64
}

/// Find cross-domain constraint edges whose vertical segments fall inside
/// an intra-domain corridor zone of a domain they are NOT connected to.
///
/// The intra-domain corridor is the strip between a domain's border and its
/// node area: `[domain.x, domain.x + lr_pad]` on the left and
/// `[domain.x + domain.width - lr_pad, domain.x + domain.width]` on the right.
///
/// A cross-domain edge is allowed to traverse through the corridor of a domain
/// one of its endpoints belongs to (necessary for same-column cross-domain
/// edges). It must NOT traverse through the corridor of an unrelated domain.
fn find_inter_domain_edges_in_intra_corridors(
    graph: &Graph,
    domains: &[DomainLayout],
    _node_layouts: &[NodeLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DomainId)> {
    let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
    let mut violations = Vec::new();

    // Pre-compute corridor zones for each domain.
    struct CorridorZone {
        id: DomainId,
        left_x_start: f64,
        left_x_end: f64,
        right_x_start: f64,
        right_x_end: f64,
        y_start: f64,
        y_end: f64,
    }

    let zones: Vec<CorridorZone> = domains
        .iter()
        .map(|dl| CorridorZone {
            id: dl.id,
            left_x_start: dl.x,
            left_x_end: dl.x + lr_pad,
            right_x_start: dl.x + dl.width - lr_pad,
            right_x_end: dl.x + dl.width,
            y_start: dl.y,
            y_end: dl.y + dl.height,
        })
        .collect();

    for &(edge_id, ref segs) in edges {
        let edge = &graph.edges[edge_id.index()];
        // Only check cross-domain constraint edges.
        match edge {
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => {
                let src_node = graph.properties[source_prop.index()].node;
                let dst_node = graph.properties[dest_prop.index()].node;
                let sd = graph.nodes[src_node.index()].domain;
                let td = graph.nodes[dst_node.index()].domain;
                if sd == td {
                    continue; // Not cross-domain.
                }
            }
            _ => continue,
        };

        // Check each vertical segment.
        for seg in segs {
            let (x, y_min, y_max) = match seg {
                LineSeg {
                    x1, y1, x2, y2, ..
                } if (x1 - x2).abs() < 0.5 => (*x1, y1.min(*y2), y1.max(*y2)),
                _ => continue, // Not a vertical segment.
            };

            for zone in &zones {
                // Cross-domain edges must never use any intra-domain corridor,
                // including corridors of their own endpoint domains.
                // Check if vertical segment is inside either corridor zone
                // and overlaps the domain's y-range.
                let in_left = x >= zone.left_x_start && x <= zone.left_x_end;
                let in_right = x >= zone.right_x_start && x <= zone.right_x_end;
                if !in_left && !in_right {
                    continue;
                }
                let y_overlap = y_max > zone.y_start && y_min < zone.y_end;
                if y_overlap {
                    violations.push((edge_id, zone.id));
                    break; // One violation per edge per domain is enough.
                }
            }
        }
    }
    violations
}

/// Find pairs of edges that share the same vertical channel x-coordinate
/// while their y-ranges overlap (channel collision).
///
/// Consecutive center-port edges (anchors in a chain) that share a common
/// node are exempt — their vertical segments naturally overlap at the shared
/// node's center x.
fn find_channel_collisions(
    graph: &Graph,
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, EdgeId)> {
    // Extract all vertical segments with their edge_id.
    let mut verticals: Vec<(EdgeId, f64, f64, f64)> = Vec::new(); // (edge_id, x, y_min, y_max)
    for &(edge_id, ref segs) in edges {
        for seg in segs {
            if (seg.x1 - seg.x2).abs() < 0.5 {
                let y_min = seg.y1.min(seg.y2);
                let y_max = seg.y1.max(seg.y2);
                if (y_max - y_min) > 1.0 {
                    // Only non-trivial vertical segments.
                    verticals.push((edge_id, seg.x1, y_min, y_max));
                }
            }
        }
    }

    // Helper: get the set of node IDs involved in an edge.
    let edge_nodes = |eid: EdgeId| -> Vec<NodeId> {
        match &graph.edges[eid.index()] {
            Edge::Anchor { parent, child, .. } => vec![*parent, *child],
            Edge::Constraint { source_prop, dest_prop, .. } => {
                vec![
                    graph.properties[source_prop.index()].node,
                    graph.properties[dest_prop.index()].node,
                ]
            }
            Edge::DerivInput { source_prop, .. } => {
                vec![graph.properties[source_prop.index()].node]
            }
        }
    };

    // Two edges share a common endpoint (node or derivation).
    let shares_endpoint = |a: EdgeId, b: EdgeId| -> bool {
        let nodes_a = edge_nodes(a);
        let nodes_b = edge_nodes(b);
        if nodes_a.iter().any(|n| nodes_b.contains(n)) {
            return true;
        }
        // Also check shared target derivation for DerivInput pairs.
        let deriv_a = match &graph.edges[a.index()] {
            Edge::DerivInput { target_deriv, .. } => Some(target_deriv),
            _ => None,
        };
        let deriv_b = match &graph.edges[b.index()] {
            Edge::DerivInput { target_deriv, .. } => Some(target_deriv),
            _ => None,
        };
        deriv_a.is_some() && deriv_a == deriv_b
    };

    // Both edges use center-port routing (Anchors sharing a node, or
    // DerivInputs converging to the same derivation target).
    let both_center_port = |a: EdgeId, b: EdgeId| -> bool {
        let is_center = |e: &Edge| matches!(e, Edge::Anchor { .. } | Edge::DerivInput { .. });
        is_center(&graph.edges[a.index()]) && is_center(&graph.edges[b.index()])
    };

    let mut collisions = Vec::new();
    for i in 0..verticals.len() {
        for j in (i + 1)..verticals.len() {
            let (eid_a, x_a, y_min_a, y_max_a) = verticals[i];
            let (eid_b, x_b, y_min_b, y_max_b) = verticals[j];
            if eid_a == eid_b {
                continue; // Same edge, different segments.
            }
            // Same x (within tolerance) and overlapping y-ranges.
            if (x_a - x_b).abs() < 0.5 && y_max_a > y_min_b + 0.5 && y_max_b > y_min_a + 0.5 {
                // Exempt center-port edges that share a node or derivation.
                if both_center_port(eid_a, eid_b) && shares_endpoint(eid_a, eid_b) {
                    continue;
                }
                collisions.push((eid_a, eid_b));
            }
        }
    }
    collisions
}

fn compute_min_node_gap(nodes: &[NodeLayout]) -> f64 {
    let mut min_gap = f64::INFINITY;
    for i in 0..nodes.len() {
        for j in (i + 1)..nodes.len() {
            let a = &nodes[i];
            let b = &nodes[j];
            // Only consider horizontal gap for nodes at similar y-positions
            // (i.e., in the same layer).
            let y_overlap = a.y < b.y + b.height && a.y + a.height > b.y;
            if y_overlap {
                let gap = if a.x + a.width <= b.x {
                    b.x - (a.x + a.width)
                } else if b.x + b.width <= a.x {
                    a.x - (b.x + b.width)
                } else {
                    0.0 // Overlapping
                };
                min_gap = min_gap.min(gap);
            }
        }
    }
    min_gap
}

/// Cluster domains into columns by x-center (within 100px tolerance).
/// Returns (column_centers, column_assignments) where column_assignments[i]
/// is the column index for domains[i].
fn cluster_domain_columns(domains: &[DomainLayout]) -> (Vec<f64>, Vec<usize>) {
    if domains.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut col_centers: Vec<f64> = Vec::new();
    for dl in domains {
        let cx = dl.x + dl.width / 2.0;
        let found = col_centers.iter().any(|&c| (c - cx).abs() < 100.0);
        if !found {
            col_centers.push(cx);
        }
    }
    col_centers.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let assign_column = |cx: f64| -> usize {
        col_centers
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (cx - **a).abs().partial_cmp(&(cx - **b).abs()).unwrap()
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    };

    let assignments: Vec<usize> = domains
        .iter()
        .map(|dl| assign_column(dl.x + dl.width / 2.0))
        .collect();
    (col_centers, assignments)
}

/// Compute per-column heights from domain layouts.
///
/// Clusters domains into columns by x-center (within 100px tolerance),
/// then computes each column's height as the span from the topmost domain
/// to the bottommost domain within that column.
fn compute_column_heights(domains: &[DomainLayout]) -> Vec<f64> {
    let (col_centers, assignments) = cluster_domain_columns(domains);
    if col_centers.is_empty() {
        return Vec::new();
    }

    let num_cols = col_centers.len();
    let mut col_min_y = vec![f64::INFINITY; num_cols];
    let mut col_max_y = vec![f64::NEG_INFINITY; num_cols];

    for (i, dl) in domains.iter().enumerate() {
        let col = assignments[i];
        col_min_y[col] = col_min_y[col].min(dl.y);
        col_max_y[col] = col_max_y[col].max(dl.y + dl.height);
    }

    (0..num_cols)
        .map(|c| {
            if col_min_y[c].is_finite() && col_max_y[c].is_finite() {
                (col_max_y[c] - col_min_y[c]).max(0.0)
            } else {
                0.0
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Collision matrix detectors (27 new pairs)
// ---------------------------------------------------------------------------

fn find_node_deriv_overlaps(
    nodes: &[NodeLayout],
    derivs: &[DerivLayout],
) -> Vec<(NodeId, DerivId)> {
    let mut out = Vec::new();
    for n in nodes {
        let nb = Aabb::from_node(n);
        for d in derivs {
            let db = Aabb::from_deriv(d);
            if nb.intersects(&db) {
                out.push((n.id, d.id));
            }
        }
    }
    out
}

fn find_label_node_overlaps(
    graph: &Graph,
    nodes: &[NodeLayout],
    labels: &[(EdgeId, &EdgeLabel)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for &(eid, label) in labels {
        let lb = Aabb::from_label(label);
        for n in nodes {
            if edge_connects_to_node(graph, eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if lb.intersects(&nb) {
                out.push((eid, n.id));
            }
        }
    }
    out
}

fn find_arrowhead_node_overlaps(
    graph: &Graph,
    nodes: &[NodeLayout],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for &(eid, ref ab) in arrowheads {
        for n in nodes {
            if edge_connects_to_node(graph, eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if ab.intersects(&nb) {
                out.push((eid, n.id));
            }
        }
    }
    out
}

fn find_stub_node_overlaps(
    graph: &Graph,
    nodes: &[NodeLayout],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for (eid, segs) in stubs {
        for n in nodes {
            if edge_connects_to_node(graph, *eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if segs.iter().any(|s| segment_intersects_aabb(s, &nb)) {
                out.push((*eid, n.id));
            }
        }
    }
    out
}

fn find_edge_domain_boundary_crossings(
    graph: &Graph,
    domains: &[DomainLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        let (sd, td) = edge_endpoint_domains(graph, eid);
        for dl in domains {
            // Skip domains that either endpoint belongs to.
            if sd == Some(dl.id) || td == Some(dl.id) {
                continue;
            }
            let db = Aabb::from_domain(dl);
            if segs.iter().any(|s| segment_crosses_aabb_border(s, &db)) {
                out.push((eid, dl.id));
            }
        }
    }
    out
}

fn find_label_domain_overlaps(
    domains: &[DomainLayout],
    labels: &[(EdgeId, &EdgeLabel)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(eid, label) in labels {
        let lb = Aabb::from_label(label);
        for dl in domains {
            let db = Aabb::from_domain(dl);
            if lb.intersects(&db) {
                out.push((eid, dl.id));
            }
        }
    }
    out
}

fn find_arrowhead_domain_overlaps(
    graph: &Graph,
    domains: &[DomainLayout],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(eid, ref ab) in arrowheads {
        let (sd, td) = edge_endpoint_domains(graph, eid);
        for dl in domains {
            if sd == Some(dl.id) || td == Some(dl.id) {
                continue;
            }
            let db = Aabb::from_domain(dl);
            if ab.intersects(&db) {
                out.push((eid, dl.id));
            }
        }
    }
    out
}

fn find_stub_domain_overlaps(
    graph: &Graph,
    domains: &[DomainLayout],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for (eid, segs) in stubs {
        let (sd, td) = edge_endpoint_domains(graph, *eid);
        for dl in domains {
            if sd == Some(dl.id) || td == Some(dl.id) {
                continue;
            }
            let db = Aabb::from_domain(dl);
            if segs.iter().any(|s| segment_intersects_aabb(s, &db)) {
                out.push((*eid, dl.id));
            }
        }
    }
    out
}

fn find_deriv_deriv_overlaps(derivs: &[DerivLayout]) -> Vec<(DerivId, DerivId)> {
    let mut out = Vec::new();
    for i in 0..derivs.len() {
        let a = Aabb::from_deriv(&derivs[i]);
        for j in (i + 1)..derivs.len() {
            let b = Aabb::from_deriv(&derivs[j]);
            if a.intersects(&b) {
                out.push((derivs[i].id, derivs[j].id));
            }
        }
    }
    out
}

fn find_edge_deriv_overlaps(
    graph: &Graph,
    derivs: &[DerivLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DerivId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        for d in derivs {
            if edge_connects_to_deriv(graph, eid, d.id) {
                continue;
            }
            let db = Aabb::from_deriv(d);
            if segs.iter().any(|s| segment_intersects_aabb(s, &db)) {
                out.push((eid, d.id));
            }
        }
    }
    out
}

fn find_label_deriv_overlaps(
    derivs: &[DerivLayout],
    labels: &[(EdgeId, &EdgeLabel)],
) -> Vec<(EdgeId, DerivId)> {
    let mut out = Vec::new();
    for &(eid, label) in labels {
        let lb = Aabb::from_label(label);
        for d in derivs {
            let db = Aabb::from_deriv(d);
            if lb.intersects(&db) {
                out.push((eid, d.id));
            }
        }
    }
    out
}

fn find_arrowhead_deriv_overlaps(
    graph: &Graph,
    derivs: &[DerivLayout],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, DerivId)> {
    let mut out = Vec::new();
    for &(eid, ref ab) in arrowheads {
        for d in derivs {
            if edge_connects_to_deriv(graph, eid, d.id) {
                continue;
            }
            let db = Aabb::from_deriv(d);
            if ab.intersects(&db) {
                out.push((eid, d.id));
            }
        }
    }
    out
}

fn find_stub_deriv_overlaps(
    graph: &Graph,
    derivs: &[DerivLayout],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DerivId)> {
    let mut out = Vec::new();
    for (eid, segs) in stubs {
        for d in derivs {
            if edge_connects_to_deriv(graph, *eid, d.id) {
                continue;
            }
            let db = Aabb::from_deriv(d);
            if segs.iter().any(|s| segment_intersects_aabb(s, &db)) {
                out.push((*eid, d.id));
            }
        }
    }
    out
}

fn find_edge_label_overlaps(
    edges: &[(EdgeId, Vec<LineSeg>)],
    labels: &[(EdgeId, &EdgeLabel)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        for &(lid, label) in labels {
            if eid == lid {
                continue; // Skip own label.
            }
            let lb = Aabb::from_label(label);
            if segs.iter().any(|s| segment_intersects_aabb(s, &lb)) {
                out.push((eid, lid));
            }
        }
    }
    out
}

fn find_edge_arrowhead_overlaps(
    edges: &[(EdgeId, Vec<LineSeg>)],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        for &(aid, ref ab) in arrowheads {
            if eid == aid {
                continue;
            }
            if segs.iter().any(|s| segment_intersects_aabb(s, ab)) {
                out.push((eid, aid));
            }
        }
    }
    out
}

fn find_edge_stub_overlaps(
    edges: &[(EdgeId, Vec<LineSeg>)],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(eid, ref e_segs) in edges {
        for (sid, s_segs) in stubs {
            if eid == *sid {
                continue;
            }
            let hit = e_segs
                .iter()
                .any(|es| s_segs.iter().any(|ss| segments_cross(es, ss)));
            if hit {
                out.push((eid, *sid));
            }
        }
    }
    out
}

fn find_edge_domain_title_overlaps(
    edges: &[(EdgeId, Vec<LineSeg>)],
    titles: &[(DomainId, Aabb)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        for &(did, ref tb) in titles {
            if segs.iter().any(|s| segment_intersects_aabb(s, tb)) {
                out.push((eid, did));
            }
        }
    }
    out
}

fn find_label_label_overlaps(labels: &[(EdgeId, &EdgeLabel)]) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for (i, &(eid_a, la)) in labels.iter().enumerate() {
        let ab = Aabb::from_label(la);
        for &(eid_b, lb) in &labels[i + 1..] {
            let bb = Aabb::from_label(lb);
            if ab.intersects(&bb) {
                out.push((eid_a, eid_b));
            }
        }
    }
    out
}

fn find_label_arrowhead_overlaps(
    labels: &[(EdgeId, &EdgeLabel)],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(lid, label) in labels {
        let lb = Aabb::from_label(label);
        for &(aid, ref ab) in arrowheads {
            if lid == aid {
                continue;
            }
            if lb.intersects(ab) {
                out.push((lid, aid));
            }
        }
    }
    out
}

fn find_label_stub_overlaps(
    labels: &[(EdgeId, &EdgeLabel)],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(lid, label) in labels {
        let lb = Aabb::from_label(label);
        for (sid, segs) in stubs {
            if lid == *sid {
                continue;
            }
            if segs.iter().any(|s| segment_intersects_aabb(s, &lb)) {
                out.push((lid, *sid));
            }
        }
    }
    out
}

fn find_label_domain_title_overlaps(
    labels: &[(EdgeId, &EdgeLabel)],
    titles: &[(DomainId, Aabb)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(lid, label) in labels {
        let lb = Aabb::from_label(label);
        for &(did, ref tb) in titles {
            if lb.intersects(tb) {
                out.push((lid, did));
            }
        }
    }
    out
}

fn find_arrowhead_arrowhead_overlaps(
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for (i, &(eid_a, ref ab_a)) in arrowheads.iter().enumerate() {
        for &(eid_b, ref ab_b) in &arrowheads[i + 1..] {
            if ab_a.intersects(ab_b) {
                out.push((eid_a, eid_b));
            }
        }
    }
    out
}

fn find_arrowhead_stub_overlaps(
    arrowheads: &[(EdgeId, Aabb)],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for &(aid, ref ab) in arrowheads {
        for (sid, segs) in stubs {
            if aid == *sid {
                continue;
            }
            if segs.iter().any(|s| segment_intersects_aabb(s, ab)) {
                out.push((aid, *sid));
            }
        }
    }
    out
}

fn find_arrowhead_domain_title_overlaps(
    arrowheads: &[(EdgeId, Aabb)],
    titles: &[(DomainId, Aabb)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for &(aid, ref ab) in arrowheads {
        for &(did, ref tb) in titles {
            if ab.intersects(tb) {
                out.push((aid, did));
            }
        }
    }
    out
}

fn find_stub_stub_overlaps(stubs: &[(EdgeId, Vec<LineSeg>)]) -> Vec<(EdgeId, EdgeId)> {
    let mut out = Vec::new();
    for (i, (eid_a, segs_a)) in stubs.iter().enumerate() {
        for (eid_b, segs_b) in &stubs[i + 1..] {
            let hit = segs_a
                .iter()
                .any(|sa| segs_b.iter().any(|sb| segments_cross(sa, sb)));
            if hit {
                out.push((*eid_a, *eid_b));
            }
        }
    }
    out
}

fn find_stub_domain_title_overlaps(
    stubs: &[(EdgeId, Vec<LineSeg>)],
    titles: &[(DomainId, Aabb)],
) -> Vec<(EdgeId, DomainId)> {
    let mut out = Vec::new();
    for (sid, segs) in stubs {
        for &(did, ref tb) in titles {
            if segs.iter().any(|s| segment_intersects_aabb(s, tb)) {
                out.push((*sid, did));
            }
        }
    }
    out
}

fn find_domain_title_title_overlaps(
    titles: &[(DomainId, Aabb)],
) -> Vec<(DomainId, DomainId)> {
    let mut out = Vec::new();
    for (i, &(did_a, ref ab_a)) in titles.iter().enumerate() {
        for &(did_b, ref ab_b) in &titles[i + 1..] {
            if ab_a.intersects(ab_b) {
                out.push((did_a, did_b));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Occlusion / hidden element detectors
// ---------------------------------------------------------------------------

fn find_edges_hidden_under_nodes(
    graph: &Graph,
    nodes: &[NodeLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for &(eid, ref segs) in edges {
        if segs.is_empty() {
            continue;
        }
        for n in nodes {
            if edge_connects_to_node(graph, eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if segs.iter().all(|s| segment_fully_inside_aabb(s, &nb)) {
                out.push((eid, n.id));
            }
        }
    }
    out
}

fn find_labels_hidden_under_nodes(
    nodes: &[NodeLayout],
    labels: &[(EdgeId, &EdgeLabel)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for &(eid, label) in labels {
        let lb = Aabb::from_label(label);
        for n in nodes {
            let nb = Aabb::from_node(n);
            if nb.contains(&lb) {
                out.push((eid, n.id));
            }
        }
    }
    out
}

fn find_arrowheads_hidden_under_nodes(
    graph: &Graph,
    nodes: &[NodeLayout],
    arrowheads: &[(EdgeId, Aabb)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for &(eid, ref ab) in arrowheads {
        for n in nodes {
            if edge_connects_to_node(graph, eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if nb.contains(ab) {
                out.push((eid, n.id));
            }
        }
    }
    out
}

fn find_stubs_hidden_under_nodes(
    graph: &Graph,
    nodes: &[NodeLayout],
    stubs: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, NodeId)> {
    let mut out = Vec::new();
    for (eid, segs) in stubs {
        if segs.is_empty() {
            continue;
        }
        for n in nodes {
            if edge_connects_to_node(graph, *eid, n.id) {
                continue;
            }
            let nb = Aabb::from_node(n);
            if segs.iter().all(|s| segment_fully_inside_aabb(s, &nb)) {
                out.push((*eid, n.id));
            }
        }
    }
    out
}

/// Compute the length of a line segment that lies inside an AABB.
/// Works for arbitrary segments but is especially efficient for axis-aligned
/// (H or V) segments from orthogonal routing. Clips the segment to the AABB
/// and returns the clipped length.
fn segment_length_inside_aabb(seg: &LineSeg, aabb: &Aabb) -> f64 {
    let ax = aabb.x;
    let ay = aabb.y;
    let bx = aabb.x + aabb.w;
    let by = aabb.y + aabb.h;

    // Liang-Barsky clipping for the segment against the AABB.
    let dx = seg.x2 - seg.x1;
    let dy = seg.y2 - seg.y1;

    let mut t_min = 0.0_f64;
    let mut t_max = 1.0_f64;

    let clips = [
        (-dx, seg.x1 - ax),  // left
        (dx, bx - seg.x1),   // right
        (-dy, seg.y1 - ay),  // top
        (dy, by - seg.y1),   // bottom
    ];

    for &(p, q) in &clips {
        if p.abs() < 1e-9 {
            // Segment is parallel to this edge — check if it's outside
            if q < 0.0 {
                return 0.0;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t_min = t_min.max(t);
            } else {
                t_max = t_max.min(t);
            }
            if t_min > t_max {
                return 0.0;
            }
        }
    }

    let seg_len = seg.length();
    (t_max - t_min) * seg_len
}

/// Find connected edges with significant occlusion behind their own endpoint
/// nodes. For each edge, measures how much total path length lies inside each
/// endpoint node's AABB (using partial clipping, not just full containment).
/// Returns entries where the hidden fraction exceeds 25%.
fn find_connected_edge_occlusion(
    graph: &Graph,
    nodes: &[NodeLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, NodeId, f64, f64)> {
    let mut out = Vec::new();
    let node_aabbs: Vec<(NodeId, Aabb)> = nodes.iter().map(|n| (n.id, Aabb::from_node(n))).collect();

    for &(eid, ref segs) in edges {
        if segs.is_empty() {
            continue;
        }
        let total_len: f64 = segs.iter().map(|s| s.length()).sum();
        if total_len < 1.0 {
            continue;
        }

        let (src_nid, dst_nid) = edge_endpoint_nodes(graph, eid);
        let endpoint_nids: Vec<NodeId> = [src_nid, dst_nid].iter().filter_map(|n| *n).collect();

        for &nid in &endpoint_nids {
            let Some(nb) = node_aabbs.iter().find(|(id, _)| *id == nid).map(|(_, a)| a) else {
                continue;
            };
            let hidden: f64 = segs.iter().map(|s| segment_length_inside_aabb(s, nb)).sum();
            let fraction = hidden / total_len;
            if fraction > 0.25 {
                out.push((eid, nid, hidden, total_len));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Canvas overflow detection
// ---------------------------------------------------------------------------

/// Compute how many pixels of an AABB extend beyond the canvas bounds.
/// Returns 0.0 if fully inside.
fn aabb_overflow(aabb: &Aabb, canvas_w: f64, canvas_h: f64) -> f64 {
    let mut overflow = 0.0_f64;
    if aabb.x < 0.0 {
        overflow += -aabb.x;
    }
    if aabb.y < 0.0 {
        overflow += -aabb.y;
    }
    let right = aabb.x + aabb.w;
    if right > canvas_w {
        overflow += right - canvas_w;
    }
    let bottom = aabb.y + aabb.h;
    if bottom > canvas_h {
        overflow += bottom - canvas_h;
    }
    overflow
}

/// Compute how many pixels of a segment extend beyond the canvas bounds.
/// For axis-aligned segments, returns the length of the portion outside.
fn segment_overflow(seg: &LineSeg, canvas_w: f64, canvas_h: f64) -> f64 {
    let canvas = Aabb {
        x: 0.0,
        y: 0.0,
        w: canvas_w,
        h: canvas_h,
    };
    let inside = segment_length_inside_aabb(seg, &canvas);
    let total = seg.length();
    (total - inside).max(0.0)
}

fn find_nodes_outside_canvas(
    nodes: &[NodeLayout],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(NodeId, f64)> {
    nodes
        .iter()
        .filter_map(|n| {
            let overflow = aabb_overflow(&Aabb::from_node(n), canvas_w, canvas_h);
            if overflow > 0.5 { Some((n.id, overflow)) } else { None }
        })
        .collect()
}

fn find_domains_outside_canvas(
    domains: &[DomainLayout],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(DomainId, f64)> {
    domains
        .iter()
        .filter_map(|d| {
            let ab = Aabb {
                x: d.x,
                y: d.y,
                w: d.width,
                h: d.height,
            };
            let overflow = aabb_overflow(&ab, canvas_w, canvas_h);
            if overflow > 0.5 { Some((d.id, overflow)) } else { None }
        })
        .collect()
}

fn find_derivs_outside_canvas(
    derivs: &[DerivLayout],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(DerivId, f64)> {
    derivs
        .iter()
        .filter_map(|d| {
            let ab = Aabb {
                x: d.x,
                y: d.y,
                w: d.width,
                h: d.height,
            };
            let overflow = aabb_overflow(&ab, canvas_w, canvas_h);
            if overflow > 0.5 { Some((d.id, overflow)) } else { None }
        })
        .collect()
}

fn find_edges_outside_canvas(
    edges: &[(EdgeId, Vec<LineSeg>)],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(EdgeId, f64)> {
    edges
        .iter()
        .filter_map(|(eid, segs)| {
            let overflow: f64 = segs.iter().map(|s| segment_overflow(s, canvas_w, canvas_h)).sum();
            if overflow > 0.5 { Some((*eid, overflow)) } else { None }
        })
        .collect()
}

fn find_labels_outside_canvas(
    labels: &[(EdgeId, &EdgeLabel)],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(EdgeId, f64)> {
    labels
        .iter()
        .filter_map(|&(eid, label)| {
            let overflow = aabb_overflow(&Aabb::from_label(label), canvas_w, canvas_h);
            if overflow > 0.5 { Some((eid, overflow)) } else { None }
        })
        .collect()
}

fn find_arrowheads_outside_canvas(
    arrowheads: &[(EdgeId, Aabb)],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(EdgeId, f64)> {
    arrowheads
        .iter()
        .filter_map(|(eid, ab)| {
            let overflow = aabb_overflow(ab, canvas_w, canvas_h);
            if overflow > 0.5 { Some((*eid, overflow)) } else { None }
        })
        .collect()
}

fn find_stubs_outside_canvas(
    stubs: &[(EdgeId, Vec<LineSeg>)],
    canvas_w: f64,
    canvas_h: f64,
) -> Vec<(EdgeId, f64)> {
    stubs
        .iter()
        .filter_map(|(eid, segs)| {
            let overflow: f64 = segs.iter().map(|s| segment_overflow(s, canvas_w, canvas_h)).sum();
            if overflow > 0.5 { Some((*eid, overflow)) } else { None }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Domain corridor correctness
// ---------------------------------------------------------------------------

/// Find intra-domain edges whose vertical segments appear in another domain's
/// corridor zone.
fn find_intra_edges_in_wrong_corridor(
    graph: &Graph,
    domains: &[DomainLayout],
    edges: &[(EdgeId, Vec<LineSeg>)],
) -> Vec<(EdgeId, DomainId)> {
    let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
    let mut violations = Vec::new();

    struct CorridorZone {
        id: DomainId,
        left_x_start: f64,
        left_x_end: f64,
        right_x_start: f64,
        right_x_end: f64,
        y_start: f64,
        y_end: f64,
    }

    let zones: Vec<CorridorZone> = domains
        .iter()
        .map(|dl| CorridorZone {
            id: dl.id,
            left_x_start: dl.x,
            left_x_end: dl.x + lr_pad,
            right_x_start: dl.x + dl.width - lr_pad,
            right_x_end: dl.x + dl.width,
            y_start: dl.y,
            y_end: dl.y + dl.height,
        })
        .collect();

    for &(eid, ref segs) in edges {
        let (sd, td) = edge_endpoint_domains(graph, eid);
        // Only check intra-domain edges.
        if sd != td || sd.is_none() {
            continue;
        }
        let own_domain = sd.unwrap();

        for seg in segs {
            let (x, y_min, y_max) = match seg {
                LineSeg { x1, y1, x2, y2, .. } if (x1 - x2).abs() < 0.5 => {
                    (*x1, y1.min(*y2), y1.max(*y2))
                }
                _ => continue,
            };

            for zone in &zones {
                if zone.id == own_domain {
                    continue; // Own corridor is fine.
                }
                let in_left = x >= zone.left_x_start && x <= zone.left_x_end;
                let in_right = x >= zone.right_x_start && x <= zone.right_x_end;
                if !in_left && !in_right {
                    continue;
                }
                let y_overlap = y_max > zone.y_start && y_min < zone.y_end;
                if y_overlap {
                    violations.push((eid, zone.id));
                    break;
                }
            }
        }
    }
    violations
}

// ---------------------------------------------------------------------------
// Layout symmetry metrics
// ---------------------------------------------------------------------------

fn compute_visual_balance(
    nodes: &[NodeLayout],
    domains: &[DomainLayout],
    width: f64,
    height: f64,
) -> f64 {
    if width <= 0.0 || height <= 0.0 {
        return 0.0;
    }
    let canvas_cx = width / 2.0;
    let canvas_cy = height / 2.0;
    let diagonal = (width * width + height * height).sqrt();

    let mut total_area = 0.0;
    let mut weighted_cx = 0.0;
    let mut weighted_cy = 0.0;

    for n in nodes {
        let area = n.width * n.height;
        weighted_cx += (n.x + n.width / 2.0) * area;
        weighted_cy += (n.y + n.height / 2.0) * area;
        total_area += area;
    }
    for d in domains {
        let area = d.width * d.height;
        weighted_cx += (d.x + d.width / 2.0) * area;
        weighted_cy += (d.y + d.height / 2.0) * area;
        total_area += area;
    }

    if total_area <= 0.0 {
        return 0.0;
    }
    let com_x = weighted_cx / total_area;
    let com_y = weighted_cy / total_area;
    let dx = com_x - canvas_cx;
    let dy = com_y - canvas_cy;
    (dx * dx + dy * dy).sqrt() / diagonal
}

fn compute_max_column_centering_error(domains: &[DomainLayout]) -> f64 {
    let (col_centers, assignments) = cluster_domain_columns(domains);
    if col_centers.is_empty() {
        return 0.0;
    }

    let num_cols = col_centers.len();
    let mut col_domain_cxs: Vec<Vec<f64>> = vec![Vec::new(); num_cols];
    for (i, dl) in domains.iter().enumerate() {
        let col = assignments[i];
        col_domain_cxs[col].push(dl.x + dl.width / 2.0);
    }

    let mut max_error = 0.0_f64;
    for cxs in &col_domain_cxs {
        if cxs.len() < 2 {
            continue;
        }
        let mean_cx: f64 = cxs.iter().sum::<f64>() / cxs.len() as f64;
        for &cx in cxs {
            max_error = max_error.max((cx - mean_cx).abs());
        }
    }
    max_error
}

fn compute_domain_size_cv(domains: &[DomainLayout]) -> f64 {
    if domains.len() < 2 {
        return 0.0;
    }
    let areas: Vec<f64> = domains.iter().map(|d| d.width * d.height).collect();
    let mean = areas.iter().sum::<f64>() / areas.len() as f64;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance = areas.iter().map(|a| (a - mean) * (a - mean)).sum::<f64>() / areas.len() as f64;
    variance.sqrt() / mean
}

// ---------------------------------------------------------------------------
// Edge routing quality metrics
// ---------------------------------------------------------------------------

fn compute_port_side_balance(edges: &[(EdgeId, Vec<LineSeg>)]) -> f64 {
    let mut left_count = 0usize;
    let mut right_count = 0usize;
    for (_, segs) in edges {
        // First segment direction indicates port side.
        if let Some(first) = segs.first() {
            let dx = first.x2 - first.x1;
            if dx.abs() > 0.5 {
                if dx < 0.0 {
                    left_count += 1;
                } else {
                    right_count += 1;
                }
            }
        }
    }
    let max = left_count.max(right_count);
    let min = left_count.min(right_count);
    if max == 0 {
        1.0
    } else {
        min as f64 / max as f64
    }
}

fn compute_edge_length_cv(edges: &[(EdgeId, Vec<LineSeg>)]) -> f64 {
    if edges.len() < 2 {
        return 0.0;
    }
    let lengths: Vec<f64> = edges
        .iter()
        .map(|(_, segs)| segs.iter().map(|s| s.length()).sum::<f64>())
        .collect();
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance =
        lengths.iter().map(|l| (l - mean) * (l - mean)).sum::<f64>() / lengths.len() as f64;
    variance.sqrt() / mean
}

fn compute_segment_complexity_distribution(edges: &[(EdgeId, Vec<LineSeg>)]) -> [usize; 3] {
    let mut dist = [0usize; 3];
    for (_, segs) in edges {
        let n = segs.len();
        match n {
            0..=2 => dist[0] += 1,
            3 => dist[1] += 1,
            _ => dist[2] += 1,
        }
    }
    dist
}

fn compute_routing_direction_balance(edges: &[(EdgeId, Vec<LineSeg>)]) -> f64 {
    let mut rightward = 0.0_f64;
    let mut leftward = 0.0_f64;
    for (_, segs) in edges {
        for seg in segs {
            let dy = (seg.y2 - seg.y1).abs();
            if dy < 0.5 {
                // Horizontal segment.
                let dx = seg.x2 - seg.x1;
                if dx > 0.5 {
                    rightward += dx;
                } else if dx < -0.5 {
                    leftward += -dx;
                }
            }
        }
    }
    let max = rightward.max(leftward);
    let min = rightward.min(leftward);
    if max <= 0.0 {
        1.0
    } else {
        min / max
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aabb_intersection() {
        let a = Aabb {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        let b = Aabb {
            x: 50.0,
            y: 25.0,
            w: 100.0,
            h: 50.0,
        };
        assert!(a.intersects(&b));

        let c = Aabb {
            x: 200.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        assert!(!a.intersects(&c));
    }

    #[test]
    fn test_aabb_contains() {
        let outer = Aabb {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let inner = Aabb {
            x: 10.0,
            y: 10.0,
            w: 50.0,
            h: 50.0,
        };
        assert!(outer.contains(&inner));
        assert!(!inner.contains(&outer));
    }

    #[test]
    fn test_segment_intersects_aabb_horizontal() {
        let aabb = Aabb {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        // Horizontal segment crossing through the box.
        let seg = LineSeg {
            x1: 0.0,
            y1: 100.0,
            x2: 200.0,
            y2: 100.0,
        };
        assert!(segment_intersects_aabb(&seg, &aabb));

        // Horizontal segment above the box.
        let seg2 = LineSeg {
            x1: 0.0,
            y1: 10.0,
            x2: 200.0,
            y2: 10.0,
        };
        assert!(!segment_intersects_aabb(&seg2, &aabb));
    }

    #[test]
    fn test_segment_intersects_aabb_vertical() {
        let aabb = Aabb {
            x: 50.0,
            y: 50.0,
            w: 100.0,
            h: 100.0,
        };
        let seg = LineSeg {
            x1: 100.0,
            y1: 0.0,
            x2: 100.0,
            y2: 200.0,
        };
        assert!(segment_intersects_aabb(&seg, &aabb));
    }

    #[test]
    fn test_segments_cross() {
        let a = LineSeg {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 100.0,
        };
        let b = LineSeg {
            x1: 0.0,
            y1: 100.0,
            x2: 100.0,
            y2: 0.0,
        };
        assert!(segments_cross(&a, &b));

        let c = LineSeg {
            x1: 200.0,
            y1: 200.0,
            x2: 300.0,
            y2: 300.0,
        };
        assert!(!segments_cross(&a, &c));
    }

    #[test]
    fn test_parse_svg_path() {
        let segs = parse_svg_path("M10,20 L30,20 L30,50");
        assert_eq!(segs.len(), 2);
        assert!((segs[0].x1 - 10.0).abs() < 1e-6);
        assert!((segs[0].y1 - 20.0).abs() < 1e-6);
        assert!((segs[0].x2 - 30.0).abs() < 1e-6);
        assert!((segs[1].y2 - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_empty_path() {
        let segs = parse_svg_path("");
        assert!(segs.is_empty());
    }
}
