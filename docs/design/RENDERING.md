# Rendering, Interactivity, and Visual Design

This document specifies the rendering output (SVG structure, CSS styling,
JavaScript interactivity) and visual design for obgraph diagrams. It covers
the interactivity model (cross-domain edge visibility, hover/select behavior)
and all aspects of SVG rendering (element hierarchy, coordinate system, node
and edge rendering, color palette).

The rendering pipeline reads state flags computed by the propagation algorithm
([GRAPH_MODEL.md](GRAPH_MODEL.md) §2.2, §2.6): `anchored` (node), `verified`
(node, emergent), `critical` (property), `constrained` (property). Visual
indicators follow a problems-only principle — red dots appear only where
something is wrong.

Layout coordinates and edge paths are produced by the layout pipeline. The
`LayoutResult` struct is defined in [LAYOUT.md](LAYOUT.md) §5.

A pixel-perfect design mockup is available at
`../../examples/design_mockup.html`.

---

## 5. Interactivity Model

### 5.1 Cross-Domain Edge Definition

A constraint edge is **cross-domain** if its two property endpoints belong to
nodes in different domains, or if either node is top-level (not in any domain).
Constraints between two nodes in the same domain are **intra-domain**.

This means:

- Node in Domain A → Node in Domain B: cross-domain
- Node in Domain A → top-level node: cross-domain
- Top-level node → top-level node: cross-domain
- Node in Domain A → Node in Domain A: intra-domain

### 5.2 Visibility States

| Element                       | Default        | On Hover         | On Select          |
| ----------------------------- | -------------- | ---------------- | ------------------ |
| Anchor                        | Visible        | Visible          | Visible            |
| Intra-domain constraint       | Visible        | Visible          | Visible            |
| Cross-domain constraint       | **Stub arrow** | Full path shown  | Full path toggled  |
| Cross-domain derivation chain | **Stubs only** | Full chain shown | Full chain toggled |
| Derivation pill               | Visible        | Visible          | Visible            |
| Intra-domain derivation edge  | Visible        | Visible          | Visible            |

Cross-domain constraint stubs are short arrow segments arriving at the
destination port, extracted from the last STUB_LENGTH pixels of the full route.
Each stub is split into two visual halves: a solid segment nearest the
destination port and a dotted segment that fades away. This solid-to-dotted
transition is visually distinct from the dashed styling used for invalid edges.
Stubs indicate to the reader that additional edges exist without showing the
full route.

#### 5.2.1 Derivation Chain Visibility

When a constraint's source expression involves a derivation, the derivation's
input edges, the derivation pill, and the output constraint edge form a
**derivation chain**. For visibility purposes, a derivation chain is treated as
a single atomic unit.

Each derivation chain has a **participant set**: the set of all nodes that have
properties feeding into or receiving from the derivation. If any node in the
participant set is active (hovered or selected), the entire chain is revealed —
all input edges, the pill's connections, and the output edge.

The derivation pill itself is **always visible** as a landmark, indicating that
a derivation exists. Only its connecting edges are toggled.

When all edges in the chain are intra-domain, the chain is always visible (no
stubs needed). When any edge crosses a domain boundary, the chain follows
cross-domain visibility rules.

### 5.3 Interaction Behavior

- **Hover**: temporarily shows all cross-domain constraint full paths and
  derivation chains where the hovered node is a participant. On mouse leave,
  reverts unless the node is also selected.
- **Click/select**: toggles the node's selected state. While selected, all
  cross-domain constraint full paths and derivation chains involving that node
  remain visible. Clicking again deselects and hides them.
- **Background click**: clicking the SVG background clears all selections.
- **Union semantics**: hover and click are combined with OR. A full path or
  chain is visible whenever any of its participant nodes is hovered or selected.
  Stubs are hidden under the same condition.
- **Default selected**: nodes annotated `@selected` in the input syntax start in
  the selected state with their cross-domain constraints and derivation chains
  visible. See [SYNTAX.md](SYNTAX.md) for annotation syntax.

### 5.4 JavaScript Requirements

The JavaScript footprint is minimal. All geometry is pre-computed. The runtime
code maintains two `Set`s of active node IDs (hovered and selected) and
toggles CSS classes to show or hide edges:

