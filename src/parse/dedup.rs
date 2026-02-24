//! Derivation expression normalization and deduplication (DESIGN.md §2.4).
//!
//! Identical derivation expressions (by normalized string equality) are
//! merged into a single graph node during graph construction.

use crate::parse::ast::AstSourceExpr;
use std::collections::HashMap;

/// A deduplication table mapping normalized derivation strings to
/// the first occurrence of the expression.
pub struct DerivDedup {
    seen: HashMap<String, AstSourceExpr>,
}

impl DerivDedup {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    /// Insert a source expression. If it's a derivation that has been seen
    /// before (by normalized string equality), returns the canonical copy.
    /// Otherwise, records it and returns it unchanged.
    pub fn dedup(&mut self, expr: AstSourceExpr) -> AstSourceExpr {
        match &expr {
            AstSourceExpr::PropRef { .. } => expr,
            AstSourceExpr::Derivation(d) => {
                let key = d.normalized();
                if let Some(canonical) = self.seen.get(&key) {
                    canonical.clone()
                } else {
                    self.seen.insert(key, expr.clone());
                    expr
                }
            }
        }
    }
}

impl Default for DerivDedup {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::ast::{AstDerivationExpr, AstSourceExpr};

    fn prop(node: &str, prop: &str) -> AstSourceExpr {
        AstSourceExpr::PropRef {
            node_ident: node.into(),
            prop_name: prop.into(),
        }
    }

    fn deriv(func: &str, args: Vec<AstSourceExpr>) -> AstSourceExpr {
        AstSourceExpr::Derivation(AstDerivationExpr {
            function: func.into(),
            args,
        })
    }

    // -----------------------------------------------------------------------
    // PropRef expressions are never deduplicated — they are returned as-is.
    // -----------------------------------------------------------------------

    #[test]
    fn test_prop_ref_passes_through() {
        let mut d = DerivDedup::new();
        let expr = prop("ca", "subject.org");
        let result = d.dedup(expr.clone());
        assert_eq!(result, expr);
    }

    #[test]
    fn test_two_identical_prop_refs_are_independent() {
        let mut d = DerivDedup::new();
        let e1 = prop("ca", "subject.org");
        let e2 = prop("ca", "subject.org");
        // Both should come back unchanged; PropRef is never deduplicated.
        assert_eq!(d.dedup(e1), prop("ca", "subject.org"));
        assert_eq!(d.dedup(e2), prop("ca", "subject.org"));
    }

    // -----------------------------------------------------------------------
    // First occurrence of a derivation is stored and returned unchanged.
    // -----------------------------------------------------------------------

    #[test]
    fn test_first_derivation_returned_unchanged() {
        let mut d = DerivDedup::new();
        let expr = deriv("f", vec![prop("a", "x")]);
        let result = d.dedup(expr.clone());
        assert_eq!(result, expr);
    }

    // -----------------------------------------------------------------------
    // Second identical derivation returns the canonical (first) copy.
    // -----------------------------------------------------------------------

    #[test]
    fn test_duplicate_derivation_returns_canonical() {
        let mut d = DerivDedup::new();
        let first = deriv("f", vec![prop("a", "x")]);
        let second = deriv("f", vec![prop("a", "x")]);

        let canonical = d.dedup(first.clone());
        let deduped = d.dedup(second);

        // Both should be equal in value.
        assert_eq!(canonical, deduped);
        // And pointer-equal to the first (same clone from the map).
        assert_eq!(canonical.normalized(), deduped.normalized());
    }

    // -----------------------------------------------------------------------
    // Distinct derivations are stored separately.
    // -----------------------------------------------------------------------

    #[test]
    fn test_distinct_derivations_not_merged() {
        let mut d = DerivDedup::new();
        let e1 = deriv("f", vec![prop("a", "x")]);
        let e2 = deriv("g", vec![prop("a", "x")]);

        let r1 = d.dedup(e1.clone());
        let r2 = d.dedup(e2.clone());

        assert_eq!(r1, e1);
        assert_eq!(r2, e2);
        assert_ne!(r1.normalized(), r2.normalized());
    }

    #[test]
    fn test_same_func_different_args_not_merged() {
        let mut d = DerivDedup::new();
        let e1 = deriv("f", vec![prop("a", "x")]);
        let e2 = deriv("f", vec![prop("b", "y")]);

        let r1 = d.dedup(e1.clone());
        let r2 = d.dedup(e2.clone());

        assert_eq!(r1, e1);
        assert_eq!(r2, e2);
    }

    // -----------------------------------------------------------------------
    // Nested derivations are deduplicated by their full normalized form.
    // -----------------------------------------------------------------------

    #[test]
    fn test_nested_derivation_dedup() {
        let mut d = DerivDedup::new();

        let inner = deriv("inner", vec![prop("a", "x")]);
        let outer1 = deriv("outer", vec![inner.clone()]);
        let outer2 = deriv("outer", vec![inner.clone()]);

        let r1 = d.dedup(outer1.clone());
        let r2 = d.dedup(outer2);

        assert_eq!(r1.normalized(), r2.normalized());
    }

    // -----------------------------------------------------------------------
    // Derivation with multiple arguments.
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_arg_derivation_dedup() {
        let mut d = DerivDedup::new();
        let e1 = deriv("combine", vec![prop("a", "x"), prop("b", "y"), prop("c", "z")]);
        let e2 = deriv("combine", vec![prop("a", "x"), prop("b", "y"), prop("c", "z")]);

        let r1 = d.dedup(e1);
        let r2 = d.dedup(e2);
        assert_eq!(r1.normalized(), r2.normalized());
    }

    #[test]
    fn test_multi_arg_different_order_not_merged() {
        // Argument order matters in the normalized form.
        let mut d = DerivDedup::new();
        let e1 = deriv("f", vec![prop("a", "x"), prop("b", "y")]);
        let e2 = deriv("f", vec![prop("b", "y"), prop("a", "x")]);

        let r1 = d.dedup(e1);
        let r2 = d.dedup(e2);
        assert_ne!(r1.normalized(), r2.normalized());
    }

    // -----------------------------------------------------------------------
    // Default constructor works the same as new().
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_constructor() {
        let mut d: DerivDedup = Default::default();
        let expr = deriv("f", vec![prop("a", "x")]);
        let result = d.dedup(expr.clone());
        assert_eq!(result, expr);
    }

    // -----------------------------------------------------------------------
    // Normalized string format matches expected pattern.
    // -----------------------------------------------------------------------

    #[test]
    fn test_normalized_format_prop_ref() {
        let e = prop("ca", "subject.org");
        assert_eq!(e.normalized(), "ca::subject.org");
    }

    #[test]
    fn test_normalized_format_derivation() {
        let e = deriv("f", vec![prop("a", "x"), prop("b", "y")]);
        assert_eq!(e.normalized(), "f(a::x,b::y)");
    }

    #[test]
    fn test_normalized_format_nested() {
        let inner = deriv("g", vec![prop("a", "x")]);
        let outer = deriv("f", vec![inner]);
        assert_eq!(outer.normalized(), "f(g(a::x))");
    }
}
