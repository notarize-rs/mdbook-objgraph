/// Unvalidated AST types produced by the parser.
///
/// These types mirror the input syntax exactly. No semantic validation
/// has been performed — references may be dangling, derivations may be
/// duplicated, etc. The model layer consumes these and produces a
/// validated `Graph`.

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
    pub is_root: bool,
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

/// A constraint: `node::prop <= source_expr [: operation]`.
#[derive(Debug, Clone)]
pub struct AstConstraint {
    pub dest_node: String,
    pub dest_prop: String,
    pub source: AstSourceExpr,
    pub operation: Option<String>,
}

/// The right-hand side of a constraint — either a property reference or a derivation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AstSourceExpr {
    /// A simple property reference: `node::property`.
    PropRef {
        node_ident: String,
        prop_name: String,
    },
    /// A derivation (function call): `func(arg1, arg2, ...)`.
    Derivation(AstDerivationExpr),
}

/// An inline derivation expression: `func(arg1, arg2, ...)`.
/// Arguments may themselves be derivations (nesting).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AstDerivationExpr {
    pub function: String,
    pub args: Vec<AstSourceExpr>,
}

impl AstSourceExpr {
    /// Returns a normalized string representation for deduplication.
    pub fn normalized(&self) -> String {
        match self {
            AstSourceExpr::PropRef {
                node_ident,
                prop_name,
            } => format!("{node_ident}::{prop_name}"),
            AstSourceExpr::Derivation(d) => d.normalized(),
        }
    }
}

impl AstDerivationExpr {
    /// Returns a normalized string representation for deduplication.
    pub fn normalized(&self) -> String {
        let args: Vec<String> = self.args.iter().map(|a| a.normalized()).collect();
        format!("{}({})", self.function, args.join(","))
    }
}
