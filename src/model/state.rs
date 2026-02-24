//! State propagation — fixed-point worklist algorithm (GRAPH_MODEL.md §2.6).

use std::collections::{HashMap, VecDeque};

use super::types::{DerivId, Edge, Graph, NodeId, PropId};

/// The result of state propagation: per-node anchored status and per-property
/// constrained status.
#[derive(Debug, Clone)]
pub struct StateResult {
    node_anchored: HashMap<NodeId, bool>,
    prop_constrained: HashMap<PropId, bool>,
}

impl StateResult {
    pub fn is_node_anchored(&self, id: NodeId) -> bool {
        self.node_anchored.get(&id).copied().unwrap_or(false)
    }

    /// A node is verified when it is anchored AND all its @critical properties
    /// are constrained.
    pub fn is_node_verified(&self, graph: &Graph, id: NodeId) -> bool {
        if !self.is_node_anchored(id) {
            return false;
        }
        let node = &graph.nodes[id.index()];
        node.properties.iter().all(|&pid| {
            let prop = &graph.properties[pid.index()];
            !prop.critical || self.is_prop_constrained(pid)
        })
    }

    pub fn is_prop_constrained(&self, id: PropId) -> bool {
        self.prop_constrained.get(&id).copied().unwrap_or(false)
    }
}

