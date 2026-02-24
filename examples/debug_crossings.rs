/// Debug tool: analyze current_time edge routing problems.
use mdbook_obgraph::model;
use mdbook_obgraph::model::types::{Edge, EdgeId};
use mdbook_obgraph::parse;

fn edge_desc(graph: &model::types::Graph, eid: EdgeId) -> String {
    let edge = &graph.edges[eid.index()];
    match edge {
        Edge::Anchor { parent, child, operation } => {
            format!("Anchor({}←{}, {:?})", 
                graph.nodes[parent.index()].ident,
                graph.nodes[child.index()].ident,
                operation)
        }
        Edge::Constraint { source_prop, dest_prop, operation } => {
            let src_node = &graph.nodes[graph.properties[source_prop.index()].node.index()];
            let dst_node = &graph.nodes[graph.properties[dest_prop.index()].node.index()];
            let src_pname = &graph.properties[source_prop.index()].name;
            let dst_pname = &graph.properties[dest_prop.index()].name;
            format!("{}::{}→{}::{} [{:?}]", 
                src_node.ident, src_pname,
                dst_node.ident, dst_pname,
                operation)
        }
        Edge::DerivInput { source_prop, target_deriv } => {
            let src_node = &graph.nodes[graph.properties[source_prop.index()].node.index()];
            let src_pname = &graph.properties[source_prop.index()].name;
            format!("DerivInput({}::{}→Deriv{})", 
                src_node.ident, src_pname, target_deriv.index())
        }
    }
}

fn main() {
    let input = include_str!("../tests/sev_snp_input.obgraph");
    let ast = parse::parse(input).expect("parse failed");
    let graph = model::build(ast).expect("build failed");
    let layout = mdbook_obgraph::layout::layout(&graph).expect("layout failed");

    // Show all edge routes with their types
    let all_paths: Vec<_> = layout.anchors.iter()
        .chain(layout.intra_domain_constraints.iter())
        .chain(layout.cross_domain_constraints.iter().map(|c| &c.full_path))
        .collect();

    println!("=== All edge routes ===");
    for ep in &all_paths {
        let edge = &graph.edges[ep.edge_id.index()];
        let is_cross = match edge {
            Edge::Constraint { source_prop, dest_prop, .. } => {
                let src_dom = graph.nodes[graph.properties[source_prop.index()].node.index()].domain;
                let dst_dom = graph.nodes[graph.properties[dest_prop.index()].node.index()].domain;
                src_dom != dst_dom
            }
            _ => false,
        };
        let marker = if is_cross { "CROSS" } else { "intra" };
        println!("  [{}] Edge {:>2} {}", marker, ep.edge_id.index(), edge_desc(&graph, ep.edge_id));
        println!("                    path: {}", ep.svg_path);
    }

    // Node layout summary
    println!("\n=== Node layouts ===");
    for nl in &layout.nodes {
        let node = &graph.nodes[nl.id.index()];
        let dom = node.domain.map(|d| graph.domains[d.index()].display_name.as_str()).unwrap_or("(none)");
        println!("  {} [{}]: x={:.0}..{:.0} y={:.0}..{:.0} center_x={:.0}", 
            node.ident, dom, nl.x, nl.x + nl.width, nl.y, nl.y + nl.height,
            nl.x + nl.width / 2.0);
    }

    // Domain layout summary
    println!("\n=== Domain layouts ===");
    for dl in &layout.domains {
        println!("  {} (id={}): x={:.0}..{:.0} y={:.0}..{:.0}",
            dl.display_name, dl.id.index(), dl.x, dl.x + dl.width, dl.y, dl.y + dl.height);
    }

    // Show column info
    println!("\n=== Column analysis ===");
    println!("Column 0 (left):  AMD SEV-SNP x=16..204, Guest vTPM x=16..204");
    println!("Column 1 (right): Verifier x=272..519, AMD KDS x=272..460, NIST x=272..460, TPM x=272..460");
    println!("Inter-column gap: x=204..272 (68px)");
    println!("Outer right edge: x=460..519 (from Verifier)");
}
