# Graph Model

This document defines the obgraph graph model: its five core primitives, the
binary state flags on nodes and properties, structural constraints, the
constraint/derivation distinction, the Rust data structures, and the
fixed-point state propagation algorithm.

---

## 2. Graph Model

### 2.1 Core Concepts

The obgraph model consists of five primitives:

| Primitive      | Description                                                                                                                                                                                                                                                                                                                                                               |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Node**       | A recorded subset of properties that an observer captured about some process. When anchored from a parent, the parent is the observer and the child node is what was recorded. Nodes annotated `@anchored` are anchored directly by annotation, with no parent anchor required. Has an optional identifier (`None` for derivation nodes) and an optional display name.    |
| **Property**   | A named attribute of a node. Rendered as a row inside the node with ports on left and right sides. Properties have two independent binary state flags: `critical` (set by `@critical` annotation — participates in the node's `verified` computation) and `constrained` (set by `@constrained` annotation or a valid incoming constraint). For derivation nodes, the property name carries the function name (e.g. `"filter"`). |
| **Anchor**     | A hierarchical directed edge between two nodes. The parent anchors the child: trust flows right-to-left, with the parent on the right and the child on the left. A valid anchor requires the parent to be both anchored and verified. Carries an optional operation name defining the integrity method for the relationship.                                              |
| **Constraint** | A binary test between two properties. Takes a destination (left-hand side) and a source expression (right-hand side), applies an operation, and returns a boolean indicating compatibility. When true, trust flows from the source to the destination. Carries an optional named operation; when omitted, equality is implied. Also used for derivation inputs (no operation). |
| **Domain**     | A visual grouping of nodes. Domains have no identity, carry no data, and do not participate in the graph structure. They are rendered as labeled bounding boxes around their member nodes.                                                                                                                                                                                |

Derivations are not a separate primitive — they are synthetic `Node`s with `ident = None` and a single property whose name is the function name. See §2.5 for details.

### 2.2 State Model

Each node and property has two independent binary state flags computed from the
graph structure. The visual renderer reads these flags directly; annotations
instruct the renderer how to initialise each flag before propagation begins.

#### Node State Flags

| Flag       | Set when                                                                  | Visual indicator (problems-only)           |
| ---------- | ------------------------------------------------------------------------- | ------------------------------------------ |
| `anchored` | Node has `@anchored` annotation, or has a valid incoming anchor               | No indicator when true; red dot when false |
| `verified` | Every `@critical` property on the node is `constrained`                   | Emergent from property row indicators      |

Every node has an implicit **anchor slot** that behaves exactly like a
`@critical` property slot: it must be satisfied for the node to be anchored.
The `@anchored` annotation satisfies the anchor slot directly — no incoming anchor
is needed — exactly as `@constrained` satisfies a property's constraint slot.
This makes anchoring unconditionally critical: there is no way to opt a node
out of requiring an anchor.

#### Node Annotations

| Annotation  | Meaning                                                                           |
| ----------- | --------------------------------------------------------------------------------- |
| `@anchored`     | Satisfies the node's anchor slot directly. The node is anchored without a parent. |
| `@selected` | The node's cross-domain constraints are visible by default in the rendered graph. |

#### Property State Flags

| Flag          | Set when                                                                   | Visual indicator (problems-only)                    |
| ------------- | -------------------------------------------------------------------------- | --------------------------------------------------- |
| `critical`    | Property has `@critical` annotation                                        | Bold property name                                  |
| `constrained` | Property has `@constrained` annotation, or has a valid incoming constraint | No indicator when true; red dot when critical+false |

A `critical` property that is not `constrained` blocks its node's `verified`
state and is highlighted with a red dot. Non-critical properties never show a
dot regardless of constraint state.

Both flags are independent; a property may carry both annotations, either, or
neither.

#### Property Annotations

| Annotation      | Sets flag     | Meaning                                                                                                               |
| --------------- | ------------- | --------------------------------------------------------------------------------------------------------------------- |
| `@critical`     | `critical`    | Property participates in node `verified` — if unconstrained, blocks the node's verified state.                        |
| `@constrained`  | `constrained` | Property is pre-constrained by annotation — no incoming constraint needed. Can immediately source other constraints.  |

#### State Computation Rules

1. A node with `@anchored` is **anchored**. Its anchor slot is satisfied by the
   annotation.
2. A non-anchored node becomes **anchored** only when it has a valid incoming anchor
   from a parent that is both anchored and verified. A node has at most one
   incoming anchor (one parent) and may have multiple outgoing anchors.
3. A node is **verified** when every `@critical` property on it is constrained.
   A node with no `@critical` properties is always verified.
4. A property with `@constrained` is **constrained** from the start — its
   constraint slot is satisfied by the annotation.
5. A property receives a valid incoming constraint only when the source property
   is constrained and belongs to a node that is both anchored and verified.
6. An anchor edge is valid only when the source (parent) node is both anchored
   and verified.
7. Trust flows **right-to-left** in all statements. In anchors
   (`child <- parent`), the parent is the source. In constraints
   (`dest <= source`), the source is on the right.
8. State propagation is a fixed-point computation over the graph. It terminates
   when no further flags can be set.

### 2.3 Structural Constraints

The complete graph — anchors, derivation edges, and constraints combined — **must
be acyclic**. This is a hard requirement, not merely a layout preference. The
acyclicity guarantee means:

- A global topological ordering exists across all edge types.
- All edges flow in a consistent direction in the layout (downward in the
  Sugiyama framework).
- The state propagation computation is guaranteed to terminate.

### 2.4 Constraints and Derivations

The obgraph model distinguishes two fundamentally different operations:

#### Constraint Operations

A constraint is a **binary boolean test** between two values: a destination
property and a source expression. It answers: "are these two values compatible?"
When the answer is true, trust flows from the source to the destination.

```
cert::issuer.common_name <= ca::subject.common_name
```

Here the constraint tests whether `cert::issuer.common_name` and
`ca::subject.common_name` are compatible (default: equality). Trust flows
right-to-left.

Constraints may specify a named operation:

```
cert::signature <= ca::public_key : verified_by
```

Here `verified_by` is the constraint operation — it tests whether
`cert::signature` is compatible with `ca::public_key` under the `verified_by`
relationship.

#### Derivations

A derivation is a **value-producing computation**. It takes one or more
properties (or other derivation outputs) as inputs and produces a new unnamed
ephemeral property. Derivations are not boolean tests — they produce values that
can then be used as the source expression in a constraint.

```
cert::subject.common_name <= difference(all_names::list, revocation::crl) : not_in
```

Here `difference(all_names::list, revocation::crl)` is a derivation — it
computes an ephemeral value from two inputs. The constraint operation `not_in`
then tests `cert::subject.common_name` against that ephemeral value.

Derivations can be nested:

```
cert::foo <= intersect(difference(X::bar, Y::crl), Z::approved) : subset_of
```

Here `difference` produces an ephemeral value, `intersect` consumes it along
with `Z::approved` to produce another ephemeral value, and `subset_of` is the
constraint operation testing `cert::foo` against the final result.

#### Key Properties of Derivations

- Derivations take one or more inputs. Nullary derivations (zero inputs) are
  rejected syntactically.
- Derivations can consume the outputs of other derivations, forming chains.
- Derivation outputs are unnamed. Identical derivation expressions are
  deduplicated by the parser into a single graph node. Deduplication uses string
  equality of the normalized expression.
- Derivation nodes are rendered as small rounded pills with the operation name
  as a label.

> Input syntax for derivations: see [SYNTAX.md](SYNTAX.md)

#### Summary: Constraints vs. Derivations

|                          | Constraint                           | Derivation                                |
| ------------------------ | ------------------------------------ | ----------------------------------------- |
| **Purpose**              | Binary compatibility test            | Value computation                         |
| **Inputs**               | Exactly 2 (destination + source)     | 1 or more                                 |
| **Output**               | Boolean (trust flows if true)        | Unnamed ephemeral property                |
| **Syntax**               | `dest <= source [: operation]`       | `function(input, ...)`                    |
| **Graph representation** | Directed edge between two properties | Node with input edges and one output edge |

### 2.5 Graph Data Structure

The model graph is a purpose-built immutable structure with first-class
port-level adjacency. All indices are `u32` newtypes for type safety.

```rust
// Index types (newtypes over u32)
struct NodeId(u32);
struct PropId(u32);       // global property index, unique across all nodes
struct EdgeId(u32);
struct DomainId(u32);

struct Graph {
    nodes: Vec<Node>,
    properties: Vec<Property>,     // all properties across all nodes
    edges: Vec<Edge>,
    domains: Vec<Domain>,

    // Port-level adjacency: PropId → Vec<EdgeId>
    prop_edges: HashMap<PropId, Vec<EdgeId>>,

    // Node-level adjacency for anchor edges only: NodeId → Vec<EdgeId>
    node_children: HashMap<NodeId, Vec<EdgeId>>,
    node_parent: HashMap<NodeId, EdgeId>,
}

struct Node {
    id: NodeId,
    ident: Option<String>,         // None for derivation nodes
    display_name: Option<String>,
    properties: Vec<PropId>,       // ordered as declared
    domain: Option<DomainId>,
    is_anchored: bool,
    is_selected: bool,
}

struct Property {
    id: PropId,
    node: NodeId,
    name: String,                  // e.g. "subject.common_name" or function name for derivations
    critical: bool,                // @critical: participates in node verified computation
    constrained: bool,             // @constrained: pre-satisfied by annotation
}

struct Domain {
    id: DomainId,
    display_name: String,
    members: Vec<NodeId>,
}
```

#### Edge Types

The graph contains two edge variants, distinguished by an enum:

```rust
enum Edge {
    // Node → Node (hierarchical). Right-to-left: parent is source.
    Anchor {
        child: NodeId,
        parent: NodeId,
        operation: Option<String>,
    },

    // Property → Property (trust verification).
    // dest_prop is verified against source_prop.
    // Field order matches syntax: dest <= source.
    // For layout purposes, source_prop is the upstream end (higher layer)
    // and dest_prop is the downstream end (lower layer).
    Constraint {
        dest_prop: PropId,
        source_prop: PropId,
        operation: Option<String>,
    },
}
```

#### Derivation Nodes

Derivations are represented as **synthetic `Node`s** with `ident = None`. They
have no separate type — they reuse the existing `Node` and `Constraint` edge
types:

- **`ident`**: `None` — derivation nodes are unnamed and unreferenceable from
  user syntax.
- **`properties`**: Exactly one property whose `name` is the function name
  (e.g. `"filter"`). This property has `critical = false`, `constrained = false`.
- **`is_anchored`**: `true` — always anchored by construction.
- **`domain`**: Inferred from inputs — same domain if all inputs share one;
  `None` if cross-domain.

Inputs to a derivation are regular `Constraint` edges targeting the derivation's
property (no operation). The derivation's property serves as `source_prop` in
the downstream constraint. Identical derivation expressions are deduplicated by
`(function_name, sorted input PropIds)`.

The derivation output becomes constrained only when **all** incoming constraints
are satisfied (conjunction semantics), unlike regular properties which become
constrained when **any** incoming constraint is satisfied.

#### Adjacency Queries Required by Layout Phases

| Phase                 | Query                                                      | Used For                           |
| --------------------- | ---------------------------------------------------------- | ---------------------------------- |
| Validation            | All edges from/to a node                                   | Cycle detection (Kahn's algorithm) |
| Layer assignment      | All edges (as source→target pairs with weights)            | Network simplex                    |
| Crossing minimization | All edges incident on a property                           | Per-property barycenter            |
| Crossing minimization | All properties of a node, in current order                 | Nested barycenter                  |
| Coordinate assignment | All edges between adjacent layers                          | Brandes-Kopf alignment             |
| Edge routing          | All edges (with full path from source port to target port) | Orthogonal routing                 |

> These queries are used by the layout phases in [LAYOUT.md](LAYOUT.md)

### 2.6 State Propagation Algorithm

The state computation rules from section 2.2 are implemented as a fixed-point
worklist algorithm over two monotone boolean maps: `anchored[node]` and
`constrained_eff[prop]`.

```
function propagate_state(graph) → (anchored: NodeSet, constrained_eff: PropSet):
    // Initialize node anchored flag
    for each node n:
        anchored[n] = n.is_anchored

    // Initialize effective constrained flag (annotation-based only)
    for each property p:
        constrained_eff[p] = p.constrained

    // Helper: node is verified when all its critical properties are constrained
    function verified(n):
        return all(constrained_eff[q] for q in n.properties where q.critical)

    node_worklist = { n | anchored[n] }           // @anchored nodes
    prop_worklist = { p | constrained_eff[p] }    // annotation-constrained props

    while node_worklist or prop_worklist is not empty:

        // ── Property phase ────────────────────────────────────────────────────
        while prop_worklist is not empty:
            p = prop_worklist.pop()
            n = p.node

            // Only an anchored+verified node propagates outgoing constraints
            if anchored[n] AND verified(n):
                for each Constraint c where c.source_prop == p:
                    dest = c.dest_prop
                    dest_node = dest.node

                    // Derivation output: constrained when ALL inputs are satisfied
                    if dest_node.is_derivation():
                        if all incoming constraints on dest have constrained sources
                           on anchored+verified nodes:
                            if NOT constrained_eff[dest]:
                                constrained_eff[dest] = true
                                prop_worklist.push(dest)
                    else:
                        // Regular property: constrained by any single valid constraint
                        if NOT constrained_eff[dest]:
                            constrained_eff[dest] = true
                            prop_worklist.push(dest)

            // A property becoming constrained may have newly verified its node
            if anchored[n] AND verified(n):
                node_worklist.push(n)    // guard in node phase handles duplicates

        // ── Node phase ────────────────────────────────────────────────────────
        while node_worklist is not empty:
            n = node_worklist.pop()
            if NOT (anchored[n] AND verified(n)):
                continue                // queued speculatively; not yet ready

            // n is anchored+verified → attempt to anchor its children
            for each Anchor(child=m, parent=n):
                if NOT anchored[m]:
                    anchored[m] = true
                    // Unlock annotation-constrained props on the newly anchored child
                    for each property q of m where constrained_eff[q]:
                        prop_worklist.push(q)
                    // If m has no critical props it is immediately verified
                    if verified(m):
                        node_worklist.push(m)

    return (anchored, constrained_eff)
```

**Termination guarantee:** `anchored[n]` and `constrained_eff[p]` are monotone
booleans — each transitions from false to true at most once. Every worklist
push corresponds to such a transition. The algorithm terminates in
O(|nodes| + |properties| + |edges|) time.

> Visual indicators driven by this state: see [RENDERING.md](RENDERING.md)

---

## See Also

- [SYNTAX.md](SYNTAX.md) -- Input syntax: node declarations, anchors, constraints, and derivation expressions
- [LAYOUT.md](LAYOUT.md) -- Layout pipeline: layer assignment, crossing minimization, coordinate assignment, and edge routing
- [RENDERING.md](RENDERING.md) -- Rendering: visual indicators, node styling, and SVG output
