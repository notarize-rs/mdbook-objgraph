//! Post-layout quality analysis.
//!
//! Computes measurable metrics from a `LayoutResult` to detect overlaps,
//! collisions, and other layout defects. Used as a test gate and diagnostic
//! tool during layout algorithm iteration.

use crate::model::types::{DerivId, DomainId, Edge, EdgeId, Graph, NodeId};

use super::{
    DerivLayout, DomainLayout, EdgePath, LayoutResult, NodeLayout, CORRIDOR_PAD, DOMAIN_PADDING,
    NODE_H_SPACING,
};

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// A quality report for a laid-out graph.
#[derive(Debug)]
pub struct QualityReport {
    pub node_overlaps: Vec<(NodeId, NodeId)>,
    pub domain_overlaps: Vec<(DomainId, DomainId)>,
    pub nodes_outside_domain: Vec<(NodeId, DomainId)>,
    pub node_edge_overlaps: Vec<(NodeId, EdgeId)>,
    pub edge_crossings: usize,
    pub min_node_gap: f64,
    pub aspect_ratio: f64,
    pub total_edge_length: f64,
    /// max(widths) - min(widths) across all nodes. 0.0 means uniform width.
    pub node_width_delta: f64,
    /// Maximum horizontal distance (px) between a parent's center X and the
    /// mean center X of its link-tree children. Measures tree centering quality.
    pub max_parent_misalignment: f64,
    /// Mean number of SVG path segments per intra-domain constraint edge.
    /// H-V-H bracket = 3 (good); H-V-H-V-H loop = 5 (spaghetti).
    pub mean_constraint_segments: f64,
    /// Cross-domain derivation pills that overlap a domain they don't belong to.
    pub derivs_inside_domains: Vec<(DerivId, DomainId)>,
    /// Domain-less nodes that overlap any domain rect.
    pub free_nodes_inside_domains: Vec<(NodeId, DomainId)>,
    /// Foreign nodes whose y-range falls between a domain's topmost and
    /// bottommost member nodes (domain contiguity violation).
    pub domain_contiguity_violations: Vec<(DomainId, NodeId)>,
    /// Cross-domain constraint edges whose vertical segments fall inside
    /// an intra-domain corridor (between domain border and node area).
    pub inter_domain_edges_in_intra_corridors: Vec<(EdgeId, DomainId)>,
    /// Pairs of edges that share the same vertical channel x-coordinate
    /// while their y-ranges overlap (channel collision).
    pub channel_collisions: Vec<(EdgeId, EdgeId)>,
    /// Total graph height in pixels (from LayoutResult).
    pub total_height: f64,
    /// Total graph width in pixels (from LayoutResult).
    pub total_width: f64,
    /// Per-column heights (ordered left to right). Column height is the
    /// bottom of the lowest domain minus the top of the highest domain.
    pub column_heights: Vec<f64>,
    /// Difference between tallest and shortest column heights.
    /// 0.0 means perfectly balanced columns.
    pub column_height_imbalance: f64,
}

impl QualityReport {
    /// True if there are hard errors (overlapping nodes/domains, nodes outside domains,
    /// elements inside foreign domains, contiguity violations, corridor violations).
    pub fn has_errors(&self) -> bool {
        !self.node_overlaps.is_empty()
            || !self.domain_overlaps.is_empty()
            || !self.nodes_outside_domain.is_empty()
            || !self.derivs_inside_domains.is_empty()
            || !self.free_nodes_inside_domains.is_empty()
            || !self.domain_contiguity_violations.is_empty()
            || !self.inter_domain_edges_in_intra_corridors.is_empty()
            || !self.channel_collisions.is_empty()
    }

