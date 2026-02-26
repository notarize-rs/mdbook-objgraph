# Layout Algorithm

This document describes the obgraph layout algorithm, a modified Sugiyama
(layered graph drawing) method adapted for typed layers, port-aware record-style
nodes, multiple weighted edge types, and pre-computed orthogonal edge routing.
It covers the full layout pipeline from layer assignment through edge routing,
and defines the output data structures consumed by the rendering pipeline.

The layout engine operates on the `Graph` data structure defined in
[GRAPH_MODEL.md](GRAPH_MODEL.md) §2.5. Key types: `NodeId`, `PropId`,
`DerivId`, `EdgeId`, `DomainId` (all `u32` newtypes). The graph contains nodes
with properties, derivation nodes, and three edge types (Anchor, Constraint,
DerivInput).

---

## 4. Layout Algorithm

The layout engine implements a modified Sugiyama (layered graph drawing)
algorithm adapted for the obgraph model. The modifications address typed layers,
port-aware nodes, multiple edge types with different visual priorities, and
pre-computed edge routing for all edges including initially hidden ones.

### 4.1 Background: The Sugiyama Framework

The standard Sugiyama method (Sugiyama, Tagawa, & Toda, 1981) lays out directed
graphs in horizontal layers with edges flowing in a consistent direction. It
proceeds in five phases:

1. **Cycle removal** — reverse edges to make the graph acyclic.
2. **Layer assignment** — assign nodes to horizontal layers such that all edges
   flow downward.
3. **Crossing minimization** — order nodes within each layer to minimize edge
   crossings.
4. **Coordinate assignment** — assign x-coordinates to nodes within each layer.
5. **Edge routing** — compute paths for edges between nodes.

Key references for the standard algorithm and its improvements:

- Sugiyama, K., Tagawa, S., & Toda, M. (1981). Methods for visual understanding
  of hierarchical system structures. _IEEE Transactions on Systems, Man and
  Cybernetics_, 11(2), 109–125.
- Gansner, E., Koutsofios, E., North, S., & Vo, K. (1993). A technique for
  drawing directed graphs. _IEEE Transactions on Software Engineering_, 19(3),
  214–229. (Introduces the network simplex algorithm for layer assignment.)
- Eiglsperger, M., Siebenhaller, M., & Kaufmann, M. (2005). An efficient
  implementation of Sugiyama's algorithm for layered graph drawing. _Graph
  Drawing (GD 2004)_, LNCS 3383, 155–166. (Reduces worst-case complexity from
  O(|V|·|E|·log|E|) to O((|V|+|E|)·log|E|) by keeping dummy node count linear.)
- Brandes, U. & Köpf, B. (2001). Fast and simple horizontal coordinate
  assignment. _Graph Drawing (GD 2001)_, LNCS 2265, 31–44. (Linear-time
  coordinate assignment.)

### 4.2 Modifications for obgraph

#### 4.2.1 Phase 1: Cycle Removal — Skipped

The obgraph model guarantees the complete graph (anchors + derivation edges +
constraints) is acyclic. No cycle removal is needed.

#### 4.2.2 Phase 2: Layer Assignment — Typed Layers with Pre-Assignment

obgraph uses **typed layers**. There are two layer types:

- **Node layers**: contain only regular nodes.
- **Derivation layers**: contain only derivation nodes.

Derivation layers are rendered with reduced vertical spacing (matching a single
property row height) compared to node layers (which are sized to the tallest
node in the layer).

##### Pre-Assignment Approach

Rather than post-processing network simplex output (which risks invalidating
edge directions), the type constraint is encoded directly into the network
simplex formulation via **layer parity**:

- Nodes are assigned to even layers (0, 2, 4, ...).
- Derivations are assigned to odd layers (1, 3, 5, ...).

The minimum edge span constraint is adjusted by edge endpoint types:

| Source Type | Target Type | Minimum Span                   |
| ----------- | ----------- | ------------------------------ |
| Node        | Node        | 2 (skips one derivation layer) |
| Node        | Derivation  | 1                              |
| Derivation  | Node        | 1                              |
| Derivation  | Derivation  | 2 (skips one node layer)       |

This ensures the optimizer naturally accounts for typed layer structure. The
objective function remains:

```
minimize: Σ w(e) × (layer(target) - layer(source))
subject to: layer(target) - layer(source) ≥ min_span(e)  for each edge (source, target)
```

Edge weights by type:

| Edge Type       | Weight | Rationale                                     |
| --------------- | ------ | --------------------------------------------- |
| Anchor          | 3      | Highest priority: keep hierarchy compact      |
| Derivation edge | 2      | Medium priority: keep derivations near inputs |
| Constraint      | 1      | Lowest priority: tolerate longer spans        |

After assignment, empty typed layers (those with no assigned elements) collapse
to zero height.

```
Even (Node) Layer 0:     [Node A]           [Node B]
Odd  (Deriv) Layer 1:         [D1]     [D2]
Even (Node) Layer 2:                              ← empty, collapses
Odd  (Deriv) Layer 3:              [D3]           ← D3 depends on D1
Even (Node) Layer 4:     [Node C]
```

##### Compound Graph Layering (Domain Contiguity)

Domains are **first-class participants** in layer assignment. The key invariant:

> **Domain Contiguity**: All member nodes of a domain occupy a contiguous
> range of layers. No foreign node or derivation may be interleaved between
> a domain's topmost and bottommost members.

This is enforced by `compound_network_simplex`, which runs a two-phase
approach:

**Phase A — Base assignment**: Run standard `network_simplex` to produce an
initial feasible layer assignment respecting edge constraints and parity.

**Phase B — Domain compaction**: Remap layers so that each domain's members
form a contiguous block, with inter-domain gap layers between meta-elements.

The meta-elements are:

| Meta-element type | Description |
| --- | --- |
| `Domain(DomainId)` | A domain containing all its member nodes and intra-domain derivations |
| `FreeNode(NodeId)` | A node not belonging to any domain |
| `CrossDomainDeriv(DerivId)` | A derivation whose inputs/output span multiple domains |

**Algorithm**:

1. **Classify elements**: Each node maps to its domain or is a free node.
   Each derivation is intra-domain (all inputs + output in same domain) or
   cross-domain.

2. **Compute domain internal layers**: For each domain, collect the sorted
   distinct layers of its member nodes and intra-domain derivations from the
   base assignment.

3. **Build ordering constraints**: For each cross-domain edge, add a
   meta-edge between the source and target meta-elements.

