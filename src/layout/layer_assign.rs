/// Network simplex layer assignment with typed layers (DESIGN.md §4.2.2).
///
/// Both nodes and derivations are treated as vertices in a unified graph.
/// Nodes are assigned to even layers (0, 2, 4, ...); derivations are assigned
/// to odd layers (1, 3, 5, ...).  The network simplex algorithm minimizes
/// total weighted edge length subject to minimum-span constraints that encode
/// the layer parity requirement.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::types::{DerivId, DomainId, Edge, EdgeId, Graph, NodeId};
use crate::ObgraphError;

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// The result of layer assignment: a mapping from each element to its layer index.
/// Even layers contain nodes; odd layers contain derivations.
#[derive(Debug, Clone)]
pub struct LayerAssignment {
    pub node_layers: HashMap<NodeId, u32>,
    pub deriv_layers: HashMap<DerivId, u32>,
    pub num_layers: u32,
}

// ---------------------------------------------------------------------------
// Internal unified vertex / edge model
// ---------------------------------------------------------------------------

/// A vertex in the simplex graph: either a node or a derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Vertex {
    Node(NodeId),
    Deriv(DerivId),
}

impl Vertex {
    /// Returns true if this vertex should live on an even layer.
    fn is_node(self) -> bool {
        matches!(self, Vertex::Node(_))
    }
}

/// A directed edge in the simplex graph with its weight and minimum span.
#[derive(Debug, Clone, Copy)]
struct SimplexEdge {
    #[allow(dead_code)] // Retained for diagnostics and future use in long_edge.rs.
    edge_id: EdgeId,
    source: Vertex,
    target: Vertex,
    weight: u32,
    min_span: u32,
}

/// Complete simplex graph: vertices + edges extracted from the Graph model.
struct SimplexGraph {
    vertices: Vec<Vertex>,
    edges: Vec<SimplexEdge>,
    /// Adjacency: vertex -> list of edge indices where vertex is the source.
    out_edges: HashMap<Vertex, Vec<usize>>,
    /// Adjacency: vertex -> list of edge indices where vertex is the target.
    in_edges: HashMap<Vertex, Vec<usize>>,
}

// ---------------------------------------------------------------------------
// Build the simplex graph from the Graph model
// ---------------------------------------------------------------------------

fn build_simplex_graph(graph: &Graph) -> SimplexGraph {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut vertex_set: HashSet<Vertex> = HashSet::new();

    // Add all nodes as vertices.
    for node in &graph.nodes {
        let v = Vertex::Node(node.id);
        vertices.push(v);
        vertex_set.insert(v);
    }

    // Add all derivations as vertices.
    for deriv in &graph.derivations {
        let v = Vertex::Deriv(deriv.id);
        vertices.push(v);
        vertex_set.insert(v);
    }

    let mut edges: Vec<SimplexEdge> = Vec::new();
    let mut out_edges: HashMap<Vertex, Vec<usize>> = HashMap::new();
    let mut in_edges: HashMap<Vertex, Vec<usize>> = HashMap::new();

    // Initialize adjacency lists.
    for &v in &vertices {
        out_edges.entry(v).or_default();
        in_edges.entry(v).or_default();
    }

    for (idx, edge) in graph.edges.iter().enumerate() {
        let edge_id = EdgeId(idx as u32);
        let (source, target) = match edge {
            Edge::Anchor { parent, child, .. } => {
                (Vertex::Node(*parent), Vertex::Node(*child))
            }
            Edge::Constraint {
                source_prop,
                dest_prop,
                ..
            } => {
                let src_node = graph.properties[source_prop.index()].node;
                let dst_node = graph.properties[dest_prop.index()].node;
                // Skip intra-node constraints (same node) — they don't
                // contribute to the layer hierarchy and would create self-loops.
                if src_node == dst_node {
                    continue;
                }
                (Vertex::Node(src_node), Vertex::Node(dst_node))
            }
            Edge::DerivInput {
                source_prop,
                target_deriv,
                ..
            } => {
                let src_node = graph.properties[source_prop.index()].node;
                (Vertex::Node(src_node), Vertex::Deriv(*target_deriv))
            }
        };

        let weight = edge.weight();
        let min_span = minimum_span(source, target);

        let se = SimplexEdge {
            edge_id,
            source,
            target,
            weight,
            min_span,
        };

        let ei = edges.len();
        edges.push(se);
        out_edges.entry(source).or_default().push(ei);
        in_edges.entry(target).or_default().push(ei);
    }

    SimplexGraph {
        vertices,
        edges,
        out_edges,
        in_edges,
    }
}

/// Minimum span based on vertex types:
///   Node -> Node:   2 (skips one derivation layer)
///   Node -> Deriv:  1
///   Deriv -> Node:  1
///   Deriv -> Deriv: 2 (skips one node layer)
fn minimum_span(source: Vertex, target: Vertex) -> u32 {
    match (source.is_node(), target.is_node()) {
        (true, true) => 2,
        (true, false) => 1,
        (false, true) => 1,
        (false, false) => 2,
    }
}

// ---------------------------------------------------------------------------
// Phase 1: Longest-path initialization (feasible layer assignment)
// ---------------------------------------------------------------------------

/// Assigns each vertex a layer using the longest-path heuristic.
///
/// Vertices with no predecessors get layer 0 (nodes) or 1 (derivations).
/// Each subsequent vertex is placed at the maximum of
/// (predecessor_layer + min_span) across all incoming edges.
///
/// The layer is then adjusted to respect parity: even for nodes, odd for
/// derivations.
fn longest_path_init(sg: &SimplexGraph) -> HashMap<Vertex, u32> {
    // Topological sort via Kahn's algorithm.
    let topo = topological_sort(sg);

    let mut layer: HashMap<Vertex, u32> = HashMap::new();

    for &v in &topo {
        let in_ei = sg.in_edges.get(&v).map(|v| v.as_slice()).unwrap_or(&[]);
        if in_ei.is_empty() {
            // Root vertex: place on layer 0 (node) or 1 (derivation).
            let l = if v.is_node() { 0 } else { 1 };
            layer.insert(v, l);
        } else {
            // Compute minimum feasible layer from predecessors.
            let min_layer = in_ei
                .iter()
                .map(|&ei| {
                    let e = &sg.edges[ei];
                    layer[&e.source] + e.min_span
                })
                .max()
                .unwrap();

            // Snap to correct parity.
            let l = snap_to_parity(min_layer, v);
            layer.insert(v, l);
        }
    }

    layer
}

