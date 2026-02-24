//! Domain bounding box computation (DESIGN.md 4.2.5).
//!
//! For each domain, computes the axis-aligned bounding box that encloses
//! all member nodes, expanded by DOMAIN_PADDING on every side.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::types::{DerivId, DomainId, Edge, Graph, NodeId};

use super::{
    DerivLayout, DomainLayout, NodeLayout, CHANNEL_GAP, CORRIDOR_PAD, DOMAIN_PADDING,
    DOMAIN_TITLE_HEIGHT, INTER_NODE_GAP,
};

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

            // Left/right padding is the corridor space (CORRIDOR_PAD * 2 = 16px
            // for a single-channel corridor) plus any extra DOMAIN_PADDING (0px).
            // Top: DOMAIN_TITLE_HEIGHT (32px) for title area (12+8+12).
            // Bottom: INTER_NODE_GAP (28px) matching inter-node spacing.
            let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;
            Some(DomainLayout {
                id: domain.id,
                display_name: domain.display_name.clone(),
                x: min_x - lr_pad,
                y: min_y - DOMAIN_TITLE_HEIGHT,
                width: (max_x - min_x) + 2.0 * lr_pad,
                height: (max_y - min_y) + DOMAIN_TITLE_HEIGHT + INTER_NODE_GAP,
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

/// Columnar domain layout: assigns domains to columns with dedicated gap
/// corridors between them for cross-domain edge routing.
///
/// Replaces the iterative overlap-push-apart algorithm with a structured
/// two-column layout based on cross-domain edge topology. The hub domain
/// (most cross-domain connections) anchors column 0; its direct neighbors
/// go to column 1; satellite domains (degree 1) join their neighbor's column.
///
/// The gap corridor between columns is dynamically sized based on the number
/// of cross-column edges.
pub fn columnar_layout(
    node_layouts: &mut [NodeLayout],
    domain_layouts: &mut [DomainLayout],
    graph: &Graph,
) {
    if domain_layouts.is_empty() {
        return;
    }

    // Step 1: Build cross-domain adjacency graph.
    let adj = build_cross_domain_adjacency(graph, domain_layouts);

    // Step 2: Assign domains to columns (anchor-aware).
    let columns = assign_columns(&adj, domain_layouts, graph);

    // Step 3: Count max simultaneous cross-column channels for gap sizing.
    let cross_col_counts =
        count_cross_domain_channels(&columns, domain_layouts, node_layouts, graph);

    // Step 4: Compute column geometry (with dynamic gap widths).
    let (col_widths, gap_widths) =
        compute_column_widths(&columns, node_layouts, domain_layouts, graph, &cross_col_counts);

    // Step 5: Reposition nodes into columns.
    reposition_to_columns(
        &columns,
        &col_widths,
        &gap_widths,
        node_layouts,
        domain_layouts,
        graph,
    );
}

/// Build an adjacency list of domain layout indices based on cross-domain edges.
fn build_cross_domain_adjacency(
    graph: &Graph,
    domain_layouts: &[DomainLayout],
) -> HashMap<usize, HashSet<usize>> {
    let mut adj: HashMap<usize, HashSet<usize>> = HashMap::new();

    // Initialize all domains with empty neighbor sets.
    for i in 0..domain_layouts.len() {
        adj.entry(i).or_default();
    }

    // Helper: find domain layout index for a DomainId.
    let domain_idx = |did: DomainId| -> Option<usize> {
        domain_layouts.iter().position(|d| d.id == did)
    };

    for edge in &graph.edges {
        let (src_domain, tgt_domain) = match edge {
            Edge::Anchor { parent, child, .. } => (
                graph.nodes[parent.index()].domain,
                graph.nodes[child.index()].domain,
            ),
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => (
                graph.nodes[graph.properties[source_prop.index()].node.index()].domain,
                graph.nodes[graph.properties[dest_prop.index()].node.index()].domain,
            ),
            Edge::DerivInput {
                source_prop,
                target_deriv,
            } => {
                let src_node = graph.properties[source_prop.index()].node;
                let deriv = &graph.derivations[target_deriv.index()];
                let tgt_node = graph.properties[deriv.output_prop.index()].node;
                (
                    graph.nodes[src_node.index()].domain,
                    graph.nodes[tgt_node.index()].domain,
                )
            }
        };

        if let (Some(sd), Some(td)) = (src_domain, tgt_domain) {
            if sd != td {
                let (si, ti) = (domain_idx(sd), domain_idx(td));
                if let (Some(si), Some(ti)) = (si, ti) {
                    adj.entry(si).or_default().insert(ti);
                    adj.entry(ti).or_default().insert(si);
                }
            }
        }
    }

    adj
}

/// Build cross-domain anchor relationships: maps domain layout indices that
/// are connected by anchor edges (parent domain -> child domain).
///
/// Returns a set of directed pairs (parent_dl_idx, child_dl_idx).
fn build_anchor_domain_pairs(
    graph: &Graph,
    domain_layouts: &[DomainLayout],
) -> HashSet<(usize, usize)> {
    let mut pairs = HashSet::new();

    let domain_idx = |did: DomainId| -> Option<usize> {
        domain_layouts.iter().position(|d| d.id == did)
    };

    for edge in &graph.edges {
        if let Edge::Anchor { parent, child, .. } = edge {
            let parent_domain = graph.nodes[parent.index()].domain;
            let child_domain = graph.nodes[child.index()].domain;
            if let (Some(pd), Some(cd)) = (parent_domain, child_domain) {
                if pd != cd {
                    if let (Some(pi), Some(ci)) = (domain_idx(pd), domain_idx(cd)) {
                        pairs.insert((pi, ci));
                    }
                }
            }
        }
    }
    pairs
}

/// Union-Find for merging domains into anchor groups.
fn find_root(parent: &mut Vec<usize>, x: usize) -> usize {
    if parent[x] != x {
        parent[x] = find_root(parent, parent[x]);
    }
    parent[x]
}

fn union(parent: &mut Vec<usize>, rank: &mut Vec<usize>, a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra == rb {
        return;
    }
    if rank[ra] < rank[rb] {
        parent[ra] = rb;
    } else if rank[ra] > rank[rb] {
        parent[rb] = ra;
    } else {
        parent[rb] = ra;
        rank[ra] += 1;
    }
}

/// Assign domains to columns, respecting anchor relationships.
///
/// Anchor edges represent vertical parent-child hierarchy. When domain A has
/// an anchor edge to domain B, domain B should be in the same column as
/// domain A (vertically below it), not in a separate column.
///
/// Algorithm:
/// 1. Build anchor groups: domains connected by cross-domain anchor edges
///    are merged into the same group (using Union-Find).
/// 2. Treat each anchor group as a single "super-domain" for column assignment.
/// 3. Assign super-domains to columns using BFS from the hub, alternating
///    columns based on non-anchor cross-domain adjacency.
/// 4. All domains in the same anchor group get the same column.
fn assign_columns(
    adj: &HashMap<usize, HashSet<usize>>,
    domain_layouts: &[DomainLayout],
    graph: &Graph,
) -> Vec<Vec<usize>> {
    let n = domain_layouts.len();
    if n == 0 {
        return Vec::new();
    }

    // Step 1: Build anchor groups using Union-Find.
    let anchor_pairs = build_anchor_domain_pairs(graph, domain_layouts);

    let mut uf_parent: Vec<usize> = (0..n).collect();
    let mut uf_rank: Vec<usize> = vec![0; n];

    for &(pi, ci) in &anchor_pairs {
        union(&mut uf_parent, &mut uf_rank, pi, ci);
    }

    // Build group mapping: root -> Vec<domain_idx>.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find_root(&mut uf_parent, i);
        groups.entry(root).or_default().push(i);
    }
    let group_roots: Vec<usize> = {
        let mut roots: Vec<usize> = groups.keys().copied().collect();
        roots.sort();
        roots
    };
    let num_groups = group_roots.len();

    // Step 2: Build group-level adjacency (non-anchor cross-domain edges).
    // Two groups are adjacent if any domain in one group has a non-anchor
    // cross-domain edge to any domain in the other group.
    // Build a stable group_of lookup.
    let domain_to_group: Vec<usize> = (0..n).map(|i| {
        let root = find_root(&mut uf_parent, i);
        group_roots.iter().position(|&r| r == root).unwrap()
    }).collect();

    let mut group_adj: HashMap<usize, HashSet<usize>> = HashMap::new();
    for gi in 0..num_groups {
        group_adj.entry(gi).or_default();
    }

    // For group-level adjacency, use only non-anchor cross-domain edges.
    for (di, neighbors) in adj {
        let gi = domain_to_group[*di];
        for &dj in neighbors {
            let gj = domain_to_group[dj];
            if gi != gj {
                group_adj.entry(gi).or_default().insert(gj);
                group_adj.entry(gj).or_default().insert(gi);
            }
        }
    }

    // Step 3: Assign groups to columns using BFS.
    let mut group_col: Vec<Option<usize>> = vec![None; num_groups];

    // Find hub group (highest degree in group-level cross-domain graph).
    let hub_group = (0..num_groups)
        .max_by_key(|&gi| group_adj.get(&gi).map_or(0, |s| s.len()))
        .unwrap_or(0);

    // If hub has no cross-domain edges, put everything in column 0.
    if group_adj.get(&hub_group).map_or(0, |s| s.len()) == 0 {
        return vec![(0..n).collect()];
    }

    // Identify satellite groups (degree 1 in group-level graph).
    let satellite_groups: HashSet<usize> = (0..num_groups)
        .filter(|&gi| group_adj.get(&gi).map_or(0, |s| s.len()) == 1)
        .collect();

    // BFS from hub group, alternating columns. Skip satellites during BFS.
    group_col[hub_group] = Some(0);
    let mut queue = VecDeque::new();
    queue.push_back(hub_group);

    while let Some(curr) = queue.pop_front() {
        let curr_col = group_col[curr].unwrap();
        let next_col = 1 - curr_col; // alternate: 0->1, 1->0

        if let Some(neighbors) = group_adj.get(&curr) {
            for &nbr in neighbors {
                if group_col[nbr].is_none() && !satellite_groups.contains(&nbr) {
                    group_col[nbr] = Some(next_col);
                    queue.push_back(nbr);
                }
            }
        }
    }

    // Assign satellite groups to the same column as their sole neighbor.
    for &sat in &satellite_groups {
        if group_col[sat].is_none() {
            if let Some(neighbors) = group_adj.get(&sat) {
                if let Some(&nbr) = neighbors.iter().next() {
                    group_col[sat] = group_col[nbr].or(Some(0));
                }
            }
        }
    }

    // Unconnected groups go to column 0.
    for gc in group_col.iter_mut() {
        if gc.is_none() {
            *gc = Some(0);
        }
    }

    // Step 4: Map group column assignments back to individual domains.
    let mut col_assignment: Vec<Option<usize>> = vec![None; n];
    for (gi, &root) in group_roots.iter().enumerate() {
        let col = group_col[gi].unwrap();
        for &di in &groups[&root] {
            col_assignment[di] = Some(col);
        }
    }

    // Build column lists.
    let max_col = col_assignment.iter().filter_map(|c| *c).max().unwrap_or(0);
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); max_col + 1];
    for (i, col) in col_assignment.iter().enumerate() {
        columns[col.unwrap()].push(i);
    }

    columns
}

