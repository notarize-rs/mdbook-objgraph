/// SVG element generation (DESIGN.md §6.2).

use std::fmt::Write;

use crate::layout::{EdgePath, LayoutResult, HEADER_HEIGHT, ROW_HEIGHT};
use crate::model::state::TrustState;
use crate::model::types::Graph;

use super::interactivity;
use super::style;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Generate the complete SVG string for a laid-out graph.
pub fn generate_svg(graph: &Graph, layout: &LayoutResult, trust: &TrustState) -> String {
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
        r#"       width="100%" preserveAspectRatio="xMidYMin meet""#
    )
    .unwrap();
    writeln!(out, r#"       class="obgraph">"#).unwrap();

    // Embedded CSS
    writeln!(out, r#"    <style>{}</style>"#, style::css()).unwrap();

    // Layer 0: domain backgrounds
    write_domains(&mut out, layout);

    // Layer 1: edges
    write_edges(&mut out, graph, layout);

    // Layer 2: derivation nodes
    write_derivations(&mut out, graph, layout);

    // Layer 3: nodes
    write_nodes(&mut out, graph, layout, trust);

    // Arrow marker defs
    write_defs(&mut out);

    // Embedded JS
    writeln!(out, r#"    <script>{}</script>"#, interactivity::js()).unwrap();

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

        // Label centered at the top of the domain box
        let label_x = domain.x + domain.width / 2.0;
        let label_y = domain.y + 14.0;
        writeln!(
            out,
            r#"        <text class="obgraph-domain-label" x="{x}" y="{y}" text-anchor="middle">{name}</text>"#,
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
// Layer 1: edges
// ---------------------------------------------------------------------------

fn write_edges(out: &mut String, graph: &Graph, layout: &LayoutResult) {
    writeln!(out, r#"    <g class="obgraph-edges">"#).unwrap();

    // --- Link paths ---
    writeln!(out, r#"      <g class="obgraph-links">"#).unwrap();
    for ep in &layout.links {
        writeln!(
            out,
            r#"        <path class="obgraph-link" d="{d}" data-edge="{id}" marker-end="url(#arrow-link)"/>"#,
            d = ep.svg_path,
            id = ep.edge_id.0
        )
        .unwrap();
        if let Some(lbl) = &ep.label {
            writeln!(
                out,
                r#"        <text class="obgraph-link-label" x="{x}" y="{y}" text-anchor="{anchor}">{text}</text>"#,
                x = lbl.x, y = lbl.y, anchor = lbl.anchor, text = escape_xml(&lbl.text)
            ).unwrap();
        }
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Derivation input edges ---
    writeln!(out, r#"      <g class="obgraph-deriv-edges">"#).unwrap();
    for ep in &layout.derivation_edges {
        writeln!(
            out,
            r#"        <path class="obgraph-deriv-edge" d="{d}" data-edge="{id}"/>"#,
            d = ep.svg_path,
            id = ep.edge_id.0
        )
        .unwrap();
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Intra-domain constraint paths ---
    writeln!(out, r#"      <g class="obgraph-constraints-intra">"#).unwrap();
    for ep in &layout.intra_domain_constraints {
        writeln!(
            out,
            r#"        <path class="obgraph-constraint" d="{d}" data-edge="{id}" marker-end="url(#arrow-constraint)"/>"#,
            d = ep.svg_path,
            id = ep.edge_id.0
        )
        .unwrap();
        if let Some(lbl) = &ep.label {
            writeln!(
                out,
                r#"        <text class="obgraph-constraint-label" x="{x}" y="{y}" text-anchor="{anchor}">{text}</text>"#,
                x = lbl.x, y = lbl.y, anchor = lbl.anchor, text = escape_xml(&lbl.text)
            ).unwrap();
        }
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Cross-domain constraint: full paths (hidden by CSS class by default) ---
    writeln!(out, r#"      <g class="obgraph-constraints-cross">"#).unwrap();
    for (node_id, cross) in &layout.cross_domain_constraints {
        // For each full path we need source-node and target-node.
        // The edge_id on the full_path lets us look up the edge.
        for ep in &cross.full_paths {
            let (src_node, dst_node) = constraint_node_ids(graph, ep);
            writeln!(
                out,
                r#"        <path class="obgraph-constraint-full" d="{d}" data-edge="{id}" data-source-node="{src}" data-target-node="{dst}" marker-end="url(#arrow-constraint-cross)"/>"#,
                d = ep.svg_path,
                id = ep.edge_id.0,
                src = src_node,
                dst = dst_node,
            )
            .unwrap();
        }
        let _ = node_id; // suppress unused warning; node_id is the key for grouping
    }
    writeln!(out, r#"      </g>"#).unwrap();

    // --- Cross-domain constraint: stub paths ---
    writeln!(out, r#"      <g class="obgraph-constraint-stubs">"#).unwrap();
    for (node_id, cross) in &layout.cross_domain_constraints {
        for ep in &cross.stub_paths {
            let src_node = node_id.0;
            let (_, dst_node) = constraint_node_ids(graph, ep);
            writeln!(
                out,
                r#"        <path class="obgraph-constraint-stub" d="{d}" data-edge="{id}" data-source-node="{src}" data-target-node="{dst}" marker-end="url(#arrow-constraint-cross)"/>"#,
                d = ep.svg_path,
                id = ep.edge_id.0,
                src = src_node,
                dst = dst_node,
            )
            .unwrap();
        }
    }
    writeln!(out, r#"      </g>"#).unwrap();

    writeln!(out, r#"    </g>"#).unwrap();
}

/// For a cross-domain constraint edge, resolve the source-node and target-node IDs.
///
/// A `Constraint` edge runs between two `PropId`s; we look up the owning node of each.
fn constraint_node_ids(graph: &Graph, ep: &EdgePath) -> (u32, u32) {
    use crate::model::types::Edge;

    let edge = &graph.edges[ep.edge_id.index()];
    match edge {
        Edge::Constraint {
            source_prop,
            dest_prop,
            ..
        } => {
            let src_node = graph.properties[source_prop.index()].node.0;
            let dst_node = graph.properties[dest_prop.index()].node.0;
            (src_node, dst_node)
        }
        // Only constraints appear in cross_domain_constraints; fall back to 0 for safety.
        _ => (0, 0),
    }
}

// ---------------------------------------------------------------------------
// Layer 2: derivation nodes
// ---------------------------------------------------------------------------

fn write_derivations(out: &mut String, graph: &Graph, layout: &LayoutResult) {
    writeln!(out, r#"    <g class="obgraph-derivations">"#).unwrap();

    for dl in &layout.derivations {
        let deriv = &graph.derivations[dl.id.index()];

        // Diamond: 4 points — top, right, bottom, left — centered at (dl.x, dl.y)
        let half_w = dl.width / 2.0;
        let half_h = dl.height / 2.0;
        let cx = dl.x + half_w;
        let cy = dl.y + half_h;
        let points = format!(
            "{},{} {},{} {},{} {},{}",
            cx,
            dl.y,
            dl.x + dl.width,
            cy,
            cx,
            dl.y + dl.height,
            dl.x,
            cy
        );

        writeln!(
            out,
            r#"      <g class="obgraph-derivation" data-deriv="{id}">"#,
            id = dl.id.0
        )
        .unwrap();
        writeln!(
            out,
            r#"        <polygon class="obgraph-deriv-shape" points="{pts}"/>"#,
            pts = points
        )
        .unwrap();
        writeln!(
            out,
            r#"        <text class="obgraph-deriv-label" x="{x}" y="{y}" text-anchor="middle" dominant-baseline="central">{op}</text>"#,
            x = cx,
            y = cy,
            op = escape_xml(&deriv.operation)
        )
        .unwrap();
        writeln!(out, r#"      </g>"#).unwrap();
    }

    writeln!(out, r#"    </g>"#).unwrap();
}

// ---------------------------------------------------------------------------
// Layer 3: nodes
// ---------------------------------------------------------------------------

fn write_nodes(out: &mut String, graph: &Graph, layout: &LayoutResult, trust: &TrustState) {
    writeln!(out, r#"    <g class="obgraph-nodes">"#).unwrap();

    for nl in &layout.nodes {
        let node = &graph.nodes[nl.id.index()];
        let node_trusted = trust.is_node_trusted(nl.id);
        let node_trust_attr = if node_trusted { "trusted" } else { "untrusted" };
        let selected_attr = node.is_selected;

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
            r#"        <rect class="obgraph-node-bg" x="{x}" y="{y}" width="{w}" height="{h}" rx="3"/>"#,
            x = nl.x,
            y = nl.y,
            w = nl.width,
            h = nl.height
        )
        .unwrap();

        // Title text — centered in header area
        let title_x = nl.x + nl.width / 2.0;
        let title_y = nl.y + HEADER_HEIGHT / 2.0;
        writeln!(
            out,
            r#"        <text class="obgraph-node-title" x="{x}" y="{y}" text-anchor="middle" dominant-baseline="central" data-trust="{trust}">{label}</text>"#,
            x = title_x,
            y = title_y,
            trust = node_trust_attr,
            label = escape_xml(node.label())
        )
        .unwrap();

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
        for (prop_idx, &pid) in node.properties.iter().enumerate() {
            let prop = &graph.properties[pid.index()];
            let prop_trusted = trust.is_prop_trusted(pid);

            // Trust attribute: "constrained" for @constrained props, else trusted/untrusted
            let trust_attr = if prop.constrained {
                "always" // backwards-compat: @constrained maps to data-trust="always"
            } else if prop_trusted {
                "trusted"
            } else {
                "untrusted"
            };

            let row_y = nl.y + HEADER_HEIGHT + prop_idx as f64 * ROW_HEIGHT;
            let port_y = row_y + ROW_HEIGHT / 2.0;

            writeln!(
                out,
                r#"        <g class="obgraph-prop" data-prop="{pid}" data-trust="{trust}">"#,
                pid = pid.0,
                trust = trust_attr
            )
            .unwrap();

            // Row background
            writeln!(
                out,
                r#"          <rect class="obgraph-prop-bg" x="{x}" y="{y}" width="{w}" height="{rh}"/>"#,
                x = nl.x,
                y = row_y,
                w = nl.width,
                rh = ROW_HEIGHT
            )
            .unwrap();

            // Property name text — left-aligned with small indent
            let text_x = nl.x + 4.0;
            let text_y = port_y;
            writeln!(
                out,
                r#"          <text class="obgraph-prop-name" x="{x}" y="{y}" dominant-baseline="central">{name}</text>"#,
                x = text_x,
                y = text_y,
                name = escape_xml(&prop.name)
            )
            .unwrap();

            writeln!(out, r#"        </g>"#).unwrap();
        }

        writeln!(out, r#"      </g>"#).unwrap();
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

    // Anchor/link arrow — 6×6px, refX=0 (path endpoint offset by 6px), green
    writeln!(
        out,
        r#"      <marker id="arrow-link" viewBox="0 0 6 6" refX="0" refY="3""#
    )
    .unwrap();
    writeln!(
        out,
        r#"              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6" orient="auto">"#
    )
    .unwrap();
    writeln!(
        out,
        r#"        <path d="M0,0 L6,3 L0,6 Z" class="obgraph-arrow-link"/>"#
    )
    .unwrap();
    writeln!(out, r#"      </marker>"#).unwrap();

    // Intra-domain constraint arrow — 6×6px, refX=0, blue
    writeln!(
        out,
        r#"      <marker id="arrow-constraint" viewBox="0 0 6 6" refX="0" refY="3""#
    )
    .unwrap();
    writeln!(
        out,
        r#"              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6" orient="auto">"#
    )
    .unwrap();
    writeln!(
        out,
        r#"        <path d="M0,0 L6,3 L0,6 Z" class="obgraph-arrow-constraint"/>"#
    )
    .unwrap();
    writeln!(out, r#"      </marker>"#).unwrap();

    // Cross-domain constraint arrow — 6×6px, refX=0, blue
    writeln!(
        out,
        r#"      <marker id="arrow-constraint-cross" viewBox="0 0 6 6" refX="0" refY="3""#
    )
    .unwrap();
    writeln!(
        out,
        r#"              markerUnits="userSpaceOnUse" markerWidth="6" markerHeight="6" orient="auto">"#
    )
    .unwrap();
    writeln!(
        out,
        r#"        <path d="M0,0 L6,3 L0,6 Z" class="obgraph-arrow-constraint-cross"/>"#
    )
    .unwrap();
    writeln!(out, r#"      </marker>"#).unwrap();

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
    use crate::layout::{CrossDomainPaths, DomainLayout, EdgePath, NodeLayout};
    use crate::model::state;
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
                Edge::DerivInput { source_prop, .. } => {
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
            derivations: vec![],
            edges,
            domains,
            prop_edges,
            node_children,
            node_parent,
        }
    }

    /// Build a minimal single-node, no-property graph and a matching layout.
    fn minimal_graph_and_layout() -> (Graph, LayoutResult, TrustState) {
        let node = Node {
            id: NodeId(0),
            ident: "root".to_string(),
            display_name: Some("Root Node".to_string()),
            properties: vec![],
            domain: None,
            is_root: true,
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
            derivations: vec![],
            domains: vec![],
            links: vec![],
            derivation_edges: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: HashMap::new(),
            width: 200.0,
            height: 100.0,
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
        assert!(svg.contains(r#"class="obgraph-derivations""#), "missing derivations layer");
        assert!(svg.contains(r#"class="obgraph-domains""#), "missing domains layer");
        assert!(svg.contains(r#"id="arrow-link""#), "missing arrow-link marker");
        assert!(svg.contains(r#"id="arrow-constraint""#), "missing arrow-constraint marker");
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
        assert!(trust.is_node_trusted(NodeId(0)), "root must be trusted for this test");

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
                ident: "root".to_string(),
                display_name: None,
                properties: vec![],
                domain: None,
                is_root: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: "child".to_string(),
                display_name: None,
                properties: vec![PropId(0)],
                domain: None,
                is_root: false,
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

        assert!(!trust_state.is_node_trusted(NodeId(1)), "child should be untrusted");

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
            derivations: vec![],
            domains: vec![],
            links: vec![EdgePath {
                edge_id: EdgeId(0),
                svg_path: "M 70,48 L 70,100".to_string(),
                label: None,
            }],
            derivation_edges: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: HashMap::new(),
            width: 200.0,
            height: 200.0,
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
                ident: "a".to_string(),
                display_name: None,
                properties: vec![PropId(0)],
                domain: Some(d0),
                is_root: true,
                is_selected: false,
            },
            Node {
                id: NodeId(1),
                ident: "b".to_string(),
                display_name: None,
                properties: vec![PropId(1)],
                domain: Some(d1),
                is_root: false,
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

        // Cross-domain paths for source node 0.
        let mut cross_map: HashMap<NodeId, CrossDomainPaths> = HashMap::new();
        cross_map.insert(
            NodeId(0),
            CrossDomainPaths {
                full_paths: vec![EdgePath {
                    edge_id: EdgeId(0),
                    svg_path: "M 100,50 L 200,150".to_string(),
                    label: None,
                }],
                stub_paths: vec![EdgePath {
                    edge_id: EdgeId(0),
                    svg_path: "M 100,50 L 120,50".to_string(),
                    label: None,
                }],
            },
        );

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
            derivations: vec![],
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
            links: vec![],
            derivation_edges: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: cross_map,
            width: 400.0,
            height: 200.0,
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
            svg.contains(r#"class="obgraph-constraint-stub""#),
            "missing obgraph-constraint-stub class"
        );

        // Stub path must carry both source and target node attributes.
        // Note: search for the path element specifically (not the group, whose class is
        // "obgraph-constraint-stubs" with a trailing 's').
        let stub_line = svg
            .lines()
            .find(|l| l.contains(r#"class="obgraph-constraint-stub""#))
            .expect("no stub line found");
        assert!(
            stub_line.contains("data-source-node="),
            "stub path must carry data-source-node"
        );
        assert!(
            stub_line.contains("data-target-node="),
            "stub path must carry data-target-node"
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: @constrained properties carry data-trust="always"
    // -----------------------------------------------------------------------

    #[test]
    fn always_prop_trust_attr() {
        let nodes = vec![Node {
            id: NodeId(0),
            ident: "root".to_string(),
            display_name: None,
            properties: vec![PropId(0)],
            domain: None,
            is_root: true,
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
            derivations: vec![],
            domains: vec![],
            links: vec![],
            derivation_edges: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: HashMap::new(),
            width: 200.0,
            height: 100.0,
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

    #[test]
    fn xml_escape_in_labels() {
        let nodes = vec![Node {
            id: NodeId(0),
            ident: "root".to_string(),
            display_name: Some("A & B <test>".to_string()),
            properties: vec![],
            domain: None,
            is_root: true,
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
            derivations: vec![],
            domains: vec![],
            links: vec![],
            derivation_edges: vec![],
            intra_domain_constraints: vec![],
            cross_domain_constraints: HashMap::new(),
            width: 300.0,
            height: 100.0,
        };

        let svg = generate_svg(&graph, &layout, &trust_state);
        assert!(svg.contains("A &amp; B &lt;test&gt;"), "XML entities must be escaped");
    }
}