4. **Topological sort**: Sort meta-elements using Kahn's algorithm, breaking
   ties by the minimum base layer (preserving the network simplex ordering).

5. **Assign contiguous ranges**: Walk the sorted meta-elements, assigning
   contiguous layer ranges. Domains keep their internal layer structure
   (relative offsets and parity preserved). An inter-domain gap of 2 layers
   is inserted between each pair of adjacent meta-elements.

6. **Normalize**: Shift all layers so the minimum is 0. Verify the parity
   invariant (nodes on even layers, derivations on odd layers).

The result is a `LayerAssignment` where same-domain nodes are contiguous
and inter-domain elements (free nodes, cross-domain derivations) sit in
dedicated gap layers between domains.

##### Network Simplex Pseudocode

The network simplex algorithm for layer assignment (Gansner et al., 1993)
operates as follows:

```
function network_simplex(graph, weights, min_spans) → layer_assignment:
    // 1. Initialize feasible layer assignment using longest-path
    //    Sources at top (layer 0), targets at bottom (higher layers).
    for each node v in topological order:
        if v has no predecessors:
            layer[v] = 0   // or appropriate even/odd for typed layers
        else:
            layer[v] = max(layer[u] + min_span(u,v) for each predecessor u of v)

    // 2. Build feasible spanning tree
    tree = init_feasible_tree(graph, layer)
    // A spanning tree where all tree edges are "tight"
    // (i.e., layer[target] - layer[source] == min_span(e))

    // 3. Iterate: pivot to reduce total weighted edge length
    loop:
        // Find a non-tree edge with negative cut value
        e_enter = find_negative_cut_edge(tree, graph, weights)
        if e_enter is None:
            break   // optimal

        // Find the tree edge to remove (the edge in the tree path
        // between e_enter's endpoints with the most slack)
        e_leave = find_leaving_edge(tree, e_enter)

        // Pivot: swap edges and update layers
        tree.remove(e_leave)
        tree.add(e_enter)
        update_layers(tree, layer, e_enter, e_leave)

    return layer

function init_feasible_tree(graph, layer) → tree:
    // The spanning tree covers ALL elements (nodes and derivations alike).
    // Network simplex treats both as graph vertices.
    tree_nodes = { arbitrary element }
    tree_edges = {}

    while |tree_nodes| < |all_elements|:   // nodes + derivations
        // Find a tight edge (span == min_span) with exactly one endpoint in tree
        for each edge e = (u, v) in graph:
            if layer[v] - layer[u] == min_span(e):
                if u in tree_nodes XOR v in tree_nodes:
                    tree_edges.add(e)
                    tree_nodes.add(u)
                    tree_nodes.add(v)
                    break
        else:
            // No tight edge found; tighten by shifting layers
            delta = min(layer[v] - layer[u] - min_span(e)
                        for each edge (u,v)
                        where u in tree_nodes, v not in tree_nodes)
            for each v not in tree_nodes:
                layer[v] -= delta

    return tree
```

The **cut value** of a tree edge is the total weight of edges that would
decrease their span minus those that would increase if the edge were contracted.
See Gansner et al. (1993), Section 3 for the precise definition.

##### Eiglsperger Optimization for Long Edges

Obgraph graphs regularly include constraints spanning many layers (e.g., a root
CA property constraining a leaf TLS session property). In standard Sugiyama,
each such edge creates a dummy node per spanned layer. For an edge spanning 10
layers, this means 9 dummy nodes that participate in every subsequent phase
(crossing minimization, coordinate assignment), bloating both time and memory.

The implementation uses the **Eiglsperger optimization** (Eiglsperger,
Siebenhaller, & Kaufmann, 2005) to handle long edges implicitly.

An edge is **long** when its actual layer span exceeds its `min_span` — that is,
it crosses at least one intermediate layer where a segment entry must be
inserted. For example, a Node→Node edge with `min_span=2` and actual span 2 is
_not_ long (it spans exactly one gap). The same edge with actual span 4 is long,
requiring segment entries in layers 1 and 3.

```rust
struct LongEdge {
    edge_id: EdgeId,
    source_layer: u32,
    target_layer: u32,
    // Position within each intermediate layer's ordering.
    // Updated during crossing minimization.
    // Key: layer index, Value: fractional position in that layer's ordering.
    positions: HashMap<u32, f64>,
}

struct LayerEntry {
    // A layer contains a mix of real elements and long edge segments
    items: Vec<LayerItem>,
}

enum LayerItem {
    Node(NodeId),
    Derivation(DerivId),
    Segment(EdgeId, u32),   // (edge_id, layer_index) — a long edge passing through
}
```

During **crossing minimization**, long edge segments participate in barycenter
computation. A segment's barycenter in layer _k_ is the position of the same
edge in layer _k-1_ (for top-down sweeps) or _k+1_ (for bottom-up sweeps). This
is a direct lookup, not a computation over dummy node adjacency.

During **coordinate assignment**, long edge segments receive x-coordinates like
any other layer element. The Brandes-Köpf alignment step treats segments as
single-port elements that chain vertically.

#### 4.2.3 Phase 3: Crossing Minimization — Weighted, Port-Aware

Crossing minimization uses a **layer-by-layer sweep** with a **weighted
barycenter heuristic** (alternating top-to-bottom and bottom-to-top sweeps until
stable).

All three edge types participate. When two edges cross, the cost is:

```
crossing_cost(e1, e2) = w(e1) + w(e2)
```

Using the same weights as layer assignment (anchors=3, derivation edges=2,
constraints=1). The optimizer prioritizes keeping the primary structural edges
(anchors) crossing-free, followed by derivation edges, with constraints as the
lowest priority.

##### Port-Aware Nested Barycenter

Nodes are record-style with vertically stacked property rows. Each property has
a left port and a right port. Crossing minimization must optimize two levels of
ordering simultaneously:

1. **Node order** within each layer.
2. **Property order** within each node.

