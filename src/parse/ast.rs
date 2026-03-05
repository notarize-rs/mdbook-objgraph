//! Unvalidated AST types produced by the parser.
//!
//! These types mirror the input syntax exactly. No semantic validation
//! has been performed — references may be dangling, etc. The model layer
//! consumes these and produces a validated `Graph`.

/// A complete parsed obgraph definition.
#[derive(Debug, Clone)]
pub struct AstGraph {
    pub domains: Vec<AstDomain>,
    pub nodes: Vec<AstNode>,
    pub anchors: Vec<AstAnchor>,
    pub constraints: Vec<AstConstraint>,
}

/// A domain grouping of nodes (visual only).
#[derive(Debug, Clone)]
pub struct AstDomain {
    pub display_name: String,
    pub nodes: Vec<AstNode>,
}

/// A node declaration with its properties.
#[derive(Debug, Clone)]
pub struct AstNode {
    pub ident: String,
    pub display_name: Option<String>,
    pub is_anchored: bool,
    pub is_selected: bool,
    pub properties: Vec<AstProperty>,
}

/// A property declaration within a node.
#[derive(Debug, Clone)]
pub struct AstProperty {
    pub name: String,
    /// `@critical` — property gates node verification.
    pub critical: bool,
    /// `@constrained` — property is pre-satisfied (annotation-constrained).
    pub constrained: bool,
}

/// An anchor between two nodes: `child <- parent [: operation]`.
#[derive(Debug, Clone)]
pub struct AstAnchor {
    pub child_ident: String,
    pub parent_ident: String,
    pub operation: Option<String>,
}

/// The source side of a constraint: either a direct property reference or a
/// derivation call like `filter(A::x, B::y)`.
#[derive(Debug, Clone)]
pub enum AstSourceExpr {
    PropRef { node: String, prop: String },
    Derivation { function: String, args: Vec<AstSourceExpr> },
}

/// A constraint: `node::prop <= source_expr [: operation]`.
#[derive(Debug, Clone)]
pub struct AstConstraint {
    pub dest_node: String,
    pub dest_prop: String,
    pub source: AstSourceExpr,
    pub operation: Option<String>,
}
