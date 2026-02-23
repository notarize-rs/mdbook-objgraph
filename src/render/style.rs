/// Inline CSS for obgraph SVG elements (RENDERING.md §6.8).

/// Returns the complete inline CSS for an obgraph SVG.
pub fn css() -> &'static str {
    r#"
.obgraph {
  --obg-bg: #f8fafc;
  --obg-text: #334155;
  --obg-text-muted: #64748b;
  --obg-border: #e2e8f0;
  --obg-node-bg: white;
  --obg-header-bg: #f1f5f9;
  --obg-anchor-valid: #22c55e;
  --obg-anchor-invalid: #ef4444;
  --obg-constraint-valid: #60a5fa;
  --obg-constraint-invalid: #ef4444;
  --obg-problem: #ef4444;
  --obg-select-ring: #475569;
  --obg-domain-bg: white;
  --obg-domain-border: #cbd5e1;
  --obg-pill-bg: #f8fafc;
  --obg-pill-border: #cbd5e1;
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
  stroke-width: 1px;
}

.obgraph-domain-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 10px;
  font-weight: 600;
  fill: var(--obg-text-muted);
}

/* Anchor edges — green, 2px */
.obgraph-link, .obgraph-anchor {
  fill: none;
  stroke: var(--obg-anchor-valid);
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

/* Cross-domain constraint full path — blue, hidden by default */
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

/* Cross-domain constraint stub — blue */
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

/* Invalid edges — red dashed */
.obgraph-link-invalid, .obgraph-anchor-invalid {
  fill: none;
  stroke: var(--obg-problem);
  stroke-dasharray: 4 2;
  stroke-width: 2px;
}

.obgraph-constraint-invalid {
  fill: none;
  stroke: var(--obg-problem);
  stroke-dasharray: 4 2;
  stroke-width: 1px;
}

/* Link-source property highlight */
.obgraph-link-source {
  fill: #dcfce7;
}

/* Node background rect — white with light border */
.obgraph-node-bg {
  fill: var(--obg-node-bg);
  stroke: var(--obg-border);
  stroke-width: 1px;
  filter: url(#shadow);
}

/* Node header background */
.obgraph-node-header {
  fill: var(--obg-header-bg);
}

/* Node title text */
.obgraph-node-title {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 12px;
  font-weight: 600;
  fill: var(--obg-text);
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

/* Property name text */
.obgraph-prop-name {
  fill: var(--obg-text-muted);
  font-size: 10px;
  font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace;
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
.obgraph-prop[data-trust="critical"] .obgraph-prop-name,
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
.obgraph-deriv-shape {
  fill: var(--obg-pill-bg);
  stroke: var(--obg-pill-border);
  stroke-width: 1px;
}

/* Derivation label */
.obgraph-deriv-label {
  fill: var(--obg-text-muted);
  font-size: 10px;
  text-anchor: middle;
  font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace;
}

/* Arrowhead markers — use direct fill, not CSS vars (SVG marker limitation) */
.obgraph-arrow-link, .obgraph-arrow-anchor {
  fill: #22c55e;
}

.obgraph-arrow-constraint {
  fill: #60a5fa;
}

.obgraph-arrow-constraint-cross {
  fill: #60a5fa;
}

/* Edge operation labels */
.obgraph-link-label, .obgraph-anchor-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 10px;
  fill: #16a34a;
  paint-order: stroke;
  stroke: white;
  stroke-width: 3px;
  stroke-linejoin: round;
}

.obgraph-constraint-label {
  font-family: 'Inter', 'Segoe UI', system-ui, sans-serif;
  font-size: 10px;
  fill: #2563eb;
  paint-order: stroke;
  stroke: white;
  stroke-width: 3px;
  stroke-linejoin: round;
}

/* Selected node highlight — 2px dark inset ring */
.obgraph-node[data-selected="true"] .obgraph-node-bg {
  stroke: var(--obg-select-ring);
  stroke-width: 2px;
}
"#
}