/// Count the maximum simultaneous channels needed in each inter-column gap.
///
/// Uses a sweep-line approach: for each cross-domain edge (regardless of
/// whether the endpoints are in the same or different columns), compute the
/// vertical extent (min/max y of source and target nodes). The router sends
/// ALL cross-domain edges through the inter-domain gap corridor, so we must
/// count all of them — not just cross-column edges.
///
/// Two edges can share a channel if their vertical extents don't overlap.
/// The maximum number of edges whose extents overlap at any single
/// y-coordinate gives the required channel count.
fn count_cross_domain_channels(
    columns: &[Vec<usize>],
    domain_layouts: &[DomainLayout],
    node_layouts: &[NodeLayout],
    graph: &Graph,
) -> Vec<usize> {
    if columns.len() < 2 {
        return Vec::new();
    }

    // Build domain_id → column index mapping (for gap assignment).
    let mut domain_to_col: HashMap<DomainId, usize> = HashMap::new();
    for (col_idx, col_domains) in columns.iter().enumerate() {
        for &dl_idx in col_domains {
            domain_to_col.insert(domain_layouts[dl_idx].id, col_idx);
        }
    }

    // Collect vertical extents of ALL cross-domain edges per gap.
    // Cross-domain edges between columns use the gap between those columns.
    // Cross-domain edges within the same column also route through the
    // nearest gap (the router sends them to corridor with domain_id=None).
    let num_gaps = columns.len() - 1;
    let mut gap_extents: Vec<Vec<(f64, f64)>> = vec![Vec::new(); num_gaps];

    for edge in &graph.edges {
        let (src_node_id, tgt_node_id, src_domain, tgt_domain) = match edge {
            Edge::Anchor { parent, child, .. } => (
                *parent,
                *child,
                graph.nodes[parent.index()].domain,
                graph.nodes[child.index()].domain,
            ),
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => {
                let sn = graph.properties[source_prop.index()].node;
                let tn = graph.properties[dest_prop.index()].node;
                (sn, tn, graph.nodes[sn.index()].domain, graph.nodes[tn.index()].domain)
            }
            Edge::DerivInput {
                source_prop,
                target_deriv,
            } => {
                let sn = graph.properties[source_prop.index()].node;
                let deriv = &graph.derivations[target_deriv.index()];
                let tn = graph.properties[deriv.output_prop.index()].node;
                (sn, tn, graph.nodes[sn.index()].domain, graph.nodes[tn.index()].domain)
            }
        };

        // Only cross-domain edges.
        let is_cross_domain = match (src_domain, tgt_domain) {
            (Some(sd), Some(td)) => sd != td,
            _ => true, // one or both domain-less → cross-domain
        };
        if !is_cross_domain {
            continue;
        }

        // Determine which gap this edge routes through.
        let gap_idx = match (src_domain, tgt_domain) {
            (Some(sd), Some(td)) => {
                let sc = domain_to_col.get(&sd).copied();
                let tc = domain_to_col.get(&td).copied();
                match (sc, tc) {
                    (Some(s), Some(t)) if s != t => s.min(t),
                    // Same column or unknown: assign to the nearest gap.
                    // For same-column cross-domain edges, the router picks
                    // the nearest inter-domain corridor, which is gap 0 in
                    // a 2-column layout. Use the source column's left gap.
                    (Some(s), _) => s.saturating_sub(1).min(num_gaps - 1),
                    (_, Some(t)) => t.saturating_sub(1).min(num_gaps - 1),
                    _ => 0,
                }
            }
            _ => 0, // Domain-less endpoints → gap 0.
        };

        let src_nl = &node_layouts[src_node_id.index()];
        let tgt_nl = &node_layouts[tgt_node_id.index()];
        let y_min = src_nl.y.min(tgt_nl.y);
        let y_max = (src_nl.y + src_nl.height).max(tgt_nl.y + tgt_nl.height);
        gap_extents[gap_idx].push((y_min, y_max));
    }

    // Sweep-line: for each gap, find the maximum number of overlapping extents.
    gap_extents
        .iter()
        .map(|extents| {
            if extents.is_empty() {
                return 0;
            }
            // Collect events: +1 at y_min, -1 at y_max.
            let mut events: Vec<(f64, i32)> = Vec::with_capacity(extents.len() * 2);
            for &(y_min, y_max) in extents {
                events.push((y_min, 1));
                events.push((y_max, -1));
            }
            events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));

            let mut max_concurrent = 0usize;
            let mut current = 0i32;
            for (_, delta) in events {
                current += delta;
                max_concurrent = max_concurrent.max(current as usize);
            }
            max_concurrent
        })
        .collect()
}