```
function minimize_crossings(layers, edges) → optimized_layers:
    best = layers.clone()
    best_crossings = count_crossings(best, edges)

    for iteration in 0..MAX_ITERATIONS:   // MAX_ITERATIONS = 24 (per ELK convention)
        // Alternate sweep direction
        if iteration % 2 == 0:
            layer_range = 1..num_layers          // top-down: fix layer k-1, optimize k
        else:
            layer_range = (num_layers-2)..=0     // bottom-up: fix layer k+1, optimize k

        for k in layer_range:
            adjacent = if top_down then k-1 else k+1

            // Step 1: Compute property barycenters within each node
            for each node n in layers[k]:
                for each property p of n:
                    positions = []
                    for each edge e incident on p connecting to layer[adjacent]:
                        opposite_prop = other_endpoint(e, p)
                        pos = position_of(opposite_prop, layers[adjacent])
                        positions.push(pos, weight(e))
                    if positions is not empty:
                        p.barycenter = weighted_mean(positions)
                    else:
                        p.barycenter = current_position(p)  // keep in place

                // Sort properties within this node by barycenter
                sort(n.properties, by: p.barycenter)

            // Step 2: Compute node barycenters from property barycenters
            for each item in layers[k]:
                match item:
                    Node(n):
                        connected_props = [p for p in n.properties if p.barycenter was computed from edges]
                        if connected_props is not empty:
                            item.barycenter = mean(p.barycenter for p in connected_props)
                        else:
                            item.barycenter = current_position(item)
                    Derivation(d):
                        // Derivation has a single implicit port
                        positions = []
                        for each edge e incident on d connecting to layer[adjacent]:
                            pos = position_of(other_endpoint(e), layers[adjacent])
                            positions.push(pos, weight(e))
                        item.barycenter = weighted_mean(positions)
                    Segment(edge_id, _):
                        // Long edge segment: barycenter is position in adjacent layer
                        item.barycenter = position_of_segment(edge_id, adjacent)

            // Step 3: Sort layer by item barycenter
            sort(layers[k].items, by: item.barycenter)

        // Check improvement
        current_crossings = count_crossings(layers, edges)
        if current_crossings < best_crossings:
            best = layers.clone()
            best_crossings = current_crossings
        if current_crossings == 0:
            break

    return best
```

##### Chiasm Pre-Seeding for Same-Domain Constraint Bundles

After barycenter sorting, same-domain node pairs connected by ≥2 constraints
receive **chiasm pre-seeding**: the connected properties on one node are
reversed relative to the other node's order. This creates a chiastic (ABBA)
arrangement — strictly nested brackets in the side corridor:

```
  ╭────────────────╮   prop_a → prop_x (outermost)
  │ ╭──────────╮   │   prop_b → prop_y
  │ │ ╭────╮   │   │   prop_c → prop_z (innermost)
  │ │ ╰────╯   │   │
  │ ╰──────────╯   │
  ╰────────────────╯
```

Without chiasm pre-seeding, parallel ordering creates overlapping bracket spans
that cause corridor collisions.

##### Sifting Refinement

After barycenter sorting and chiasm pre-seeding, a **sifting** pass refines
intra-node property ordering (replacing the simpler bubble-sort approach). For
each property, ordered by decreasing connectivity, sifting removes it from the
list and evaluates every possible insertion position, choosing the one that
minimizes a multi-term cost:

1. **Same-node constraint crossings** — penalizes crossed src/dst pairs.
2. **Cross-node bracket intrusion** — penalizes cross-node edges that fall
   inside same-node bracket spans.
3. **Bracket span** — penalizes large src-to-dst distances.
4. **Inter-node bipartite crossings** — for same-domain pairs, penalizes
   parallel ordering (prefers chiasm); for cross-domain pairs, penalizes
   inverted ordering (prefers parallel).
5. **Edge-length proximity** — pulls properties toward their cross-node
   endpoints (above=top, below=bottom), weighted as a tiebreaker.

Sifting makes non-local moves in a single step (unlike bubble sort which can
only swap adjacent elements), allowing it to escape local minima. The sifting
heuristic is from BDD variable reordering (Matuszewski et al., GD 1999).

##### position_of: Fractional Position for Ordering

The `position_of` function returns the fractional position of a property port
within its layer's ordering. This is an abstract rank used during crossing
minimization, not a pixel coordinate. For a node at layer position _i_, with
properties ordered top-to-bottom, property _j_ has position:

```
position_of(prop) = node_layer_position + prop_index_within_node / (num_properties_in_node + 1)
```

This fractional position ensures that property-level edges are ordered correctly
relative to node-level positions.

#### 4.2.4 Phase 4: Coordinate Assignment

Coordinate assignment uses the **Brandes-Köpf algorithm** (Brandes & Köpf,
2001), which runs in linear time. The algorithm produces x-coordinates that
minimize total edge length while respecting minimum spacing, with a strong
preference for straight inner segments (vertical alignment of connected nodes).

##### Node Dimensions

Node dimensions are computed before layout:

- **Node height**: `HEADER_HEIGHT + (property_count × ROW_HEIGHT)`
- **Node width**: `max(header_text_width, max(property_name_widths)) + CONTENT_PAD × 2`
- **Uniform domain width**: Within a domain, all nodes use the same width:
  `max(width(n) for n in domain.members)`. This produces clean vertical
  alignment of corridors and node edges within each domain. Top-level nodes
  (not in any domain) retain their individual content-driven widths.
- **Derivation height**: `PILL_HEIGHT` (same as a single property row)
- **Derivation width**: `label_width + PILL_CONTENT_PAD × 2`

##### Sizing Constants

All values are multiples of the 4px base unit. All pixel dimensions (sizes,
padding, radii, stroke widths, font sizes) are even. Coordinates derived from
even sizes are even.

These constants are also used by the rendering pipeline in [RENDERING.md](RENDERING.md).