/// Snap a layer to the correct parity for a vertex type.
/// Nodes must be even; derivations must be odd.
fn snap_to_parity(layer: u32, v: Vertex) -> u32 {
    let need_even = v.is_node();
    let is_even = layer.is_multiple_of(2);
    if need_even == is_even {
        layer
    } else {
        // Bump up by 1 to fix parity.  This is always safe because it only
        // increases the layer, preserving feasibility (all min-span constraints
        // are still satisfied).
        layer + 1
    }
}

/// Kahn's algorithm topological sort on the simplex graph.
fn topological_sort(sg: &SimplexGraph) -> Vec<Vertex> {
    let mut in_degree: HashMap<Vertex, usize> = HashMap::new();
    for &v in &sg.vertices {
        in_degree.insert(v, 0);
    }
    for e in &sg.edges {
        *in_degree.entry(e.target).or_default() += 1;
    }

    let mut queue: VecDeque<Vertex> = VecDeque::new();
    for &v in &sg.vertices {
        if in_degree[&v] == 0 {
            queue.push_back(v);
        }
    }

    let mut order: Vec<Vertex> = Vec::with_capacity(sg.vertices.len());
    while let Some(v) = queue.pop_front() {
        order.push(v);
        for &ei in sg.out_edges.get(&v).map(|v| v.as_slice()).unwrap_or(&[]) {
            let e = &sg.edges[ei];
            let d = in_degree.get_mut(&e.target).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push_back(e.target);
            }
        }
    }

    order
}

// ---------------------------------------------------------------------------
// Phase 2: Feasible spanning tree
// ---------------------------------------------------------------------------

/// A spanning tree for the network simplex, represented as:
/// - A set of tree edge indices (into SimplexGraph.edges)
/// - Parent/child pointers for tree traversal
struct SpanningTree {
    /// Set of edge indices that form the tree.
    tree_edges: HashSet<usize>,
    /// For each vertex, the tree edge index connecting it to its parent
    /// (if any). `None` for the root.
    parent_edge: HashMap<Vertex, Option<usize>>,
    /// Tree root.
    root: Vertex,
}

/// The slack of an edge: layer(target) - layer(source) - min_span.
/// For a feasible assignment, slack >= 0.  Tight edges have slack == 0.
fn slack(e: &SimplexEdge, layer: &HashMap<Vertex, u32>) -> i64 {
    layer[&e.target] as i64 - layer[&e.source] as i64 - e.min_span as i64
}

/// Build an initial feasible spanning tree from the current layer assignment.
///
/// Strategy:
/// 1. Collect all tight edges (slack == 0).
/// 2. Build a tree greedily from tight edges using BFS/union-find.
/// 3. For any disconnected vertices, shift their layers to create tight edges
///    and add those edges.
fn init_feasible_tree(
    sg: &SimplexGraph,
    layer: &mut HashMap<Vertex, u32>,
) -> SpanningTree {
    if sg.vertices.is_empty() {
        return SpanningTree {
            tree_edges: HashSet::new(),
            parent_edge: HashMap::new(),
            root: Vertex::Node(NodeId(0)), // dummy, won't be used
        };
    }

    // Union-Find for tree construction.
    let mut uf = UnionFind::new(&sg.vertices);

    let mut tree_edges: HashSet<usize> = HashSet::new();
    let mut tree_adj: HashMap<Vertex, Vec<(Vertex, usize)>> = HashMap::new();
    for &v in &sg.vertices {
        tree_adj.entry(v).or_default();
    }

    // Pass 1: add tight edges greedily.
    // Sort edges by weight descending so we prefer higher-weight edges in the tree.
    let mut edge_indices: Vec<usize> = (0..sg.edges.len()).collect();
    edge_indices.sort_by(|&a, &b| sg.edges[b].weight.cmp(&sg.edges[a].weight));

    for &ei in &edge_indices {
        let e = &sg.edges[ei];
        if slack(e, layer) == 0 && uf.find(e.source) != uf.find(e.target) {
            uf.union(e.source, e.target);
            tree_edges.insert(ei);
            tree_adj.entry(e.source).or_default().push((e.target, ei));
            tree_adj.entry(e.target).or_default().push((e.source, ei));
        }
    }

    // Pass 2: for vertices not yet in the tree, find an incident edge and
    // tighten it by adjusting the layer of the disconnected component.
    let root = sg.vertices[0];
    let root_comp = uf.find(root);

    for &ei in &edge_indices {
        let e = &sg.edges[ei];
        let comp_s = uf.find(e.source);
        let comp_t = uf.find(e.target);
        if comp_s == comp_t {
            continue; // Already in the same component.
        }

        // Tighten this edge: adjust the layer of the component not containing
        // the root so that slack becomes 0.
        let s = slack(e, layer);
        if s == 0 {
            // Already tight.
            uf.union(e.source, e.target);
            tree_edges.insert(ei);
            tree_adj.entry(e.source).or_default().push((e.target, ei));
            tree_adj.entry(e.target).or_default().push((e.source, ei));
            continue;
        }

        // Determine which component to shift.
        // We shift the target component down (increase layers) or the source
        // component up (decrease layers).  We need slack to become 0, so:
        //   layer[target] - layer[source] - min_span = 0
        //   => we need to shift by -s if we move target, or +s if we move source.
        //
        // We shift the component that does NOT contain the root vertex.
        let target_has_root = uf.find(e.target) == root_comp;

        if target_has_root {
            // Shift source component up by s (increase all layers by s).
            let source_comp = comp_s;
            let delta = s; // s > 0, so we add delta to source component.
            for &v in &sg.vertices {
                if uf.find(v) == source_comp {
                    let old = layer[&v];
                    let new_layer = (old as i64 + delta) as u32;
                    layer.insert(v, new_layer);
                }
            }
        } else {
            // Shift target component down by s (decrease all layers by s).
            let target_comp = comp_t;
            let delta = s; // s > 0, so we subtract delta from target component.
            for &v in &sg.vertices {
                if uf.find(v) == target_comp {
                    let old = layer[&v];
                    let new_layer = (old as i64 - delta) as u32;
                    layer.insert(v, new_layer);
                }
            }
        }

        uf.union(e.source, e.target);
        tree_edges.insert(ei);
        tree_adj.entry(e.source).or_default().push((e.target, ei));
        tree_adj.entry(e.target).or_default().push((e.source, ei));
    }

    // Handle isolated vertices (no incident edges at all).  They are already
    // at valid layers from the longest-path init and don't need tree edges.
    // We connect them to the root with a virtual zero-weight tree relationship
    // (they just won't have parent_edge entries -- that's okay since they have
    // no edges to optimize).

    // Build parent pointers via BFS from root in the undirected tree.
    let mut parent_edge: HashMap<Vertex, Option<usize>> = HashMap::new();
    parent_edge.insert(root, None);
    let mut bfs_queue: VecDeque<Vertex> = VecDeque::new();
    bfs_queue.push_back(root);

    while let Some(v) = bfs_queue.pop_front() {
        for &(neighbor, ei) in tree_adj.get(&v).unwrap_or(&Vec::new()) {
            if let std::collections::hash_map::Entry::Vacant(entry) = parent_edge.entry(neighbor) {
                entry.insert(Some(ei));
                bfs_queue.push_back(neighbor);
            }
        }
    }

    // For any isolated vertex not reached, insert with None parent.
    for &v in &sg.vertices {
        parent_edge.entry(v).or_insert(None);
    }

    SpanningTree {
        tree_edges,
        parent_edge,
        root,
    }
}