/// Compute column widths and gap widths.
///
/// Column width = max domain natural width across all domains in the column.
/// Domain natural width = max(member node widths) + 2 * lr_pad.
/// Gap width = CORRIDOR_PAD * 2 + max(0, n_edges - 1) * CHANNEL_GAP.
fn compute_column_widths(
    columns: &[Vec<usize>],
    node_layouts: &[NodeLayout],
    domain_layouts: &[DomainLayout],
    graph: &Graph,
    cross_col_counts: &[usize],
) -> (Vec<f64>, Vec<f64>) {
    let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;

    let col_widths: Vec<f64> = columns
        .iter()
        .map(|col_domains| {
            col_domains
                .iter()
                .map(|&dl_idx| {
                    let did = domain_layouts[dl_idx].id;
                    let domain = graph.domains.iter().find(|d| d.id == did).unwrap();
                    let max_node_width = domain
                        .members
                        .iter()
                        .map(|&nid| node_layouts[nid.index()].width)
                        .fold(0.0_f64, f64::max);
                    max_node_width + 2.0 * lr_pad
                })
                .fold(0.0_f64, f64::max)
        })
        .collect();

    let gap_widths: Vec<f64> = cross_col_counts
        .iter()
        .map(|&n| {
            let n = n.max(1); // at least 1 channel
            CORRIDOR_PAD * 2.0 + (n as f64 - 1.0).max(0.0) * CHANNEL_GAP
        })
        .collect();

    (col_widths, gap_widths)
}