| Constant                 | Value | Description                                              |
| ------------------------ | ----- | -------------------------------------------------------- |
| `BASE_UNIT`              | 4px   | Grid base unit; all spacing values are multiples of this |
| `HEADER_HEIGHT`          | 32px  | Node header (pad 12 + cap-height 8 + pad 12)             |
| `ROW_HEIGHT`             | 20px  | Each property row                                        |
| `CONTENT_PAD`            | 12px  | Horizontal padding inside nodes (left/right)             |
| `DOT_RADIUS`             | 2px   | Problem indicator dot radius                             |
| `INTER_NODE_GAP`         | 28px  | Vertical gap between nodes in same column                |
| `NODE_H_SPACING`         | 40px  | Minimum horizontal gap between nodes in same layer       |
| `LAYER_V_SPACING`        | 48px  | Vertical gap between node layers                         |
| `DERIV_V_SPACING`        | 24px  | Vertical gap for derivation layers                       |
| `DOMAIN_TITLE_HEIGHT`    | 32px  | Domain title area (pad 12 + cap-height 8 + pad 12)       |
| `CORRIDOR_PAD`           | 8px   | Padding from corridor edge to channel center             |
| `CHANNEL_GAP`            | 4px   | Padding between adjacent channels in a corridor          |
| `PILL_HEIGHT`            | 20px  | Derivation pill height (matches row height)              |
| `PILL_CONTENT_PAD`       | 12px  | Horizontal padding inside derivation pill (left/right)   |
| `EDGE_SPACING`           | 8px   | Parallel edge offset in shared channels                  |
| `STUB_LENGTH`            | 20px  | Cross-domain constraint stub arrow length                |
| `ARROWHEAD_SIZE`         | 6px   | All arrowheads are 6×6; path endpoint offset             |
| `TITLE_FONT_SIZE`        | 12px  | Node title text                                          |
| `PROP_FONT_SIZE`         | 10px  | Property name text (monospace)                           |
| `DOMAIN_FONT_SIZE`       | 10px  | Domain label text                                        |
| `ANCHOR_LABEL_SIZE`      | 8px   | Anchor edge label text                                   |
| `CONSTRAINT_LABEL_SIZE`  | 6px   | Constraint edge label text                               |
| `PILL_FONT_SIZE`         | 8px   | Derivation pill label text (monospace)                   |

Corridor widths are computed from channel count: single-channel =
`CORRIDOR_PAD + CORRIDOR_PAD` = 16px. Dual-channel =
`CORRIDOR_PAD + CHANNEL_GAP + CORRIDOR_PAD` = 20px.

##### Brandes-Köpf Algorithm

The algorithm has three phases: vertical alignment, horizontal compaction, and
balancing.

**Phase A: Vertical alignment (4 passes)**

Each pass picks a direction pair (upper/lower × left/right) and builds maximal
vertical chains of aligned elements:

```
function vertical_alignment(layers, edges, direction) → (root[], align[]):
    // direction is one of: (upper-left, upper-right, lower-left, lower-right)
    // "upper" means we scan from top layer to bottom; "left" means we prefer
    // leftmost median neighbor.

    root[v] = v for all v    // each element is its own root
    align[v] = v for all v   // no alignment yet

    for each layer k in scan_order(direction):
        // For each element in this layer, find its "median" neighbor
        // in the adjacent fixed layer
        r = -1  // rightmost aligned position so far (prevents conflicts)

        for each element v in layers[k] in left_right_order(direction):
            neighbors = adjacent_elements(v, fixed_layer, edges)
            if neighbors is empty: continue

            // Pick median neighbor(s)
            medians = median_indices(neighbors, direction)

            for m in medians:
                if align[m] == m:  // m is not yet aligned
                    pos = position_of(m, fixed_layer)
                    if pos > r:    // no conflict with prior alignment
                        align[m] = v
                        root[v] = root[m]
                        align[v] = root[v]
                        r = pos
```

**Phase B: Horizontal compaction**

For each of the 4 alignments, compute x-coordinates by placing aligned chains
and respecting minimum spacing:

```
function horizontal_compaction(layers, root, align) → x[]:
    // Process elements in topological order of alignment chains
    sink[v] = v for all v
    shift[v] = ∞ for all v
    x[v] = undefined for all v

    // Place roots
    for each element v in layer order:
        if root[v] == v:  // v is a root of its chain
            place_block(v, layers, root, align, sink, shift, x)

    // Apply shifts for class merging
    for each element v in layer order:
        x[v] = x[root[v]]
        if shift[sink[root[v]]] < ∞:
            x[v] += shift[sink[root[v]]]

function place_block(v, layers, root, align, sink, shift, x):
    if x[v] is defined: return
    x[v] = 0
    w = v
    loop:
        if w has a left neighbor pred in its layer:
            place_block(root[pred], ...)
            if sink[v] == v: sink[v] = sink[root[pred]]
            if sink[v] != sink[root[pred]]:
                shift[sink[root[pred]]] = min(shift[sink[root[pred]]],
                    x[v] - x[root[pred]] - min_separation(pred, w))
            else:
                x[v] = max(x[v], x[root[pred]] + min_separation(pred, w))
        w = align[w]
        if w == v: break
```

The `min_separation` function computes the minimum horizontal distance between
two adjacent elements in the same layer:

```
function min_separation(left, right) → f64:
    return width(left) / 2 + NODE_H_SPACING + width(right) / 2
```

**Phase C: Balancing**

Run all four passes, producing four candidate x-coordinate arrays. The final
x-coordinate for each element is the median of its four candidates (after
shifting each array so that minimum x = 0):

```
function balanced_coordinates(layers, edges) → x[]:
    x_ul = compact(align(layers, edges, upper_left))
    x_ur = compact(align(layers, edges, upper_right))
    x_ll = compact(align(layers, edges, lower_left))
    x_lr = compact(align(layers, edges, lower_right))

    // Normalize: shift each so min = 0
    for each arr in [x_ul, x_ur, x_ll, x_lr]:
        min_val = min(arr)
        for each v: arr[v] -= min_val

    // Final coordinate: average of two middle values (median of four)
    for each element v:
        sorted = sort([x_ul[v], x_ur[v], x_ll[v], x_lr[v]])
        x[v] = (sorted[1] + sorted[2]) / 2

    return x
```

##### Y-Coordinate Assignment

Y-coordinates are determined directly from layer assignment:

```
function assign_y_coordinates(layers) → y[]:
    y_offset = 0
    for each layer k:
        if layer_is_empty(k) or layer_is_segment_only(k):
            continue   // collapsed empty layer or virtual routing layer
        else if layer_is_derivation(k):
            // Derivation layer: shorter spacing
            for each item in layers[k]:
                y[item] = y_offset
            y_offset += DERIV_V_SPACING + ROW_HEIGHT
        else:
            // Node layer: spacing based on tallest node
            max_height = max(height(node) for node in layers[k])
            for each item in layers[k]:
                y[item] = y_offset
            y_offset += INTER_NODE_GAP + max_height
    return y
```

##### Derivation Centering

After Brandes-Köpf produces x-coordinates, derivation nodes are centered
horizontally on the mean x-position of their input ports. This is a post-pass
adjustment:

```
for each derivation d:
    input_xs = [x[port_of(inp)] for inp in d.inputs]
    x[d] = mean(input_xs)
    // Resolve conflicts: if centering overlaps a neighbor, shift minimally
```

##### Tree Centering Post-processing

