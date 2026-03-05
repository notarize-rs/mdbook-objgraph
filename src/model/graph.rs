//! Graph construction from AST (DESIGN.md §2.5, Appendix A.3).

use std::collections::HashMap;

use crate::parse::ast::{AstGraph, AstSourceExpr};
use crate::ObgraphError;

use super::types::{Domain, DomainId, Edge, EdgeId, Graph, Node, NodeId, PropId, Property};
use super::validate;

// ---------------------------------------------------------------------------
// Internal builder state
// ---------------------------------------------------------------------------

struct Builder {
    nodes: Vec<Node>,
    properties: Vec<Property>,
    edges: Vec<Edge>,
    domains: Vec<Domain>,

    prop_edges: HashMap<PropId, Vec<EdgeId>>,
    node_children: HashMap<NodeId, Vec<EdgeId>>,
    node_parent: HashMap<NodeId, EdgeId>,

    /// Map from (node_ident, prop_name) -> PropId for quick lookup.
    prop_lookup: HashMap<(String, String), PropId>,

    /// Map from node_ident -> NodeId for quick lookup.
    node_lookup: HashMap<String, NodeId>,
}

impl Builder {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            properties: Vec::new(),
            edges: Vec::new(),
            domains: Vec::new(),
            prop_edges: HashMap::new(),
            node_children: HashMap::new(),
            node_parent: HashMap::new(),
            prop_lookup: HashMap::new(),
            node_lookup: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Allocation helpers
    // -----------------------------------------------------------------------

    fn next_node_id(&self) -> NodeId {
        NodeId(self.nodes.len() as u32)
    }

    fn next_prop_id(&self) -> PropId {
        PropId(self.properties.len() as u32)
    }

    fn next_edge_id(&self) -> EdgeId {
        EdgeId(self.edges.len() as u32)
    }

    fn next_domain_id(&self) -> DomainId {
        DomainId(self.domains.len() as u32)
    }

    // -----------------------------------------------------------------------
    // Node allocation
    // -----------------------------------------------------------------------

    fn alloc_node(
        &mut self,
        ident: Option<&str>,
        display_name: Option<String>,
        is_anchored: bool,
        is_selected: bool,
        domain: Option<DomainId>,
    ) -> Result<NodeId, ObgraphError> {
        if let Some(name) = ident.filter(|n| self.node_lookup.contains_key(*n)) {
            return Err(ObgraphError::Validation(format!(
                "duplicate node identifier: {name}"
            )));
        }
        let id = self.next_node_id();
        let node = Node {
            id,
            ident: ident.map(|s| s.to_string()),
            display_name,
            properties: Vec::new(),
            domain,
            is_anchored,
            is_selected,
        };
        self.nodes.push(node);
        if let Some(name) = ident {
            self.node_lookup.insert(name.to_string(), id);
        }
        Ok(id)
    }

    // -----------------------------------------------------------------------
    // Property allocation
    // -----------------------------------------------------------------------

    fn alloc_property(
        &mut self,
        node_id: NodeId,
        node_ident: &str,
        name: &str,
        critical: bool,
        constrained: bool,
    ) -> Result<PropId, ObgraphError> {
        let key = (node_ident.to_string(), name.to_string());
        if self.prop_lookup.contains_key(&key) {
            return Err(ObgraphError::Validation(format!(
                "duplicate property {name} on node {node_ident}"
            )));
        }
        let id = self.next_prop_id();
        let prop = Property {
            id,
            node: node_id,
            name: name.to_string(),
            critical,
            constrained,
        };
        self.properties.push(prop);
        self.prop_lookup.insert(key, id);
        // Register on the node.
        self.nodes[node_id.index()].properties.push(id);
        Ok(id)
    }

    /// Allocate a property on a derivation node (no prop_lookup entry needed).
    fn alloc_derivation_property(
        &mut self,
        node_id: NodeId,
        name: &str,
    ) -> Result<PropId, ObgraphError> {
        let id = self.next_prop_id();
        let prop = Property {
            id,
            node: node_id,
            name: name.to_string(),
            critical: false,
            constrained: false,
        };
        self.properties.push(prop);
        self.nodes[node_id.index()].properties.push(id);
        Ok(id)
    }

    // -----------------------------------------------------------------------
    // Edge helpers
    // -----------------------------------------------------------------------

    fn push_edge(&mut self, edge: Edge) -> EdgeId {
        let id = self.next_edge_id();
        self.edges.push(edge);
        id
    }