```javascript
var selected = new Set();
var hovered = new Set();

function isActive(id) { return selected.has(id) || hovered.has(id); }

function updateEdges() {
  // Full paths: visible when any participant node is active.
  fullPaths.forEach((p) => {
    var participants = p.dataset.participants.split(",");
    var active = participants.some(isActive);
    p.classList.toggle("obgraph-active", active);
  });
  // Derivation chains: toggled atomically when any participant is active.
  derivChains.forEach((g) => {
    var participants = g.dataset.participants.split(",");
    var active = participants.some(isActive);
    g.classList.toggle("obgraph-active", active);
  });
  // Stubs: hidden when any participant node is active.
  stubs.forEach((p) => {
    var participants = p.dataset.participants.split(",");
    var hide = participants.some(isActive);
    p.classList.toggle("obgraph-hidden", hide);
  });
}

nodes.forEach((node) => {
  var id = node.dataset.node;
  node.addEventListener("mouseenter", () => { hovered.add(id); updateEdges(); });
  node.addEventListener("mouseleave", () => { hovered.delete(id); updateEdges(); });
  node.addEventListener("click", (e) => {
    e.stopPropagation();
    selected.has(id) ? selected.delete(id) : selected.add(id);
    node.dataset.selected = selected.has(id);
    updateEdges();
  });
});

// Background click clears all selections.
svg.addEventListener("click", () => { selected.clear(); updateEdges(); });

// Apply initial state for @selected nodes.
updateEdges();
```

The key change from simple source/target lookups to a `data-participants`
attribute enables derivation chains: a single derivation chain element lists
all participating node IDs, so activating any one reveals the whole chain.

Visibility transitions use CSS `opacity` and `transition: opacity 0.15s ease`
rather than `display` toggling, so the browser animates the fade smoothly.

> **Note:** The pre-computed data structure (`LayoutResult` and associated
> types) is defined in [LAYOUT.md](LAYOUT.md) §5.

## 6. Rendering

### 6.1 Output Format

The preprocessor emits a self-contained block of inline SVG with embedded CSS
and minimal JavaScript, replacing the `obgraph` code block in the markdown
source. No external dependencies are required.

### 6.2 SVG Element Hierarchy

The SVG uses a layered z-order via group ordering. Groups listed first are drawn
first (behind later groups).