After Brandes-Köpf and derivation centering, a tree centering post-pass adjusts
node x-positions so that each parent is visually centered over its anchor-tree
children:

1. **Bottom-up centering**: Process layers from deepest to shallowest. Leaf
   nodes keep their Brandes-Köpf positions. For each node with anchor-tree
   children, set its center X to the mean of its children's center X values.
2. **Left-to-right spacing sweep**: Within each layer, sort nodes by X and
   enforce a minimum gap of `NODE_H_SPACING`. If a node was pushed into a
   neighbor by the centering step, shift it right to maintain clearance. This
   preserves the crossing-minimized order from Phase 3.
3. **Re-normalize**: Shift all node positions so that the leftmost node starts
   at x = 0.

```
function tree_center_nodes(node_layouts, graph, layers):
    w = uniform_node_width(graph)

    // Bottom-up pass
    for layer in reversed(layers):
        for node n in layer:
            children = [c for c in anchor_children(n)]
            if children is empty: continue
            centers = sorted([x[c] + w/2 for c in children])
            mean_center = (centers[0] + centers[-1]) / 2
            x[n] = mean_center - w/2

    // Spacing sweep per layer
    for layer in layers:
        nodes_in_layer = sorted nodes by x
        for i in 1..len(nodes_in_layer):
            prev_right = x[nodes_in_layer[i-1]] + w
            if x[nodes_in_layer[i]] < prev_right + NODE_H_SPACING:
                x[nodes_in_layer[i]] = prev_right + NODE_H_SPACING

    // Re-normalize
    min_x = min(x[n] for all n)
    if min_x < 0: shift all x by -min_x
```

##### Port Side Assignment

After coordinate assignment, each edge endpoint is assigned to either the left
or right port of its property. This step requires final x-coordinates and runs
between coordinate assignment and edge routing.

```
function assign_port_sides(graph, x) → side[]:
    for each edge e in graph:
        (upstream, downstream) = layout_endpoints(e)

        match e:
            Anchor { .. }:
                // Anchors use center top/bottom ports, not left/right.
                // No side assignment needed; skip.
                continue
            DerivInput { source_prop, target_deriv, .. }:
                // Both upstream (source property) and downstream (derivation)
                // get side assignments based on relative horizontal position.
                // Inputs enter the derivation pill from the left or right,
                // using corridor-based H-V-H routing like constraints.
                src_cx = x[property_node(upstream)] + width(property_node(upstream)) / 2
                tgt_cx = derivation_layout[target_deriv].x
                       + derivation_layout[target_deriv].width / 2
                if src_cx < tgt_cx:
                    side[e, Upstream] = Right    // exit right toward derivation
                    side[e, Downstream] = Left   // enter derivation from left
                else if src_cx > tgt_cx:
                    side[e, Upstream] = Left
                    side[e, Downstream] = Right
                else:
                    side[e, Upstream] = alternating  // per-node counter
                    side[e, Downstream] = same_side  // mirror on derivation
            _:
                // Constraint edges use property ports.
                src_node = property_node(upstream)
                tgt_node = property_node(downstream)

        if src_node == tgt_node:
            // Intra-node edge: force opposite sides
            side[e, Upstream] = Left
            side[e, Downstream] = Right
        else:
            src_cx = x[src_node] + width(src_node) / 2
            tgt_cx = x[tgt_node] + width(tgt_node) / 2

            if src_cx < tgt_cx:
                side[e, Upstream] = Right   // source faces right toward target
                side[e, Downstream] = Left  // target faces left toward source
            else if src_cx > tgt_cx:
                side[e, Upstream] = Left
                side[e, Downstream] = Right
            else:
                // Directly above/below (intra-column): both use left port
                // so the router can produce a clean H-V-H bracket on the left side
                side[e, Upstream] = Left
                side[e, Downstream] = Left
    return side
```

Intra-node edges are estimated at 1–3% of all constraints. They are handled as a
special case with forced opposite-side assignment. No iterative side-assignment
refinement is needed at this frequency.

#### 4.2.5 Phase 5: Domain Bounding Boxes, Columnar Layout, and Vertical Compaction

After coordinate assignment, each domain's bounding box is computed from its
member nodes with padding:

- **Top**: `DOMAIN_TITLE_HEIGHT` (32px) for the title area (label left-aligned)
- **Bottom**: `INTER_NODE_GAP` (28px) matching inter-node spacing
- **Left/Right**: `DOMAIN_PADDING + CORRIDOR_PAD * 2` (26px) to accommodate
  intra-domain routing corridors

##### Phase 5b: Columnar Domain Layout

Domains are assigned to **columns** based on the cross-domain edge topology,
creating dedicated gap corridors between columns for cross-domain edge routing.

1. **Build cross-domain adjacency**: For each edge connecting nodes in
   different domains, record an adjacency between those domains.

2. **Assign columns**: The **hub domain** (most cross-domain neighbors) anchors
   column 0. BFS from the hub alternates column assignment: direct neighbors go
   to column 1, their unvisited neighbors to column 0, etc. **Satellite
   domains** (connected to exactly one other domain) join their sole neighbor's
   column rather than alternating. Unconnected domains go to column 0.

3. **Size the gap corridor dynamically**: Count the number of **cross-domain**
   edges (`n`) — including same-column cross-domain edges, not just cross-column
   ones. Same-column cross-domain edges route through corridor zones shared with
   their endpoint domains and must be counted. The count uses a sweep-line over
   vertical extents to find the maximum simultaneous overlap. Gap width =
   `CORRIDOR_PAD * 2 + max(0, n - 1) * CHANNEL_GAP`. For a single edge: 16px.
   For 12 edges: 60px.

4. **Outer corridor**: Column 0 starts at `CORRIDOR_PAD * 2` (16px) instead
   of x=0, creating a left-edge corridor for edges that exit leftward.

5. **Reposition nodes**: Shift each domain's member nodes so they fall within
   their assigned column. Domains narrower than the column width are centered.

6. **Recompute bounding boxes** from the repositioned node coordinates.

##### Phase 5c: Vertical Compaction

After compound layer assignment + Brandes-Köpf, nodes are correctly ordered
but vertical spacing is inflated by empty gap layers between domains. The
vertical compaction pass collapses this excess spacing.

The structural model for vertical layout within each column is a flat
sequence of elements separated by inter-domain corridors:

```
inter-domain corridor (gap)
<domain A: title | node | gap | node | gap | derivation | gap | node>
inter-domain corridor (gap = INTER_NODE_GAP)
<cross-domain derivation pill>
inter-domain corridor (gap = INTER_NODE_GAP)
<domain B: title | node | gap | node>
inter-domain corridor (gap)
<free node>
inter-domain corridor (gap)
```

**Algorithm**:

1. Identify which column each element belongs to (by x-center proximity to
   domain column centers).
2. Collect all vertical elements per column: domains (as blocks), free nodes,
   and cross-domain derivations.
3. Sort each column's elements by current y-center.
4. Walk top-to-bottom: the first element keeps its position; each subsequent
   element is placed at `previous_bottom + INTER_NODE_GAP`.
5. When shifting a domain, all its member nodes shift by the same delta.
6. Recompute domain bounding boxes from final node positions.

This replaces the previous pair-wise domain overlap resolution and
cross-domain derivation repositioning passes.

#### 4.2.6 Phase 6: Edge Routing — Orthogonal, Corridor-Based

All edges are routed using **orthogonal (right-angle) routing** through
**corridors** — fixed-width zones between node edges and domain boundaries.
The orthogonal style was selected for maximum visual order, symmetry, and
clarity with record-style port-based nodes. Edge types are distinguished by
line style, not geometry.

##### Corridor Model

A **corridor** is the space between a node edge and its domain boundary. Every
domain has a left and right corridor. An **inter-domain corridor** exists
between every pair of adjacent domains (or between a domain and the canvas
edge), behaving identically to intra-domain corridors.

Each corridor contains one or more vertical **channels**. Channels have
`CORRIDOR_PAD` (8px) padding to corridor edges and `CHANNEL_GAP` (4px)
padding between adjacent channels:

- Single-channel corridor: `CORRIDOR_PAD + CORRIDOR_PAD` = 16px wide.
  Channel at center.
- Dual-channel corridor: `CORRIDOR_PAD + CHANNEL_GAP + CORRIDOR_PAD` = 20px
  wide. Two channels at offset positions.

The channel count for each corridor is the minimum needed to avoid edge
collisions (overlapping vertical extents) within the corridor's y-range. Two
edges may share a channel if their vertical extents do not overlap.

##### Corridor Merging for Same-Column Domains

When multiple domains occupy the same column, their left corridors share the
same x-range (as do their right corridors). Rather than creating separate
corridor objects with identical x-ranges — which would allow independent
channel allocation and cause collisions — corridors at the same x-range are
**merged** into a single corridor that tracks multiple domain owners.

```rust
struct Corridor {
    x_start: f64,
    x_end: f64,
    domain_ids: Vec<DomainId>,  // all domains sharing this corridor
    channels: Vec<Channel>,
}
```

The `merge_or_create_corridor` function checks whether a corridor already
exists at the given x-range. If so, the new domain is added to the existing
corridor's `domain_ids` list. If not, a new corridor is created. This ensures
that edges from different domains in the same column share channel state and
never allocate the same vertical channel position.

##### Corridor Routing Invariant

> **No cross-domain edge may use an intra-domain corridor.** Cross-domain
> edges must always route through the inter-column gap corridor, regardless
> of whether the endpoints' domains are in the same column or different
> columns.

This invariant is absolute. Even when two domains are vertically adjacent
in the same column and share a merged corridor, cross-domain edges between
them must exit to the inter-column gap. The longer horizontal segments are
the accepted cost of visual clarity.

##### Channel Collision Invariant

> **No two edges may share the same vertical channel x-coordinate with
> overlapping y-ranges**, except for consecutive center-port Anchor edges
> that share a common node endpoint.

The center-port exemption covers one case:

1. **Anchor chains**: A parent→child→grandchild anchor chain naturally
   overlaps at the child node's center x.

DerivInput edges use side-based corridor routing (entering derivation pills
from the left or right at mid-height), so they no longer share center-port
channels and are not exempt from the collision invariant.

##### Routing Data Structures

```rust
struct Route {
    edge_id: EdgeId,
    segments: Vec<Segment>,   // ordered from source port to target port
}

enum Segment {
    Horizontal { y: f64, x_start: f64, x_end: f64 },
    Vertical { x: f64, y_start: f64, y_end: f64 },
}

struct Corridor {
    x_start: f64,            // left edge of corridor
    x_end: f64,              // right edge of corridor
    domain_ids: Vec<DomainId>,  // owning domains (merged for same-column domains)
    channels: Vec<Channel>,  // vertical channels within this corridor
}

struct Channel {
    x: f64,                  // x-position of this vertical channel
    occupants: Vec<(EdgeId, f64, f64)>,  // (edge, y_start, y_end) — vertical extents
}
```

##### Edge Layout Direction Mapping

The `Edge` enum uses different field names per variant. For layout and routing,
every edge needs a uniform upstream/downstream interpretation. The following
accessor defines the mapping:

```
function layout_endpoints(edge) → (upstream_element, downstream_element):
    match edge:
        Anchor { parent, child, .. }:
            upstream = parent          // NodeId — use anchor_port_bottom_y
            downstream = child         // NodeId — use anchor_port_top_y
        Constraint { source_prop, dest_prop, .. }:
            upstream = source_prop     // PropId — use port_left_x/port_right_x, port_y
            downstream = dest_prop     // PropId
        DerivInput { source_prop, target_deriv, .. }:
            upstream = source_prop     // PropId
            downstream = target_deriv  // DerivId — use derivation left/right/top
```

"Upstream" is the higher layer (lower y-coordinate); "downstream" is the lower
layer (higher y-coordinate). This matches the trust flow direction: sources at
top, destinations at bottom.

##### Port Position Dispatch

The routing algorithm calls `port_position(endpoint)` for each edge endpoint.
Because Anchors connect nodes (at center top/bottom) while Constraints and
DerivInputs connect properties (at left/right ports), `port_position` dispatches
by edge type:

```
function port_position(edge, endpoint_role) → (x, y):
    match edge:
        Anchor { parent, child, .. }:
            node = if endpoint_role == Upstream then parent else child
            x = anchor_port_x(node)           // node.x + node.width / 2
            y = if endpoint_role == Upstream:
                    anchor_port_bottom_y(node) // parent: bottom edge
                else:
                    anchor_port_top_y(node)    // child: top edge

        Constraint { source_prop, dest_prop, .. }:
            prop = if endpoint_role == Upstream then source_prop else dest_prop
            side = port_side_assignment[edge, endpoint_role]
            x = if side == Left then port_left_x(prop.node) else port_right_x(prop.node)
            y = port_y(prop.node, prop.index)

        DerivInput { source_prop, target_deriv, .. }:
            if endpoint_role == Upstream:
                // Upstream is a property port
                side = port_side_assignment[edge, Upstream]
                x = if side == Left then port_left_x(source_prop.node)
                    else port_right_x(source_prop.node)
                y = port_y(source_prop.node, source_prop.index)
            else:
                // Downstream connects to derivation pill at assigned side
                d = derivation_layout[target_deriv]
                side = port_side_assignment[edge, Downstream]
                if side == Left:
                    x = d.x                    // left edge of pill
                    y = d.y + d.height / 2     // mid-height
                else if side == Right:
                    x = d.x + d.width          // right edge of pill
                    y = d.y + d.height / 2     // mid-height
                else:
                    x = d.x + d.width / 2      // top center (fallback)
                    y = d.y
```

##### Routing Algorithm

The router operates in a layered graph where the y-positions of layers and the
x-positions of nodes are already fixed. Corridors provide vertical channels;
the inter-node gaps provide horizontal routing space.

```
function route_all_edges(graph, x, y, node_dims, domains) → Vec<Route>:
    routes = []

    // 1. Build corridors from domain and node geometry
    corridors = []
    for each domain d:
        // Left corridor: between domain left edge and leftmost node
        left_cor = Corridor {
            x_start: d.x,
            x_end: d.x + corridor_width(left_edge_count),
            channels: allocate_channels(...)
        }
        // Right corridor: between rightmost node and domain right edge
        right_cor = Corridor {
            x_start: d.x + d.width - corridor_width(right_edge_count),
            x_end: d.x + d.width,
            channels: allocate_channels(...)
        }
        corridors.push(left_cor, right_cor)

    // Inter-domain corridors (between adjacent domains)
    for each pair of adjacent domains (d1, d2):
        gap_cor = Corridor {
            x_start: d1.x + d1.width,
            x_end: d2.x,
            channels: allocate_channels(...)
        }
        corridors.push(gap_cor)

    // 2. Route edges by priority
    edges_by_priority = sort_edges(graph, [Anchor=0, DerivInput=1, Constraint=2])

    for each edge e in edges_by_priority:
        (src_x, src_y) = port_position(e, Upstream)
        (tgt_x, tgt_y) = port_position(e, Downstream)

        match e:
            Anchor { .. }:
                src_side = None
                tgt_side = None
            _:
                src_side = Some(port_side_assignment[e, Upstream])
                tgt_side = Some(port_side_assignment[e, Downstream])

        // 3. Determine domain affinity for corridor selection
        (src_domain, tgt_domain) = domain_affinity(e, graph, domain_layouts)

        route = route_single_edge(
            src_x, src_y, src_side,
            tgt_x, tgt_y, tgt_side,
            src_domain, tgt_domain,
            corridors
        )
        reserve_channel(route, e.id, corridors)
        routes.push(route)

    return routes
```

##### Domain Affinity for Corridor Selection

Each edge is assigned a **domain affinity** that determines which corridor
it may use for its vertical segments. The `find_best_corridor_idx` function
uses this affinity to select corridors whose `domain_ids` match the edge's
affinity.

```text
function domain_affinity(edge, graph) → (Option<DomainId>, Option<DomainId>):
    src_domain = domain of source node
    tgt_domain = domain of target node

    if src_domain == tgt_domain:
        return (src_domain, tgt_domain)     // Intra-domain: use own corridors
    else:
        return (None, None)                 // Cross-domain: use inter-column gap corridor
```

Two cases:

1. **Intra-domain**: Both endpoints in the same domain. The edge uses that
   domain's own corridor channels.

2. **Cross-domain**: Endpoints in different domains. The edge always routes
   through the inter-column gap corridor, regardless of whether the domains
   are in the same column or different columns.

##### Single Edge Routing

Edges route as orthogonal doglegs: exit horizontally from a port, travel
vertically along a corridor channel, then enter horizontally at the target
port. The corridor side (left or right) is chosen to minimize crossings and
avoid collisions.

```text
function find_corridor_channel(port_x, port_side, corridors) → Channel:
    // Search in the direction the port faces — ensures edges exit horizontally
    // away from the node body before turning vertically.
    if port_side == Left:
        corridor = nearest corridor to the left of port_x
    else:
        corridor = nearest corridor to the right of port_x
    return first available channel in corridor (no vertical overlap)
```

For a single edge between a source port and target port:

```
function route_single_edge(src_x, src_y, src_side,
                           tgt_x, tgt_y, tgt_side,
                           src_domain, tgt_domain,
                           corridors) → Route:
    // src_side and tgt_side are Option<Side>.
    // Anchors pass None (center top/bottom ports); Constraints/DerivInputs pass Some(Left|Right).

    // Case 1: Center-port edges (Anchors) — no side-directed horizontal exit
    if src_side is None:
        if src_x == tgt_x:
            // Vertically aligned: straight drop
            return Route { segments: [Vertical { x: src_x, y_start: src_y, y_end: tgt_y }] }
        else:
            // Offset horizontally: vertical down, horizontal across, vertical down
            mid_y = midpoint between source and target layers
            segments = [
                Vertical { x: src_x, y_start: src_y, y_end: mid_y },
                Horizontal { y: mid_y, x_start: src_x, x_end: tgt_x },
                Vertical { x: tgt_x, y_start: mid_y, y_end: tgt_y },
            ]
            return Route { segments: collapse_zero_length(segments) }

    // Case 2a: Intra-column constraint — both on same corridor side
    // Produces a clean H-V-H "bracket" through the corridor
    if src and tgt use the same corridor:
        ch = find_corridor_channel(src_x, src_side, corridors)
        segments = [
            Horizontal { y: src_y, x_start: src_x, x_end: ch.x },
            Vertical { x: ch.x, y_start: src_y, y_end: tgt_y },
            Horizontal { y: tgt_y, x_start: ch.x, x_end: tgt_x },
        ]
        return Route { segments: collapse_zero_length(segments) }

    // Case 2b: Cross-corridor routing — H-V-H or H-V-H-V-H dogleg
    // 1. Horizontal from source port to source corridor channel
    ch_src = find_corridor_channel(src_x, src_side, corridors)
    // 2. Vertical through source corridor
    // 3. Horizontal across to target corridor channel (if different)
    ch_tgt = find_corridor_channel(tgt_x, tgt_side, corridors)
    // 4. Vertical to target y
    // 5. Horizontal to target port

    segments = [
        Horizontal { y: src_y, x_start: src_x, x_end: ch_src.x },
        Vertical { x: ch_src.x, y_start: src_y, y_end: tgt_y },
        Horizontal { y: tgt_y, x_start: ch_src.x, x_end: tgt_x },
    ]
    // When source and target corridors differ, insert horizontal jog
    // between corridors. Collapse zero-length segments.
    return Route { segments: collapse_zero_length(segments) }
```

