pub mod ast;
pub mod lexer;

use ast::{
    AstConstraint, AstDomain, AstGraph, AstAnchor, AstNode, AstProperty, AstSourceExpr,
};
use crate::ObgraphError;
use lexer::{Lexer, Spanned, Token};

/// Parse an obgraph definition string into an unvalidated AST.
pub fn parse(input: &str) -> Result<AstGraph, ObgraphError> {
    let tokens = Lexer::new(input).tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_graph()
}

// ---------------------------------------------------------------------------
// Parser state
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Spanned>,
    /// Index of the next token to consume.
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Spanned>) -> Self {
        Self { tokens, pos: 0 }
    }

    // -----------------------------------------------------------------------
    // Low-level token access
    // -----------------------------------------------------------------------

    /// Peek at the current token without consuming it.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    /// Return (line, col) of the current token.
    fn here(&self) -> (usize, usize) {
        let s = &self.tokens[self.pos];
        (s.line, s.col)
    }

    /// Consume and return the current token.
    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos].token;
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    /// Consume the current token if it matches `expected`, else error.
    fn expect(&mut self, expected: &Token) -> Result<(), ObgraphError> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            let (line, col) = self.here();
            Err(ObgraphError::Parse {
                line,
                col,
                message: format!(
                    "expected `{expected:?}`, found `{:?}`",
                    self.peek()
                ),
            })
        }
    }

    /// Consume and return the inner string if the current token is `Ident`,
    /// else error.
    fn expect_ident(&mut self) -> Result<String, ObgraphError> {
        match self.peek().clone() {
            Token::Ident(s) => {
                self.advance();
                Ok(s)
            }
            _ => {
                let (line, col) = self.here();
                Err(ObgraphError::Parse {
                    line,
                    col,
                    message: format!("expected identifier, found `{:?}`", self.peek()),
                })
            }
        }
    }

    /// Skip over `Newline` tokens.
    fn skip_newlines(&mut self) {
        while *self.peek() == Token::Newline {
            self.advance();
        }
    }

    /// Consume a `Newline` token (required as a statement terminator),
    /// or accept `Eof` in its place.
    fn expect_newline_or_eof(&mut self) -> Result<(), ObgraphError> {
        match self.peek() {
            Token::Newline | Token::Eof => {
                self.advance();
                Ok(())
            }
            _ => {
                let (line, col) = self.here();
                Err(ObgraphError::Parse {
                    line,
                    col,
                    message: format!(
                        "expected end of line, found `{:?}`",
                        self.peek()
                    ),
                })
            }
        }
    }

    // -----------------------------------------------------------------------
    // Parse a property name: ident ('.' ident)*
    //
    // The result is the dotted string, e.g. "subject.common_name".
    // We consume tokens one at a time so dots within a property name are
    // treated as part of the name.
    // -----------------------------------------------------------------------
    fn parse_prop_name(&mut self) -> Result<String, ObgraphError> {
        let first = self.expect_ident()?;
        let mut name = first;
        while *self.peek() == Token::Dot {
            self.advance(); // consume '.'
            let segment = self.expect_ident()?;
            name.push('.');
            name.push_str(&segment);
        }
        Ok(name)
    }

    // -----------------------------------------------------------------------
    // Parse a property reference: ident '::' prop_name
    //
    // Returns (node_ident, prop_name).
    // -----------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // Parse a source expression: either a prop_ref or a derivation call.
    //
    //   source_expr ← derivation / prop_ref
    //   derivation  ← ident '(' source_expr (',' source_expr)* ')'
    //   prop_ref    ← ident '::' prop_name
    // -----------------------------------------------------------------------
    fn parse_source_expr(&mut self) -> Result<AstSourceExpr, ObgraphError> {
        let ident = self.expect_ident()?;
        match self.peek() {
            Token::ColonColon => {
                // prop_ref: node::prop
                self.advance();
                let prop_name = self.parse_prop_name()?;
                Ok(AstSourceExpr::PropRef { node: ident, prop: prop_name })
            }
            Token::LParen => {
                // derivation: function(args...)
                self.advance();
                let mut args = Vec::new();
                if *self.peek() != Token::RParen {
                    args.push(self.parse_source_expr()?);
                    while *self.peek() == Token::Comma {
                        self.advance();
                        args.push(self.parse_source_expr()?);
                    }
                }
                self.expect(&Token::RParen)?;
                Ok(AstSourceExpr::Derivation { function: ident, args })
            }
            _ => {
                let (line, col) = self.here();
                Err(ObgraphError::Parse {
                    line,
                    col,
                    message: format!(
                        "expected `::` (prop ref) or `(` (derivation) after `{ident}`, found `{:?}`",
                        self.peek()
                    ),
                })
            }
        }
    }

    // -----------------------------------------------------------------------
    // Parse a property declaration line inside a node body.
    //
    //   prop_decl ← prop_name (_ prop_annot)?
    // -----------------------------------------------------------------------
    fn parse_prop_decl(&mut self) -> Result<AstProperty, ObgraphError> {
        let name = self.parse_prop_name()?;
        let mut critical = false;
        let mut constrained = false;
        loop {
            match self.peek() {
                Token::AtCritical => {
                    self.advance();
                    critical = true;
                }
                Token::AtConstrained => {
                    self.advance();
                    constrained = true;
                }
                _ => break,
            }
        }
        Ok(AstProperty { name, critical, constrained })
    }

    // -----------------------------------------------------------------------
    // Parse a node body (the prop_list between `{` and `}`).
    //
    // Blank lines are skipped. Each non-blank line must contain a prop_decl
    // followed by an optional trailing comment (already stripped by the lexer)
    // and then a Newline.
    // -----------------------------------------------------------------------
    fn parse_node_body(&mut self) -> Result<Vec<AstProperty>, ObgraphError> {
        let mut props = Vec::new();
        loop {
            // Skip blank lines.
            self.skip_newlines();
            // End of body?
            if *self.peek() == Token::RBrace {
                break;
            }
            // A property line starts with an identifier (the property name).
            match self.peek() {
                Token::Ident(_) => {
                    let prop = self.parse_prop_decl()?;
                    props.push(prop);
                    self.expect_newline_or_eof()?;
                }
                _ => {
                    let (line, col) = self.here();
                    return Err(ObgraphError::Parse {
                        line,
                        col,
                        message: format!(
                            "expected property name or `}}`, found `{:?}`",
                            self.peek()
                        ),
                    });
                }
            }
        }
        Ok(props)
    }

    // -----------------------------------------------------------------------
    // Parse a `node` declaration.
    //
    //   node_decl ← 'node' ident string_lit? node_annot* '{' trailing? '\n'
    //               prop_list
    //               '}' trailing? '\n'
    // -----------------------------------------------------------------------
    fn parse_node(&mut self) -> Result<AstNode, ObgraphError> {
        self.expect(&Token::KwNode)?;
        let ident = self.expect_ident()?;

        // Optional display name.
        let display_name = match self.peek() {
            Token::StringLit(_) => {
                if let Token::StringLit(s) = self.advance().clone() {
                    Some(s)
                } else {
                    unreachable!()
                }
            }
            _ => None,
        };

        // Zero or more annotations.
        let mut is_anchored = false;
        let mut is_selected = false;
        loop {
            match self.peek() {
                Token::AtAnchored => {
                    self.advance();
                    is_anchored = true;
                }
                Token::AtSelected => {
                    self.advance();
                    is_selected = true;
                }
                _ => break,
            }
        }

        // `{` then newline.
        self.expect(&Token::LBrace)?;
        self.expect_newline_or_eof()?;

        // Property list.
        let properties = self.parse_node_body()?;

        // `}` then newline (or EOF).
        self.expect(&Token::RBrace)?;
        self.expect_newline_or_eof()?;

        Ok(AstNode {
            ident,
            display_name,
            is_anchored,
            is_selected,
            properties,
        })
    }

    // -----------------------------------------------------------------------
    // Parse a `domain` block.
    //
    //   domain ← 'domain' string_lit '{' trailing? '\n'
    //             node_decl*
    //             '}' trailing? '\n'
    // -----------------------------------------------------------------------
    fn parse_domain(&mut self) -> Result<AstDomain, ObgraphError> {
        self.expect(&Token::KwDomain)?;

        let display_name = match self.peek().clone() {
            Token::StringLit(s) => {
                self.advance();
                s
            }
            _ => {
                let (line, col) = self.here();
                return Err(ObgraphError::Parse {
                    line,
                    col,
                    message: format!(
                        "expected domain name string, found `{:?}`",
                        self.peek()
                    ),
                });
            }
        };

        self.expect(&Token::LBrace)?;
        self.expect_newline_or_eof()?;

        let mut nodes = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                Token::RBrace => break,
                Token::KwNode => {
                    nodes.push(self.parse_node()?);
                }
                _ => {
                    let (line, col) = self.here();
                    return Err(ObgraphError::Parse {
                        line,
                        col,
                        message: format!(
                            "expected `node` or `}}` inside domain, found `{:?}`",
                            self.peek()
                        ),
                    });
                }
            }
        }

        self.expect(&Token::RBrace)?;
        self.expect_newline_or_eof()?;

        Ok(AstDomain { display_name, nodes })
    }

    // -----------------------------------------------------------------------
    // Parse an anchor statement.
    //
    //   anchor ← ident '<-' ident (':' ident)? trailing? '\n'
    //
    // The leading ident has already been consumed by the caller and is passed
    // in as `child_ident`.
    // -----------------------------------------------------------------------
    fn parse_anchor(&mut self, child_ident: String) -> Result<AstAnchor, ObgraphError> {
        self.expect(&Token::LeftArrow)?;
        let parent_ident = self.expect_ident()?;

        let operation = if *self.peek() == Token::Colon {
            self.advance(); // consume ':'
            Some(self.expect_ident()?)
        } else {
            None
        };

        self.expect_newline_or_eof()?;
        Ok(AstAnchor { child_ident, parent_ident, operation })
    }

    // -----------------------------------------------------------------------
    // Parse a constraint statement.
    //
    //   constraint ← prop_ref '<=' prop_ref (':' ident)? trailing? '\n'
    //
    // The `dest_node` ident and `::` have already been consumed and `prop_ref`
    // parsing has already started.  We receive dest_node and dest_prop.
    // -----------------------------------------------------------------------
    fn parse_constraint(
        &mut self,
        dest_node: String,
        dest_prop: String,
    ) -> Result<AstConstraint, ObgraphError> {
        self.expect(&Token::LeftAngleEq)?;
        let source = self.parse_source_expr()?;

        let operation = if *self.peek() == Token::Colon {
            self.advance(); // consume ':'
            Some(self.expect_ident()?)
        } else {
            None
        };

        self.expect_newline_or_eof()?;
        Ok(AstConstraint { dest_node, dest_prop, source, operation })
    }

    // -----------------------------------------------------------------------
    // Top-level graph parser
    // -----------------------------------------------------------------------

    fn parse_graph(&mut self) -> Result<AstGraph, ObgraphError> {
        let mut graph = AstGraph {
            domains: Vec::new(),
            nodes: Vec::new(),
            anchors: Vec::new(),
            constraints: Vec::new(),
        };

        loop {
            // Skip blank lines.
            self.skip_newlines();

            match self.peek().clone() {
                Token::Eof => break,

                Token::KwDomain => {
                    graph.domains.push(self.parse_domain()?);
                }

                Token::KwNode => {
                    graph.nodes.push(self.parse_node()?);
                }

                // A bare identifier starts either an anchor or a constraint.
                // We parse the identifier, then check what follows:
                //   - '<-'  → anchor      (child <- parent ...)
                //   - '::'  → constraint  (node::prop <= source ...)
                Token::Ident(_) => {
                    let first_ident = self.expect_ident()?;

                    match self.peek() {
                        Token::LeftArrow => {
                            graph.anchors.push(self.parse_anchor(first_ident)?);
                        }
                        Token::ColonColon => {
                            self.advance(); // consume '::'
                            let prop = self.parse_prop_name()?;
                            graph.constraints.push(
                                self.parse_constraint(first_ident, prop)?,
                            );
                        }
                        _ => {
                            let (line, col) = self.here();
                            return Err(ObgraphError::Parse {
                                line,
                                col,
                                message: format!(
                                    "expected `<-` (anchor) or `::` (constraint) after identifier `{first_ident}`, found `{:?}`",
                                    self.peek()
                                ),
                            });
                        }
                    }
                }

                tok => {
                    let (line, col) = self.here();
                    return Err(ObgraphError::Parse {
                        line,
                        col,
                        message: format!(
                            "unexpected token `{tok:?}` at top level"
                        ),
                    });
                }
            }
        }

        Ok(graph)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: parse and unwrap.
    fn p(input: &str) -> AstGraph {
        parse(input).expect("parse failed")
    }

    // Helper: parse and expect an error.
    fn p_err(input: &str) -> ObgraphError {
        parse(input).expect_err("expected parse error")
    }

    // -----------------------------------------------------------------------
    // Empty / blank input
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_input() {
        let g = p("");
        assert!(g.domains.is_empty());
        assert!(g.nodes.is_empty());
        assert!(g.anchors.is_empty());
        assert!(g.constraints.is_empty());
    }

    #[test]
    fn test_blank_lines_only() {
        let g = p("\n\n\n");
        assert!(g.domains.is_empty());
    }

    #[test]
    fn test_comments_only() {
        let g = p("# hello\n# world\n");
        assert!(g.domains.is_empty());
    }

    // -----------------------------------------------------------------------
    // Node declarations
    // -----------------------------------------------------------------------

    #[test]
    fn test_minimal_node() {
        let g = p("node ca {\n}\n");
        assert_eq!(g.nodes.len(), 1);
        let n = &g.nodes[0];
        assert_eq!(n.ident, "ca");
        assert_eq!(n.display_name, None);
        assert!(!n.is_anchored);
        assert!(!n.is_selected);
        assert!(n.properties.is_empty());
    }

    #[test]
    fn test_node_with_display_name() {
        let g = p("node ca \"Certificate Authority\" {\n}\n");
        assert_eq!(g.nodes[0].display_name, Some("Certificate Authority".into()));
    }

    #[test]
    fn test_node_root_annotation() {
        let g = p("node ca @anchored {\n}\n");
        assert!(g.nodes[0].is_anchored);
        assert!(!g.nodes[0].is_selected);
    }

    #[test]
    fn test_node_selected_annotation() {
        let g = p("node ca @selected {\n}\n");
        assert!(!g.nodes[0].is_anchored);
        assert!(g.nodes[0].is_selected);
    }

    #[test]
    fn test_node_root_and_selected() {
        let g = p("node ca @anchored @selected {\n}\n");
        assert!(g.nodes[0].is_anchored);
        assert!(g.nodes[0].is_selected);
    }

    #[test]
    fn test_node_with_properties() {
        let g = p("node ca {\n  public_key\n  subject.org\n}\n");
        let props = &g.nodes[0].properties;
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].name, "public_key");
        assert!(!props[0].critical);
        assert!(!props[0].constrained);
        assert_eq!(props[1].name, "subject.org");
    }

    #[test]
    fn test_node_property_critical() {
        let g = p("node ca {\n  public_key @critical\n}\n");
        assert!(g.nodes[0].properties[0].critical);
        assert!(!g.nodes[0].properties[0].constrained);
    }

    #[test]
    fn test_node_property_constrained() {
        let g = p("node ca {\n  public_key @constrained\n}\n");
        assert!(!g.nodes[0].properties[0].critical);
        assert!(g.nodes[0].properties[0].constrained);
    }

    #[test]
    fn test_node_property_critical_and_constrained() {
        let g = p("node ca {\n  public_key @critical @constrained\n}\n");
        assert!(g.nodes[0].properties[0].critical);
        assert!(g.nodes[0].properties[0].constrained);
    }

    #[test]
    fn test_node_body_with_blank_lines() {
        let g = p("node ca {\n\n  public_key\n\n  subject.org\n\n}\n");
        assert_eq!(g.nodes[0].properties.len(), 2);
    }

    #[test]
    fn test_node_trailing_comment_on_open_brace_line() {
        let g = p("node ca { # this is fine\n}\n");
        assert!(g.nodes[0].properties.is_empty());
    }

    #[test]
    fn test_node_trailing_comment_on_property_line() {
        let g = p("node ca {\n  public_key # a key\n}\n");
        assert_eq!(g.nodes[0].properties[0].name, "public_key");
    }

    // -----------------------------------------------------------------------
    // Domain declarations
    // -----------------------------------------------------------------------

    #[test]
    fn test_minimal_domain() {
        let g = p("domain \"PKI\" {\n}\n");
        assert_eq!(g.domains.len(), 1);
        assert_eq!(g.domains[0].display_name, "PKI");
        assert!(g.domains[0].nodes.is_empty());
    }

    #[test]
    fn test_domain_with_node() {
        let g = p("domain \"PKI\" {\n  node ca {\n  }\n}\n");
        assert_eq!(g.domains[0].nodes.len(), 1);
        assert_eq!(g.domains[0].nodes[0].ident, "ca");
    }

    #[test]
    fn test_domain_missing_name_error() {
        let err = p_err("domain {\n}\n");
        assert!(matches!(err, ObgraphError::Parse { .. }));
    }

    // -----------------------------------------------------------------------
    // Anchor statements
    // -----------------------------------------------------------------------

    #[test]
    fn test_simple_link() {
        let g = p("cert <- ca\n");
        assert_eq!(g.anchors.len(), 1);
        let l = &g.anchors[0];
        assert_eq!(l.child_ident, "cert");
        assert_eq!(l.parent_ident, "ca");
        assert_eq!(l.operation, None);
    }

    #[test]
    fn test_link_with_operation() {
        let g = p("cert <- ca : sign\n");
        let l = &g.anchors[0];
        assert_eq!(l.operation, Some("sign".into()));
    }

    // -----------------------------------------------------------------------
    // Constraint statements
    // -----------------------------------------------------------------------

    #[test]
    fn test_simple_constraint_prop_ref() {
        let g = p("cert::issuer.org <= ca::subject.org\n");
        assert_eq!(g.constraints.len(), 1);
        let c = &g.constraints[0];
        assert_eq!(c.dest_node, "cert");
        assert_eq!(c.dest_prop, "issuer.org");
        match &c.source {
            AstSourceExpr::PropRef { node, prop } => {
                assert_eq!(node, "ca");
                assert_eq!(prop, "subject.org");
            }
            _ => panic!("expected PropRef"),
        }
        assert_eq!(c.operation, None);
    }

    #[test]
    fn test_constraint_with_operation() {
        let g = p("cert::signature <= ca::public_key : verified_by\n");
        let c = &g.constraints[0];
        assert_eq!(c.operation, Some("verified_by".into()));
    }

    // -----------------------------------------------------------------------
    // Multiple top-level items
    // -----------------------------------------------------------------------

    #[test]
    fn test_multiple_top_level_nodes() {
        let g = p("node a {\n}\nnode b {\n}\n");
        assert_eq!(g.nodes.len(), 2);
    }

    #[test]
    fn test_multiple_domains() {
        let g = p("domain \"A\" {\n}\ndomain \"B\" {\n}\n");
        assert_eq!(g.domains.len(), 2);
    }

    #[test]
    fn test_mixed_top_level() {
        let g = p("node a {\n  p\n  q\n}\nb <- a\na::p <= a::q\n");
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(g.anchors.len(), 1);
        assert_eq!(g.constraints.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Error cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_node_missing_brace() {
        let err = p_err("node ca\n");
        assert!(matches!(err, ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_unknown_top_level_token() {
        let err = p_err("<- foo\n");
        assert!(matches!(err, ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_constraint_missing_source() {
        let err = p_err("cert::issuer.org <=\n");
        assert!(matches!(err, ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_bare_ident_not_link_or_constraint() {
        let err = p_err("foo bar\n");
        assert!(matches!(err, ObgraphError::Parse { .. }));
    }

    // -----------------------------------------------------------------------
    // Complete example from DESIGN.md §3.2
    // -----------------------------------------------------------------------

    const FULL_EXAMPLE: &str = r#"domain "PKI" {
  node ca "Certificate Authority" @anchored @selected {
    subject.common_name    @constrained
    subject.org            @constrained
    public_key             @constrained
  }

  node cert "Certificate" {
    issuer.common_name     @critical
    issuer.org             @critical
    subject.common_name
    subject.org
    public_key
    signature              @critical
  }
}

domain "Transport" {
  node tls "TLS Session" {
    server_cert            @critical
    cipher_suite
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
    fn test_full_example_parses() {
        let g = p(FULL_EXAMPLE);

        // Domains
        assert_eq!(g.domains.len(), 2);
        assert_eq!(g.domains[0].display_name, "PKI");
        assert_eq!(g.domains[1].display_name, "Transport");

        // PKI domain nodes
        let pki = &g.domains[0];
        assert_eq!(pki.nodes.len(), 2);

        let ca = &pki.nodes[0];
        assert_eq!(ca.ident, "ca");
        assert_eq!(ca.display_name, Some("Certificate Authority".into()));
        assert!(ca.is_anchored);
        assert!(ca.is_selected);
        assert_eq!(ca.properties.len(), 3);
        assert_eq!(ca.properties[0].name, "subject.common_name");
        assert!(!ca.properties[0].critical);
        assert!(ca.properties[0].constrained);

        let cert = &pki.nodes[1];
        assert_eq!(cert.ident, "cert");
        assert_eq!(cert.properties.len(), 6);
        assert_eq!(cert.properties[0].name, "issuer.common_name");
        assert!(cert.properties[0].critical);
        assert!(!cert.properties[0].constrained);

        // Transport domain
        let transport = &g.domains[1];
        assert_eq!(transport.nodes.len(), 1);
        let tls = &transport.nodes[0];
        assert_eq!(tls.ident, "tls");
        assert_eq!(tls.properties.len(), 2);

        // Top-level node
        assert_eq!(g.nodes.len(), 1);
        let rev = &g.nodes[0];
        assert_eq!(rev.ident, "revocation");
        assert!(rev.is_anchored);
        assert!(!rev.is_selected);
        assert!(!rev.properties[0].critical);
        assert!(rev.properties[0].constrained);

        // Anchors
        assert_eq!(g.anchors.len(), 2);
        assert_eq!(g.anchors[0].child_ident, "cert");
        assert_eq!(g.anchors[0].parent_ident, "ca");
        assert_eq!(g.anchors[0].operation, Some("sign".into()));
        assert_eq!(g.anchors[1].child_ident, "tls");
        assert_eq!(g.anchors[1].parent_ident, "cert");
        assert_eq!(g.anchors[1].operation, None);

        // Constraints
        assert_eq!(g.constraints.len(), 4);

        let c0 = &g.constraints[0];
        assert_eq!(c0.dest_node, "cert");
        assert_eq!(c0.dest_prop, "issuer.common_name");
        match &c0.source {
            AstSourceExpr::PropRef { node, prop } => {
                assert_eq!(node, "ca");
                assert_eq!(prop, "subject.common_name");
            }
            _ => panic!("expected PropRef"),
        }

        let c2 = &g.constraints[2];
        assert_eq!(c2.dest_prop, "signature");
        assert_eq!(c2.operation, Some("verified_by".into()));

        let c3 = &g.constraints[3];
        assert_eq!(c3.dest_prop, "subject.common_name");
        assert_eq!(c3.operation, Some("not_in".into()));
    }
}
