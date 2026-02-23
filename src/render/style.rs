/// Inline CSS for obgraph SVG elements (RENDERING.md §6.8).

/// Returns the complete inline CSS for an obgraph SVG.
pub fn css() -> &'static str {
    r#"
.obgraph {
  /* Base */
  --obg-bg: #f8fafc;
  --obg-text: #334155;
  --obg-text-muted: #64748b;
  --obg-border: #e2e8f0;
  --obg-border-strong: #cbd5e1;

  /* Node */
  --obg-node-bg: white;
  --obg-header-bg: #f1f5f9;
  --obg-header-text: #334155;

  /* State indicators (problems-only) */
  --obg-anchor-valid: #22c55e;
  --obg-constraint-valid: #60a5fa;
  --obg-problem: #ef4444;

  /* Selection */
  --obg-select-ring: #475569;

  /* Domains */
  --obg-domain-bg: white;
  --obg-domain-border: #cbd5e1;
  --obg-domain-label: #64748b;

  /* Derivation pills */
  --obg-pill-bg: #f8fafc;
  --obg-pill-border: #cbd5e1;
  --obg-pill-text: #64748b;

  background: #ffffff;
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 10px;
  color: var(--obg-text);
  overflow: visible;
}

/* Domain background and label */
.obgraph-domain-bg {
  fill: var(--obg-domain-bg);
  stroke: var(--obg-domain-border);
  stroke-width: 2px;
}

.obgraph-domain-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 10px;
  font-weight: 600;
  fill: var(--obg-domain-label);
  paint-order: stroke;
  stroke: white;
  stroke-width: 3px;
  stroke-linejoin: round;
}

/* Anchor edges — green, 2px, solid when valid */
.obgraph-anchor {
  fill: none;
  stroke: var(--obg-anchor-valid);
  stroke-width: 2px;
}

/* Invalid anchor edges — red dashed, 2px */
.obgraph-anchor-invalid {
  fill: none;
  stroke: var(--obg-problem);
  stroke-dasharray: 4 2;
  stroke-width: 2px;
}

/* Derivation edges — blue (same as constraints) */
.obgraph-deriv-edge {
  fill: none;
  stroke: var(--obg-constraint-valid);
  stroke-width: 1px;
}

/* Intra-domain constraint — blue, 1px */
.obgraph-constraint {
  fill: none;
  stroke: var(--obg-constraint-valid);
  stroke-width: 1px;
}

/* Invalid constraint/derivation edges — red dashed, 1px */
.obgraph-constraint-invalid {
  fill: none;
  stroke: var(--obg-problem);
  stroke-dasharray: 4 2;
  stroke-width: 1px;
}

/* Cross-domain constraint full path — hidden by default */
.obgraph-constraint-full {
  fill: none;
  stroke: var(--obg-constraint-valid);
  stroke-width: 1px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.15s ease;
}

.obgraph-constraint-full.obgraph-active {
  opacity: 1;
  pointer-events: auto;
}

/* Cross-domain constraint stub */
.obgraph-constraint-stub {
  fill: none;
  stroke: var(--obg-constraint-valid);
  stroke-width: 1px;
  stroke-dasharray: 4 3;
  transition: opacity 0.15s ease;
}

.obgraph-constraint-stub.obgraph-hidden {
  opacity: 0;
  pointer-events: none;
}

/* Node background rect — white with light border and shadow */
.obgraph-node-bg {
  fill: var(--obg-node-bg);
  stroke: none;
  filter: url(#shadow);
}

/* Node header background (rounded-top rect) */
.obgraph-node-header {
  fill: var(--obg-header-bg);
}

/* Node header fill (square-bottom rect overlapping lower half of header) */
.obgraph-node-header-fill {
  fill: var(--obg-header-bg);
}

/* Node border: 1px default, 2px dark when selected */
.obgraph-node-border {
  stroke: var(--obg-border);
  stroke-width: 1px;
}

.obgraph-node[data-selected="true"] .obgraph-node-border {
  stroke: var(--obg-select-ring);
  stroke-width: 2px;
}

/* Node title text */
.obgraph-node-title {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 12px;
  font-weight: 600;
  fill: var(--obg-header-text);
}

/* Separator line between title and properties */
.obgraph-node-sep {
  stroke: var(--obg-border);
  stroke-width: 1px;
}

/* Property row background */
.obgraph-prop-bg {
  fill: transparent;
}

/* Property name text — monospace */
.obgraph-prop-name {
  fill: var(--obg-text-muted);
  font-size: 10px;
  font-family: Menlo, Consolas, monospace;
}

/* Trusted property name */
.obgraph-prop[data-trust="trusted"] .obgraph-prop-name {
  fill: var(--obg-text);
}

/* Always-trusted / constrained property name */
.obgraph-prop[data-trust="always"] .obgraph-prop-name,
.obgraph-prop[data-trust="constrained"] .obgraph-prop-name {
  fill: var(--obg-text);
}

/* Critical property — bold */
.obgraph-prop[data-critical="true"] .obgraph-prop-name {
  font-weight: 700;
}

/* Property row divider */
.obgraph-prop-divider {
  stroke: var(--obg-border);
  stroke-width: 0.5px;
}

/* Problem indicator dots */
.obgraph-node-dot, .obgraph-prop-dot {
  fill: var(--obg-problem);
}

/* Derivation pill shape */
.obgraph-pill {
  fill: var(--obg-pill-bg);
  stroke: var(--obg-pill-border);
  stroke-width: 1px;
}

/* Derivation pill label — monospace */
.obgraph-pill-label {
  fill: var(--obg-pill-text);
  font-size: 8px;
  text-anchor: middle;
  font-family: Menlo, Consolas, monospace;
}

/* Arrowhead fills are set directly on marker <path> elements via fill attr.
   CSS vars inside <marker> have spotty cross-browser support. */

/* Edge operation labels */
.obgraph-anchor-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 8px;
  fill: #16a34a;
  paint-order: stroke;
  stroke: white;
  stroke-width: 3px;
  stroke-linejoin: round;
}

.obgraph-constraint-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 6px;
  fill: #2563eb;
  paint-order: stroke;
  stroke: white;
  stroke-width: 3px;
  stroke-linejoin: round;
}
"#
}