For **long edges** (spanning multiple layers), the route passes through multiple
horizontal channels. Each intermediate layer contributes an additional
vertical-horizontal-vertical jog if the edge must navigate around obstacles.

##### Stub Generation for Hidden Cross-Domain Constraints

Visibility semantics for stubs: see [RENDERING.md](RENDERING.md).

```
function generate_stub(route) → StubParts:
    // Extract the last STUB_LENGTH pixels from the destination end of the route
    tail_segments = extract_tail(route.segments, STUB_LENGTH)
    half = total_length(tail_segments) / 2
    // Split into dotted (fading, farther from dest) and solid (near dest port)
    (dotted, solid) = split_at(tail_segments, half)
    return StubParts { solid, dotted }
```

The stub appears at the **destination** end of the edge, split into two SVG
paths: a solid segment closest to the destination port and a dotted segment
that fades away from it. This solid-to-dotted transition creates a visual
"fading" effect that is distinct from the dashed styling used for invalid edges.
The solid half carries the arrowhead marker.

The full route and the stub parts are all emitted as SVG paths. The full route
has `opacity: 0` by default; stubs are visible. JavaScript toggles between
them.

##### Edge Bundling (Deferred)

Edge bundling — merging shared segments of edges with common source or target
nodes into a single visual line that splits at junction points — is a quality
optimization that reduces visual clutter. It is not required for correctness and
should be implemented after the core routing is working. When implemented,
bundling operates as a post-pass over the computed routes, merging collinear
segments that share a channel.

##### Pre-Computation

All edge routes are pre-computed, including constraint edges that are initially
hidden. No geometry computation occurs at runtime in the browser. The layout
engine outputs every edge as a complete SVG path.

##### Arrowhead Clearance

Because markers use `refX="0"`, the arrowhead base is placed at the path
endpoint and drawn forward. All arrowheads are 6×6, so the path generation
step must shorten the final segment of each route by 6px so the arrowhead
tip lands exactly at the target boundary. This prevents the stroke from
bleeding past the arrowhead.

##### Visual Styling

| Edge Type                      | Line Style     | Width | Color                   | Default Visibility |
| ------------------------------ | -------------- | ----- | ----------------------- | ------------------ |
| Anchor                         | Solid          | 2px   | State-based (green/red) | Always visible     |
| Derivation edge                | Solid / Dashed | 1px   | State-based (blue/red)  | Always visible     |
| Intra-domain constraint        | Solid / Dashed | 1px   | State-based (blue/red)  | Always visible     |
| Cross-domain constraint (full) | Solid / Dashed | 1px   | State-based (blue/red)  | Hidden             |
| Cross-domain constraint (stub) | Solid→Dotted fade / Dashed (invalid) | 1px | State-based (blue/red) | Always visible |

Derivation edges use the same valid/invalid visual language as constraints:
blue solid when valid, red dashed when invalid. Valid edges are always solid;
invalid edges are always dashed. All arrowheads are 6×6. Edge types are
distinguished by color and stroke width (anchors: 2px, constraints: 1px),
not by arrow size.

##### Edge Labels

Anchor edges and constraint edges display their operation name as a label.
Labels are positioned along the vertical corridor segment of the edge route,
offset horizontally from the channel to avoid overlapping the edge line.
Anchor labels use the same color as the anchor edge. Constraint labels use
the same color as the constraint edge (blue for valid, red for invalid).
Derivation input edges carry no label — the derivation pill's operation name
provides sufficient context.

## 5. Output Contract

This section defines the output types produced by the layout pipeline and consumed by the rendering pipeline ([RENDERING.md](RENDERING.md)).

The layout engine outputs per-node constraint data for the runtime:

```rust
struct NodeLayout {
    id: NodeId,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    // Property y-offsets are computed from y + HEADER_HEIGHT + index * ROW_HEIGHT
}

struct DerivLayout {
    id: DerivId,
    x: f64,              // center of pill
    y: f64,              // center of pill
    width: f64,
    height: f64,         // PILL_HEIGHT
}

struct EdgePath {
    edge_id: EdgeId,
    svg_path: String,     // SVG path data, e.g. "M10,20 L10,50 L30,50"
}

struct DomainLayout {
    id: DomainId,
    display_name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// A single stub with solid (near destination) and dotted (fading) halves.
struct StubPath {
    edge_id: EdgeId,
    solid_svg: String,            // SVG path for the solid half near destination
    dotted_svg: String,           // SVG path for the dotted half fading away
}

/// A derivation chain groups the full derivation path (all input edges +
/// output edge) for atomic visibility toggling.
struct DerivChain {
    deriv_id: DerivId,
    participants: Vec<NodeId>,    // all nodes with properties in this chain
    full_paths: Vec<EdgePath>,    // all input edges + output edge
    stub_paths: Vec<StubPath>,    // stubs at destination property ports
}

struct LayoutResult {
    nodes: Vec<NodeLayout>,
    derivations: Vec<DerivLayout>,
    domains: Vec<DomainLayout>,
    anchors: Vec<EdgePath>,
    intra_domain_constraints: Vec<EdgePath>,
    cross_domain_constraints: Vec<CrossDomainPaths>,
    cross_domain_deriv_chains: Vec<DerivChain>,
}

struct CrossDomainPaths {
    participants: Vec<NodeId>,    // source + target node IDs
    full_path: EdgePath,
    stub_paths: Vec<StubPath>,    // one stub per endpoint
}
```

---

## See Also

- [OVERVIEW.md](OVERVIEW.md) — High-level architecture and goals
- [GRAPH_MODEL.md](GRAPH_MODEL.md) — Graph data structures and type definitions
- [SYNTAX.md](SYNTAX.md) — Input syntax and parsing
- [RENDERING.md](RENDERING.md) — SVG rendering pipeline
- [WORKED_EXAMPLE.md](WORKED_EXAMPLE.md) — End-to-end walkthrough