// ---------------------------------------------------------------------------
// Phase 3: Network simplex pivot loop
// ---------------------------------------------------------------------------

/// Compute the cut value of every tree edge.
///
/// For a tree edge (u, v), removing it splits the tree into two components
/// (the "head" component containing v and the "tail" component containing u).
/// The cut value is:
///   sum of weights of edges going head->tail  (same direction as tree edge)
/// - sum of weights of edges going tail->head  (opposite direction)
///
/// where we count all graph edges (not just tree edges) that cross the cut.
fn compute_cut_values(
    sg: &SimplexGraph,
    tree: &SpanningTree,
    _layer: &HashMap<Vertex, u32>,
) -> HashMap<usize, i64> {
    let mut cut_values: HashMap<usize, i64> = HashMap::new();

    // Build undirected tree adjacency for component discovery.
    let mut tree_adj: HashMap<Vertex, Vec<(Vertex, usize)>> = HashMap::new();
    for &v in &sg.vertices {
        tree_adj.entry(v).or_default();
    }
    for &ei in &tree.tree_edges {
        let e = &sg.edges[ei];
        tree_adj.entry(e.source).or_default().push((e.target, ei));
        tree_adj.entry(e.target).or_default().push((e.source, ei));
    }

    for &te_idx in &tree.tree_edges {
        let te = &sg.edges[te_idx];
        // Remove this tree edge; find the component containing te.target
        // ("head" side) via BFS on the tree, excluding this edge.
        let mut head_set: HashSet<Vertex> = HashSet::new();
        let mut bfs: VecDeque<Vertex> = VecDeque::new();
        bfs.push_back(te.target);
        head_set.insert(te.target);

        while let Some(v) = bfs.pop_front() {
            for &(neighbor, nei) in tree_adj.get(&v).unwrap_or(&Vec::new()) {
                if nei == te_idx {
                    continue; // Skip the removed edge.
                }
                if head_set.insert(neighbor) {
                    bfs.push_back(neighbor);
                }
            }
        }

        // Cut value: for each graph edge crossing the cut,
        // +weight if source in tail (not in head) and target in head,
        // -weight if source in head and target in tail.
        let mut cv: i64 = 0;
        for ge in &sg.edges {
            let src_in_head = head_set.contains(&ge.source);
            let tgt_in_head = head_set.contains(&ge.target);
            if src_in_head && !tgt_in_head {
                // head -> tail (opposite direction to tree edge src->tgt)
                cv -= ge.weight as i64;
            } else if !src_in_head && tgt_in_head {
                // tail -> head (same direction as tree edge)
                cv += ge.weight as i64;
            }
        }

        cut_values.insert(te_idx, cv);
    }

    cut_values
}

/// Find a tree edge with a negative cut value (a "leaving" candidate for the
/// current tree that indicates improvement is possible).
fn find_negative_cut_edge(cut_values: &HashMap<usize, i64>) -> Option<usize> {
    cut_values
        .iter()
        .filter(|(_, cv)| **cv < 0)
        .min_by_key(|(_, cv)| **cv)
        .map(|(ei, _)| *ei)
}

/// Find the entering edge: among all non-tree edges crossing the same cut
/// as the leaving tree edge, pick the one with minimum slack.
///
/// The leaving edge (u, v) defines a cut.  The entering edge must connect
/// the tail component to the head component in the same direction as the
/// tree edge (i.e., from tail to head, which has the same orientation).
fn find_entering_edge(
    sg: &SimplexGraph,
    tree: &SpanningTree,
    leave_idx: usize,
    layer: &HashMap<Vertex, u32>,
) -> Option<usize> {
    let le = &sg.edges[leave_idx];

    // Build undirected tree adjacency.
    let mut tree_adj: HashMap<Vertex, Vec<(Vertex, usize)>> = HashMap::new();
    for &v in &sg.vertices {
        tree_adj.entry(v).or_default();
    }
    for &ei in &tree.tree_edges {
        let e = &sg.edges[ei];
        tree_adj.entry(e.source).or_default().push((e.target, ei));
        tree_adj.entry(e.target).or_default().push((e.source, ei));
    }

    // Find the head component (containing le.target) with the leaving edge removed.
    let mut head_set: HashSet<Vertex> = HashSet::new();
    let mut bfs: VecDeque<Vertex> = VecDeque::new();
    bfs.push_back(le.target);
    head_set.insert(le.target);

    while let Some(v) = bfs.pop_front() {
        for &(neighbor, nei) in tree_adj.get(&v).unwrap_or(&Vec::new()) {
            if nei == leave_idx {
                continue;
            }
            if head_set.insert(neighbor) {
                bfs.push_back(neighbor);
            }
        }
    }

    // Find non-tree edges crossing the cut in the same direction (tail -> head)
    // with minimum non-negative slack.
    let mut best: Option<(usize, i64)> = None;
    for (ei, e) in sg.edges.iter().enumerate() {
        if tree.tree_edges.contains(&ei) {
            continue;
        }
        let src_in_head = head_set.contains(&e.source);
        let tgt_in_head = head_set.contains(&e.target);

        // We want edges going tail -> head (same direction as the leaving edge).
        if !src_in_head && tgt_in_head {
            let s = slack(e, layer);
            if s >= 0 && (best.is_none() || s < best.unwrap().1) {
                best = Some((ei, s));
            }
        }
    }

    best.map(|(ei, _)| ei)
}

