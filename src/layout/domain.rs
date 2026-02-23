//! Domain bounding box computation (DESIGN.md 4.2.5).
//!
//! For each domain, computes the axis-aligned bounding box that encloses
//! all member nodes, expanded by DOMAIN_PADDING on every side.

use crate::model::types::Graph;

use super::{DomainLayout, NodeLayout, CORRIDOR_PAD, DOMAIN_PADDING};

/// Compute bounding boxes for all domains from final node positions.
pub fn compute_domain_bounds(graph: &Graph, node_layouts: &[NodeLayout]) -> Vec<DomainLayout> {
    graph
        .domains
        .iter()
        .filter_map(|domain| {
            // Collect layouts of member nodes.
            let member_layouts: Vec<&NodeLayout> = domain
                .members
                .iter()
                .map(|nid| &node_layouts[nid.index()])
                .collect();

            if member_layouts.is_empty() {
                return None;
            }

            let min_x = member_layouts
                .iter()
                .map(|nl| nl.x)
                .fold(f64::INFINITY, f64::min);
            let min_y = member_layouts
                .iter()
                .map(|nl| nl.y)
                .fold(f64::INFINITY, f64::min);
            let max_x = member_layouts
                .iter()
                .map(|nl| nl.x + nl.width)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = member_layouts
                .iter()
                .map(|nl| nl.y + nl.height)
                .fold(f64::NEG_INFINITY, f64::max);

            // Left/right padding includes corridor space (CORRIDOR_PAD * 2 = 16px
            // for a single-channel corridor) plus DOMAIN_PADDING.
            // Top/bottom use DOMAIN_PADDING only.
            let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
            Some(DomainLayout {
                id: domain.id,
                display_name: domain.display_name.clone(),
                x: min_x - lr_pad,
                y: min_y - DOMAIN_PADDING,
                width: (max_x - min_x) + 2.0 * lr_pad,
                height: (max_y - min_y) + 2.0 * DOMAIN_PADDING,
            })
        })
        .collect()
}