```xml
<div class="obgraph-container">
  <svg xmlns="http://www.w3.org/2000/svg"
       viewBox="0 0 {width} {height}"
       width="100%" preserveAspectRatio="xMidYMin meet"
       class="obgraph">
    <style>/* embedded CSS — see 6.8 */</style>

    <!-- Reusable definitions (markers, filters) -->
    <defs>
      <!-- Drop shadow for nodes -->
      <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
        <feDropShadow dx="0" dy="2" stdDeviation="2" flood-color="#00000018"/>
      </filter>
      <!-- Anchor arrowhead: 6×6, refX=0 (base at path endpoint) -->
      <marker id="arrow-anchor-valid" viewBox="0 0 6 6" refX="0" refY="3"
              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6"
              orient="auto">
        <path d="M0,0 L6,3 L0,6 Z" fill="var(--obg-anchor-valid)"/>
      </marker>
      <marker id="arrow-anchor-invalid" viewBox="0 0 6 6" refX="0" refY="3"
              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6"
              orient="auto">
        <path d="M0,0 L6,3 L0,6 Z" fill="var(--obg-problem)"/>
      </marker>
      <!-- Constraint/derivation arrowhead: 6×6, refX=0 (base at path endpoint) -->
      <marker id="arrow-constraint-valid" viewBox="0 0 6 6" refX="0" refY="3"
              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6"
              orient="auto">
        <path d="M0,0 L6,3 L0,6 Z" fill="var(--obg-constraint-valid)"/>
      </marker>
      <marker id="arrow-constraint-invalid" viewBox="0 0 6 6" refX="0" refY="3"
              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6"
              orient="auto">
        <path d="M0,0 L6,3 L0,6 Z" fill="var(--obg-problem)"/>
      </marker>
    </defs>

    <!-- Layer 0: domain backgrounds (behind everything) -->
    <g class="obgraph-domains">
      <g class="obgraph-domain" data-domain="{index}">
        <rect class="obgraph-domain-bg" x="" y="" width="" height="" rx="10"/>
        <text class="obgraph-domain-label" x="" y="" text-anchor="start">{display_name}</text>
      </g>
    </g>

    <!-- Layer 1: edges (behind nodes) -->
    <g class="obgraph-edges">
      <!-- Anchors (always visible) -->
      <g class="obgraph-anchors">
        <path class="obgraph-anchor" d="M... L..." data-edge="{id}"
              marker-end="url(#arrow-anchor-valid)"/>
        <!-- Anchor label (operation name) -->
        <text class="obgraph-anchor-label" x="" y="">{operation}</text>
      </g>
      <!-- Intra-domain constraint edges (always visible) -->
      <g class="obgraph-constraints-intra">
        <path class="obgraph-constraint" d="M... L..." data-edge="{id}"
              marker-end="url(#arrow-constraint-valid)"/>
        <!-- Constraint label (operation name, positioned along vertical corridor segment) -->
        <text class="obgraph-constraint-label" x="" y="">{operation}</text>
      </g>
      <!-- Cross-domain constraint full paths (hidden by default via CSS) -->
      <g class="obgraph-constraints-cross">
        <path class="obgraph-constraint-full" d="M... L..."
              data-edge="{id}" data-participants="{node_id},{node_id}"
              marker-end="url(#arrow-constraint-valid)"/>
        <text class="obgraph-constraint-label" x="" y="">{operation}</text>
      </g>
      <!-- Cross-domain derivation chains (edges hidden by default, toggled atomically) -->
      <g class="obgraph-deriv-chains">
        <g class="obgraph-deriv-chain" data-deriv="{id}"
           data-participants="{node_id},{node_id},...">
          <!-- All input edges + output edge for this derivation -->
          <path class="obgraph-constraint-full" d="M... L..."
                marker-end="url(#arrow-constraint-valid)"/>
        </g>
      </g>
      <!-- Stubs (visible by default, hidden when full path is shown) -->
      <!-- Each stub has two paths: dotted (fading) and solid (near destination port) -->
      <g class="obgraph-constraint-stubs">
        <path class="obgraph-constraint-stub obgraph-stub-dotted" d="M... L..."
              data-edge="{id}" data-participants="{node_id},{node_id}"/>
        <path class="obgraph-constraint-stub obgraph-stub-solid" d="M... L..."
              data-edge="{id}" data-participants="{node_id},{node_id}"
              marker-end="url(#arrow-constraint-valid)"/>
      </g>
    </g>

    <!-- Layer 2: derivation pills (always visible as landmarks) -->
    <g class="obgraph-derivations">
      <g class="obgraph-derivation" data-deriv="{id}">
        <rect class="obgraph-pill" x="" y="" width="" height="20" rx="10"/>
        <text class="obgraph-pill-label" x="" y=""
              font-family="Menlo,Consolas,monospace">{operation}</text>
      </g>
    </g>

    <!-- Layer 3: nodes (on top) -->
    <g class="obgraph-nodes">
      <g class="obgraph-node" data-node="{id}" data-selected="{bool}">
        <!-- Shadow rect behind node -->
        <rect class="obgraph-node-bg" x="" y="" width="" height="" rx="8"
              filter="url(#shadow)"/>
        <!-- Header background (always neutral).
             Two overlapping rects produce rounded-top / square-bottom corners:
             the first rect has rx="8" covering the full header height, and a
             second flat rect covers the bottom portion where header meets
             property rows. -->
        <rect class="obgraph-node-header" x="" y="" width="" height="32"
              rx="8" fill="var(--obg-header-bg)"/>
        <rect class="obgraph-node-header-fill" x="" y="{header_y + 16}"
              width="" height="16" fill="var(--obg-header-bg)"/>
        <text class="obgraph-node-title" x="" y="">{display_name}</text>
        <!-- Unanchored dot (only present when node is unanchored) -->
        <circle class="obgraph-node-dot" cx="" cy="" r="2"
                fill="var(--obg-problem)"/>
        <!-- Property rows -->
        <g class="obgraph-prop" data-prop="{id}">
          <rect class="obgraph-prop-bg" x="" y="" width="" height="20"/>
          <line class="obgraph-prop-divider" x1="" y1="" x2="" y2=""/>
          <text class="obgraph-prop-name" x="" y=""
                font-family="Menlo,Consolas,monospace">{name}</text>
          <!-- Problem dot (only present when critical + unconstrained) -->
          <circle class="obgraph-prop-dot" cx="" cy="" r="2"
                  fill="var(--obg-problem)"/>
        </g>
        <!-- Border: 1px default, 2px dark when selected -->
        <rect class="obgraph-node-border" x="" y="" width="" height="" rx="8"
              fill="none"/>
      </g>
    </g>

    <script>/* embedded JS — see section 5.4 */</script>
  </svg>
</div>
```

### 6.3 Coordinate System

The SVG coordinate system has origin (0, 0) at the top-left. Positive x is
right, positive y is down. All coordinates are in CSS pixels.

The `viewBox` dimensions are computed from the bounding box of all elements plus
a global margin (default 20px). The `<svg>` element uses `width="100%"` and
`preserveAspectRatio="xMidYMin meet"` to scale responsively within the mdbook
page.