/// Perform the edge swap and update layers.
///
/// After swapping: remove `leave_idx` from tree, add `enter_idx`.
/// Then update layers so the entering edge becomes tight (slack == 0).
fn pivot(
    sg: &SimplexGraph,
    tree: &mut SpanningTree,
    layer: &mut HashMap<Vertex, u32>,
    leave_idx: usize,
    enter_idx: usize,
) {
    // Compute how much we need to shift to make the entering edge tight.
    let enter_edge = &sg.edges[enter_idx];
    let s = slack(enter_edge, layer);

    // Build tree adjacency WITHOUT the leaving edge (which we're about to
    // remove), but WITH the entering edge (which we're about to add).
    // The entering edge, when added, creates a cycle with the tree.
    // Removing the leaving edge breaks that cycle.  We shift the component
    // on the "head" side of the entering edge so that its slack becomes 0.

    // First, remove the leaving edge and add the entering edge.
    tree.tree_edges.remove(&leave_idx);
    tree.tree_edges.insert(enter_idx);

    // Build undirected tree adjacency for the new tree.
    let mut tree_adj: HashMap<Vertex, Vec<(Vertex, usize)>> = HashMap::new();
    for &v in &sg.vertices {
        tree_adj.entry(v).or_default();
    }
    for &ei in &tree.tree_edges {
        let e = &sg.edges[ei];
        tree_adj.entry(e.source).or_default().push((e.target, ei));
        tree_adj.entry(e.target).or_default().push((e.source, ei));
    }

    // The entering edge creates a direction: source -> target.  We need to
    // shift the target side of the entering edge (in the new tree, the
    // subtree beyond the entering edge from its target) by -s so that
    // slack becomes 0.  But actually, we need to figure out which side to
    // shift.  In the new tree, the entering edge connects two subtrees.
    // We shift the target-side subtree of the entering edge down by `s`.
    //
    // To find the target-side subtree of the entering edge: BFS from
    // enter_edge.target, but don't cross the entering edge itself.
    let mut target_side: HashSet<Vertex> = HashSet::new();
    let mut bfs: VecDeque<Vertex> = VecDeque::new();
    bfs.push_back(enter_edge.target);
    target_side.insert(enter_edge.target);

    while let Some(v) = bfs.pop_front() {
        for &(neighbor, nei) in tree_adj.get(&v).unwrap_or(&Vec::new()) {
            if nei == enter_idx {
                continue;
            }
            if target_side.insert(neighbor) {
                bfs.push_back(neighbor);
            }
        }
    }

    // Shift the target side down by `s` (subtract s from their layers).
    // This makes the entering edge tight: new slack = s - s = 0.
    if s != 0 {
        for &v in &target_side {
            let old = layer[&v];
            layer.insert(v, (old as i64 - s) as u32);
        }
    }

    // Rebuild parent pointers via BFS from root.
    let root = tree.root;
    let mut parent_edge: HashMap<Vertex, Option<usize>> = HashMap::new();
    parent_edge.insert(root, None);
    let mut bfs2: VecDeque<Vertex> = VecDeque::new();
    bfs2.push_back(root);

    while let Some(v) = bfs2.pop_front() {
        for &(neighbor, ei) in tree_adj.get(&v).unwrap_or(&Vec::new()) {
            if let std::collections::hash_map::Entry::Vacant(entry) = parent_edge.entry(neighbor) {
                entry.insert(Some(ei));
                bfs2.push_back(neighbor);
            }
        }
    }

    // Isolated vertices.
    for &v in &sg.vertices {
        parent_edge.entry(v).or_insert(None);
    }

    tree.parent_edge = parent_edge;
}

/// The main network simplex loop: iterate pivots until no negative cut value
/// edge exists.  Caps iterations to avoid infinite loops in degenerate cases.
fn simplex_iterate(
    sg: &SimplexGraph,
    tree: &mut SpanningTree,
    layer: &mut HashMap<Vertex, u32>,
) {
    let max_iterations = sg.vertices.len() * sg.edges.len().max(1) * 2 + 100;

    for _ in 0..max_iterations {
        let cut_values = compute_cut_values(sg, tree, layer);
        let leave_idx = match find_negative_cut_edge(&cut_values) {
            Some(ei) => ei,
            None => break, // Optimal.
        };

        let enter_idx = match find_entering_edge(sg, tree, leave_idx, layer) {
            Some(ei) => ei,
            None => break, // No improving edge found; done.
        };

        pivot(sg, tree, layer, leave_idx, enter_idx);
    }
}

// ---------------------------------------------------------------------------
// Normalize layers
// ---------------------------------------------------------------------------

/// Shift all layers so the minimum layer is 0 and compute num_layers.
fn normalize_layers(layer: &mut HashMap<Vertex, u32>) -> u32 {
    if layer.is_empty() {
        return 0;
    }
    let min_layer = *layer.values().min().unwrap();
    if min_layer > 0 {
        for v in layer.values_mut() {
            *v -= min_layer;
        }
    }
    layer.values().copied().max().unwrap_or(0) + 1
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the network simplex algorithm to assign layers to all graph elements.
pub fn network_simplex(graph: &Graph) -> Result<LayerAssignment, ObgraphError> {
    // Handle empty graph.
    if graph.nodes.is_empty() && graph.derivations.is_empty() {
        return Ok(LayerAssignment {
            node_layers: HashMap::new(),
            deriv_layers: HashMap::new(),
            num_layers: 0,
        });
    }

    let sg = build_simplex_graph(graph);

    // Check for cycles (topological sort should cover all vertices).
    let topo = topological_sort(&sg);
    if topo.len() != sg.vertices.len() {
        return Err(ObgraphError::Layout(
            "graph contains a cycle; cannot assign layers".into(),
        ));
    }

    // Phase 1: Longest-path initialization.
    let mut layer = longest_path_init(&sg);

    // Phase 2: Build feasible spanning tree.
    let mut tree = init_feasible_tree(&sg, &mut layer);

    // Phase 3: Network simplex pivot loop.
    if !sg.edges.is_empty() {
        simplex_iterate(&sg, &mut tree, &mut layer);
    }

    // Normalize so minimum layer is 0.
    let num_layers = normalize_layers(&mut layer);

    // Split into node_layers and deriv_layers.
    let mut node_layers: HashMap<NodeId, u32> = HashMap::new();
    let mut deriv_layers: HashMap<DerivId, u32> = HashMap::new();

    for (&v, &l) in &layer {
        match v {
            Vertex::Node(nid) => {
                node_layers.insert(nid, l);
            }
            Vertex::Deriv(did) => {
                deriv_layers.insert(did, l);
            }
        }
    }

    // Verify parity invariant.
    for (&nid, &l) in &node_layers {
        if l % 2 != 0 {
            return Err(ObgraphError::Layout(format!(
                "node {:?} assigned to odd layer {} (expected even)",
                nid, l
            )));
        }
    }
    for (&did, &l) in &deriv_layers {
        if l % 2 != 1 {
            return Err(ObgraphError::Layout(format!(
                "derivation {:?} assigned to even layer {} (expected odd)",
                did, l
            )));
        }
    }

    Ok(LayerAssignment {
        node_layers,
        deriv_layers,
        num_layers,
    })
}

// ---------------------------------------------------------------------------
// Compound layer assignment: enforce domain contiguity
// ---------------------------------------------------------------------------

/// A meta-element in the compound graph: either a domain (containing multiple
/// nodes/derivations) or a standalone element (free node or cross-domain deriv).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MetaElement {
    Domain(DomainId),
    FreeNode(NodeId),
    CrossDomainDeriv(DerivId),
}