/// Run the state propagation algorithm on a validated graph.
///
/// Returns the state result for every node and property.
///
/// Algorithm (GRAPH_MODEL.md §2.6):
///
/// Two worklists (node_worklist and prop_worklist) with an outer loop.
/// Property phase runs to exhaustion before node phase.
/// Constraints only fire from anchored+verified nodes.
/// Node anchoring is separate from node verification.
pub fn propagate(graph: &Graph) -> StateResult {
    // --- State maps ---
    let mut node_anchored: HashMap<NodeId, bool> =
        graph.nodes.iter().map(|n| (n.id, n.is_anchored)).collect();

    let mut constrained_eff: HashMap<PropId, bool> =
        graph.properties.iter().map(|p| (p.id, p.constrained)).collect();

    // --- Build a reverse index: PropId → Vec<DerivId> ---
    let mut prop_to_derivs: HashMap<PropId, Vec<DerivId>> = HashMap::new();
    for deriv in &graph.derivations {
        for &inp in &deriv.inputs {
            prop_to_derivs.entry(inp).or_default().push(deriv.id);
        }
    }

    // --- Helper: check if a node is verified ---
    let verified = |nid: NodeId,
                    anchored: &HashMap<NodeId, bool>,
                    constrained: &HashMap<PropId, bool>|
     -> bool {
        if !anchored.get(&nid).copied().unwrap_or(false) {
            return false;
        }
        let node = &graph.nodes[nid.index()];
        node.properties.iter().all(|&pid| {
            let prop = &graph.properties[pid.index()];
            !prop.critical || constrained.get(&pid).copied().unwrap_or(false)
        })
    };

    // --- Worklists ---
    let mut node_worklist: VecDeque<NodeId> = VecDeque::new();
    let mut prop_worklist: VecDeque<PropId> = VecDeque::new();

    // Seed node_worklist with root nodes.
    for node in &graph.nodes {
        if node_anchored[&node.id] {
            node_worklist.push_back(node.id);
        }
    }

    // Seed prop_worklist with annotation-constrained properties.
    for prop in &graph.properties {
        if constrained_eff[&prop.id] {
            prop_worklist.push_back(prop.id);
        }
    }

    // --- Outer loop: alternate property phase and node phase ---
    loop {
        let mut progress = false;

        // --- Property phase: run to exhaustion ---
        while let Some(p) = prop_worklist.pop_front() {
            if !constrained_eff[&p] {
                continue;
            }

            let prop = &graph.properties[p.index()];
            let nid = prop.node;

            // Constraints and derivations only fire from anchored+verified nodes.
            if verified(nid, &node_anchored, &constrained_eff) {
                // Propagate through Constraint edges where p is the source_prop.
                for &eid in graph.edges_on_prop(p) {
                    if let Edge::Constraint {
                        dest_prop,
                        source_prop,
                        ..
                    } = &graph.edges[eid.index()]
                        && *source_prop == p
                    {
                        let dest_entry =
                            constrained_eff.entry(*dest_prop).or_insert(false);
                        if !*dest_entry {
                            *dest_entry = true;
                            prop_worklist.push_back(*dest_prop);
                            progress = true;
                        }
                    }
                }

                // Propagate through Derivations where p is an input.
                if let Some(deriv_ids) = prop_to_derivs.get(&p) {
                    for &did in deriv_ids {
                        let deriv = &graph.derivations[did.index()];
                        let all_inputs = deriv.inputs.iter().all(|inp| {
                            constrained_eff.get(inp).copied().unwrap_or(false)
                        });
                        if all_inputs {
                            let out = deriv.output_prop;
                            let out_entry =
                                constrained_eff.entry(out).or_insert(false);
                            if !*out_entry {
                                *out_entry = true;
                                prop_worklist.push_back(out);
                                progress = true;
                            }
                        }
                    }
                }
            }

            // Check if newly constrained prop verified its node.
            if verified(nid, &node_anchored, &constrained_eff) {
                node_worklist.push_back(nid);
            }
        }

        // --- Node phase: run to exhaustion ---
        while let Some(n) = node_worklist.pop_front() {
            if !verified(n, &node_anchored, &constrained_eff) {
                continue;
            }

            // Anchor children.
            for &eid in graph.children_of(n) {
                if let Edge::Anchor { child: m, .. } = &graph.edges[eid.index()] {
                    let child_entry = node_anchored.entry(*m).or_insert(false);
                    if !*child_entry {
                        *child_entry = true;
                        progress = true;

                        // Push already-constrained props on newly anchored child.
                        let child_node = &graph.nodes[m.index()];
                        for &qid in &child_node.properties {
                            if constrained_eff.get(&qid).copied().unwrap_or(false) {
                                prop_worklist.push_back(qid);
                            }
                        }

                        if verified(*m, &node_anchored, &constrained_eff) {
                            node_worklist.push_back(*m);
                        }
                    }
                }
            }
        }

        if !progress {
            break;
        }
    }

    StateResult {
        node_anchored,
        prop_constrained: constrained_eff,
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
    fn node(id: u32, ident: &str, properties: Vec<PropId>, is_anchored: bool) -> Node {
        Node {
            id: NodeId(id),
            ident: ident.to_string(),
            display_name: None,
            properties,
            domain: None,
            is_anchored,
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
    // Test 1: simple root-only graph — all @constrained properties are constrained.
    // -----------------------------------------------------------------------

    #[test]
    fn root_only_constrained_props() {
        // Graph: one root node with two @constrained properties and no edges.
        let nodes = vec![node(0, "root", vec![PropId(0), PropId(1)], true)];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", false, true),
        ];
        let graph = make_graph(nodes, properties, vec![], vec![]);

        let state = propagate(&graph);

        assert!(state.is_node_anchored(NodeId(0)), "root should be anchored");
        assert!(state.is_node_verified(&graph, NodeId(0)), "root should be verified (no critical props)");
        assert!(
            state.is_prop_constrained(PropId(0)),
            "Constrained prop P0 should be constrained"
        );
        assert!(
            state.is_prop_constrained(PropId(1)),
            "Constrained prop P1 should be constrained"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: chain where state fully propagates.
    //
    // anchored (@anchored) -> child (non-anchored)
    //   root has @constrained property P0
    //   child has @critical property P1, constrained by P0
    // -----------------------------------------------------------------------

    #[test]
    fn chain_full_propagation() {
        let nodes = vec![
            node(0, "root", vec![PropId(0)], true),
            node(1, "child", vec![PropId(1)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 1, "p1", true, false),
        ];
        let edges = vec![
            Edge::Anchor {
                parent: NodeId(0),
                child: NodeId(1),
                operation: None,
            },
            Edge::Constraint {
                dest_prop: PropId(1),
                source_prop: PropId(0),
                operation: None,
            },
        ];
        let graph = make_graph(nodes, properties, vec![], edges);

        let state = propagate(&graph);

        assert!(state.is_node_anchored(NodeId(0)), "root should be anchored");
        assert!(state.is_node_verified(&graph, NodeId(0)), "root verified (no critical props)");
        assert!(state.is_prop_constrained(PropId(0)), "P0 (@constrained) should be constrained");
        assert!(state.is_prop_constrained(PropId(1)), "P1 constrained by P0");
        assert!(state.is_node_anchored(NodeId(1)), "child should be anchored (parent verified)");
        assert!(
            state.is_node_verified(&graph, NodeId(1)),
            "child verified: anchored + critical P1 constrained"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: chain where a missing constraint blocks verification.
    //
    // root -> child
    //   child has @critical property P0 with NO incoming constraint.
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

        assert!(state.is_node_anchored(NodeId(0)), "root is anchored");
        assert!(state.is_node_verified(&graph, NodeId(0)), "root verified (no critical props)");
        assert!(state.is_node_anchored(NodeId(1)), "child is anchored (parent verified)");
        assert!(
            !state.is_prop_constrained(PropId(0)),
            "P0 has no incoming constraint — unconstrained"
        );
        assert!(
            !state.is_node_verified(&graph, NodeId(1)),
            "child: critical P0 unconstrained — node not verified"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: derivation — all inputs constrained → output constrained.
    //
    // root has P0 and P1 (@constrained).
    // child has P2 (@critical, derivation output).
    // Derivation D0: inputs=[P0,P1], output=P2.
    // When P0 and P1 both constrained on verified root → D0 fires → P2
    // constrained → child verified (P2 is the only critical prop).
    // -----------------------------------------------------------------------

    #[test]
    fn derivation_all_inputs_constrained() {
        let nodes = vec![
            node(0, "root", vec![PropId(0), PropId(1)], true),
            node(1, "child", vec![PropId(2)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", false, true),
            // P2 is the derivation output — @critical, starts unconstrained
            prop(2, 1, "p2_deriv_out", true, false),
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

        assert!(state.is_prop_constrained(PropId(0)), "P0 @constrained → constrained");
        assert!(state.is_prop_constrained(PropId(1)), "P1 @constrained → constrained");
        assert!(
            state.is_prop_constrained(PropId(2)),
            "P2 deriv output: all inputs constrained → constrained"
        );
        assert!(state.is_node_anchored(NodeId(1)), "child anchored (parent verified)");
        assert!(state.is_node_verified(&graph, NodeId(1)), "child verified (P2 constrained)");
    }

    // -----------------------------------------------------------------------
    // Test 5: derivation — one input unconstrained → output unconstrained.
    // -----------------------------------------------------------------------

    #[test]
    fn derivation_one_input_unconstrained() {
        // root has P0 (@constrained) and P1 (@critical, no constraint → unconstrained).
        // Derivation D0: inputs=[P0,P1], output=P2.
        // Since P1 is unconstrained, D0 doesn't fire → P2 stays at its annotation.
        // But P2 has constrained: true (annotation), so it IS constrained.
        // The derivation doesn't fire because P1 is not constrained.
        // However P2's annotation-constrained status is independent.

        let nodes = vec![
            node(0, "root", vec![PropId(0), PropId(1)], true),
            node(1, "child", vec![PropId(2)], false),
        ];
        let properties = vec![
            prop(0, 0, "p0", false, true),
            prop(1, 0, "p1", true, false), // no incoming constraint
            // P2 has constrained: false to truly test derivation non-firing
            prop(2, 1, "p2_deriv_out", false, false),
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

        assert!(state.is_prop_constrained(PropId(0)), "P0 @constrained → constrained");
        assert!(
            !state.is_prop_constrained(PropId(1)),
            "P1 @critical, no constraint → unconstrained"
        );
        assert!(
            !state.is_prop_constrained(PropId(2)),
            "P2 deriv output: P1 unconstrained → derivation doesn't fire"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: PKI example from Appendix A.5.
    //
    // Nodes:
    //   ca        @anchored   properties: P0(subject.common_name, @constrained),
    //                                     P1(public_key, @constrained),
    //                                     P2(crl_url, @constrained)
    //   revocation @anchored  properties: P11(crl, @constrained)
    //   cert      child of ca properties: P3(issuer.common_name, @critical),
    //                                     P4(signature, @critical),
    //                                     P5(subject.common_name, @constrained),
    //                                     P6(not_after, @constrained),
    //                                     P7(public_key, @critical, no constraint)
    //   tls       child of cert properties: P8(server_name, @critical),
    //                                       P9(not_after, @critical),
    //                                       P10(cipher_suite, @constrained)
    //
    // Constraints:
    //   P0  → P3   (ca.subject.common_name constrains cert.issuer.common_name)
    //   P1  → P4   (ca.public_key constrains cert.signature)
    //   P11 → P5   (revocation.crl constrains cert.subject.common_name)
    //   P2  → P8   (ca.crl_url constrains tls.server_name)
    //
    // Expected final state:
    //   ca         anchored=true, verified=true  (root, no critical props)
    //   revocation anchored=true, verified=true  (root, no critical props)
    //   cert       anchored=true, verified=false (P7 unconstrained)
    //   tls        anchored=false (parent cert not verified)
    //   P0-P2      constrained (annotation)
    //   P3         constrained (from P0)
    //   P4         constrained (from P1)
    //   P5         constrained (annotation + from P11)
    //   P6         constrained (annotation)
    //   P7         unconstrained (no constraint → blocks cert verification)
    //   P8         constrained (from P2, ca is verified)
    //   P9         unconstrained (no constraint)
    //   P10        constrained (annotation)
    //   P11        constrained (annotation)
    // -----------------------------------------------------------------------

    #[test]
    fn pki_example_appendix_a5() {
        // Node IDs
        let ca_id = NodeId(0);
        let rev_id = NodeId(1);
        let cert_id = NodeId(2);
        let tls_id = NodeId(3);

        // Property IDs
        let p0 = PropId(0); // ca.subject.common_name   @constrained
        let p1 = PropId(1); // ca.public_key             @constrained
        let p2 = PropId(2); // ca.crl_url                @constrained
        let p11 = PropId(11); // revocation.crl           @constrained
        let p3 = PropId(3); // cert.issuer.common_name   @critical
        let p4 = PropId(4); // cert.signature            @critical
        let p5 = PropId(5); // cert.subject.common_name  @constrained
        let p6 = PropId(6); // cert.not_after            @constrained
        let p7 = PropId(7); // cert.public_key           @critical (no constraint → blocks cert)
        let p8 = PropId(8); // tls.server_name           @critical
        let p9 = PropId(9); // tls.not_after             @critical
        let p10 = PropId(10); // tls.cipher_suite         @constrained

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
            // Anchors
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
            // Constraints
            Edge::Constraint {
                source_prop: p0,
                dest_prop: p3,
                operation: None,
            },
            Edge::Constraint {
                source_prop: p1,
                dest_prop: p4,
                operation: None,
            },
            Edge::Constraint {
                source_prop: p11,
                dest_prop: p5,
                operation: None,
            },
            Edge::Constraint {
                source_prop: p2,
                dest_prop: p8,
                operation: None,
            },
        ];

        let graph = make_graph(nodes, properties, vec![], edges);
        let state = propagate(&graph);

        // Root nodes: anchored and verified (no critical props)
        assert!(state.is_node_anchored(ca_id), "ca: root → anchored");
        assert!(state.is_node_verified(&graph, ca_id), "ca: verified (no critical props)");
        assert!(state.is_node_anchored(rev_id), "revocation: root → anchored");
        assert!(state.is_node_verified(&graph, rev_id), "revocation: verified (no critical props)");

        // @constrained props on roots
        assert!(state.is_prop_constrained(p0), "P0 @constrained on ca");
        assert!(state.is_prop_constrained(p1), "P1 @constrained on ca");
        assert!(state.is_prop_constrained(p2), "P2 @constrained on ca");
        assert!(state.is_prop_constrained(p11), "P11 @constrained on revocation");

        // cert: anchored (ca is verified) but NOT verified (P7 unconstrained)
        assert!(state.is_node_anchored(cert_id), "cert: anchored (parent ca verified)");
        assert!(!state.is_node_verified(&graph, cert_id), "cert: not verified (P7 unconstrained)");

        // cert properties
        assert!(state.is_prop_constrained(p3), "P3 constrained by P0");
        assert!(state.is_prop_constrained(p4), "P4 constrained by P1");
        assert!(state.is_prop_constrained(p5), "P5 @constrained + constrained by P11");
        assert!(state.is_prop_constrained(p6), "P6 @constrained (annotation)");
        assert!(!state.is_prop_constrained(p7), "P7: no constraint → unconstrained (blocks cert)");

        // tls: NOT anchored (parent cert not verified)
        assert!(!state.is_node_anchored(tls_id), "tls: not anchored (cert not verified)");
        assert!(!state.is_node_verified(&graph, tls_id), "tls: not verified (not anchored)");

        // P8 is constrained by P2 (ca is verified, constraint fires)
        assert!(state.is_prop_constrained(p8), "P8 constrained by P2 (ca verified)");

        // P9: no constraint, unconstrained
        assert!(!state.is_prop_constrained(p9), "P9: no constraint → unconstrained");

        // P10: @constrained annotation → constrained
        assert!(state.is_prop_constrained(p10), "P10 @constrained (annotation)");
    }
}