    fn record_prop_edge(&mut self, prop: PropId, edge: EdgeId) {
        self.prop_edges.entry(prop).or_default().push(edge);
    }

    // -----------------------------------------------------------------------
    // Resolve helpers
    // -----------------------------------------------------------------------

    fn resolve_node(&self, ident: &str) -> Result<NodeId, ObgraphError> {
        self.node_lookup.get(ident).copied().ok_or_else(|| {
            ObgraphError::Validation(format!("unknown node identifier: {ident}"))
        })
    }

    fn resolve_prop(&self, node_ident: &str, prop_name: &str) -> Result<PropId, ObgraphError> {
        let key = (node_ident.to_string(), prop_name.to_string());
        self.prop_lookup.get(&key).copied().ok_or_else(|| {
            ObgraphError::Validation(format!(
                "unknown property {prop_name} on node {node_ident}"
            ))
        })
    }

    // -----------------------------------------------------------------------
    // Final graph assembly
    // -----------------------------------------------------------------------

    fn finish(self) -> Graph {
        Graph {
            nodes: self.nodes,
            properties: self.properties,
            edges: self.edges,
            domains: self.domains,
            prop_edges: self.prop_edges,
            node_children: self.node_children,
            node_parent: self.node_parent,
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Build a validated `Graph` from a parsed AST.
///
/// This function:
/// 1. Allocates all nodes, properties, domains.
/// 2. Resolves all references (node idents, property names).
/// 3. Builds the adjacency indices (prop_edges, node_children, node_parent).
/// 4. Runs validation.
///
/// Returns the immutable `Graph` or an error.
pub fn build(ast: AstGraph) -> Result<Graph, ObgraphError> {
    let mut b = Builder::new();

    // ------------------------------------------------------------------
    // Phase 1: Allocate domains and their member nodes (in order).
    // ------------------------------------------------------------------
    for ast_domain in &ast.domains {
        let domain_id = b.next_domain_id();
        let mut member_ids: Vec<NodeId> = Vec::new();

        for ast_node in &ast_domain.nodes {
            let node_id = b.alloc_node(
                Some(&ast_node.ident),
                ast_node.display_name.clone(),
                ast_node.is_anchored,
                ast_node.is_selected,
                Some(domain_id),
            )?;
            member_ids.push(node_id);

            for ast_prop in &ast_node.properties {
                b.alloc_property(node_id, &ast_node.ident, &ast_prop.name, ast_prop.critical, ast_prop.constrained)?;
            }
        }

        let domain = Domain {
            id: domain_id,
            display_name: ast_domain.display_name.clone(),
            members: member_ids,
        };
        b.domains.push(domain);
    }

    // ------------------------------------------------------------------
    // Phase 2: Allocate top-level nodes (no domain).
    // ------------------------------------------------------------------
    for ast_node in &ast.nodes {
        let node_id = b.alloc_node(
            Some(&ast_node.ident),
            ast_node.display_name.clone(),
            ast_node.is_anchored,
            ast_node.is_selected,
            None,
        )?;

        for ast_prop in &ast_node.properties {
            b.alloc_property(node_id, &ast_node.ident, &ast_prop.name, ast_prop.critical, ast_prop.constrained)?;
        }
    }

    // ------------------------------------------------------------------
    // Phase 3: Process anchors.
    // ------------------------------------------------------------------
    for ast_anchor in &ast.anchors {
        let child_id = b.resolve_node(&ast_anchor.child_ident)?;
        let parent_id = b.resolve_node(&ast_anchor.parent_ident)?;

        let eid = b.push_edge(Edge::Anchor {
            child: child_id,
            parent: parent_id,
            operation: ast_anchor.operation.clone(),
        });

        // node_children: parent -> [edge_ids for each child anchor]
        b.node_children.entry(parent_id).or_default().push(eid);

        // node_parent: child -> edge_id of its parent anchor
        // If a child already has a parent, that's a validation error.
        if b.node_parent.contains_key(&child_id) {
            return Err(ObgraphError::Validation(format!(
                "node {} has more than one parent anchor",
                ast_anchor.child_ident
            )));
        }
        b.node_parent.insert(child_id, eid);
    }

    // ------------------------------------------------------------------
    // Phase 4: Process constraints (including derivation desugaring).
    // ------------------------------------------------------------------
    // Dedup map: (function_name, sorted input PropIds) -> output PropId
    let mut deriv_cache: HashMap<(String, Vec<PropId>), PropId> = HashMap::new();

    for ast_constraint in &ast.constraints {
        let dest_prop_id = b.resolve_prop(&ast_constraint.dest_node, &ast_constraint.dest_prop)?;
        let source_prop_id = resolve_source_expr(&mut b, &ast_constraint.source, &mut deriv_cache)?;

        let eid = b.push_edge(Edge::Constraint {
            dest_prop: dest_prop_id,
            source_prop: source_prop_id,
            operation: ast_constraint.operation.clone(),
        });

        b.record_prop_edge(dest_prop_id, eid);
        b.record_prop_edge(source_prop_id, eid);
    }

    // ------------------------------------------------------------------
    // Phase 5: Validate and return.
    // ------------------------------------------------------------------
    let graph = b.finish();
    validate::validate(&graph)?;
    Ok(graph)
}

// ---------------------------------------------------------------------------
// Derivation desugaring
// ---------------------------------------------------------------------------

/// Resolve a source expression to a `PropId`. For direct prop refs this is a
/// simple lookup. For derivations, it creates a synthetic node (or reuses a
/// deduplicated one) and wires up constraint edges from each input.
fn resolve_source_expr(
    b: &mut Builder,
    expr: &AstSourceExpr,
    deriv_cache: &mut HashMap<(String, Vec<PropId>), PropId>,
) -> Result<PropId, ObgraphError> {
    match expr {
        AstSourceExpr::PropRef { node, prop } => b.resolve_prop(node, prop),
        AstSourceExpr::Derivation { function, args } => {
            // Recursively resolve each argument to a PropId.
            let mut input_props = Vec::with_capacity(args.len());
            for arg in args {
                input_props.push(resolve_source_expr(b, arg, deriv_cache)?);
            }

            // Dedup key: (function_name, sorted input PropIds).
            let mut sorted_inputs = input_props.clone();
            sorted_inputs.sort();
            let key = (function.clone(), sorted_inputs);

            if let Some(&cached_prop) = deriv_cache.get(&key) {
                return Ok(cached_prop);
            }

            // Infer domain: if all input source nodes share a domain, use it.
            let domain = {
                let mut domains = input_props.iter().map(|pid| {
                    let node_id = b.properties[pid.index()].node;
                    b.nodes[node_id.index()].domain
                });
                let first = domains.next().unwrap_or(None);
                if domains.all(|d| d == first) { first } else { None }
            };

            // Create synthetic derivation node: ident=None, display_name=None.
            let node_id = b.alloc_node(
                None,           // no ident — derivation node
                None,           // no display_name
                true,           // always anchored
                false,          // never selected
                domain,
            )?;

            // Single property whose name is the function name.
            let prop_id = b.alloc_derivation_property(node_id, function)?;

            // Wire input constraints: each input prop -> derivation property.
            for &input_pid in &input_props {
                let eid = b.push_edge(Edge::Constraint {
                    dest_prop: prop_id,
                    source_prop: input_pid,
                    operation: None,
                });
                b.record_prop_edge(prop_id, eid);
                b.record_prop_edge(input_pid, eid);
            }

            // If this derivation is in a domain, register it as a member.
            if let Some(did) = domain {
                b.domains[did.index()].members.push(node_id);
            }

            deriv_cache.insert(key, prop_id);
            Ok(prop_id)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ast::{
        AstAnchor, AstConstraint, AstDomain, AstGraph, AstNode, AstProperty, AstSourceExpr,
    };

    // Helper to build a minimal AstNode (non-root).
    fn ast_node(ident: &str, props: Vec<(&str, bool, bool)>) -> AstNode {
        ast_node_root(ident, props, false)
    }

    // Helper to build a minimal AstNode with explicit is_anchored flag.
    // Props are (name, critical, constrained).
    fn ast_node_root(ident: &str, props: Vec<(&str, bool, bool)>, is_anchored: bool) -> AstNode {
        AstNode {
            ident: ident.to_string(),
            display_name: None,
            is_anchored,
            is_selected: false,
            properties: props
                .into_iter()
                .map(|(name, critical, constrained)| AstProperty {
                    name: name.to_string(),
                    critical,
                    constrained,
                })
                .collect(),
        }
    }

    // -----------------------------------------------------------------------
    // Test: empty graph
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_graph() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![],
            anchors: vec![],
            constraints: vec![],
        };
        let g = build(ast).expect("empty graph should build");
        assert_eq!(g.nodes.len(), 0);
        assert_eq!(g.properties.len(), 0);
        assert_eq!(g.edges.len(), 0);
        assert_eq!(g.domains.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test: single node with properties
    // -----------------------------------------------------------------------

    #[test]
    fn test_single_node_properties() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![ast_node_root(
                "ca",
                vec![
                    ("subject.common_name", false, true),  // @constrained
                    ("subject.org", false, true),           // @constrained
                    ("public_key", false, true),            // @constrained
                ],
                true,
            )],
            anchors: vec![],
            constraints: vec![],
        };
        let g = build(ast).expect("single node should build");

        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.properties.len(), 3);

        let node = &g.nodes[0];
        assert_eq!(node.ident.as_deref(), Some("ca"));
        assert_eq!(node.id, NodeId(0));
        assert_eq!(node.properties.len(), 3);
        assert_eq!(node.properties[0], PropId(0));
        assert_eq!(node.properties[1], PropId(1));
        assert_eq!(node.properties[2], PropId(2));

        assert_eq!(g.properties[0].name, "subject.common_name");
        assert!(g.properties[0].constrained);
        assert!(!g.properties[0].critical);
        assert_eq!(g.properties[1].name, "subject.org");
        assert_eq!(g.properties[2].name, "public_key");
    }

    // -----------------------------------------------------------------------
    // Test: PKI example — nodes, properties, links, constraints
    // -----------------------------------------------------------------------

    fn make_pki_ast() -> AstGraph {
        AstGraph {
            domains: vec![],
            nodes: vec![
                // ca is a root (no parent anchor).
                ast_node_root(
                    "ca",
                    vec![
                        ("subject.common_name", false, true),  // @constrained
                        ("subject.org", false, true),           // @constrained
                        ("public_key", false, true),            // @constrained
                    ],
                    true,
                ),
                ast_node(
                    "cert",
                    vec![
                        ("issuer.common_name", true, false),    // @critical
                        ("issuer.org", true, false),            // @critical
                        ("subject.common_name", false, false),  // informational (receives constraint from revocation::crl)
                        ("subject.org", false, true),           // @constrained
                        ("public_key", true, false),            // @critical
                        ("signature", true, false),             // @critical
                    ],
                ),
                ast_node(
                    "tls",
                    vec![
                        ("server_cert", true, false),           // @critical
                        ("cipher_suite", false, true),          // @constrained
                    ],
                ),
                // revocation is also a root (no parent anchor).
                ast_node_root(
                    "revocation",
                    vec![("crl", false, true)],                 // @constrained
                    true,
                ),
            ],
            anchors: vec![
                AstAnchor {
                    child_ident: "cert".to_string(),
                    parent_ident: "ca".to_string(),
                    operation: Some("sign".to_string()),
                },
                AstAnchor {
                    child_ident: "tls".to_string(),
                    parent_ident: "cert".to_string(),
                    operation: None,
                },
            ],
            constraints: vec![
                // ca::subject.common_name -> cert::issuer.common_name
                AstConstraint {
                    dest_node: "cert".to_string(),
                    dest_prop: "issuer.common_name".to_string(),
                    source: AstSourceExpr::PropRef { node: "ca".to_string(), prop: "subject.common_name".to_string() },
                    operation: Some("equality".to_string()),
                },
                // ca::subject.org -> cert::issuer.org
                AstConstraint {
                    dest_node: "cert".to_string(),
                    dest_prop: "issuer.org".to_string(),
                    source: AstSourceExpr::PropRef { node: "ca".to_string(), prop: "subject.org".to_string() },
                    operation: Some("equality".to_string()),
                },
                // ca::public_key -> cert::signature
                AstConstraint {
                    dest_node: "cert".to_string(),
                    dest_prop: "signature".to_string(),
                    source: AstSourceExpr::PropRef { node: "ca".to_string(), prop: "public_key".to_string() },
                    operation: Some("verified_by".to_string()),
                },
                // revocation::crl -> cert::subject.common_name
                AstConstraint {
                    dest_node: "cert".to_string(),
                    dest_prop: "subject.common_name".to_string(),
                    source: AstSourceExpr::PropRef { node: "revocation".to_string(), prop: "crl".to_string() },
                    operation: Some("not_in".to_string()),
                },
            ],
        }
    }

    #[test]
    fn test_pki_node_count() {
        let g = build(make_pki_ast()).expect("PKI graph should build");
        assert_eq!(g.nodes.len(), 4);
    }

    #[test]
    fn test_pki_property_ids() {
        let g = build(make_pki_ast()).expect("PKI graph should build");

        // P0..P2: ca
        assert_eq!(g.properties[0].name, "subject.common_name");
        assert_eq!(g.properties[0].node, NodeId(0)); // ca
        assert!(g.properties[0].constrained);
        assert!(!g.properties[0].critical);

        assert_eq!(g.properties[1].name, "subject.org");
        assert_eq!(g.properties[2].name, "public_key");

        // P3..P8: cert
        assert_eq!(g.properties[3].name, "issuer.common_name");
        assert_eq!(g.properties[3].node, NodeId(1)); // cert
        assert!(g.properties[3].critical);
        assert!(!g.properties[3].constrained);

        assert_eq!(g.properties[4].name, "issuer.org");
        assert!(g.properties[4].critical);

        assert_eq!(g.properties[5].name, "subject.common_name");
        assert!(!g.properties[5].constrained); // informational (receives constraint edge)

        assert_eq!(g.properties[6].name, "subject.org");
        assert!(g.properties[6].constrained);

        assert_eq!(g.properties[7].name, "public_key");
        assert!(g.properties[7].critical);

        assert_eq!(g.properties[8].name, "signature");
        assert!(g.properties[8].critical);

        // P9..P10: tls
        assert_eq!(g.properties[9].name, "server_cert");
        assert_eq!(g.properties[9].node, NodeId(2)); // tls
        assert!(g.properties[9].critical);

        assert_eq!(g.properties[10].name, "cipher_suite");
        assert!(g.properties[10].constrained);

        // P11: revocation
        assert_eq!(g.properties[11].name, "crl");
        assert_eq!(g.properties[11].node, NodeId(3)); // revocation
        assert!(g.properties[11].constrained);

        assert_eq!(g.properties.len(), 12);
    }

    #[test]
    fn test_pki_anchors() {
        let g = build(make_pki_ast()).expect("PKI graph should build");

        // E0: ca -> cert (sign)
        // E1: cert -> tls
        let anchor_edges: Vec<_> = g
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_anchor())
            .collect();
        assert_eq!(anchor_edges.len(), 2);

        match &anchor_edges[0].1 {
            Edge::Anchor {
                child,
                parent,
                operation,
            } => {
                assert_eq!(*child, NodeId(1));  // cert
                assert_eq!(*parent, NodeId(0)); // ca
                assert_eq!(operation.as_deref(), Some("sign"));
            }
            _ => panic!("expected Anchor"),
        }

        match &anchor_edges[1].1 {
            Edge::Anchor {
                child,
                parent,
                operation,
            } => {
                assert_eq!(*child, NodeId(2));  // tls
                assert_eq!(*parent, NodeId(1)); // cert
                assert!(operation.is_none());
            }
            _ => panic!("expected Anchor"),
        }
    }

    #[test]
    fn test_pki_constraints() {
        let g = build(make_pki_ast()).expect("PKI graph should build");

        let constraint_edges: Vec<_> = g
            .edges
            .iter()
            .filter(|e| e.is_constraint())
            .collect();
        assert_eq!(constraint_edges.len(), 4);

        // E2: P0 -> P3 (equality)
        match &constraint_edges[0] {
            Edge::Constraint {
                dest_prop,
                source_prop,
                operation,
            } => {
                assert_eq!(*source_prop, PropId(0)); // ca::subject.common_name
                assert_eq!(*dest_prop, PropId(3));   // cert::issuer.common_name
                assert_eq!(operation.as_deref(), Some("equality"));
            }
            _ => panic!("expected Constraint"),
        }

        // E5: P11 -> P5 (not_in)
        match &constraint_edges[3] {
            Edge::Constraint {
                dest_prop,
                source_prop,
                operation,
            } => {
                assert_eq!(*source_prop, PropId(11)); // revocation::crl
                assert_eq!(*dest_prop, PropId(5));    // cert::subject.common_name
                assert_eq!(operation.as_deref(), Some("not_in"));
            }
            _ => panic!("expected Constraint"),
        }
    }

    #[test]
    fn test_pki_adjacency() {
        let g = build(make_pki_ast()).expect("PKI graph should build");

        // ca (NodeId(0)) should have one child anchor: cert
        let ca_children = g.children_of(NodeId(0));
        assert_eq!(ca_children.len(), 1);

        // cert (NodeId(1)) should have one parent (ca) and one child (tls)
        assert!(g.node_parent.contains_key(&NodeId(1)));
        let cert_children = g.children_of(NodeId(1));
        assert_eq!(cert_children.len(), 1);

        // tls (NodeId(2)) should have a parent (cert) and no children
        assert!(g.node_parent.contains_key(&NodeId(2)));
        assert_eq!(g.children_of(NodeId(2)).len(), 0);

        // P0 (ca::subject.common_name) is involved in one constraint edge.
        let p0_edges = g.edges_on_prop(PropId(0));
        assert_eq!(p0_edges.len(), 1);
        assert!(g.edges[p0_edges[0].index()].is_constraint());
    }

    // -----------------------------------------------------------------------
    // Test: domains
    // -----------------------------------------------------------------------

    #[test]
    fn test_domains() {
        // All nodes are roots since there are no anchors in this test.
        let ast = AstGraph {
            domains: vec![AstDomain {
                display_name: "Infra".to_string(),
                nodes: vec![
                    ast_node_root("alpha", vec![("x", false, true)], true),  // @constrained
                    ast_node_root("beta", vec![("y", true, false)], true),   // @critical
                ],
            }],
            nodes: vec![ast_node_root("gamma", vec![("z", false, true)], true)],  // @constrained
            anchors: vec![],
            constraints: vec![],
        };
        let g = build(ast).expect("domain graph should build");

        // Domain nodes come first, then top-level nodes.
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.nodes[0].ident.as_deref(), Some("alpha"));
        assert_eq!(g.nodes[0].id, NodeId(0));
        assert_eq!(g.nodes[0].domain, Some(DomainId(0)));
        assert_eq!(g.nodes[1].ident.as_deref(), Some("beta"));
        assert_eq!(g.nodes[1].domain, Some(DomainId(0)));
        assert_eq!(g.nodes[2].ident.as_deref(), Some("gamma"));
        assert_eq!(g.nodes[2].domain, None);

        assert_eq!(g.domains.len(), 1);
        assert_eq!(g.domains[0].display_name, "Infra");
        assert_eq!(g.domains[0].members, vec![NodeId(0), NodeId(1)]);

        // Properties: P0=alpha::x, P1=beta::y, P2=gamma::z
        assert_eq!(g.properties.len(), 3);
        assert_eq!(g.properties[0].name, "x");
        assert_eq!(g.properties[1].name, "y");
        assert_eq!(g.properties[2].name, "z");
    }