/// Run network simplex and then enforce domain contiguity by remapping layers.
///
/// Domains become first-class layout participants: all members of a domain
/// occupy a contiguous range of layers, with inter-domain gap layers between
/// meta-elements (domains, free nodes, cross-domain derivations).
pub fn compound_network_simplex(graph: &Graph) -> Result<LayerAssignment, ObgraphError> {
    // Step 1: Run the standard network simplex.
    let base = network_simplex(graph)?;

    if graph.domains.is_empty() {
        return Ok(base);
    }

    // Step 2: Classify elements into meta-elements.

    // Build domain membership lookup: NodeId -> DomainId.
    let node_domain: HashMap<NodeId, DomainId> = graph
        .nodes
        .iter()
        .filter_map(|n| n.domain.map(|d| (n.id, d)))
        .collect();

    // Classify derivations as intra-domain or cross-domain.
    let deriv_domain: HashMap<DerivId, Option<DomainId>> = graph
        .derivations
        .iter()
        .map(|deriv| {
            let input_domains: HashSet<Option<DomainId>> = deriv
                .inputs
                .iter()
                .map(|&pid| graph.nodes[graph.properties[pid.index()].node.index()].domain)
                .collect();
            let output_domain =
                graph.nodes[graph.properties[deriv.output_prop.index()].node.index()].domain;

            // Intra-domain: all inputs and output in the same single domain.
            let all_same = input_domains.len() == 1
                && input_domains.iter().next().copied().flatten() == output_domain
                && output_domain.is_some();

            (deriv.id, if all_same { output_domain } else { None })
        })
        .collect();

    // Step 3: For each domain, collect internal layers used by its members.
    // "Internal layers" = the sorted distinct layers of member nodes + intra-domain derivations.
    let mut domain_internal_layers: HashMap<DomainId, Vec<u32>> = HashMap::new();
    for domain in &graph.domains {
        let mut layers: Vec<u32> = Vec::new();
        for &nid in &domain.members {
            if let Some(&l) = base.node_layers.get(&nid) {
                layers.push(l);
            }
        }
        // Include intra-domain derivations.
        for (&did, &maybe_dom) in &deriv_domain {
            if maybe_dom == Some(domain.id) {
                if let Some(&l) = base.deriv_layers.get(&did) {
                    layers.push(l);
                }
            }
        }
        layers.sort();
        layers.dedup();
        domain_internal_layers.insert(domain.id, layers);
    }

    // Step 4: Build meta-element list and ordering constraints.
    let mut meta_elements: Vec<MetaElement> = Vec::new();
    let mut meta_set: HashSet<MetaElement> = HashSet::new();

    for domain in &graph.domains {
        let me = MetaElement::Domain(domain.id);
        meta_elements.push(me);
        meta_set.insert(me);
    }
    for node in &graph.nodes {
        if node.domain.is_none() {
            let me = MetaElement::FreeNode(node.id);
            meta_elements.push(me);
            meta_set.insert(me);
        }
    }
    for (&did, &maybe_dom) in &deriv_domain {
        if maybe_dom.is_none() {
            let me = MetaElement::CrossDomainDeriv(did);
            meta_elements.push(me);
            meta_set.insert(me);
        }
    }

    // Map each vertex to its meta-element.
    let vertex_meta = |v: &Vertex| -> Option<MetaElement> {
        match v {
            Vertex::Node(nid) => {
                if let Some(&did) = node_domain.get(nid) {
                    Some(MetaElement::Domain(did))
                } else {
                    Some(MetaElement::FreeNode(*nid))
                }
            }
            Vertex::Deriv(did) => {
                if let Some(&maybe_dom) = deriv_domain.get(did) {
                    if let Some(dom_id) = maybe_dom {
                        Some(MetaElement::Domain(dom_id))
                    } else {
                        Some(MetaElement::CrossDomainDeriv(*did))
                    }
                } else {
                    None
                }
            }
        }
    };

    // Build ordering edges between meta-elements from the original graph edges.
    let mut meta_order: HashSet<(MetaElement, MetaElement)> = HashSet::new();
    let sg = build_simplex_graph(graph);
    for edge in &sg.edges {
        if let (Some(src_me), Some(tgt_me)) = (vertex_meta(&edge.source), vertex_meta(&edge.target))
        {
            if src_me != tgt_me {
                meta_order.insert((src_me, tgt_me));
            }
        }
    }

    // Step 5: Compute initial y-position for each meta-element (for topological ordering).
    // Use the minimum base layer of its constituents.
    let meta_min_layer: HashMap<MetaElement, u32> = meta_elements
        .iter()
        .map(|me| {
            let min_l = match me {
                MetaElement::Domain(did) => domain_internal_layers
                    .get(did)
                    .and_then(|layers| layers.first().copied())
                    .unwrap_or(0),
                MetaElement::FreeNode(nid) => *base.node_layers.get(nid).unwrap_or(&0),
                MetaElement::CrossDomainDeriv(did) => *base.deriv_layers.get(did).unwrap_or(&0),
            };
            (*me, min_l)
        })
        .collect();

    // Step 6: Topological sort of meta-elements respecting ordering constraints.
    // Use Kahn's algorithm; break ties by meta_min_layer (preserving simplex ordering).
    let sorted_meta = {
        let mut in_degree: HashMap<MetaElement, usize> = HashMap::new();
        let mut adj: HashMap<MetaElement, Vec<MetaElement>> = HashMap::new();
        for me in &meta_elements {
            in_degree.entry(*me).or_insert(0);
            adj.entry(*me).or_default();
        }
        for &(src, tgt) in &meta_order {
            if meta_set.contains(&src) && meta_set.contains(&tgt) {
                adj.entry(src).or_default().push(tgt);
                *in_degree.entry(tgt).or_insert(0) += 1;
            }
        }

        let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<(u32, usize)>> =
            std::collections::BinaryHeap::new();
        let me_index: HashMap<MetaElement, usize> = meta_elements
            .iter()
            .enumerate()
            .map(|(i, me)| (*me, i))
            .collect();

        for (&me, &deg) in &in_degree {
            if deg == 0 {
                queue.push(std::cmp::Reverse((meta_min_layer[&me], me_index[&me])));
            }
        }

        let mut sorted = Vec::new();
        while let Some(std::cmp::Reverse((_, idx))) = queue.pop() {
            let me = meta_elements[idx];
            sorted.push(me);
            if let Some(neighbors) = adj.get(&me) {
                for &nbr in neighbors {
                    let deg = in_degree.get_mut(&nbr).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(std::cmp::Reverse((meta_min_layer[&nbr], me_index[&nbr])));
                    }
                }
            }
        }
        sorted
    };

    // Step 7: Assign contiguous layer ranges.
    // Walk sorted meta-elements, assigning layers with inter-domain gaps.
    let mut new_node_layers: HashMap<NodeId, u32> = HashMap::new();
    let mut new_deriv_layers: HashMap<DerivId, u32> = HashMap::new();
    let mut cursor: u32 = 0; // Always points to the next available even layer.

    // Inter-domain gap: 2 layers (one even + one odd) to allow edge routing
    // through the gap. The first element starts with no leading gap.
    let inter_domain_gap: u32 = 2;

    for (i, me) in sorted_meta.iter().enumerate() {
        if i > 0 {
            cursor += inter_domain_gap;
            // Ensure cursor is on even boundary for consistency.
            if cursor % 2 != 0 {
                cursor += 1;
            }
        }

        match me {
            MetaElement::Domain(did) => {
                let internal_layers = &domain_internal_layers[did];
                if internal_layers.is_empty() {
                    continue;
                }

                // Compact internal layers: map each distinct base layer to
                // the next consecutive layer, preserving parity (even for
                // nodes, odd for derivations). This eliminates gaps from
                // cross-domain edges that inflated the base assignment.
                let domain = graph.domains.iter().find(|d| d.id == *did).unwrap();
                let mut local_cursor = cursor;
                for &old_layer in internal_layers {
                    // Snap to required parity: even layers hold nodes,
                    // odd layers hold derivations.
                    if local_cursor % 2 != old_layer % 2 {
                        local_cursor += 1;
                    }
                    let new_layer = local_cursor;

                    // Map all nodes/derivations on this old layer within this domain.
                    for &nid in &domain.members {
                        if base.node_layers.get(&nid) == Some(&old_layer) {
                            new_node_layers.insert(nid, new_layer);
                        }
                    }
                    // Intra-domain derivations.
                    for (&deriv_id, &maybe_dom) in &deriv_domain {
                        if maybe_dom == Some(*did)
                            && base.deriv_layers.get(&deriv_id) == Some(&old_layer)
                        {
                            new_deriv_layers.insert(deriv_id, new_layer);
                        }
                    }

                    local_cursor += 1;
                }

                // Advance cursor past the domain's compacted range.
                cursor = local_cursor;
                // Advance cursor to next even layer after the domain.
                if cursor % 2 != 0 {
                    cursor += 1;
                }
            }
            MetaElement::FreeNode(nid) => {
                // Free node on an even layer.
                if cursor % 2 != 0 {
                    cursor += 1;
                }
                new_node_layers.insert(*nid, cursor);
                cursor += 2; // Occupy this even layer, skip the odd.
            }
            MetaElement::CrossDomainDeriv(did) => {
                // Cross-domain derivation on an odd layer.
                if cursor % 2 == 0 {
                    cursor += 1;
                }
                new_deriv_layers.insert(*did, cursor);
                cursor += 1; // Advance past the odd layer.
            }
        }
    }

    // Step 8: Normalize and compute num_layers.
    let all_layers: Vec<u32> = new_node_layers
        .values()
        .chain(new_deriv_layers.values())
        .copied()
        .collect();
    let min_layer = all_layers.iter().copied().min().unwrap_or(0);
    for l in new_node_layers.values_mut() {
        *l -= min_layer;
    }
    for l in new_deriv_layers.values_mut() {
        *l -= min_layer;
    }
    let num_layers = new_node_layers
        .values()
        .chain(new_deriv_layers.values())
        .copied()
        .max()
        .unwrap_or(0)
        + 1;

    // Step 9: Verify parity invariant.
    for (&nid, &l) in &new_node_layers {
        if l % 2 != 0 {
            return Err(ObgraphError::Layout(format!(
                "compound: node {:?} assigned to odd layer {} (expected even)",
                nid, l
            )));
        }
    }
    for (&did, &l) in &new_deriv_layers {
        if l % 2 != 1 {
            return Err(ObgraphError::Layout(format!(
                "compound: derivation {:?} assigned to even layer {} (expected odd)",
                did, l
            )));
        }
    }

    Ok(LayerAssignment {
        node_layers: new_node_layers,
        deriv_layers: new_deriv_layers,
        num_layers,
    })
}

