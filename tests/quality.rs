// Layout quality integration tests.
//
// These tests run both example graphs through the full pipeline and check
// that the layout has no hard errors (node overlaps, domain violations).

use mdbook_obgraph::layout::quality;
use mdbook_obgraph::model;
use mdbook_obgraph::parse;

fn run_quality(input: &str) -> quality::QualityReport {
    let ast = parse::parse(input).expect("parse failed");
    let graph = model::build(ast).expect("build failed");
    let layout = mdbook_obgraph::layout::layout(&graph).expect("layout failed");
    quality::analyze(&graph, &layout)
}

const PKI_EXAMPLE: &str = r#"
domain "PKI" {
  node ca "Certificate Authority" @anchored @selected {
    subject.common_name    @constrained
    subject.org            @constrained
    public_key             @constrained
  }

  node cert "Certificate" {
    issuer.common_name     @critical
    issuer.org             @critical
    subject.common_name
    subject.org            @constrained
    public_key             @critical
    signature              @critical
  }
}

domain "Transport" {
  node tls "TLS Session" {
    server_cert            @critical
    cipher_suite           @constrained
  }
}

node revocation "Revocation List" @anchored {
  crl                      @constrained
}

cert <- ca : sign
tls <- cert

cert::issuer.common_name <= ca::subject.common_name
cert::issuer.org <= ca::subject.org
cert::signature <= ca::public_key : verified_by
cert::subject.common_name <= revocation::crl : not_in
"#;

#[test]
fn pki_no_node_overlaps() {
    let report = run_quality(PKI_EXAMPLE);
    eprintln!("{}", report.summary());
    assert!(
        report.node_overlaps.is_empty(),
        "PKI example should have no node-node overlaps: {:?}",
        report.node_overlaps
    );
}

#[test]
fn pki_no_domain_errors() {
    let report = run_quality(PKI_EXAMPLE);
    assert!(
        report.domain_overlaps.is_empty(),
        "PKI example should have no domain overlaps"
    );
    assert!(
        report.nodes_outside_domain.is_empty(),
        "PKI example should have no nodes outside their domain"
    );
}

#[test]
fn sev_snp_no_node_overlaps() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    eprintln!("{}", report.summary());
    assert!(
        report.node_overlaps.is_empty(),
        "SEV-SNP example should have no node-node overlaps: {:?}",
        report.node_overlaps
    );
}

#[test]
fn sev_snp_quality_summary() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    eprintln!("{}", report.summary());
    assert!(
        report.error_count() == 0,
        "SEV-SNP should have zero requirement violations (got {}):\n{}",
        report.error_count(),
        report.summary()
    );
}

#[test]
fn sev_snp_no_free_nodes_inside_domains() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    assert!(
        report.free_nodes_inside_domains.is_empty(),
        "Domain-less nodes must not overlap any domain: {:?}",
        report.free_nodes_inside_domains
    );
}

#[test]
fn sev_snp_domain_contiguity() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    assert!(
        report.domain_contiguity_violations.is_empty(),
        "Domains must be vertically contiguous: {:?}",
        report.domain_contiguity_violations
    );
}

#[test]
fn sev_snp_no_inter_domain_edges_in_intra_corridors() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    assert!(
        report.inter_domain_edges_in_intra_corridors.is_empty(),
        "Inter-domain edges must not route through intra-domain corridors: {:?}",
        report.inter_domain_edges_in_intra_corridors
    );
}

#[test]
fn sev_snp_no_channel_collisions() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    // Known: 6 channel collisions from bundle routing merge.
    // Track regression — should not increase.
    assert!(
        report.channel_collisions.len() <= 6,
        "Channel collisions should not exceed 6 (got {}): {:?}",
        report.channel_collisions.len(),
        report.channel_collisions
    );
}

#[test]
fn sev_snp_column_height_balance() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    eprintln!(
        "Column heights: {:?}, imbalance: {:.0}px",
        report.column_heights, report.column_height_imbalance
    );
    // Column heights should be reasonably balanced (within 600px).
    // With derivation constraints removed, KDS and NVD nodes are
    // disconnected, creating less balanced columns.
    assert!(
        report.column_height_imbalance < 600.0,
        "Column heights should be balanced: imbalance {:.0}px (threshold 600px), heights: {:?}",
        report.column_height_imbalance,
        report.column_heights,
    );
}

