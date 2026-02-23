//! Post-layout quality analysis.
//!
//! Computes measurable metrics from a `LayoutResult` to detect overlaps,
//! collisions, and other layout defects. Used as a test gate and diagnostic
//! tool during layout algorithm iteration.

use crate::model::types::{DomainId, Edge, EdgeId, Graph, NodeId};

use super::{DomainLayout, EdgePath, LayoutResult, NodeLayout, NODE_H_SPACING};

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
}

impl QualityReport {
    /// True if there are hard errors (overlapping nodes/domains, nodes outside domains).
    pub fn has_errors(&self) -> bool {
        !self.node_overlaps.is_empty()
            || !self.domain_overlaps.is_empty()
            || !self.nodes_outside_domain.is_empty()
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

        lines.join("\n")
    }

    fn error_count(&self) -> usize {
        self.node_overlaps.len()
            + self.domain_overlaps.len()
            + self.nodes_outside_domain.len()
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