    /// True if there are warnings (node-edge overlaps, tight spacing).
    pub fn has_warnings(&self) -> bool {
        !self.node_edge_overlaps.is_empty() || self.min_node_gap < NODE_H_SPACING - 1.0
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Quality Report: {} errors, {} warnings",
            self.error_count(),
            self.warning_count()
        ));
        lines.push(format!(
            "  Node-node overlaps:    {}",
            self.node_overlaps.len()
        ));
        lines.push(format!(
            "  Domain overlaps:       {}",
            self.domain_overlaps.len()
        ));
        lines.push(format!(
            "  Nodes outside domain:  {}",
            self.nodes_outside_domain.len()
        ));
        lines.push(format!(
            "  Node-edge overlaps:    {}",
            self.node_edge_overlaps.len()
        ));
        lines.push(format!(
            "  Edge-edge crossings:   {}",
            self.edge_crossings
        ));
        lines.push(format!(
            "  Min node gap:          {:.1}px (threshold: {:.1}px)",
            self.min_node_gap, NODE_H_SPACING
        ));
        lines.push(format!("  Aspect ratio:          {:.2}", self.aspect_ratio));
        lines.push(format!(
            "  Total edge length:     {:.0}px",
            self.total_edge_length
        ));
        lines.push(format!(
            "  Node width delta:      {:.1}px (0 = uniform)",
            self.node_width_delta
        ));
        lines.push(format!(
            "  Max parent misalign:   {:.1}px (0 = perfect tree centering)",
            self.max_parent_misalignment
        ));
        lines.push(format!(
            "  Mean constraint segs:  {:.1} (3 = H-V-H bracket, 5 = spaghetti)",
            self.mean_constraint_segments
        ));
        lines.push(format!(
            "  Total dimensions:      {:.0}w x {:.0}h px",
            self.total_width, self.total_height
        ));
        lines.push(format!(
            "  Column heights:        {:?}",
            self.column_heights
                .iter()
                .map(|h| format!("{:.0}", h))
                .collect::<Vec<_>>()
        ));
        lines.push(format!(
            "  Column height imbal:   {:.0}px (0 = balanced)",
            self.column_height_imbalance
        ));

        for &(a, b) in &self.node_overlaps {
            lines.push(format!("  ERROR: Node {} overlaps Node {}", a.0, b.0));
        }
        for &(a, b) in &self.domain_overlaps {
            lines.push(format!("  ERROR: Domain {} overlaps Domain {}", a.0, b.0));
        }
        for &(n, d) in &self.nodes_outside_domain {
            lines.push(format!(
                "  ERROR: Node {} is outside its domain {}",
                n.0, d.0
            ));
        }
        for &(d, dom) in &self.derivs_inside_domains {
            lines.push(format!(
                "  ERROR: Derivation {} inside foreign domain {}",
                d.0, dom.0
            ));
        }
        for &(n, dom) in &self.free_nodes_inside_domains {
            lines.push(format!(
                "  ERROR: Free node {} inside domain {}",
                n.0, dom.0
            ));
        }
        for &(dom, n) in &self.domain_contiguity_violations {
            lines.push(format!(
                "  ERROR: Domain {} contiguity violated by foreign node {}",
                dom.0, n.0
            ));
        }
        for &(eid, did) in &self.inter_domain_edges_in_intra_corridors {
            lines.push(format!(
                "  ERROR: Inter-domain edge {} routes through intra-domain corridor of domain {}",
                eid.0, did.0
            ));
        }
        for &(a, b) in &self.channel_collisions {
            lines.push(format!(
                "  ERROR: Channel collision between edge {} and edge {}",
                a.0, b.0
            ));
        }

        lines.join("\n")
    }

    fn error_count(&self) -> usize {
        self.node_overlaps.len()
            + self.domain_overlaps.len()
            + self.nodes_outside_domain.len()
            + self.derivs_inside_domains.len()
            + self.free_nodes_inside_domains.len()
            + self.domain_contiguity_violations.len()
            + self.inter_domain_edges_in_intra_corridors.len()
            + self.channel_collisions.len()
    }

    fn warning_count(&self) -> usize {
        self.node_edge_overlaps.len()
            + if self.min_node_gap < NODE_H_SPACING - 1.0 {
                1
            } else {
                0
            }
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

    let parsed: Vec<(EdgeId, Vec<LineSeg>)> = all_paths
        .iter()
        .map(|ep| (ep.edge_id, parse_svg_path(&ep.svg_path)))
        .collect();

    let node_edge_overlaps = find_node_edge_overlaps(graph, &layout.nodes, &parsed);
    let edge_crossings = count_edge_crossings(&parsed);
    let total_edge_length = parsed
        .iter()
        .flat_map(|(_, segs)| segs.iter())
        .map(|s| s.length())
        .sum();

    let node_width_delta = compute_node_width_delta(&layout.nodes);
    let max_parent_misalignment = compute_max_parent_misalignment(graph, &layout.nodes);
    let mean_constraint_segments = compute_mean_constraint_segments(&layout.intra_domain_constraints);
    let derivs_inside_domains =
        find_derivs_inside_domains(graph, &layout.derivations, &layout.domains);
    let free_nodes_inside_domains =
        find_free_nodes_inside_domains(graph, &layout.nodes, &layout.domains);
    let domain_contiguity_violations =
        find_domain_contiguity_violations(graph, &layout.nodes, &layout.domains);
    let inter_domain_edges_in_intra_corridors =
        find_inter_domain_edges_in_intra_corridors(graph, &layout.domains, &parsed);
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
        channel_collisions,
        total_height,
        total_width,
        column_heights,
        column_height_imbalance,
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

fn count_edge_crossings(edges: &[(EdgeId, Vec<LineSeg>)]) -> usize {
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

/// Compute per-column heights from domain layouts.
///
/// Clusters domains into columns by x-center (within 100px tolerance),
/// then computes each column's height as the span from the topmost domain
/// to the bottommost domain within that column.
fn compute_column_heights(domains: &[DomainLayout]) -> Vec<f64> {
    if domains.is_empty() {
        return Vec::new();
    }

    // Cluster domains by x-center.
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

    let num_cols = col_centers.len().max(1);
    let mut col_min_y = vec![f64::INFINITY; num_cols];
    let mut col_max_y = vec![f64::NEG_INFINITY; num_cols];

    for dl in domains {
        let cx = dl.x + dl.width / 2.0;
        let col = assign_column(cx);
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