// ── Realistic SEV-SNP+TPM example ─────────────────────────────────────

#[test]
fn sev_snp_realistic_no_node_overlaps() {
    let input = include_str!("sev_snp_realistic.objgraph");
    let report = run_quality(input);
    eprintln!("{}", report.summary());
    assert!(
        report.node_overlaps.is_empty(),
        "Realistic example should have no node-node overlaps: {:?}",
        report.node_overlaps
    );
}

#[test]
fn sev_snp_realistic_quality_summary() {
    let input = include_str!("sev_snp_realistic.objgraph");
    let ast = parse::parse(input).expect("parse failed");
    let graph = model::build(ast).expect("build failed");
    let layout = mdbook_obgraph::layout::layout(&graph).expect("layout failed");
    let report = quality::analyze(&graph, &layout);
    eprintln!("{}", report.summary());
    if !report.crossing_pairs.is_empty() {
        eprintln!("\n  Crossing pairs:");
        for (a, b) in &report.crossing_pairs {
            eprintln!("    {} x {}", describe_edge(&graph, *a), describe_edge(&graph, *b));
        }
    }
    // All requirement violations should be zero: no inter-domain edges in
    // intra-corridors, no channel collisions, no label/node overlaps.
    assert!(
        report.error_count() == 0,
        "Realistic SEV-SNP should have zero requirement violations (got {}):\n{}",
        report.error_count(),
        report.summary()
    );
}

fn describe_edge(graph: &model::types::Graph, eid: model::types::EdgeId) -> String {
    use model::types::Edge;
    match &graph.edges[eid.index()] {
        Edge::Anchor { parent, child, operation } => {
            format!("A{}: {} <- {} ({})", eid.index(),
                graph.nodes[child.index()].label(),
                graph.nodes[parent.index()].label(),
                operation.as_deref().unwrap_or(""))
        }
        Edge::Constraint { source_prop, dest_prop, operation } => {
            let sp = &graph.properties[source_prop.index()];
            let dp = &graph.properties[dest_prop.index()];
            format!("C{}: {}::{} <= {}::{} ({})", eid.index(),
                graph.nodes[sp.node.index()].label(), sp.name,
                graph.nodes[dp.node.index()].label(), dp.name,
                operation.as_deref().unwrap_or(""))
        }
    }
}

#[test]
fn sev_snp_realistic_domain_contiguity() {
    let input = include_str!("sev_snp_realistic.objgraph");
    let report = run_quality(input);
    assert!(
        report.domain_contiguity_violations.is_empty(),
        "Domains must be vertically contiguous: {:?}",
        report.domain_contiguity_violations
    );
}

// ── New error-class assertions ───────────────────────────────────────

#[test]
fn sev_snp_no_intra_edges_in_wrong_corridor() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    assert!(
        report.intra_edges_in_wrong_corridor.is_empty(),
        "Intra-domain edges must not route through another domain's corridor: {:?}",
        report.intra_edges_in_wrong_corridor
    );
}

#[test]
fn sev_snp_realistic_no_intra_edges_in_wrong_corridor() {
    let input = include_str!("sev_snp_realistic.objgraph");
    let report = run_quality(input);
    assert!(
        report.intra_edges_in_wrong_corridor.is_empty(),
        "Intra-domain edges must not route through another domain's corridor: {:?}",
        report.intra_edges_in_wrong_corridor
    );
}

// ── Symmetry baseline assertion ──────────────────────────────────────

#[test]
fn sev_snp_visual_balance() {
    let input = include_str!("sev_snp_input.objgraph");
    let report = run_quality(input);
    eprintln!("Visual balance: {:.3}", report.visual_balance);
    assert!(
        report.visual_balance < 0.15,
        "Layout should be reasonably centered: balance {:.3} (threshold 0.15)",
        report.visual_balance,
    );
}

#[test]
fn sev_snp_realistic_visual_balance() {
    let input = include_str!("sev_snp_realistic.objgraph");
    let report = run_quality(input);
    eprintln!("Visual balance: {:.3}", report.visual_balance);
    assert!(
        report.visual_balance < 0.15,
        "Layout should be reasonably centered: balance {:.3} (threshold 0.15)",
        report.visual_balance,
    );
}