### 6.4 Port Positions

Port positions are computed from the node's coordinates and property index.
Ports are invisible attachment points at the node boundary — no port circles
are rendered. Port formulas depend on the layout constants defined in
[LAYOUT.md](LAYOUT.md) §1.

```
// Property ports (for constraints and derivation edges)
port_left_x(node)  = node.x
port_right_x(node) = node.x + node.width
port_y(node, prop_index) = node.y + HEADER_HEIGHT + prop_index * ROW_HEIGHT + ROW_HEIGHT / 2

// Anchor ports (for hierarchical anchor edges between nodes)
// Anchors attach at the center of the node's top or bottom edge.
anchor_port_x(node) = node.x + node.width / 2
anchor_port_top_y(node) = node.y                        // incoming anchor (from parent)
anchor_port_bottom_y(node) = node.y + node_height(node)  // outgoing anchor (to child)
```

When a side of a property row has k connections, the row height is divided
into k+1 equal segments, and ports are placed at each segment boundary. Port
positions are rounded to even pixel values to maintain grid alignment. Port
distribution is dynamic per-row per-side.

Anchors always connect the bottom of the parent to the top of the child (since
parents are in higher layers). Edge paths connect to these exact port positions.

### 6.5 Node Rendering

Nodes are rendered as record-style rectangles:

```
┌──────────────────┐
│   Display Name   │  ← header (neutral background, always #f1f5f9)
├──────────────────┤
  property.one       ← property row (no visible port circles)
  property.two
  property.three
└──────────────────┘
```

- Header contains the display name (or ident if no display name). Header
  background is always neutral (`#f1f5f9`). The header has rounded top corners
  (`rx=8`) but square bottom corners where it meets property rows; this is
  achieved with two overlapping rects. An unanchored node shows a red problem
  dot (`DOT_RADIUS`) in the header; anchored nodes show no dot.
- Each property is a row. Property names use monospace font. Critical
  properties use bold text; informational properties use normal weight.
  A critical + unconstrained property shows a red problem dot; all other
  states show no dot. There are no visible port circles — ports are implicit
  edge attachment points at the node boundary.

### 6.6 Derivation Rendering

Derivations are rendered as **rounded pills** — small rectangles with fully
rounded ends (`rx = PILL_HEIGHT / 2`), height `PILL_HEIGHT`, with the
operation name centered in monospace text. They are visually subordinate to
regular nodes: smaller, no property rows. Pill width accommodates the label
text plus `PILL_CONTENT_PAD` on each side.

Derivation pills have no status dot. Their operational status is communicated
entirely by the validity colors of their input and output edges (blue when
valid, red dashed when invalid — same visual language as constraints).

Derivation nodes accept input and output edges on all four sides. The output
edge typically exits from the bottom. These visual attachment points are a
rendering convention — the data model tracks inputs and output via
`Derivation.inputs` and `Derivation.output_prop`, not per-side ports.
Derivation input edges carry no label — the derivation pill's operation name
is sufficient context.

When any input or output of a derivation crosses a domain boundary, the
derivation pill is placed outside all domains. Pills are centered horizontally
under their input nodes when possible.

### 6.7 Domain Rendering

Domains are rendered as labeled rounded rectangles (`rx="10"`,
`stroke-width="2"`, `stroke: var(--obg-domain-border)`) drawn behind their
member nodes. Each domain has a title area of `DOMAIN_TITLE_HEIGHT`
(pad 12 + cap-height 8 + pad 12 = 32px) at the top. The domain title label
is positioned at the **top-left corner** (left-aligned with `CONTENT_PAD`
inset) to avoid collisions with anchor edges that pass through the top-center
of the domain. The gap after the last node to the domain bottom edge matches
`INTER_NODE_GAP`. Corridors (the space between node edges and domain
boundaries) provide routing channels for edges.

### 6.8 Color Palette and State Visualization

The visual system follows a **problems-only** principle: the default state has
no visual indicator. Red marks appear only where something is wrong.
Verified/unverified is never shown directly on a node — it is emergent from
whether any of its critical properties have problem indicators.

Colors use CSS custom properties for theme adaptability:

