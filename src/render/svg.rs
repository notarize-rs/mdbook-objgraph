//! SVG element generation (DESIGN.md §6.2).

use std::fmt::Write;

use crate::layout::{
    LayoutResult, CONTENT_PAD, DOMAIN_TITLE_HEIGHT, DOT_RADIUS, HEADER_HEIGHT, PILL_HEIGHT,
    ROW_HEIGHT,
};
use crate::model::state::StateResult;
use crate::model::types::{Edge, EdgeId, Graph, NodeId};

use super::interactivity;
use super::style;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate the complete SVG string for a laid-out graph.
pub fn generate_svg(graph: &Graph, layout: &LayoutResult, state: &StateResult) -> String {
    let mut out = String::new();

    // Outer wrapper div
    writeln!(out, r#"<div class="obgraph-container">"#).unwrap();

    // SVG root element
    writeln!(
        out,
        r#"  <svg xmlns="http://www.w3.org/2000/svg""#
    )
    .unwrap();
    writeln!(
        out,
        r#"       viewBox="0 0 {width} {height}""#,
        width = layout.width,
        height = layout.height
    )
    .unwrap();
    writeln!(
        out,
        r#"       width="{width}" height="{height}""#,
        width = layout.width,
        height = layout.height
    )
    .unwrap();
    writeln!(out, r#"       class="obgraph">"#).unwrap();

    // Embedded CSS — blank lines removed so mdbook's markdown parser
    // doesn't break out of inline HTML mode.
    let css: String = style::css()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    writeln!(out, "    <style>\n{css}\n    </style>").unwrap();

    // Reusable definitions (markers, filters)
    write_defs(&mut out);

    // Global margin: translate all content inward.
    // content_offset_x adds extra horizontal shift when edge labels extend
    // past the left edge of the content area.
    let margin_x = crate::layout::GLOBAL_MARGIN + layout.content_offset_x;
    let margin_y = crate::layout::GLOBAL_MARGIN;
    writeln!(
        out,
        r#"    <g transform="translate({mx}, {my})">"#,
        mx = margin_x,
        my = margin_y,
    )
    .unwrap();

    // Layer 0: domain backgrounds
    write_domains(&mut out, layout);

    // Layer 1: edges
    write_edges(&mut out, graph, layout, state);

    // Layer 2: nodes
    write_nodes(&mut out, graph, layout, state);

    // Close global margin group
    writeln!(out, r#"    </g>"#).unwrap();

    // Embedded JS — blank lines removed for same reason as CSS above.
    let js: String = interactivity::js()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    writeln!(out, "    <script>\n{js}\n    </script>").unwrap();

    writeln!(out, r#"  </svg>"#).unwrap();
    writeln!(out, r#"</div>"#).unwrap();

    out
}

// ---------------------------------------------------------------------------
// Layer 0: domain backgrounds
// ---------------------------------------------------------------------------

fn write_domains(out: &mut String, layout: &LayoutResult) {
    writeln!(out, r#"    <g class="obgraph-domains">"#).unwrap();

    for domain in &layout.domains {
        writeln!(
            out,
            r#"      <g class="obgraph-domain" data-domain="{index}">"#,
            index = domain.id.0
        )
        .unwrap();

        writeln!(
            out,
            r#"        <rect class="obgraph-domain-bg" x="{x}" y="{y}" width="{w}" height="{h}" rx="10"/>"#,
            x = domain.x,
            y = domain.y,
            w = domain.width,
            h = domain.height
        )
        .unwrap();

        // Label at the top-left corner of the domain, with left padding.
        // Positioned to avoid collisions with edges passing through the top-center.
        let label_x = domain.x + CONTENT_PAD;
        let label_y = domain.y + DOMAIN_TITLE_HEIGHT / 2.0;
        writeln!(
            out,
            r#"        <text class="obgraph-domain-label" x="{x}" y="{y}" text-anchor="start" dominant-baseline="central">{name}</text>"#,
            x = label_x,
            y = label_y,
            name = escape_xml(&domain.display_name)
        )
        .unwrap();

        writeln!(out, r#"      </g>"#).unwrap();
    }

    writeln!(out, r#"    </g>"#).unwrap();
}

// ---------------------------------------------------------------------------
// Edge path rendering helper
// ---------------------------------------------------------------------------

/// Write an edge path element with optional label.
///
/// `class_prefix` is e.g. "obgraph-anchor" — appended with "-invalid" when invalid.
/// `marker_prefix` is e.g. "arrow-anchor" — appended with "-valid"/"-invalid".
/// `valid_color` is the label fill when the edge is valid (e.g. green or blue).
fn write_edge_path(
    out: &mut String,
    ep: &crate::layout::EdgePath,
    valid: bool,
    class_prefix: &str,
    marker_prefix: &str,
    valid_color: &str,
) {
    let (class, marker) = if valid {
        (class_prefix.to_string(), format!("{marker_prefix}-valid"))
    } else {
        (format!("{class_prefix}-invalid"), format!("{marker_prefix}-invalid"))
    };
    writeln!(
        out,
        r#"        <path class="{class}" d="{d}" data-edge="{id}" marker-end="url(#{marker})"/>"#,
        class = class,
        d = ep.svg_path,
        id = ep.edge_id.0,
        marker = marker,
    )
    .unwrap();
    if let Some(lbl) = &ep.label {
        let label_fill = if valid { valid_color } else { "#ef4444" };
        writeln!(
            out,
            r##"        <text class="{cls}-label" x="{x}" y="{y}" fill="{fill}" text-anchor="{anchor}" dominant-baseline="central">{text}</text>"##,
            cls = class_prefix,
            x = lbl.x, y = lbl.y, fill = label_fill, anchor = lbl.anchor, text = escape_xml(&lbl.text)
        ).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Layer 1: edges
// ---------------------------------------------------------------------------

/// Determine whether an edge is valid based on state propagation results.
///
/// - Anchor: valid if parent is anchored+verified.
/// - Constraint: valid if source node is anchored and source prop is constrained.
///
/// Note: constraints require only "anchored" (not "verified") on the source
/// node, matching the propagation algorithm.  The stronger "verified" gate
/// applies only to anchor edges (child anchoring).
fn is_edge_valid(edge_id: EdgeId, graph: &Graph, state: &StateResult) -> bool {
    let edge = &graph.edges[edge_id.index()];
    match edge {
        Edge::Anchor { parent, .. } => {
            state.is_node_anchored(*parent) && state.is_node_verified(graph, *parent)
        }
        Edge::Constraint { source_prop, .. } => {
            let src_node_id = graph.properties[source_prop.index()].node;
            state.is_node_anchored(src_node_id)
                && state.is_prop_constrained(*source_prop)
        }
    }
}

fn write_edges(out: &mut String, graph: &Graph, layout: &LayoutResult, state: &StateResult) {
    writeln!(out, r#"    <g class="obgraph-edges">"#).unwrap();

    // --- Anchor paths ---
    writeln!(out, r#"      <g class="obgraph-anchors">"#).unwrap();
    for ep in &layout.anchors {
        let valid = is_edge_valid(ep.edge_id, graph, state);
        write_edge_path(out, ep, valid, "obgraph-anchor", "arrow-anchor", "#22c55e");
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Intra-domain constraint paths ---
    writeln!(out, r#"      <g class="obgraph-constraints-intra">"#).unwrap();
    for ep in &layout.intra_domain_constraints {
        let valid = is_edge_valid(ep.edge_id, graph, state);
        write_edge_path(out, ep, valid, "obgraph-constraint", "arrow-constraint", "#60a5fa");
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Cross-domain constraint: full paths (hidden by CSS class by default) ---
    writeln!(out, r#"      <g class="obgraph-constraints-cross">"#).unwrap();
    for cross in &layout.cross_domain_constraints {
        let ep = &cross.full_path;
        let participants_str = participants_attr(&cross.participants);
        let props = props_attr(ep.edge_id, graph);
        let valid = is_edge_valid(ep.edge_id, graph, state);
        let (class, marker) = if valid {
            ("obgraph-constraint-full", "arrow-constraint-valid")
        } else {
            ("obgraph-constraint-full obgraph-constraint-invalid", "arrow-constraint-invalid")
        };
        writeln!(
            out,
            r#"        <path class="{class}" d="{d}" data-edge="{id}" data-participants="{p}" data-props="{props}" marker-end="url(#{marker})"/>"#,
            class = class,
            d = ep.svg_path,
            id = ep.edge_id.0,
            p = participants_str,
            props = props,
            marker = marker,
        )
        .unwrap();
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Cross-domain constraint: stub paths ---
    writeln!(out, r#"      <g class="obgraph-constraint-stubs">"#).unwrap();
    for cross in &layout.cross_domain_constraints {
        let participants_str = participants_attr(&cross.participants);
        let props = props_attr(cross.full_path.edge_id, graph);
        let valid = is_edge_valid(cross.full_path.edge_id, graph, state);
        let (stub_class, stub_marker) = if valid {
            ("obgraph-constraint-stub", "arrow-constraint-valid")
        } else {
            ("obgraph-constraint-stub obgraph-constraint-stub-invalid", "arrow-constraint-invalid")
        };
        for sp in &cross.stub_paths {
            if !sp.dotted_svg.is_empty() {
                writeln!(
                    out,
                    r#"        <path class="{cls} obgraph-stub-dotted" d="{d}" data-edge="{id}" data-participants="{p}" data-props="{props}" marker-end="url(#{marker})"/>"#,
                    cls = stub_class,
                    d = sp.dotted_svg,
                    id = sp.edge_id.0,
                    p = participants_str,
                    props = props,
                    marker = stub_marker,
                )
                .unwrap();
            }
        }
    }
    writeln!(out, r#"      </g>"#).unwrap();

    writeln!(out, r#"    </g>"#).unwrap();
}

/// Format a list of participant NodeIds as a comma-separated string for data-participants.
fn participants_attr(participants: &[NodeId]) -> String {
    participants
        .iter()
        .map(|n| n.0.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Format the property IDs connected by an edge as a comma-separated string for data-props.
fn props_attr(edge_id: EdgeId, graph: &Graph) -> String {
    let edge = &graph.edges[edge_id.index()];
    match edge {
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => format!("{},{}", source_prop.0, dest_prop.0),
        Edge::Anchor { .. } => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Layer 2: nodes
// ---------------------------------------------------------------------------

fn write_nodes(out: &mut String, graph: &Graph, layout: &LayoutResult, state: &StateResult) {
    writeln!(out, r#"    <g class="obgraph-nodes">"#).unwrap();

    for nl in &layout.nodes {
        let node = &graph.nodes[nl.id.index()];
        let node_anchored = state.is_node_anchored(nl.id);
        let node_verified = state.is_node_verified(graph, nl.id);
        let node_trust_attr = if node_verified { "trusted" } else { "untrusted" };
        let selected_attr = node.is_selected;

        if node.is_derivation() {
            // Derivation pill: rounded rect with centered label from property name.
            let pill_label = &graph.properties[node.properties[0].index()].name;
            let rx = PILL_HEIGHT / 2.0;
            writeln!(
                out,
                r#"      <g class="obgraph-node obgraph-pill" data-node="{id}" data-trust="{trust}" data-selected="{sel}">"#,
                id = nl.id.0,
                trust = node_trust_attr,
                sel = selected_attr
            ).unwrap();
            writeln!(
                out,
                r#"        <rect class="obgraph-pill-bg" x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>"#,
                x = nl.x,
                y = nl.y,
                w = nl.width,
                h = nl.height,
                rx = rx
            ).unwrap();
            writeln!(
                out,
                r#"        <text class="obgraph-pill-text" x="{x}" y="{y}" dominant-baseline="central" text-anchor="middle">{label}</text>"#,
                x = nl.x + nl.width / 2.0,
                y = nl.y + nl.height / 2.0,
                label = escape_xml(pill_label)
            ).unwrap();
            writeln!(out, r#"      </g>"#).unwrap();
        } else {
            // Regular node: header + property rows.
            writeln!(
                out,
                r#"      <g class="obgraph-node" data-node="{id}" data-selected="{sel}">"#,
                id = nl.id.0,
                sel = selected_attr
            )
            .unwrap();

            // Background rect — full node height
            writeln!(
                out,
                r#"        <rect class="obgraph-node-bg" x="{x}" y="{y}" width="{w}" height="{h}" rx="8"/>"#,
                x = nl.x,
                y = nl.y,
                w = nl.width,
                h = nl.height
            )
            .unwrap();

            // Header background: two overlapping rects for rounded-top / square-bottom.
            writeln!(
                out,
                r#"        <rect class="obgraph-node-header" x="{x}" y="{y}" width="{w}" height="{h}" rx="8"/>"#,
                x = nl.x,
                y = nl.y,
                w = nl.width,
                h = HEADER_HEIGHT
            )
            .unwrap();
            writeln!(
                out,
                r#"        <rect class="obgraph-node-header-fill" x="{x}" y="{y}" width="{w}" height="16"/>"#,
                x = nl.x,
                y = nl.y + 16.0,
                w = nl.width,
            )
            .unwrap();

            // Title text
            let title_x = nl.x + CONTENT_PAD;
            let title_y = nl.y + HEADER_HEIGHT / 2.0;
            writeln!(
                out,
                r#"        <text class="obgraph-node-title" x="{x}" y="{y}" dominant-baseline="central" data-trust="{trust}">{label}</text>"#,
                x = title_x,
                y = title_y,
                trust = node_trust_attr,
                label = escape_xml(node.label())
            )
            .unwrap();

            // Problem dot on header for unanchored nodes
            if !node_anchored {
                let dot_x = nl.x + nl.width - CONTENT_PAD - DOT_RADIUS;
                let dot_y = nl.y + HEADER_HEIGHT / 2.0;
                writeln!(
                    out,
                    r#"        <circle class="obgraph-node-dot" cx="{x}" cy="{y}" r="{r}"/>"#,
                    x = dot_x,
                    y = dot_y,
                    r = DOT_RADIUS
                )
                .unwrap();
            }

            // Separator line between title and properties
            let sep_y = nl.y + HEADER_HEIGHT;
            writeln!(
                out,
                r#"        <line class="obgraph-node-sep" x1="{x1}" y1="{y}" x2="{x2}" y2="{y}"/>"#,
                x1 = nl.x,
                x2 = nl.x + nl.width,
                y = sep_y
            )
            .unwrap();

            // Property rows
            for (prop_idx, &pid) in layout.property_order.props_of(node.id).iter().enumerate() {
                let prop = &graph.properties[pid.index()];
                let prop_constrained = state.is_prop_constrained(pid);

                let trust_attr = if prop.constrained {
                    "always"
                } else if prop_constrained {
                    "trusted"
                } else {
                    "untrusted"
                };

                let critical_attr = if prop.critical { "true" } else { "false" };

                let row_y = nl.y + HEADER_HEIGHT + prop_idx as f64 * ROW_HEIGHT;
                let port_y = row_y + ROW_HEIGHT / 2.0;

                writeln!(
                    out,
                    r#"        <g class="obgraph-prop" data-prop="{pid}" data-trust="{trust}" data-critical="{crit}">"#,
                    pid = pid.0,
                    trust = trust_attr,
                    crit = critical_attr
                )
                .unwrap();

                writeln!(
                    out,
                    r#"          <rect class="obgraph-prop-bg" x="{x}" y="{y}" width="{w}" height="{rh}"/>"#,
                    x = nl.x,
                    y = row_y,
                    w = nl.width,
                    rh = ROW_HEIGHT
                )
                .unwrap();

                let text_x = nl.x + CONTENT_PAD;
                let text_y = port_y;
                writeln!(
                    out,
                    r#"          <text class="obgraph-prop-name" x="{x}" y="{y}" dominant-baseline="central">{name}</text>"#,
                    x = text_x,
                    y = text_y,
                    name = escape_xml(&prop.name)
                )
                .unwrap();

                if prop.critical && !prop_constrained {
                    let dot_x = nl.x + nl.width - CONTENT_PAD - DOT_RADIUS;
                    let dot_y = port_y;
                    writeln!(
                        out,
                        r#"          <circle class="obgraph-prop-dot" cx="{x}" cy="{y}" r="{r}"/>"#,
                        x = dot_x,
                        y = dot_y,
                        r = DOT_RADIUS
                    )
                    .unwrap();
                }

                writeln!(out, r#"        </g>"#).unwrap();

                if prop_idx < node.properties.len() - 1 {
                    let div_y = row_y + ROW_HEIGHT;
                    writeln!(
                        out,
                        r#"        <line class="obgraph-prop-divider" x1="{x1}" y1="{y}" x2="{x2}" y2="{y}"/>"#,
                        x1 = nl.x,
                        x2 = nl.x + nl.width,
                        y = div_y
                    )
                    .unwrap();
                }
            }

            // Border rect
            writeln!(
                out,
                r#"        <rect class="obgraph-node-border" x="{x}" y="{y}" width="{w}" height="{h}" rx="7" fill="none"/>"#,
                x = nl.x + 1.0,
                y = nl.y + 1.0,
                w = nl.width - 2.0,
                h = nl.height - 2.0
            )
            .unwrap();

            writeln!(out, r#"      </g>"#).unwrap();
        }
    }

    writeln!(out, r#"    </g>"#).unwrap();
}

// ---------------------------------------------------------------------------
// Arrow marker defs
// ---------------------------------------------------------------------------

fn write_defs(out: &mut String) {
    writeln!(out, r#"    <defs>"#).unwrap();

    // Shadow filter for node cards
    writeln!(
        out,
        r##"      <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="2" stdDeviation="2" flood-color="#00000018"/></filter>"##
    )
    .unwrap();

    for (id, color) in [
        ("arrow-anchor-valid", "#22c55e"),
        ("arrow-anchor-invalid", "#ef4444"),
        ("arrow-constraint-valid", "#60a5fa"),
        ("arrow-constraint-invalid", "#ef4444"),
    ] {
        writeln!(
            out,
            r##"      <marker id="{id}" viewBox="0 0 6 6" refX="0" refY="3" markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6" orient="auto"><path d="M0,0 L6,3 L0,6 Z" fill="{color}"/></marker>"##,
            id = id, color = color,
        ).unwrap();
    }

    writeln!(out, r#"    </defs>"#).unwrap();
}

// ---------------------------------------------------------------------------
// XML escaping
// ---------------------------------------------------------------------------

/// Escape the five XML special characters in text content.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::layout::{CrossDomainPaths, DomainLayout, EdgePath, NodeLayout, StubPath};
    use crate::layout::crossing::PropertyOrder;
    use crate::model::state;
    #[allow(unused_imports)]
    use crate::model::types::{
        Domain, DomainId, Edge, EdgeId, Graph, Node, NodeId, Property, PropId,
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn build_prop_edges(edges: &[Edge]) -> HashMap<PropId, Vec<EdgeId>> {
        let mut map: HashMap<PropId, Vec<EdgeId>> = HashMap::new();
        for (i, edge) in edges.iter().enumerate() {
            let eid = EdgeId(i as u32);
            match edge {
                Edge::Constraint {
                    dest_prop,
                    source_prop,
                    ..
                } => {
                    map.entry(*dest_prop).or_default().push(eid);
                    map.entry(*source_prop).or_default().push(eid);
                }
                Edge::Anchor { .. } => {}
            }
        }
        map
    }

    fn build_node_adjacency(
        edges: &[Edge],
    ) -> (HashMap<NodeId, Vec<EdgeId>>, HashMap<NodeId, EdgeId>) {
        let mut children: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut parent: HashMap<NodeId, EdgeId> = HashMap::new();
        for (i, edge) in edges.iter().enumerate() {
            let eid = EdgeId(i as u32);
            if let Edge::Anchor {
                parent: p,
                child: c,
                ..
            } = edge
            {
                children.entry(*p).or_default().push(eid);
                parent.insert(*c, eid);
            }
        }
        (children, parent)
    }

    fn make_graph(
        nodes: Vec<Node>,
        properties: Vec<Property>,
        edges: Vec<Edge>,
        domains: Vec<Domain>,
    ) -> Graph {
        let prop_edges = build_prop_edges(&edges);
        let (node_children, node_parent) = build_node_adjacency(&edges);
        Graph {
            nodes,
            properties,
            edges,
            domains,
            prop_edges,
            node_children,
            node_parent,
        }
    }

    /// Build a minimal single-node, no-property graph and a matching layout.
    fn minimal_graph_and_layout() -> (Graph, LayoutResult, StateResult) {
        let node = Node {
            id: NodeId(0),
            ident: Some("root".to_string()),
            display_name: Some("Root Node".to_string()),
            properties: vec![],
            domain: None,
            is_anchored: true,
            is_selected: false,
        };

        let graph = make_graph(vec![node], vec![], vec![], vec![]);
        let trust_state = state::propagate(&graph);

        let layout = LayoutResult {
            nodes: vec![NodeLayout {
                id: NodeId(0),
                x: 20.0,
                y: 20.0,
                width: 120.0,
                height: 28.0, // header only — no properties
            }],
            domains: vec![],
            anchors: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 200.0,
            height: 100.0,
            content_offset_x: 0.0,
        };

        (graph, layout, trust_state)
    }

    // -----------------------------------------------------------------------
    // Test 1: Minimal graph produces an SVG with key structural classes
    // -----------------------------------------------------------------------

    #[test]
    fn minimal_graph_produces_svg_structure() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(svg.contains(r#"class="obgraph-container""#), "missing container");
        assert!(svg.contains(r#"class="obgraph""#), "missing svg root class");
        assert!(svg.contains(r#"class="obgraph-nodes""#), "missing nodes layer");
        assert!(svg.contains(r#"class="obgraph-edges""#), "missing edges layer");
        assert!(svg.contains(r#"class="obgraph-domains""#), "missing domains layer");
        assert!(svg.contains(r#"id="arrow-anchor-valid""#), "missing arrow-anchor-valid marker");
        assert!(svg.contains(r#"id="arrow-constraint-valid""#), "missing arrow-constraint-valid marker");
    }

    // -----------------------------------------------------------------------
    // Test 2: Node appears with correct label and data-node attribute
    // -----------------------------------------------------------------------

    #[test]
    fn node_label_and_data_attribute() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(svg.contains(r#"data-node="0""#), "missing data-node=0");
        assert!(svg.contains("Root Node"), "missing node display name");
        assert!(svg.contains(r#"class="obgraph-node-sep""#), "missing node separator line");
        assert!(svg.contains(r#"class="obgraph-node-title""#), "missing node title text");
    }

    // -----------------------------------------------------------------------
    // Test 3: Trust state data attributes on node header
    // -----------------------------------------------------------------------

    #[test]
    fn trust_state_data_attributes() {
        // Root node is trusted; its header should carry data-trust="trusted".
        let (graph, layout, trust) = minimal_graph_and_layout();
        assert!(trust.is_node_verified(&graph, NodeId(0)), "root must be verified for this test");

        let svg = generate_svg(&graph, &layout, &trust);
        assert!(
            svg.contains(r#"data-trust="trusted""#),
            "missing data-trust=trusted on trusted root header"
        );
    }

    #[test]
    fn untrusted_node_header_attr() {
        // Build a non-root node with a Critical property and no constraint — stays untrusted.
        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: Some("root".to_string()),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("child".to_string()),
                display_name: None,
                properties: vec![PropId(0)],
                domain: None,
                is_anchored: false,
                is_selected: false,
            },
        ];
        let properties = vec![Property {
            id: PropId(0),
            node: NodeId(1),
            name: "secret".to_string(),
            critical: true,
            constrained: false,
        }];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, properties, edges, vec![]);
        let trust_state = state::propagate(&graph);

        assert!(!trust_state.is_node_verified(&graph, NodeId(1)), "child should not be verified");

        let layout = LayoutResult {
            nodes: vec![
                NodeLayout {
                    id: NodeId(0),
                    x: 20.0,
                    y: 20.0,
                    width: 100.0,
                    height: 28.0,
                },
                NodeLayout {
                    id: NodeId(1),
                    x: 20.0,
                    y: 100.0,
                    width: 100.0,
                    height: 52.0,
                },
            ],
            domains: vec![],
            anchors: vec![EdgePath {
                edge_id: EdgeId(0),
                svg_path: "M 70,48 L 70,100".to_string(),
                label: None,
            }],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 200.0,
            height: 200.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        assert!(
            svg.contains(r#"data-trust="untrusted""#),
            "child header should be data-trust=untrusted"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: Cross-domain constraint full paths carry style="display:none"
    // -----------------------------------------------------------------------

    #[test]
    fn cross_domain_full_paths_hidden() {
        // Build two nodes in separate domains connected by a cross-domain constraint.
        let d0 = DomainId(0);
        let d1 = DomainId(1);

        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: Some("a".to_string()),
                display_name: None,
                properties: vec![PropId(0)],
                domain: Some(d0),
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("b".to_string()),
                display_name: None,
                properties: vec![PropId(1)],
                domain: Some(d1),
                is_anchored: false,
                is_selected: false,
            },
        ];
        let properties = vec![
            Property {
                id: PropId(0),
                node: NodeId(0),
                name: "src_prop".to_string(),
                critical: false,
                constrained: true,
            },
            Property {
                id: PropId(1),
                node: NodeId(1),
                name: "dst_prop".to_string(),
                critical: true,
            constrained: false,
            },
        ];
        let edges = vec![Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        }];

        let domains = vec![
            Domain {
                id: d0,
                display_name: "Domain A".to_string(),
                members: vec![NodeId(0)],
            },
            Domain {
                id: d1,
                display_name: "Domain B".to_string(),
                members: vec![NodeId(1)],
            },
        ];

        let graph = make_graph(nodes, properties, edges, domains);
        let trust_state = state::propagate(&graph);

        // Cross-domain paths.
        let cross_vec = vec![CrossDomainPaths {
            participants: vec![NodeId(0), NodeId(1)],
            full_path: EdgePath {
                edge_id: EdgeId(0),
                svg_path: "M 100,50 L 200,150".to_string(),
                label: None,
            },
            stub_paths: vec![StubPath {
                edge_id: EdgeId(0),
                dotted_svg: "M 100,50 L 120,50".to_string(),
            }],
        }];

        let layout = LayoutResult {
            nodes: vec![
                NodeLayout {
                    id: NodeId(0),
                    x: 20.0,
                    y: 20.0,
                    width: 120.0,
                    height: 52.0,
                },
                NodeLayout {
                    id: NodeId(1),
                    x: 200.0,
                    y: 20.0,
                    width: 120.0,
                    height: 52.0,
                },
            ],
            domains: vec![
                DomainLayout {
                    id: d0,
                    display_name: "Domain A".to_string(),
                    x: 10.0,
                    y: 10.0,
                    width: 150.0,
                    height: 80.0,
                },
                DomainLayout {
                    id: d1,
                    display_name: "Domain B".to_string(),
                    x: 180.0,
                    y: 10.0,
                    width: 150.0,
                    height: 80.0,
                },
            ],
            anchors: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: cross_vec,
            property_order: PropertyOrder::from_graph(&graph),
            width: 400.0,
            height: 200.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        // Full cross-domain path must carry the CSS class (opacity-based hiding),
        // not an inline style="display:none".
        assert!(
            svg.contains(r#"class="obgraph-constraint-full""#),
            "missing obgraph-constraint-full class"
        );
        assert!(
            !svg.contains(r#"style="display:none""#),
            "cross-domain full path must not use inline display:none (use CSS class instead)"
        );
        assert!(
            svg.contains("obgraph-constraint-stub"),
            "missing obgraph-constraint-stub class"
        );
        // Stub must have a dotted path (no solid half).
        assert!(
            svg.contains("obgraph-stub-dotted"),
            "missing obgraph-stub-dotted class on stub"
        );
        assert!(
            !svg.contains("obgraph-stub-solid"),
            "stub should not contain solid half"
        );

        // Stub dotted path must carry participants attribute.
        let stub_dotted_line = svg
            .lines()
            .find(|l| l.contains("<path") && l.contains("obgraph-stub-dotted"))
            .expect("no stub-dotted path element found");
        assert!(
            stub_dotted_line.contains("data-participants="),
            "stub dotted path must carry data-participants"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: @constrained properties carry data-trust="always"
    // -----------------------------------------------------------------------

    #[test]
    fn always_prop_trust_attr() {
        let nodes = vec![Node {
            id: NodeId(0),
            ident: Some("root".to_string()),
            display_name: None,
            properties: vec![PropId(0)],
            domain: None,
            is_anchored: true,
            is_selected: false,
        }];
        let properties = vec![Property {
            id: PropId(0),
            node: NodeId(0),
            name: "always_prop".to_string(),
            critical: false,
            constrained: true,
        }];
        let graph = make_graph(nodes, properties, vec![], vec![]);
        let trust_state = state::propagate(&graph);

        let layout = LayoutResult {
            nodes: vec![NodeLayout {
                id: NodeId(0),
                x: 20.0,
                y: 20.0,
                width: 120.0,
                height: 52.0,
            }],
            domains: vec![],
            anchors: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 200.0,
            height: 100.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        assert!(
            svg.contains(r#"data-trust="always""#),
            "Always property must carry data-trust=always"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: viewBox dimensions come from layout.width / layout.height
    // -----------------------------------------------------------------------

    #[test]
    fn viewbox_matches_layout_dimensions() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(
            svg.contains(r#"viewBox="0 0 200 100""#),
            "viewBox must match layout width=200 height=100"
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: XML escaping for special characters in labels
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Test 8: Two-overlapping-rect header structure
    // -----------------------------------------------------------------------

    #[test]
    fn node_header_two_rect_structure() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(
            svg.contains(r#"class="obgraph-node-header""#),
            "missing obgraph-node-header rect"
        );
        assert!(
            svg.contains(r#"class="obgraph-node-header-fill""#),
            "missing obgraph-node-header-fill rect (square-bottom fill)"
        );
        // The header rect must have rx="8" for rounded top corners
        let header_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-node-header""#))
            .expect("no header rect line found");
        assert!(
            header_line.contains(r#"rx="8""#),
            "header rect must have rx=8 for rounded top corners"
        );
        // The header-fill rect must NOT have rx (square bottom)
        let fill_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-node-header-fill""#))
            .expect("no header-fill rect line found");
        assert!(
            !fill_line.contains("rx="),
            "header-fill rect must not have rx (square bottom corners)"
        );
    }

    // -----------------------------------------------------------------------
    // Test 9: Node border rect present with rx="8"
    // -----------------------------------------------------------------------

    #[test]
    fn node_border_rect_present() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(
            svg.contains(r#"class="obgraph-node-border""#),
            "missing obgraph-node-border rect"
        );
        let border_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-node-border""#))
            .expect("no border rect line found");
        assert!(
            border_line.contains(r#"rx="7""#),
            "border rect must have rx=7 (inset selection ring)"
        );
        assert!(
            border_line.contains(r#"fill="none""#),
            "border rect must have fill=none"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10: Dot radius uses DOT_RADIUS constant (value 2)
    // -----------------------------------------------------------------------

    #[test]
    fn dot_radius_matches_constant() {
        // Build a non-root node without anchor — it gets a header problem dot.
        let nodes = vec![Node {
            id: NodeId(0),
            ident: Some("orphan".to_string()),
            display_name: None,
            properties: vec![],
            domain: None,
            is_anchored: false,
            is_selected: false,
        }];
        let graph = make_graph(nodes, vec![], vec![], vec![]);
        let trust_state = state::propagate(&graph);

        let layout = LayoutResult {
            nodes: vec![NodeLayout {
                id: NodeId(0),
                x: 20.0,
                y: 20.0,
                width: 120.0,
                height: 28.0,
            }],
            domains: vec![],
            anchors: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 200.0,
            height: 100.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        // The node-dot should use DOT_RADIUS = 2
        let dot_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-node-dot""#))
            .expect("non-root unanchored node should have a problem dot");
        assert!(
            dot_line.contains(r#"r="2""#),
            "dot radius must be 2 (DOT_RADIUS), got: {}",
            dot_line.trim()
        );
    }

    // -----------------------------------------------------------------------
    // Test 11: Invalid marker definitions present
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_markers_defined() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        assert!(
            svg.contains(r#"id="arrow-anchor-invalid""#),
            "missing arrow-anchor-invalid marker"
        );
        assert!(
            svg.contains(r#"id="arrow-constraint-invalid""#),
            "missing arrow-constraint-invalid marker"
        );
    }

    // -----------------------------------------------------------------------
    // Test 12: Defs block appears before Layer 0 (domains)
    // -----------------------------------------------------------------------

    #[test]
    fn defs_before_domains() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        let defs_pos = svg.find("<defs>").expect("missing <defs>");
        let domains_pos = svg
            .find(r#"class="obgraph-domains""#)
            .expect("missing domains layer");
        assert!(
            defs_pos < domains_pos,
            "<defs> must appear before domains layer"
        );
    }

    // -----------------------------------------------------------------------
    // Test 13: Edge validity — valid anchor gets obgraph-anchor class
    // -----------------------------------------------------------------------

    #[test]
    fn valid_anchor_edge_class() {
        // Root → child with constrained property → anchor is valid
        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: Some("root".to_string()),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("child".to_string()),
                display_name: None,
                properties: vec![],
                domain: None,
                is_anchored: false,
                is_selected: false,
            },
        ];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, vec![], edges, vec![]);
        let trust_state = state::propagate(&graph);

        // Root is anchored+verified, so anchor from root→child is valid
        assert!(trust_state.is_node_anchored(NodeId(0)));

        let layout = LayoutResult {
            nodes: vec![
                NodeLayout { id: NodeId(0), x: 20.0, y: 20.0, width: 100.0, height: 28.0 },
                NodeLayout { id: NodeId(1), x: 20.0, y: 100.0, width: 100.0, height: 28.0 },
            ],
            domains: vec![],
            anchors: vec![EdgePath {
                edge_id: EdgeId(0),
                svg_path: "M 70,48 L 70,100".to_string(),
                label: None,
            }],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 200.0,
            height: 200.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        let anchor_line = svg
            .lines()
            .find(|l| l.contains(r#"data-edge="0""#) && l.contains("obgraph-anchor"))
            .expect("anchor edge line not found");
        assert!(
            anchor_line.contains(r#"class="obgraph-anchor""#),
            "valid anchor should use obgraph-anchor class, got: {}",
            anchor_line.trim()
        );
        assert!(
            anchor_line.contains("arrow-anchor-valid"),
            "valid anchor should use arrow-anchor-valid marker"
        );
    }

    // -----------------------------------------------------------------------
    // Test 15: Edge validity — invalid constraint gets invalid class/marker
    // -----------------------------------------------------------------------

    #[test]
    fn invalid_constraint_edge_class() {
        // Non-root node with critical property, no parent anchor → constraint is invalid
        let nodes = vec![
            Node {
                id: NodeId(0),
                ident: Some("src".to_string()),
                display_name: None,
                properties: vec![PropId(0)],
                domain: None,
                is_anchored: false, // not root, not anchored
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: Some("dst".to_string()),
                display_name: None,
                properties: vec![PropId(1)],
                domain: None,
                is_anchored: false,
                is_selected: false,
            },
        ];
        let properties = vec![
            Property {
                id: PropId(0),
                node: NodeId(0),
                name: "src_val".to_string(),
                critical: false,
                constrained: true,
            },
            Property {
                id: PropId(1),
                node: NodeId(1),
                name: "dst_val".to_string(),
                critical: true,
                constrained: false,
            },
        ];
        let edges = vec![Edge::Constraint {
            source_prop: PropId(0),
            dest_prop: PropId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, properties, edges, vec![]);
        let trust_state = state::propagate(&graph);

        // Source node is not anchored, so constraint is invalid
        assert!(!trust_state.is_node_anchored(NodeId(0)));

        let layout = LayoutResult {
            nodes: vec![
                NodeLayout { id: NodeId(0), x: 20.0, y: 20.0, width: 100.0, height: 52.0 },
                NodeLayout { id: NodeId(1), x: 200.0, y: 20.0, width: 100.0, height: 52.0 },
            ],
            domains: vec![],
            anchors: vec![],
            intra_domain_constraints: vec![EdgePath {
                edge_id: EdgeId(0),
                svg_path: "M 120,46 L 200,46".to_string(),
                label: None,
            }],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 400.0,
            height: 200.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);

        let constraint_line = svg
            .lines()
            .find(|l| l.contains(r#"data-edge="0""#) && l.contains("obgraph-constraint"))
            .expect("constraint edge line not found");
        assert!(
            constraint_line.contains(r#"class="obgraph-constraint-invalid""#),
            "invalid constraint should use obgraph-constraint-invalid class, got: {}",
            constraint_line.trim()
        );
        assert!(
            constraint_line.contains("arrow-constraint-invalid"),
            "invalid constraint should use arrow-constraint-invalid marker"
        );
    }

    // -----------------------------------------------------------------------
    // Test 16: Node background has rx="8"
    // -----------------------------------------------------------------------

    #[test]
    fn node_bg_rx_8() {
        let (graph, layout, trust) = minimal_graph_and_layout();
        let svg = generate_svg(&graph, &layout, &trust);

        let bg_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-node-bg""#))
            .expect("no node-bg rect line found");
        assert!(
            bg_line.contains(r#"rx="8""#),
            "node-bg rect must have rx=8, got: {}",
            bg_line.trim()
        );
    }

    // -----------------------------------------------------------------------
    // Test 7: XML escaping for special characters in labels
    // -----------------------------------------------------------------------

    #[test]
    fn xml_escape_in_labels() {
        let nodes = vec![Node {
            id: NodeId(0),
            ident: Some("root".to_string()),
            display_name: Some("A & B <test>".to_string()),
            properties: vec![],
            domain: None,
            is_anchored: true,
            is_selected: false,
        }];
        let graph = make_graph(nodes, vec![], vec![], vec![]);
        let trust_state = state::propagate(&graph);

        let layout = LayoutResult {
            nodes: vec![NodeLayout {
                id: NodeId(0),
                x: 20.0,
                y: 20.0,
                width: 200.0,
                height: 28.0,
            }],
            domains: vec![],
            anchors: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: vec![],
            property_order: PropertyOrder::from_graph(&graph),
            width: 300.0,
            height: 100.0,
            content_offset_x: 0.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);
        assert!(svg.contains("A &amp; B &lt;test&gt;"), "XML entities must be escaped");
    }
}
