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
  node ca "Certificate Authority" @root @selected {
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

node revocation "Revocation List" @root {
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
    let input = include_str!("sev_snp_input.obgraph");
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
    let input = include_str!("sev_snp_input.obgraph");
    let report = run_quality(input);
    eprintln!("{}", report.summary());
    // This test just prints the report for visibility.
    // As layout improves, we can tighten these assertions.
    assert!(
        !report.has_errors(),
        "SEV-SNP should have no hard errors:\n{}",
        report.summary()
    );
}