/// Post-processing pass that shifts overlapping domain bounding boxes apart.
///
/// After Brandes-Köpf coordinate assignment, domain bounding boxes may overlap
/// when nodes from different domains end up at adjacent horizontal positions.
/// This pass detects overlapping domain pairs and shifts the rightward domain
/// (and its member nodes) to the right until separation is achieved.
///
/// Must be called after `compute_domain_bounds` but before edge routing so
/// that routes use the corrected node positions.
pub fn separate_domains(
    node_layouts: &mut [NodeLayout],
    domain_layouts: &mut [DomainLayout],
    graph: &Graph,
) {
    const MAX_ITERS: usize = 100;

    for _ in 0..MAX_ITERS {
        let mut any_overlap = false;

        let n = domain_layouts.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let (ax, ay, aw, ah) = (
                    domain_layouts[i].x,
                    domain_layouts[i].y,
                    domain_layouts[i].width,
                    domain_layouts[i].height,
                );
                let (bx, by, bw, bh) = (
                    domain_layouts[j].x,
                    domain_layouts[j].y,
                    domain_layouts[j].width,
                    domain_layouts[j].height,
                );

                // Skip pairs that don't overlap in both axes.
                let overlaps_x = ax < bx + bw && ax + aw > bx;
                let overlaps_y = ay < by + bh && ay + ah > by;
                if !overlaps_x || !overlaps_y {
                    continue;
                }

                any_overlap = true;

                // Shift the rightward domain (by center x) to the right.
                let a_center = ax + aw / 2.0;
                let b_center = bx + bw / 2.0;

                let (shift_idx, overlap_amount) = if a_center <= b_center {
                    // i is to the left; shift j right.
                    (j, (ax + aw) - bx)
                } else {
                    // j is to the left; shift i right.
                    (i, (bx + bw) - ax)
                };

                if overlap_amount <= 0.0 {
                    continue;
                }

                // Gap must accommodate an inter-domain corridor (CORRIDOR_PAD * 2).
                let shift = overlap_amount + CORRIDOR_PAD * 2.0;
                let domain_id = domain_layouts[shift_idx].id;

                // Move member nodes.
                if let Some(domain) = graph.domains.iter().find(|d| d.id == domain_id) {
                    for &nid in &domain.members {
                        node_layouts[nid.index()].x += shift;
                    }
                }

                // Move the domain box itself.
                domain_layouts[shift_idx].x += shift;
            }
        }

        if !any_overlap {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::*;
    use std::collections::HashMap;

    fn make_node_layout(id: u32, x: f64, y: f64, width: f64, height: f64) -> NodeLayout {
        NodeLayout {
            id: NodeId(id),
            x,
            y,
            width,
            height,
        }
    }

    fn make_graph_with_domain(node_count: usize, domain_members: &[u32]) -> Graph {
        let mut nodes = Vec::new();
        for i in 0..node_count {
            nodes.push(Node {
                id: NodeId(i as u32),
                ident: format!("node{}", i),
                display_name: None,
                properties: Vec::new(),
                domain: if domain_members.contains(&(i as u32)) {
                    Some(DomainId(0))
                } else {
                    None
                },
                is_root: i == 0,
                is_selected: false,
            });
        }

        let domains = vec![Domain {
            id: DomainId(0),
            display_name: "TestDomain".to_string(),
            members: domain_members.iter().map(|&id| NodeId(id)).collect(),
        }];

        Graph {
            nodes,
            properties: Vec::new(),
            derivations: Vec::new(),
            edges: Vec::new(),
            domains,
            prop_edges: HashMap::new(),
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
        }
    }

    // Test: Domain bounding box encloses member nodes with padding
    #[test]
    fn test_domain_bounds_single_node() {
        let graph = make_graph_with_domain(1, &[0]);
        let node_layouts = vec![make_node_layout(0, 100.0, 50.0, 80.0, 60.0)];

        let domains = compute_domain_bounds(&graph, &node_layouts);

        assert_eq!(domains.len(), 1);
        let d = &domains[0];
        assert_eq!(d.display_name, "TestDomain");

        let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
        let eps = 1e-6;
        assert!((d.x - (100.0 - lr_pad)).abs() < eps);
        assert!((d.y - (50.0 - DOMAIN_PADDING)).abs() < eps);
        assert!((d.width - (80.0 + 2.0 * lr_pad)).abs() < eps);
        assert!((d.height - (60.0 + 2.0 * DOMAIN_PADDING)).abs() < eps);
    }

    // Test: Domain bounding box encloses multiple member nodes
    #[test]
    fn test_domain_bounds_multiple_nodes() {
        let graph = make_graph_with_domain(3, &[0, 1, 2]);
        let node_layouts = vec![
            make_node_layout(0, 10.0, 20.0, 50.0, 40.0),
            make_node_layout(1, 100.0, 30.0, 60.0, 50.0),
            make_node_layout(2, 50.0, 100.0, 70.0, 30.0),
        ];

        let domains = compute_domain_bounds(&graph, &node_layouts);

        assert_eq!(domains.len(), 1);
        let d = &domains[0];

        // min_x = 10, min_y = 20
        // max_x = max(10+50, 100+60, 50+70) = max(60, 160, 120) = 160
        // max_y = max(20+40, 30+50, 100+30) = max(60, 80, 130) = 130
        let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
        let eps = 1e-6;
        assert!((d.x - (10.0 - lr_pad)).abs() < eps);
        assert!((d.y - (20.0 - DOMAIN_PADDING)).abs() < eps);
        assert!((d.width - (150.0 + 2.0 * lr_pad)).abs() < eps);
        assert!((d.height - (110.0 + 2.0 * DOMAIN_PADDING)).abs() < eps);
    }

    // Test: Only domain member nodes are included in bounds
    #[test]
    fn test_domain_bounds_excludes_non_members() {
        let graph = make_graph_with_domain(3, &[0, 2]);
        let node_layouts = vec![
            make_node_layout(0, 10.0, 20.0, 50.0, 40.0),
            make_node_layout(1, 500.0, 500.0, 60.0, 50.0), // Not in domain
            make_node_layout(2, 50.0, 100.0, 70.0, 30.0),
        ];

        let domains = compute_domain_bounds(&graph, &node_layouts);

        assert_eq!(domains.len(), 1);
        let d = &domains[0];

        // Node 1 at (500,500) should NOT affect the bounding box.
        let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
        let eps = 1e-6;
        assert!((d.x - (10.0 - lr_pad)).abs() < eps);
        assert!((d.y - (20.0 - DOMAIN_PADDING)).abs() < eps);
        // max_x = max(10+50, 50+70) = 120, so width = 120-10 + 2*lr_pad
        assert!((d.width - (110.0 + 2.0 * lr_pad)).abs() < eps);
        // max_y = max(20+40, 100+30) = 130, so height = 130-20 + 2*padding
        assert!((d.height - (110.0 + 2.0 * DOMAIN_PADDING)).abs() < eps);
    }
}