```css
.obgraph {
  /* Base */
  --obg-bg: #f8fafc;
  --obg-text: #334155;
  --obg-text-muted: #64748b;
  --obg-border: #e2e8f0;
  --obg-border-strong: #cbd5e1;

  /* Node */
  --obg-node-bg: white;
  --obg-header-bg: #f1f5f9;       /* always neutral regardless of state */
  --obg-header-text: #334155;

  /* State indicators (problems-only) */
  --obg-anchor-valid: #22c55e;     /* green — valid anchor edge */
  --obg-constraint-valid: #60a5fa; /* blue — valid constraint/derivation edge */
  --obg-problem: #ef4444;          /* red — unmet slot (node or property dot, invalid edge) */

  /* Selection */
  --obg-select-ring: #475569;      /* dark ring around selected node */

  /* Domains */
  --obg-domain-bg: white;
  --obg-domain-border: #cbd5e1;
  --obg-domain-label: #64748b;

  /* Derivation pills */
  --obg-pill-bg: #f8fafc;
  --obg-pill-border: #cbd5e1;
  --obg-pill-text: #64748b;
}
```

State indicator rendering rules (problems-only):

| Element          | Condition                        | Rendering                                             |
| ---------------- | -------------------------------- | ----------------------------------------------------- |
| Node header      | always                           | `fill: var(--obg-header-bg)` (neutral, never changes) |
| Node dot         | anchored                         | absent (no indicator needed)                          |
| Node dot         | unanchored                       | red dot `fill: var(--obg-problem)`                    |
| Property text    | critical                         | bold (`font-weight: 700`)                             |
| Property text    | not critical                     | normal (`font-weight: 400`)                           |
| Property dot     | critical + unconstrained         | red dot `fill: var(--obg-problem)`                    |
| Property dot     | critical + constrained           | absent (no problem)                                   |
| Property dot     | not critical (any state)         | absent (never shown)                                  |
| Anchor edge      | valid                            | solid `stroke: var(--obg-anchor-valid)`, width 2      |
| Anchor edge      | invalid                          | dashed `stroke: var(--obg-problem)`, width 2          |
| Constraint edge  | valid                            | solid `stroke: var(--obg-constraint-valid)`, width 1  |
| Constraint edge  | invalid                          | dashed `stroke: var(--obg-problem)`, width 1          |
| Derivation edge  | valid                            | solid `stroke: var(--obg-constraint-valid)`, width 1  |
| Derivation edge  | invalid                          | dashed `stroke: var(--obg-problem)`, width 1          |
| Derivation pill  | always                           | no dot; status shown by edge colors only              |

Node and property state is determined per [GRAPH_MODEL.md](GRAPH_MODEL.md) §2.6.
The key requirement is that problems are visually apparent without interaction.

#### Selection Ring

A selected node is indicated by a 2px dark stroke ring
(`var(--obg-select-ring)`) fully inset inside the node rect boundary. The ring
rect is inset by 1px so the stroke's outer edge aligns with the node boundary.
Edge endpoints target the rect boundary and land cleanly against the ring with
no overlap. Unselected nodes use a 1px light border (`var(--obg-border)`) on
the rect boundary.

#### Additional CSS Classes

| Class                                     | Purpose                                                                                                 |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `.obgraph-constraint-full`                | Cross-domain full path. `opacity: 0` by default; `transition: opacity 0.15s ease` for smooth animation. |
| `.obgraph-constraint-full.obgraph-active` | Added by JS when a participating node is hovered or selected. Sets `opacity: 1`.                        |
| `.obgraph-constraint-stub`                | Base stub class. Visible by default; `transition: opacity 0.15s ease` for smooth animation.             |
| `.obgraph-constraint-stub.obgraph-stub-solid`  | Solid half of stub (nearest destination port). No dash array. Carries arrowhead marker.            |
| `.obgraph-constraint-stub.obgraph-stub-dotted` | Dotted half of stub (fading away). Uses `stroke-dasharray: 2 2`.                                  |
| `.obgraph-constraint-stub.obgraph-hidden` | Added by JS when a participating node is hovered or selected. Sets `opacity: 0`.                        |
| `.obgraph-deriv-chain`                    | Derivation chain group (all edges in a cross-domain derivation). Toggled atomically.                    |

Property names and derivation pill labels use a monospace font stack
(`Menlo, Consolas, monospace`) for visual distinction from node titles.

Domain labels use `paint-order: stroke` with a white halo to remain legible
when rendered over domain background rectangles.

---

## See Also

- [OVERVIEW.md](OVERVIEW.md) — High-level project overview and terminology
- [GRAPH_MODEL.md](GRAPH_MODEL.md) — Graph data model, state flags, and propagation algorithm
- [SYNTAX.md](SYNTAX.md) — Input syntax and parsing rules
- [LAYOUT.md](LAYOUT.md) — Layout algorithm, coordinate computation, and `LayoutResult` data structure
- [WORKED_EXAMPLE.md](WORKED_EXAMPLE.md) — End-to-end worked example