/// Reposition all domain member nodes into their assigned columns.
fn reposition_to_columns(
    columns: &[Vec<usize>],
    col_widths: &[f64],
    gap_widths: &[f64],
    node_layouts: &mut [NodeLayout],
    domain_layouts: &mut [DomainLayout],
    graph: &Graph,
) {
    let lr_pad = DOMAIN_PADDING + CORRIDOR_PAD * 2.0;

    // Compute column x-starts.
    // Outer corridor on the left edge for edges exiting leftward.
    let outer_corridor = CORRIDOR_PAD * 2.0;

    let mut col_x: Vec<f64> = Vec::with_capacity(columns.len());
    for i in 0..columns.len() {
        if i == 0 {
            col_x.push(outer_corridor);
        } else {
            col_x.push(col_x[i - 1] + col_widths[i - 1] + gap_widths[i - 1]);
        }
    }

    // For each domain in each column, shift member nodes.
    for (col_idx, col_domains) in columns.iter().enumerate() {
        for &dl_idx in col_domains {
            let did = domain_layouts[dl_idx].id;
            let domain = graph.domains.iter().find(|d| d.id == did).unwrap();

            if domain.members.is_empty() {
                continue;
            }

            // Current min-x of member nodes.
            let current_min_x = domain
                .members
                .iter()
                .map(|&nid| node_layouts[nid.index()].x)
                .fold(f64::INFINITY, f64::min);

            // Domain natural width (max node width).
            let max_node_width = domain
                .members
                .iter()
                .map(|&nid| node_layouts[nid.index()].width)
                .fold(0.0_f64, f64::max);
            let domain_natural_width = max_node_width + 2.0 * lr_pad;

            // Centering offset within the column.
            let centering_offset = (col_widths[col_idx] - domain_natural_width).max(0.0) / 2.0;

            // Target min-x for nodes inside this domain.
            let target_min_x = col_x[col_idx] + lr_pad + centering_offset;
            let delta_x = target_min_x - current_min_x;

            // Shift all member nodes.
            for &nid in &domain.members {
                node_layouts[nid.index()].x += delta_x;
            }
        }
    }

    // Recompute domain bounding boxes from new node positions.
    let new_bounds = compute_domain_bounds(graph, node_layouts);
    for dl in domain_layouts.iter_mut() {
        if let Some(nb) = new_bounds.iter().find(|b| b.id == dl.id) {
            dl.x = nb.x;
            dl.y = nb.y;
            dl.width = nb.width;
            dl.height = nb.height;
        }
    }
}

