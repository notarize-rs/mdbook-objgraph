# Trust Graph Terminology

## Core Concepts

| Term | Definition |
|------|------------|
| **Process** | Something with inputs and outputs—computational, business, or physical |
| **Observer** | Wraps a process and produces observations about it |
| **Observation** | A structured record produced by an observer—a collection of properties |
| **Property** | A named value within an observation. The atomic unit of trust decisions |

## Relationships

| Term | Scope | Definition |
|------|-------|------------|
| **Link** | Property → Node | A parent's property guarantees a child's complete integrity. Links form chains. |
| **Pin** | Property → Property (same node) | Compares properties within a single observation. |
| **Bond** | Property → Property (same chain) | Compares properties across observations within the same chain. |
| **Bridge** | Property → Property (cross-domain) | Compares properties across observation domains. |

## Structure

| Term | Definition |
|------|------------|
| **Chain** | A sequence of observations connected by links. Each parent guarantees the next child's integrity. |
| **Observation Domain** | A chain rooted at an explicitly trusted observer. A coherent region of trust with unified authority. |
| **Federation** | The collection of independent observation domains. |
| **Tree** | When chains branch—one parent, multiple children—the structure becomes a tree. |

## Observations

| Term | Definition |
|------|------------|
| **Non-terminal** | An observation that endorses another observer. It links to a child. |
| **Terminal** | An observation that ends the chain. It observes something other than another observer. |
| **Critical property** | A property that constrains what the child can legitimately claim. |
| **Non-critical property** | Informational property, not enforced during trust evaluation. |

## Key Principles

### Delegation
An observer can delegate authority to another observer. The parent observes the child's key management/operational controls and, if satisfied, issues an observation endorsing the child as an observer.

### Chains
With delegation, observers form chains:
```
Root → Intermediate → Intermediate → Leaf
```

A **non-terminal observation** endorses the next observer. A **terminal observation** ends the chain.

### Federation
Different authorities run different observation domains. No single root governs them all:
- Hardware manufacturers (AMD ARK/ASK, Intel attestation CA)
- Software publishers (signing roots)
- Vulnerability databases (CVE/NVD)
- Enterprises (policy)
- Auditors (SOC 2, security assessments)

### Bridges Connect Domains
Domains are independent, but trust decisions often span them. Bridges compare properties across domains:

| Domain 1 | Property | Domain 2 | Property |
|----------|----------|----------|----------|
| Runtime attestation | measurement | Software publisher | release.hash |
| Hardware attestation | chip.id | Manufacturer database | device.serial |
| Software release | version | CVE database | affected_versions |

When `attestation.measurement == release.hash`, the runtime is running exactly what the publisher released.

## Visual Mapping to obgraph

| Concept | obgraph Representation |
|---------|------------------------|
| Observer | @anchored node at chain root |
| Observation | Node |
| Property | Property row in node |
| Link | Anchor edge (`Child <- Parent`) |
| Pin | Constraint with same node (`Node::prop1 <= Node::prop2`) |
| Bond | Constraint within same domain chain |
| Bridge | Constraint across domains (different @anchored roots) |
| Chain | Sequence of anchor edges |
| Domain | `domain "Name" { ... }` grouping |
| Terminal/Target | @selected node (visual only, no semantic effect) |

## Annotations Reference

| Annotation | Applies To | Semantic Effect | Description |
|------------|-----------|-----------------|-------------|
| `@anchored` | Node | Yes | Trust root - starts anchored without parent |
| `@selected` | Node | No | Visual highlight - typically marks the terminal |
| `@critical` | Property | Yes | Must be constrained for node to become verified |
| `@constrained` | Property | Yes | Pre-satisfied - can source constraints immediately |