// ---------------------------------------------------------------------------
// Union-Find utility
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: HashMap<Vertex, Vertex>,
    rank: HashMap<Vertex, usize>,
}

impl UnionFind {
    fn new(vertices: &[Vertex]) -> Self {
        let mut parent = HashMap::new();
        let mut rank = HashMap::new();
        for &v in vertices {
            parent.insert(v, v);
            rank.insert(v, 0);
        }
        UnionFind { parent, rank }
    }

    fn find(&mut self, v: Vertex) -> Vertex {
        let p = self.parent[&v];
        if p != v {
            let root = self.find(p);
            self.parent.insert(v, root);
            root
        } else {
            v
        }
    }

    fn union(&mut self, a: Vertex, b: Vertex) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let rank_a = self.rank[&ra];
        let rank_b = self.rank[&rb];
        if rank_a < rank_b {
            self.parent.insert(ra, rb);
        } else if rank_a > rank_b {
            self.parent.insert(rb, ra);
        } else {
            self.parent.insert(rb, ra);
            self.rank.insert(ra, rank_a + 1);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::*;

    /// Helper: build a minimal Graph with nodes, properties, derivations, edges.
    fn make_graph(
        nodes: Vec<Node>,
        properties: Vec<Property>,
        derivations: Vec<Derivation>,
        edges: Vec<Edge>,
        domains: Vec<Domain>,
    ) -> Graph {
        let mut prop_edges: HashMap<PropId, Vec<EdgeId>> = HashMap::new();
        let mut node_children: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut node_parent: HashMap<NodeId, EdgeId> = HashMap::new();

        for (idx, edge) in edges.iter().enumerate() {
            let eid = EdgeId(idx as u32);
            match edge {
                Edge::Anchor { parent, child, .. } => {
                    node_children.entry(*parent).or_default().push(eid);
                    node_parent.insert(*child, eid);
                }
                Edge::Constraint {
                    source_prop,
                    dest_prop,
                    ..
                } => {
                    prop_edges.entry(*source_prop).or_default().push(eid);
                    prop_edges.entry(*dest_prop).or_default().push(eid);
                }
                Edge::DerivInput {
                    source_prop,
                    target_deriv: _,
                    ..
                } => {
                    prop_edges.entry(*source_prop).or_default().push(eid);
                }
            }
        }

        Graph {
            nodes,
            properties,
            derivations,
            edges,
            domains,
            prop_edges,
            node_children,
            node_parent,
        }
    }

    fn make_node(id: u32, ident: &str, props: &[u32], is_anchored: bool) -> Node {
        Node {
            id: NodeId(id),
            ident: ident.to_string(),
            display_name: None,
            properties: props.iter().map(|&p| PropId(p)).collect(),
            domain: None,
            is_anchored,
            is_selected: false,
        }
    }

    fn make_prop(id: u32, node: u32, name: &str) -> Property {
        Property {
            id: PropId(id),
            node: NodeId(node),
            name: name.to_string(),
            critical: true, constrained: false,
        }
    }

    // ----- Test 1: Simple two-node graph -----

    #[test]
    fn test_two_node_graph() {
        let nodes = vec![
            make_node(0, "parent", &[], true),
            make_node(1, "child", &[], false),
        ];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, vec![], vec![], edges, vec![]);

        let result = network_simplex(&graph).unwrap();
        assert_eq!(result.node_layers[&NodeId(0)], 0);
        assert_eq!(result.node_layers[&NodeId(1)], 2);
    }