/// A vertical element in a column: either a domain block, a free node, or a
/// cross-domain derivation.
#[derive(Debug, Clone, Copy)]
enum ColumnElement {
    Domain(usize), // index into domain_layouts
    FreeNode(NodeId),
    CrossDomainDeriv(DerivId),
}

/// Compact vertical separation for all column elements.
///
/// After compound layer assignment + Brandes-Köpf, nodes are correctly ordered
/// but the vertical spacing is inflated by empty gap layers. This pass walks
/// each column top-to-bottom and places elements (domains, free nodes,
/// cross-domain derivations) with tight `INTER_NODE_GAP` spacing.
///
/// Replaces the old `separate_domains_vertically` + `reposition_cross_domain_derivations`.
pub fn separate_column_elements_vertically(
    node_layouts: &mut [NodeLayout],
    deriv_layouts: &mut [DerivLayout],
    domain_layouts: &mut [DomainLayout],
    graph: &Graph,
) {
    if domain_layouts.is_empty() && deriv_layouts.is_empty() {
        return;
    }

    // Step 1: Identify which column each domain belongs to by x-center.
    // Use a simple clustering: elements within the same column share similar x.
    // Collect column x-centers from domain_layouts.
    let mut col_centers: Vec<f64> = Vec::new();
    for dl in domain_layouts.iter() {
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

    // Step 2: Build per-column element lists.
    let num_cols = col_centers.len().max(1);
    let mut columns: Vec<Vec<(ColumnElement, f64)>> = vec![Vec::new(); num_cols];

    // Add domains.
    for (dl_idx, dl) in domain_layouts.iter().enumerate() {
        let cx = dl.x + dl.width / 2.0;
        let col = assign_column(cx);
        let y_center = dl.y + dl.height / 2.0;
        columns[col].push((ColumnElement::Domain(dl_idx), y_center));
    }

    // Add free nodes (domain-less).
    for node in &graph.nodes {
        if node.domain.is_none() {
            let nl = &node_layouts[node.id.index()];
            let cx = nl.x + nl.width / 2.0;
            let col = assign_column(cx);
            let y_center = nl.y + nl.height / 2.0;
            columns[col].push((ColumnElement::FreeNode(node.id), y_center));
        }
    }

    // Add cross-domain derivations.
    for deriv in &graph.derivations {
        let input_domains: Vec<Option<DomainId>> = deriv
            .inputs
            .iter()
            .map(|&pid| graph.nodes[graph.properties[pid.index()].node.index()].domain)
            .collect();
        let output_domain =
            graph.nodes[graph.properties[deriv.output_prop.index()].node.index()].domain;

        let all_same_domain = {
            let mut all_doms: Vec<Option<DomainId>> = input_domains.clone();
            all_doms.push(output_domain);
            all_doms.iter().all(|d| *d == all_doms[0]) && all_doms[0].is_some()
        };

        if !all_same_domain {
            let dl = &deriv_layouts[deriv.id.index()];
            let cx = dl.x + dl.width / 2.0;
            let col = assign_column(cx);
            let y_center = dl.y + dl.height / 2.0;
            columns[col].push((ColumnElement::CrossDomainDeriv(deriv.id), y_center));
        }
    }

    // Step 3: Sort each column by current y-center.
    for col in &mut columns {
        col.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    }

    // Step 4: Walk top-to-bottom, placing elements with INTER_NODE_GAP between
    // them.  Every column starts at y = 0 so that no column has wasted vertical
    // space above its first element.
    let gap = INTER_NODE_GAP;

    for col in &columns {
        if col.is_empty() {
            continue;
        }

        let mut cursor: f64 = 0.0;

        for (i, &(elem, _)) in col.iter().enumerate() {
            // First element starts at y=0; subsequent elements start after a gap.
            let target_top = if i == 0 { 0.0 } else { cursor + gap };

            match elem {
                ColumnElement::Domain(dl_idx) => {
                    let current_top = domain_layouts[dl_idx].y;
                    let shift = target_top - current_top;
                    if shift.abs() > 0.01 {
                        // Shift domain box and all member nodes.
                        let did = domain_layouts[dl_idx].id;
                        if let Some(domain) = graph.domains.iter().find(|d| d.id == did) {
                            for &nid in &domain.members {
                                node_layouts[nid.index()].y += shift;
                            }
                        }
                        domain_layouts[dl_idx].y += shift;
                    }
                    cursor = domain_layouts[dl_idx].y + domain_layouts[dl_idx].height;
                }
                ColumnElement::FreeNode(nid) => {
                    let nl = &mut node_layouts[nid.index()];
                    nl.y = target_top;
                    cursor = nl.y + nl.height;
                }
                ColumnElement::CrossDomainDeriv(did) => {
                    let dl = &mut deriv_layouts[did.index()];
                    dl.y = target_top;
                    cursor = dl.y + dl.height;
                }
            }
        }
    }

    // Step 5: Recompute domain bounds from shifted node positions.
    let new_bounds = compute_domain_bounds(graph, node_layouts);
    for dl in domain_layouts.iter_mut() {
        if let Some(nb) = new_bounds.iter().find(|b| b.id == dl.id) {
            dl.x = nb.x;
            dl.y = nb.y;
            dl.width = nb.width;
            dl.height = nb.height;
        }
    }
}

/// Vertical separation pass for domains (legacy).
///
/// Two concerns:
/// 1. **Cross-domain anchor hierarchy**: When a node in domain A anchors a node
///    in domain B, domain B must appear below domain A.
/// 2. **General vertical overlap**: Domains whose x-ranges overlap (same column)
///    must not overlap vertically.
///
/// Must be called after columnar layout (horizontal) so that domain boxes
/// are already horizontally positioned.
pub fn separate_domains_vertically(
    node_layouts: &mut [NodeLayout],
    domain_layouts: &mut [DomainLayout],
    graph: &Graph,
) {
    // Build cross-domain anchor relationships: (above_idx, below_idx) in domain_layouts.
    let mut anchor_order: HashSet<(usize, usize)> = HashSet::new();

    for edge in &graph.edges {
        if let Edge::Anchor { parent, child, .. } = edge {
            let parent_domain = graph.nodes[parent.index()].domain;
            let child_domain = graph.nodes[child.index()].domain;
            if let (Some(pd), Some(cd)) = (parent_domain, child_domain) {
                if pd != cd {
                    let positions = domain_layouts
                        .iter()
                        .position(|d| d.id == pd)
                        .zip(domain_layouts.iter().position(|d| d.id == cd));
                    if let Some((pi, ci)) = positions {
                        anchor_order.insert((pi, ci));
                    }
                }
            }
        }
    }

    let required_gap = CORRIDOR_PAD * 2.0;

    // Helper to shift a domain (and its member nodes) downward.
    let shift_domain_down = |shift: f64,
                              dl_idx: usize,
                              node_layouts: &mut [NodeLayout],
                              domain_layouts: &mut [DomainLayout]| {
        let domain_id = domain_layouts[dl_idx].id;
        if let Some(domain) = graph.domains.iter().find(|d| d.id == domain_id) {
            for &nid in &domain.members {
                node_layouts[nid.index()].y += shift;
            }
        }
        domain_layouts[dl_idx].y += shift;
    };

    const MAX_ITERS: usize = 100;
    for _ in 0..MAX_ITERS {
        let mut any_shift = false;
        let n = domain_layouts.len();

        for i in 0..n {
            for j in (i + 1)..n {
                // Check if domains overlap in x (same column or adjacent).
                let overlaps_x = domain_layouts[i].x < domain_layouts[j].x + domain_layouts[j].width
                    && domain_layouts[i].x + domain_layouts[i].width > domain_layouts[j].x;
                if !overlaps_x {
                    continue;
                }

                // Check if they overlap in y.
                let overlaps_y = domain_layouts[i].y
                    < domain_layouts[j].y + domain_layouts[j].height + required_gap
                    && domain_layouts[i].y + domain_layouts[i].height + required_gap
                        > domain_layouts[j].y;
                if !overlaps_y {
                    continue;
                }

                // Determine which goes above. Use anchor order if available,
                // otherwise use current y-center as tiebreaker.
                let (above, below) = if anchor_order.contains(&(i, j)) {
                    (i, j)
                } else if anchor_order.contains(&(j, i)) {
                    (j, i)
                } else {
                    // No anchor relationship — put the one with smaller y above.
                    let ci = domain_layouts[i].y + domain_layouts[i].height / 2.0;
                    let cj = domain_layouts[j].y + domain_layouts[j].height / 2.0;
                    if ci <= cj {
                        (i, j)
                    } else {
                        (j, i)
                    }
                };

                let above_bottom =
                    domain_layouts[above].y + domain_layouts[above].height;
                let below_top = domain_layouts[below].y;

                if below_top < above_bottom + required_gap {
                    let shift = (above_bottom + required_gap) - below_top;
                    shift_domain_down(shift, below, node_layouts, domain_layouts);
                    any_shift = true;
                }
            }
        }

        if !any_shift {
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
                is_anchored: i == 0,
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
        assert!((d.y - (50.0 - DOMAIN_TITLE_HEIGHT)).abs() < eps);
        assert!((d.width - (80.0 + 2.0 * lr_pad)).abs() < eps);
        assert!((d.height - (60.0 + DOMAIN_TITLE_HEIGHT + INTER_NODE_GAP)).abs() < eps);
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
        assert!((d.y - (20.0 - DOMAIN_TITLE_HEIGHT)).abs() < eps);
        assert!((d.width - (150.0 + 2.0 * lr_pad)).abs() < eps);
        assert!((d.height - (110.0 + DOMAIN_TITLE_HEIGHT + INTER_NODE_GAP)).abs() < eps);
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
        assert!((d.y - (20.0 - DOMAIN_TITLE_HEIGHT)).abs() < eps);
        // max_x = max(10+50, 50+70) = 120, so width = 120-10 + 2*lr_pad
        assert!((d.width - (110.0 + 2.0 * lr_pad)).abs() < eps);
        // max_y = max(20+40, 100+30) = 130, so height = 130-20 + title + gap
        assert!((d.height - (110.0 + DOMAIN_TITLE_HEIGHT + INTER_NODE_GAP)).abs() < eps);
    }
}