    // -----------------------------------------------------------------------
    // Test: duplicate node ident is rejected
    // -----------------------------------------------------------------------

    #[test]
    fn test_duplicate_node_rejected() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![
                ast_node("foo", vec![]),
                ast_node("foo", vec![]),
            ],
            anchors: vec![],
            constraints: vec![],
        };
        assert!(build(ast).is_err());
    }

    // -----------------------------------------------------------------------
    // Test: unknown node in anchor is rejected
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_node_in_anchor_rejected() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![ast_node_root("known", vec![], false)],
            anchors: vec![AstAnchor {
                child_ident: "known".to_string(),
                parent_ident: "unknown".to_string(), // "unknown" doesn't exist
                operation: None,
            }],
            constraints: vec![],
        };
        assert!(build(ast).is_err());
    }

    // -----------------------------------------------------------------------
    // Test: unknown prop in constraint is rejected
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_prop_in_constraint_rejected() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![
                ast_node_root("a", vec![("x", false, true)], true),  // @constrained
                ast_node("b", vec![("y", true, false)]),              // @critical
            ],
            anchors: vec![AstAnchor {
                child_ident: "b".to_string(),
                parent_ident: "a".to_string(),
                operation: None,
            }],
            constraints: vec![AstConstraint {
                dest_node: "b".to_string(),
                dest_prop: "y".to_string(),
                source: AstSourceExpr::PropRef { node: "a".to_string(), prop: "NONEXISTENT".to_string() },
                operation: None,
            }],
        };
        assert!(build(ast).is_err());
    }

    // -----------------------------------------------------------------------
    // Test: multi-parent anchor is rejected
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_parent_rejected() {
        let ast = AstGraph {
            domains: vec![],
            nodes: vec![
                ast_node_root("p1", vec![], true),
                ast_node_root("p2", vec![], true),
                ast_node("child", vec![]),
            ],
            anchors: vec![
                AstAnchor {
                    child_ident: "child".to_string(),
                    parent_ident: "p1".to_string(),
                    operation: None,
                },
                AstAnchor {
                    child_ident: "child".to_string(),
                    parent_ident: "p2".to_string(),
                    operation: None,
                },
            ],
            constraints: vec![],
        };
        assert!(build(ast).is_err());
    }
}
