//! Trust propagation — fixed-point worklist algorithm (DESIGN.md §2.6).

use std::collections::{HashMap, VecDeque};

use super::types::{DerivId, Edge, Graph, NodeId, PropId};

/// The result of trust propagation: per-node and per-property trust booleans.
#[derive(Debug, Clone)]
pub struct TrustState {
    node_trusted: HashMap<NodeId, bool>,
    prop_trusted: HashMap<PropId, bool>,
}

impl TrustState {
    pub fn is_node_trusted(&self, id: NodeId) -> bool {
        self.node_trusted.get(&id).copied().unwrap_or(false)
    }

    pub fn is_prop_trusted(&self, id: PropId) -> bool {
        self.prop_trusted.get(&id).copied().unwrap_or(false)
    }
}

/// Run the trust propagation algorithm on a validated graph.
///
/// Returns the trust state for every node and property.
///
/// Algorithm (DESIGN.md §2.6):
///
/// 1. Initialize root nodes as Trusted, all others as Untrusted.
/// 2. Initialize Always properties on Trusted nodes as Trusted.
/// 3. Run a worklist starting from all initially-Trusted properties.
/// 4. For each property dequeued, propagate trust via Constraint edges and
///    Derivations, then check whether its node can now become Trusted.
pub fn propagate(graph: &Graph) -> TrustState {
    // --- State maps (true = Trusted, false = Untrusted) ---
    let mut node_trusted: HashMap<NodeId, bool> =
        graph.nodes.iter().map(|n| (n.id, n.is_root)).collect();

    let mut prop_trusted: HashMap<PropId, bool> =
        graph.properties.iter().map(|p| (p.id, false)).collect();

    // --- Build a reverse index: PropId → Vec<DerivId> ---
    // Maps each property to every derivation that lists it as an input.
    let mut prop_to_derivs: HashMap<PropId, Vec<DerivId>> = HashMap::new();
    for deriv in &graph.derivations {
        for &inp in &deriv.inputs {
            prop_to_derivs.entry(inp).or_default().push(deriv.id);
        }
    }

    // --- Worklist ---
    let mut worklist: VecDeque<PropId> = VecDeque::new();

    // Initialize @constrained properties on Trusted (root) nodes.
    for node in &graph.nodes {
        if node_trusted[&node.id] {
            for &pid in &node.properties {
                let prop = &graph.properties[pid.index()];
                if prop.constrained {
                    *prop_trusted.entry(pid).or_insert(false) = true;
                    worklist.push_back(pid);
                }
            }
        }
    }

    // --- Fixed-point iteration ---
    while let Some(p) = worklist.pop_front() {
        // Only process currently-trusted properties.
        if !prop_trusted[&p] {
            continue;
        }

        // 1. Propagate through Constraint edges where p is the source_prop.
        for &eid in graph.edges_on_prop(p) {
            if let Edge::Constraint {
                dest_prop,
                source_prop,
                ..
            } = &graph.edges[eid.index()]
                && *source_prop == p
            {
                // Trust flows from p (source) to dest_prop.
                let dest_entry = prop_trusted.entry(*dest_prop).or_insert(false);
                if !*dest_entry {
                    *dest_entry = true;
                    worklist.push_back(*dest_prop);
                }
            }
        }

        // 2. Propagate through Derivations where p is an input.
        if let Some(deriv_ids) = prop_to_derivs.get(&p) {
            for &did in deriv_ids {
                let deriv = &graph.derivations[did.index()];
                // Derivation output is trusted only if ALL inputs are trusted.
                let all_inputs_trusted = deriv
                    .inputs
                    .iter()
                    .all(|inp| prop_trusted.get(inp).copied().unwrap_or(false));
                if all_inputs_trusted {
                    let out = deriv.output_prop;
                    let out_entry = prop_trusted.entry(out).or_insert(false);
                    if !*out_entry {
                        *out_entry = true;
                        worklist.push_back(out);
                    }
                }
            }
        }

        // 3. Check if p's node should now become Trusted.
        let prop = &graph.properties[p.index()];
        let nid = prop.node;
        if !node_trusted[&nid] {
            let node = &graph.nodes[nid.index()];

            // Parent condition: root OR has a trusted parent.
            let parent_ok = node.is_root
                || graph
                    .parent_of(nid)
                    .map(|pid| node_trusted.get(&pid).copied().unwrap_or(false))
                    .unwrap_or(false);

            if parent_ok {
                // All @critical properties must be Trusted.
                let all_critical_trusted = node.properties.iter().all(|&qid| {
                    let q = &graph.properties[qid.index()];
                    !q.critical
                        || prop_trusted.get(&qid).copied().unwrap_or(false)
                });

                if all_critical_trusted {
                    *node_trusted.entry(nid).or_insert(false) = true;

                    // Node becoming Trusted unlocks its @constrained properties.
                    for &qid in &node.properties {
                        let q = &graph.properties[qid.index()];
                        if q.constrained {
                            let q_entry = prop_trusted.entry(qid).or_insert(false);
                            if !*q_entry {
                                *q_entry = true;
                                worklist.push_back(qid);
                            }
                        }
                    }
                }
            }
        }
    }

    TrustState {
        node_trusted,
        prop_trusted,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::model::types::{
        Derivation, Edge, EdgeId, Graph, Node, NodeId, Property, PropId,
    };

    // -----------------------------------------------------------------------
    // Helpers to build minimal Graph structs for testing.
    // -----------------------------------------------------------------------

    /// Build the prop_edges adjacency map from the edge list.
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
                Edge::DerivInput {
                    source_prop,
                    target_deriv: _,
                } => {
                    map.entry(*source_prop).or_default().push(eid);
                }
                Edge::Anchor { .. } => {}
            }
        }
        map
    }

    /// Build node_children and node_parent from the edge list.
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

    /// Convenience: build a complete Graph from its components.
    fn make_graph(
        nodes: Vec<Node>,
        properties: Vec<Property>,
        derivations: Vec<Derivation>,
        edges: Vec<Edge>,
    ) -> Graph {
        let prop_edges = build_prop_edges(&edges);
        let (node_children, node_parent) = build_node_adjacency(&edges);
        Graph {
            nodes,
            properties,
            derivations,
            edges,
            domains: vec![],
            prop_edges,
            node_children,
            node_parent,
        }
    }

    /// Convenience: create a Node with no domain.
    fn node(id: u32, ident: &str, properties: Vec<PropId>, is_root: bool) -> Node {
        Node {
            id: NodeId(id),
            ident: ident.to_string(),
            display_name: None,
            properties,
            domain: None,
            is_root,
            is_selected: false,
        }
    }

    /// Convenience: create a Property.
    fn prop(id: u32, node_id: u32, name: &str, critical: bool, constrained: bool) -> Property {
        Property {
            id: PropId(id),
            node: NodeId(node_id),
            name: name.to_string(),
            critical,
            constrained,
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: simple root-only graph — all Always properties are trusted.
    // -----------------------------------------------------------------------

    #[test]
    fn root_only_always_props_trusted() {
        // Graph: one root node with two Always properties and no edges.
        let nodes = vec![node(0, "root", vec![PropId(0), PropId(1)], true)];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", false, true),
        ];
        let graph = make_graph(nodes, properties, vec![], vec![]);

        let state = propagate(&graph);

        assert!(state.is_node_trusted(NodeId(0)), "root should be trusted");
        assert!(
            state.is_prop_trusted(PropId(0)),
            "Always prop P0 should be trusted"
        );
        assert!(
            state.is_prop_trusted(PropId(1)),
            "Always prop P1 should be trusted"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: chain where trust fully propagates.
    //
    // root (@root) -> child (non-root)
    //   root has Critical property P0 (Always, so always trusted for root)
    //   child has Critical property P1, constrained by P0
    // -----------------------------------------------------------------------

    #[test]
    fn chain_full_propagation() {
        // Node 0: root, has P0 (Always)
        // Node 1: child of root, has P1 (Critical)
        // Constraint: P0 -> P1  (P1 gets trust from P0)
        // Link: node1 <- node0
        let nodes = vec![
            node(0, "root", vec![PropId(0)], true),
            node(1, "child", vec![PropId(1)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 1, "p1", true, false),
        ];
        let edges = vec![
            // Link: parent=0, child=1
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            // Constraint: source=P0, dest=P1
            Edge::Constraint {
                dest_prop: PropId(1),
                source_prop: PropId(0),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let state = propagate(&graph);

        assert!(state.is_node_trusted(NodeId(0)), "root should be trusted");
        assert!(state.is_prop_trusted(PropId(0)), "P0 (Always) should be trusted");
        assert!(state.is_prop_trusted(PropId(1)), "P1 constrained by trusted P0");
        assert!(
            state.is_node_trusted(NodeId(1)),
            "child: parent trusted + critical P1 trusted"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: chain where a missing constraint blocks propagation.
    //
    // root -> child
    //   child has Critical property P1 with NO incoming constraint.
    // -----------------------------------------------------------------------

    #[test]
    fn chain_missing_constraint_blocks() {
        let nodes = vec![
            node(0, "root", vec![], true),
            node(1, "child", vec![PropId(0)], false),
        ];
        let properties = vec![prop(0, 1, "p0", true, false)];
        let edges = vec![Edge::Anchor {
            parent: NodeId(0),
            child: NodeId(1),
            operation: None,
        }];
        let graph = make_graph(nodes, properties, vec![], edges);

        let state = propagate(&graph);

        assert!(state.is_node_trusted(NodeId(0)), "root is trusted");
        assert!(
            !state.is_prop_trusted(PropId(0)),
            "P0 has no incoming constraint — untrusted"
        );
        assert!(
            !state.is_node_trusted(NodeId(1)),
            "child: critical P0 is untrusted — node is untrusted"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: derivation trust — all inputs trusted → output trusted.
    // -----------------------------------------------------------------------

    #[test]
    fn derivation_all_inputs_trusted() {
        // root has P0 (Always) and P1 (Always).
        // Derivation D0: inputs=[P0,P1], output=P2 (on child node).
        // child has P2 (Critical), constrained by D0's output (but here the
        // derivation output IS P2, simulating a derivation that produces P2).
        //
        // Simpler model: derivation output is P2 (an anonymous prop on child).
        // Constraint: source=P2 (deriv output), dest=P3 (child critical prop).
        // When P0 and P1 both trusted → D0 fires → P2 trusted → P3 trusted → child trusted.

        // Node 0: root, props [P0, P1]
        // Node 1: child, props [P2 (output of deriv, Always), P3 (Critical)]
        // Deriv D0: inputs=[P0,P1], output=P2
        // Constraint: P2 -> P3

        let nodes = vec![
            node(0, "root", vec![PropId(0), PropId(1)], true),
            node(1, "child", vec![PropId(2), PropId(3)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", false, true),
            // P2 is the derivation output — treated as Constrained (gets trust from deriv)
            prop(2, 1, "p2_deriv_out", false, true),
            prop(3, 1, "p3", true, false),
        ];
        let derivations = vec![Derivation {
            id: DerivId(0),
            operation: "combine".to_string(),
            inputs: vec![PropId(0), PropId(1)],
            output_prop: PropId(2),
        }];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Constraint {
                dest_prop: PropId(3),
                source_prop: PropId(2),
                operation: None,
            },
            Edge::DerivInput {
                source_prop: PropId(0),
                target_deriv: DerivId(0),
            },
            Edge::DerivInput {
                source_prop: PropId(1),
                target_deriv: DerivId(0),
            },
        ];
        let graph = make_graph(nodes, properties, derivations, edges);

        let state = propagate(&graph);

        assert!(state.is_prop_trusted(PropId(0)), "P0 Always → trusted");
        assert!(state.is_prop_trusted(PropId(1)), "P1 Always → trusted");
        assert!(
            state.is_prop_trusted(PropId(2)),
            "P2 deriv output: all inputs trusted → trusted"
        );
        assert!(
            state.is_prop_trusted(PropId(3)),
            "P3 constrained by trusted P2 → trusted"
        );
        assert!(state.is_node_trusted(NodeId(1)), "child node trusted");
    }

    // -----------------------------------------------------------------------
    // Test 5: derivation trust — one input untrusted → output untrusted.
    // -----------------------------------------------------------------------

    #[test]
    fn derivation_one_input_untrusted() {
        // root has P0 (Always) and P1 (Critical, no constraint → untrusted).
        // Derivation D0: inputs=[P0,P1], output=P2.
        // Since P1 is untrusted, D0 output stays untrusted.

        let nodes = vec![
            node(0, "root", vec![PropId(0), PropId(1)], true),
            node(1, "child", vec![PropId(2)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", true, false), // no incoming constraint
            prop(2, 1, "p2_deriv_out", false, true),
        ];
        let derivations = vec![Derivation {
            id: DerivId(0),
            operation: "combine".to_string(),
            inputs: vec![PropId(0), PropId(1)],
            output_prop: PropId(2),
        }];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::DerivInput {
                source_prop: PropId(0),
                target_deriv: DerivId(0),
            },
            Edge::DerivInput {
                source_prop: PropId(1),
                target_deriv: DerivId(0),
            },
        ];
        let graph = make_graph(nodes, properties, derivations, edges);

        let state = propagate(&graph);

        assert!(state.is_prop_trusted(PropId(0)), "P0 Always → trusted");
        assert!(
            !state.is_prop_trusted(PropId(1)),
            "P1 Critical, no constraint → untrusted"
        );
        assert!(
            !state.is_prop_trusted(PropId(2)),
            "P2 deriv output: P1 untrusted → output untrusted"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: PKI example from Appendix A.5.
    //
    // Nodes:
    //   ca        @root       properties: P0(subject.common_name, Always),
    //                                     P1(public_key, Always),
    //                                     P2(crl_url, Always)
    //   revocation @root      properties: P11(crl, Always)
    //   cert      child of ca properties: P3(issuer.common_name, Critical),
    //                                     P4(signature, Critical),
    //                                     P5(subject.common_name, Always),
    //                                     P6(not_after, Constrained),
    //                                     P7(public_key, Critical)
    //   tls       child of cert properties: P8(server_name, Critical),
    //                                       P9(not_after, Critical),
    //                                       P10(cipher_suite, Constrained)
    //
    // Constraints (trust flows source → dest):
    //   P0  → P3   (ca.subject.common_name constrains cert.issuer.common_name)
    //   P1  → P4   (ca.public_key constrains cert.signature)
    //   P11 → P5   (revocation.crl constrains cert.subject.common_name)
    //   P2  → P8   (ca.crl_url ... constrains tls.server_name — via some path)
    //             NOTE: We must route through cert for trust to propagate to tls.
    //             Since P7 has no constraint, cert stays Untrusted, so tls stays
    //             Untrusted regardless. We just set P8 constrained by P2.
    //
    // Expected final state (per Appendix A.5):
    //   ca        Trusted
    //   P0,P1,P2  Trusted  (Always on root)
    //   revocation Trusted
    //   P11       Trusted  (Always on root)
    //   cert      Untrusted (P7 has no incoming constraint)
    //   P3        Trusted  (constrained by P0)
    //   P4        Trusted  (constrained by P1)
    //   P5        Trusted  (constrained by P11)
    //   P6        Untrusted (no incoming constraint)
    //   P7        Untrusted (no incoming constraint)
    //   P8        Trusted  (constrained by P2, even though cert untrusted)
    //   tls       Untrusted (parent cert untrusted; P9 unconstrained)
    //   P9,P10    Untrusted (node not trusted)
    // -----------------------------------------------------------------------

    #[test]
    fn pki_example_appendix_a5() {
        // Node IDs
        let ca_id = NodeId(0);
        let rev_id = NodeId(1);
        let cert_id = NodeId(2);
        let tls_id = NodeId(3);

        // Property IDs
        let p0 = PropId(0); // ca.subject.common_name   Always
        let p1 = PropId(1); // ca.public_key             Always
        let p2 = PropId(2); // ca.crl_url                Always
        let p11 = PropId(11); // revocation.crl           Always
        let p3 = PropId(3); // cert.issuer.common_name   Critical
        let p4 = PropId(4); // cert.signature            Critical
        let p5 = PropId(5); // cert.subject.common_name  Always (constrained by P11)
        let p6 = PropId(6); // cert.not_after            Constrained
        let p7 = PropId(7); // cert.public_key           Critical  (no constraint → blocks cert)
        let p8 = PropId(8); // tls.server_name           Critical
        let p9 = PropId(9); // tls.not_after             Critical
        let p10 = PropId(10); // tls.cipher_suite         Constrained

        let nodes = vec![
            node(0, "ca", vec![p0, p1, p2], true),
            node(1, "revocation", vec![p11], true),
            node(2, "cert", vec![p3, p4, p5, p6, p7], false),
            node(3, "tls", vec![p8, p9, p10], false),
        ];

        // Properties vector (indexed 0..=11, use placeholder for gaps)
        let mut properties_map: Vec<Option<Property>> = (0..12).map(|_| None).collect();
        properties_map[0] = Some(prop(0, 0, "subject.common_name", false, true));
        properties_map[1] = Some(prop(1, 0, "public_key", false, true));
        properties_map[2] = Some(prop(2, 0, "crl_url", false, true));
        properties_map[3] = Some(prop(3, 2, "issuer.common_name", true, false));
        properties_map[4] = Some(prop(4, 2, "signature", true, false));
        properties_map[5] = Some(prop(5, 2, "subject.common_name", false, true));
        properties_map[6] = Some(prop(6, 2, "not_after", false, true));
        properties_map[7] = Some(prop(7, 2, "public_key", true, false));
        properties_map[8] = Some(prop(8, 3, "server_name", true, false));
        properties_map[9] = Some(prop(9, 3, "not_after", true, false));
        properties_map[10] = Some(prop(10, 3, "cipher_suite", false, true));
        properties_map[11] = Some(prop(11, 1, "crl", false, true));

        // Fill in placeholders with dummy entries so that indexing by PropId works.
        // (The graph only indexes into properties by PropId.index(), so all slots
        // used by real PropIds must be present.)
        let properties: Vec<Property> = properties_map
            .into_iter()
            .enumerate()
            .map(|(i, opt)| {
                opt.unwrap_or_else(|| Property {
                    id: PropId(i as u32),
                    node: NodeId(0),
                    name: format!("_placeholder_{}", i),
                    critical: true,
                    constrained: false,
                })
            })
            .collect();

        let edges = vec![
            // Links
            Edge::Anchor {
                parent: ca_id,
                child: cert_id,
                operation: None,
            },
            Edge::Anchor {
                parent: cert_id,
                child: tls_id,
                operation: None,
            },
            // Constraints (trust flows source → dest)
            Edge::Constraint {
                source_prop: p0,
                dest_prop: p3,
                operation: None,
            }, // ca.subject → cert.issuer
            Edge::Constraint {
                source_prop: p1,
                dest_prop: p4,
                operation: None,
            }, // ca.public_key → cert.signature
            Edge::Constraint {
                source_prop: p11,
                dest_prop: p5,
                operation: None,
            }, // rev.crl → cert.subject (Always — unlocked when cert trusted, but already constrained)
            Edge::Constraint {
                source_prop: p2,
                dest_prop: p8,
                operation: None,
            }, // ca.crl_url → tls.server_name
               // P6, P7, P9, P10 intentionally have no incoming constraints.
        ];

        let graph = make_graph(nodes, properties, vec![], edges);
        let state = propagate(&graph);

        // Root nodes
        assert!(state.is_node_trusted(ca_id), "ca: root → Trusted");
        assert!(state.is_node_trusted(rev_id), "revocation: root → Trusted");

        // Always props on roots
        assert!(state.is_prop_trusted(p0), "P0 (Always on ca) → Trusted");
        assert!(state.is_prop_trusted(p1), "P1 (Always on ca) → Trusted");
        assert!(state.is_prop_trusted(p2), "P2 (Always on ca) → Trusted");
        assert!(state.is_prop_trusted(p11), "P11 (Always on revocation) → Trusted");

        // cert: Untrusted because P7 (Critical) has no incoming constraint.
        assert!(!state.is_node_trusted(cert_id), "cert: P7 untrusted → cert Untrusted");

        // cert properties with constraints
        assert!(state.is_prop_trusted(p3), "P3 constrained by P0 → Trusted");
        assert!(state.is_prop_trusted(p4), "P4 constrained by P1 → Trusted");

        // P5 is Always on cert. cert is Untrusted, so P5 stays Untrusted
        // (Always props are only trusted when their node is trusted).
        // However the constraint P11→P5 exists. The constraint propagation
        // sets P5=Trusted via the constraint path — this is consistent with the
        // expected table: P5=Trusted.
        assert!(state.is_prop_trusted(p5), "P5 constrained by trusted P11 → Trusted");

        // Unconstrained props on cert
        assert!(!state.is_prop_trusted(p6), "P6: no incoming constraint → Untrusted");
        assert!(!state.is_prop_trusted(p7), "P7: no incoming constraint → Untrusted (blocks cert)");

        // tls: Untrusted (parent cert is Untrusted)
        assert!(!state.is_node_trusted(tls_id), "tls: parent cert Untrusted → tls Untrusted");

        // P8 is constrained by trusted P2, so it becomes Trusted via constraint propagation
        // even though tls node itself is Untrusted.
        assert!(state.is_prop_trusted(p8), "P8 constrained by trusted P2 → Trusted");

        // P9, P10: no incoming constraints and node untrusted
        assert!(!state.is_prop_trusted(p9), "P9: no constraint → Untrusted");
        assert!(!state.is_prop_trusted(p10), "P10: no constraint → Untrusted");
    }
}