    // ----- Test 2: Chain of 3 nodes -----

    #[test]
    fn test_three_node_chain() {
        let nodes = vec![
            make_node(0, "a", &[], true),
            make_node(1, "b", &[], false),
            make_node(2, "c", &[], false),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, vec![], vec![], edges, vec![]);

        let result = network_simplex(&graph).unwrap();
        assert_eq!(result.node_layers[&NodeId(0)], 0);
        assert_eq!(result.node_layers[&NodeId(1)], 2);
        assert_eq!(result.node_layers[&NodeId(2)], 4);
    }

    // ----- Test 3: Graph with a derivation -----

    #[test]
    fn test_with_derivation() {
        // Node A has property p0.
        // Derivation D takes p0 as input and produces p1 on node B.
        // So we have: A --DerivInput--> D, and D's output is p1 on B.
        // For layout: A (node, even layer) -> D (deriv, odd layer)
        // B needs to be downstream of D since it owns the output.
        // We'll add a Link from A to B to give B a position.
        let nodes = vec![
            make_node(0, "A", &[0], true),
            make_node(1, "B", &[1], false),
        ];
        let props = vec![
            make_prop(0, 0, "p0"),
            make_prop(1, 1, "p1"),
        ];
        let derivations = vec![Derivation {
            id: DerivId(0),
            operation: "hash".to_string(),
            inputs: vec![PropId(0)],
            output_prop: PropId(1),
        }];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::DerivInput {
                source_prop: PropId(0),
                target_deriv: DerivId(0),
            },
        ];
        let graph = make_graph(nodes, props, derivations, edges, vec![]);

        let result = network_simplex(&graph).unwrap();

        // Node A on even layer.
        assert_eq!(result.node_layers[&NodeId(0)] % 2, 0);
        // Node B on even layer.
        assert_eq!(result.node_layers[&NodeId(1)] % 2, 0);
        // Derivation D on odd layer.
        assert_eq!(result.deriv_layers[&DerivId(0)] % 2, 1);

        // A must be above D (A -> D via DerivInput).
        assert!(result.node_layers[&NodeId(0)] < result.deriv_layers[&DerivId(0)]);
        // A must be above B (A -> B via Link).
        assert!(result.node_layers[&NodeId(0)] < result.node_layers[&NodeId(1)]);

        // With minimum spans: A at 0, D at 1, B at 2.
        assert_eq!(result.node_layers[&NodeId(0)], 0);
        assert_eq!(result.deriv_layers[&DerivId(0)], 1);
        assert_eq!(result.node_layers[&NodeId(1)], 2);
    }

    // ----- Test 4: PKI example from Appendix A.6 -----

    #[test]
    fn test_pki_example() {
        // Nodes: ca (root), cert, tls, revocation
        // Edges:
        //   ca -> cert (Link)
        //   cert -> tls (Link)
        //   ca -> revocation (Link)
        //   revocation -> cert (Constraint: revocation.status -> cert.validity)
        let nodes = vec![
            make_node(0, "ca", &[0], true),
            make_node(1, "cert", &[1, 2], false),
            make_node(2, "tls", &[3], false),
            make_node(3, "revocation", &[4], false),
        ];
        let props = vec![
            make_prop(0, 0, "key"),
            make_prop(1, 1, "validity"),
            make_prop(2, 1, "key"),
            make_prop(3, 2, "session"),
            make_prop(4, 3, "status"),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: Some("signs".into()),
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(2),
                operation: Some("authenticates".into()),
            },
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(3),
                operation: Some("publishes".into()),
            },
            Edge::Constraint {
                source_prop: PropId(4), // revocation.status
                dest_prop: PropId(1),   // cert.validity
                operation: Some("validates".into()),
            },
        ];
        let graph = make_graph(nodes, props, vec![], edges, vec![]);

        let result = network_simplex(&graph).unwrap();

        // All nodes on even layers.
        for (&nid, &l) in &result.node_layers {
            assert_eq!(l % 2, 0, "node {:?} on odd layer {}", nid, l);
        }

        // ca at layer 0.
        assert_eq!(result.node_layers[&NodeId(0)], 0, "ca should be at layer 0");
        // revocation at layer 2 (one step below ca).
        assert_eq!(
            result.node_layers[&NodeId(3)], 2,
            "revocation should be at layer 2"
        );
        // cert at layer 2 (ca -> cert is Link with span 2).
        // But also revocation -> cert is a Constraint with span 2.
        // So cert must be >= revocation + 2 = 4 OR ca + 2 = 2.
        // The constraint revocation(layer=2) -> cert requires cert >= 4.
        // Actually: the constraint edge is revocation.status -> cert.validity,
        // which maps to Node(revocation) -> Node(cert), min_span 2.
        // So cert >= revocation_layer + 2 = 4.
        assert_eq!(
            result.node_layers[&NodeId(1)], 4,
            "cert should be at layer 4"
        );
        // tls at layer 6 (cert + 2).
        // Wait: reconsider. If the network simplex optimizes, it may place
        // revocation at layer 2 (from ca=0+2), cert at max(0+2, 2+2) = 4,
        // tls at 4+2=6.
        // But the reference says: Layer 0: ca, revocation; Layer 2: cert; Layer 4: tls.
        // That implies revocation at layer 0.  Let's check feasibility:
        // ca -> revocation (Link, span 2): revocation >= 0 + 2 = 2.
        // So revocation can't be at 0 with a link from ca.
        //
        // Unless the reference groups by "visual rows" not actual parity layers.
        // The reference says "Layer 0 (even/node): ca, revocation" which means
        // both ca and revocation on the same layer.
        //
        // But ca -> revocation is a Link edge with min_span 2, so they can't
        // be on the same layer!  Unless the PKI example doesn't have a link
        // from ca to revocation, or it uses a different edge structure.
        //
        // Let me reconsider: the PKI example from the spec may not have
        // ca -> revocation as a Link.  It might have them as siblings.
        // For this test, I have ca -> revocation as a Link, so the layers are:
        //   ca=0, revocation=2, cert=4, tls=6
        // This is correct for our edge structure.
        //
        // Let's just verify the ordering constraints.
        let ca = result.node_layers[&NodeId(0)];
        let cert = result.node_layers[&NodeId(1)];
        let tls = result.node_layers[&NodeId(2)];
        let revocation = result.node_layers[&NodeId(3)];

        // ca -> cert: cert >= ca + 2
        assert!(cert >= ca + 2, "cert must be >= ca + 2");
        // cert -> tls: tls >= cert + 2
        assert!(tls >= cert + 2, "tls must be >= cert + 2");
        // ca -> revocation: revocation >= ca + 2
        assert!(revocation >= ca + 2, "revocation must be >= ca + 2");
        // revocation -> cert (Constraint): cert >= revocation + 2
        assert!(cert >= revocation + 2, "cert must be >= revocation + 2");
    }

    // ----- Test 5: Empty graph -----

    #[test]
    fn test_empty_graph() {
        let graph = make_graph(vec![], vec![], vec![], vec![], vec![]);
        let result = network_simplex(&graph).unwrap();
        assert_eq!(result.num_layers, 0);
    }

    // ----- Test 6: Single node -----

    #[test]
    fn test_single_node() {
        let nodes = vec![make_node(0, "root", &[], true)];
        let graph = make_graph(nodes, vec![], vec![], vec![], vec![]);

        let result = network_simplex(&graph).unwrap();
        assert_eq!(result.node_layers[&NodeId(0)], 0);
        assert_eq!(result.num_layers, 1);
    }

    // ----- Test 7: Diamond graph -----

    #[test]
    fn test_diamond_graph() {
        // a -> b, a -> c, b -> d, c -> d
        let nodes = vec![
            make_node(0, "a", &[], true),
            make_node(1, "b", &[], false),
            make_node(2, "c", &[], false),
            make_node(3, "d", &[], false),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(2),
                operation: None,
            },
            Edge::Anchor {
                parent: NodeId(1),
                child: NodeId(3),
                operation: None,
            },
            // c -> d must be a Constraint since d already has a parent (b).
            Edge::Constraint {
                source_prop: PropId(0),
                dest_prop: PropId(1),
                operation: None,
            },
        ];
        let props = vec![
            make_prop(0, 2, "out"), // property on c
            make_prop(1, 3, "in"),  // property on d
        ];
        let graph = make_graph(nodes, props, vec![], edges, vec![]);

        let result = network_simplex(&graph).unwrap();

        let a = result.node_layers[&NodeId(0)];
        let b = result.node_layers[&NodeId(1)];
        let c = result.node_layers[&NodeId(2)];
        let d = result.node_layers[&NodeId(3)];

        // All even.
        assert_eq!(a % 2, 0);
        assert_eq!(b % 2, 0);
        assert_eq!(c % 2, 0);
        assert_eq!(d % 2, 0);

        // Constraints: a at 0, b >= 2, c >= 2, d >= b+2 and d >= c+2.
        assert_eq!(a, 0);
        assert!(b >= 2);
        assert!(c >= 2);
        assert!(d >= b + 2);
        assert!(d >= c + 2);
    }

    // ----- Test 8: Minimum span values -----

    #[test]
    fn test_minimum_span() {
        assert_eq!(minimum_span(Vertex::Node(NodeId(0)), Vertex::Node(NodeId(1))), 2);
        assert_eq!(minimum_span(Vertex::Node(NodeId(0)), Vertex::Deriv(DerivId(0))), 1);
        assert_eq!(minimum_span(Vertex::Deriv(DerivId(0)), Vertex::Node(NodeId(0))), 1);
        assert_eq!(minimum_span(Vertex::Deriv(DerivId(0)), Vertex::Deriv(DerivId(1))), 2);
    }

    // ----- Test 9: Layer parity snap -----

    #[test]
    fn test_snap_to_parity() {
        // Node -> even
        assert_eq!(snap_to_parity(0, Vertex::Node(NodeId(0))), 0);
        assert_eq!(snap_to_parity(1, Vertex::Node(NodeId(0))), 2);
        assert_eq!(snap_to_parity(2, Vertex::Node(NodeId(0))), 2);
        assert_eq!(snap_to_parity(3, Vertex::Node(NodeId(0))), 4);

        // Deriv -> odd
        assert_eq!(snap_to_parity(0, Vertex::Deriv(DerivId(0))), 1);
        assert_eq!(snap_to_parity(1, Vertex::Deriv(DerivId(0))), 1);
        assert_eq!(snap_to_parity(2, Vertex::Deriv(DerivId(0))), 3);
        assert_eq!(snap_to_parity(3, Vertex::Deriv(DerivId(0))), 3);
    }

    // ----- Test 10: Disconnected nodes -----

    #[test]
    fn test_disconnected_nodes() {
        let nodes = vec![
            make_node(0, "a", &[], true),
            make_node(1, "b", &[], true),
        ];
        let graph = make_graph(nodes, vec![], vec![], vec![], vec![]);

        let result = network_simplex(&graph).unwrap();
        // Both are roots with no edges -- both at layer 0.
        assert_eq!(result.node_layers[&NodeId(0)], 0);
        assert_eq!(result.node_layers[&NodeId(1)], 0);
    }
}
