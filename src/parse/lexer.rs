/// Tokenizer for the obgraph DSL (DESIGN.md §3.2).

/// Token types produced by the lexer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A bare identifier: `[a-zA-Z_][a-zA-Z0-9_]*`
    Ident(String),
    /// A double-quoted string literal (no escape sequences).
    StringLit(String),
    /// `node` keyword
    KwNode,
    /// `domain` keyword
    KwDomain,
    /// `@root`
    AtRoot,
    /// `@selected`
    AtSelected,
    /// `@critical`
    AtCritical,
    /// `@constrained`
    AtConstrained,
    /// `<-` link arrow
    LeftArrow,
    /// `<=` constraint operator
    LeftAngleEq,
    /// `::`
    ColonColon,
    /// `:`
    Colon,
    /// `.`
    Dot,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// End of line (significant as statement terminator).
    Newline,
    /// End of input.
    Eof,
}

/// A token with its source location.
#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub line: usize,
    pub col: usize,
}

/// The lexer state.
pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Peek at the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Advance past the current character and return it.
    fn advance(&mut self) -> Option<char> {
        let ch = self.input[self.pos..].chars().next()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    /// Skip horizontal whitespace (spaces and tabs only).
    fn skip_horizontal_whitespace(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.advance();
        }
    }

    /// Skip a `#`-comment to the end of the line (but do not consume the newline).
    fn skip_comment(&mut self) {
        // Assumes current char is '#'
        while !matches!(self.peek(), Some('\n') | None) {
            self.advance();
        }
    }

    /// Parse a string literal: `"..."`. Assumes the opening `"` has not been consumed.
    fn lex_string(&mut self, start_line: usize, start_col: usize) -> Result<Token, crate::ObgraphError> {
        // consume opening quote
        self.advance();
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(crate::ObgraphError::Parse {
                        line: start_line,
                        col: start_col,
                        message: "unterminated string literal".to_string(),
                    });
                }
                Some('"') => {
                    self.advance(); // closing quote
                    return Ok(Token::StringLit(s));
                }
                Some(_) => {
                    s.push(self.advance().unwrap());
                }
            }
        }
    }

    /// Parse an identifier or keyword/annotation that starts with a letter or underscore.
    fn lex_ident(&mut self) -> Token {
        let mut s = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            s.push(self.advance().unwrap());
        }
        match s.as_str() {
            "node" => Token::KwNode,
            "domain" => Token::KwDomain,
            _ => Token::Ident(s),
        }
    }

    /// Parse an `@`-annotation. Assumes `@` has not yet been consumed.
    fn lex_at(&mut self, line: usize, col: usize) -> Result<Token, crate::ObgraphError> {
        self.advance(); // consume '@'
        let mut name = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            name.push(self.advance().unwrap());
        }
        match name.as_str() {
            "root" => Ok(Token::AtRoot),
            "selected" => Ok(Token::AtSelected),
            "critical" => Ok(Token::AtCritical),
            "constrained" => Ok(Token::AtConstrained),
            other => Err(crate::ObgraphError::Parse {
                line,
                col,
                message: format!("unknown annotation `@{other}`"),
            }),
        }
    }

    /// Tokenize the entire input, returning all tokens.
    ///
    /// Comments are stripped. Multiple consecutive newlines are collapsed into
    /// one `Newline` token. A final `Eof` token is always appended.
    pub fn tokenize(&mut self) -> Result<Vec<Spanned>, crate::ObgraphError> {
        let mut tokens: Vec<Spanned> = Vec::new();
        // Whether the last meaningful token was a Newline (used for collapsing).
        let mut last_was_newline = true; // treat start-of-file as if preceded by newline

        loop {
            // Skip horizontal whitespace before every token.
            self.skip_horizontal_whitespace();

            let tok_line = self.line;
            let tok_col = self.col;

            let ch = match self.peek() {
                None => {
                    // Ensure a trailing newline before EOF if the last token wasn't one.
                    if !last_was_newline && !tokens.is_empty() {
                        tokens.push(Spanned { token: Token::Newline, line: tok_line, col: tok_col });
                    }
                    tokens.push(Spanned { token: Token::Eof, line: tok_line, col: tok_col });
                    break;
                }
                Some(c) => c,
            };

            match ch {
                // --- Comment: skip to end of line, then treat the \n normally ---
                '#' => {
                    self.skip_comment();
                    // The newline (if present) will be handled in the next iteration.
                    continue;
                }

                // --- Newline ---
                '\n' => {
                    self.advance();
                    // Collapse consecutive newlines.
                    if !last_was_newline {
                        tokens.push(Spanned { token: Token::Newline, line: tok_line, col: tok_col });
                        last_was_newline = true;
                    }
                    continue;
                }

                // --- String literal ---
                '"' => {
                    let tok = self.lex_string(tok_line, tok_col)?;
                    tokens.push(Spanned { token: tok, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }

                // --- Annotation ---
                '@' => {
                    let tok = self.lex_at(tok_line, tok_col)?;
                    tokens.push(Spanned { token: tok, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }

                // --- Identifier / keyword ---
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let tok = self.lex_ident();
                    tokens.push(Spanned { token: tok, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }

                // --- Two-character operators ---
                '<' => {
                    self.advance(); // consume '<'
                    match self.peek() {
                        Some('-') => {
                            self.advance();
                            tokens.push(Spanned { token: Token::LeftArrow, line: tok_line, col: tok_col });
                        }
                        Some('=') => {
                            self.advance();
                            tokens.push(Spanned { token: Token::LeftAngleEq, line: tok_line, col: tok_col });
                        }
                        _ => {
                            return Err(crate::ObgraphError::Parse {
                                line: tok_line,
                                col: tok_col,
                                message: "expected `<-` or `<=`".to_string(),
                            });
                        }
                    }
                    last_was_newline = false;
                }

                ':' => {
                    self.advance(); // consume first ':'
                    match self.peek() {
                        Some(':') => {
                            self.advance();
                            tokens.push(Spanned { token: Token::ColonColon, line: tok_line, col: tok_col });
                        }
                        _ => {
                            tokens.push(Spanned { token: Token::Colon, line: tok_line, col: tok_col });
                        }
                    }
                    last_was_newline = false;
                }

                // --- Single-character tokens ---
                '.' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::Dot, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }
                '(' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::LParen, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }
                ')' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::RParen, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }
                ',' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::Comma, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }
                '{' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::LBrace, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }
                '}' => {
                    self.advance();
                    tokens.push(Spanned { token: Token::RBrace, line: tok_line, col: tok_col });
                    last_was_newline = false;
                }

                other => {
                    return Err(crate::ObgraphError::Parse {
                        line: tok_line,
                        col: tok_col,
                        message: format!("unexpected character `{other}`"),
                    });
                }
            }
        }

        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<Token> {
        Lexer::new(input)
            .tokenize()
            .expect("tokenize failed")
            .into_iter()
            .map(|s| s.token)
            .collect()
    }

    fn tokenize_spanned(input: &str) -> Vec<Spanned> {
        Lexer::new(input).tokenize().expect("tokenize failed")
    }

    // -------------------------------------------------------------------------
    // Individual token types
    // -------------------------------------------------------------------------

    #[test]
    fn test_eof_on_empty_input() {
        let toks = tokenize("");
        assert_eq!(toks, vec![Token::Eof]);
    }

    #[test]
    fn test_ident() {
        let toks = tokenize("hello");
        assert_eq!(toks, vec![Token::Ident("hello".into()), Token::Newline, Token::Eof]);
    }

    #[test]
    fn test_ident_with_digits_and_underscore() {
        let toks = tokenize("foo_bar123");
        assert_eq!(toks[0], Token::Ident("foo_bar123".into()));
    }

    #[test]
    fn test_keyword_node() {
        let toks = tokenize("node");
        assert_eq!(toks[0], Token::KwNode);
    }

    #[test]
    fn test_keyword_domain() {
        let toks = tokenize("domain");
        assert_eq!(toks[0], Token::KwDomain);
    }

    #[test]
    fn test_string_literal() {
        let toks = tokenize("\"hello world\"");
        assert_eq!(toks[0], Token::StringLit("hello world".into()));
    }

    #[test]
    fn test_string_literal_empty() {
        let toks = tokenize("\"\"");
        assert_eq!(toks[0], Token::StringLit(String::new()));
    }

    #[test]
    fn test_at_root() {
        let toks = tokenize("@root");
        assert_eq!(toks[0], Token::AtRoot);
    }

    #[test]
    fn test_at_selected() {
        let toks = tokenize("@selected");
        assert_eq!(toks[0], Token::AtSelected);
    }

    #[test]
    fn test_at_critical() {
        let toks = tokenize("@critical");
        assert_eq!(toks[0], Token::AtCritical);
    }

    #[test]
    fn test_at_constrained() {
        let toks = tokenize("@constrained");
        assert_eq!(toks[0], Token::AtConstrained);
    }

    #[test]
    fn test_left_arrow() {
        let toks = tokenize("<-");
        assert_eq!(toks[0], Token::LeftArrow);
    }

    #[test]
    fn test_left_angle_eq() {
        let toks = tokenize("<=");
        assert_eq!(toks[0], Token::LeftAngleEq);
    }

    #[test]
    fn test_colon_colon() {
        let toks = tokenize("::");
        assert_eq!(toks[0], Token::ColonColon);
    }

    #[test]
    fn test_colon() {
        let toks = tokenize(":");
        assert_eq!(toks[0], Token::Colon);
    }

    #[test]
    fn test_dot() {
        let toks = tokenize(".");
        assert_eq!(toks[0], Token::Dot);
    }

    #[test]
    fn test_punctuation() {
        let toks = tokenize("(){}.,");
        assert_eq!(
            &toks[..6],
            &[Token::LParen, Token::RParen, Token::LBrace, Token::RBrace, Token::Dot, Token::Comma]
        );
    }

    // -------------------------------------------------------------------------
    // Newline collapsing
    // -------------------------------------------------------------------------

    #[test]
    fn test_newline_collapsing() {
        // Three blank lines between two tokens should produce only one Newline.
        let toks = tokenize("foo\n\n\nbar");
        assert_eq!(
            toks,
            vec![
                Token::Ident("foo".into()),
                Token::Newline,
                Token::Ident("bar".into()),
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_no_leading_newline() {
        // Input that starts with a newline should not emit a leading Newline token.
        let toks = tokenize("\nfoo");
        assert_eq!(toks[0], Token::Ident("foo".into()));
    }

    // -------------------------------------------------------------------------
    // Comment stripping
    // -------------------------------------------------------------------------

    #[test]
    fn test_full_line_comment() {
        let toks = tokenize("# this is a comment\nfoo");
        // The comment line itself should not produce any tokens except the newline
        // (collapsed since it's the first real content), and then foo.
        assert_eq!(toks[0], Token::Ident("foo".into()));
    }

    #[test]
    fn test_trailing_comment() {
        let toks = tokenize("foo # comment\nbar");
        assert_eq!(
            toks,
            vec![
                Token::Ident("foo".into()),
                Token::Newline,
                Token::Ident("bar".into()),
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_comment_only_input() {
        let toks = tokenize("# just a comment\n");
        assert_eq!(toks, vec![Token::Eof]);
    }

    // -------------------------------------------------------------------------
    // Source locations
    // -------------------------------------------------------------------------

    #[test]
    fn test_location_tracking() {
        let spanned = tokenize_spanned("foo\nbar");
        let foo = &spanned[0];
        assert_eq!(foo.line, 1);
        assert_eq!(foo.col, 1);

        let bar = &spanned[2]; // [foo, Newline, bar, ...]
        assert_eq!(bar.line, 2);
        assert_eq!(bar.col, 1);
    }

    #[test]
    fn test_col_advances() {
        let spanned = tokenize_spanned("foo bar");
        let bar = &spanned[1];
        assert_eq!(bar.col, 5);
    }

    // -------------------------------------------------------------------------
    // Error cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_unterminated_string() {
        let err = Lexer::new("\"hello").tokenize().unwrap_err();
        assert!(matches!(err, crate::ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_unknown_annotation() {
        let err = Lexer::new("@foo").tokenize().unwrap_err();
        assert!(matches!(err, crate::ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_unknown_annotation_word() {
        let err = Lexer::new("@trust").tokenize().unwrap_err();
        assert!(matches!(err, crate::ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_unexpected_character() {
        let err = Lexer::new("$").tokenize().unwrap_err();
        assert!(matches!(err, crate::ObgraphError::Parse { .. }));
    }

    #[test]
    fn test_bare_less_than() {
        let err = Lexer::new("< ").tokenize().unwrap_err();
        assert!(matches!(err, crate::ObgraphError::Parse { .. }));
    }

    // -------------------------------------------------------------------------
    // Compound / integration
    // -------------------------------------------------------------------------

    #[test]
    fn test_node_declaration_tokens() {
        let toks = tokenize("node ca \"Certificate Authority\" @root @selected {");
        assert_eq!(
            toks,
            vec![
                Token::KwNode,
                Token::Ident("ca".into()),
                Token::StringLit("Certificate Authority".into()),
                Token::AtRoot,
                Token::AtSelected,
                Token::LBrace,
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_link_tokens() {
        let toks = tokenize("cert <- ca : sign");
        assert_eq!(
            toks,
            vec![
                Token::Ident("cert".into()),
                Token::LeftArrow,
                Token::Ident("ca".into()),
                Token::Colon,
                Token::Ident("sign".into()),
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_constraint_tokens() {
        let toks = tokenize("cert::issuer.org <= ca::subject.org");
        assert_eq!(
            toks,
            vec![
                Token::Ident("cert".into()),
                Token::ColonColon,
                Token::Ident("issuer".into()),
                Token::Dot,
                Token::Ident("org".into()),
                Token::LeftAngleEq,
                Token::Ident("ca".into()),
                Token::ColonColon,
                Token::Ident("subject".into()),
                Token::Dot,
                Token::Ident("org".into()),
                Token::Newline,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn test_property_annotation_tokens() {
        let toks = tokenize("subject.common_name @constrained");
        assert_eq!(
            toks,
            vec![
                Token::Ident("subject".into()),
                Token::Dot,
                Token::Ident("common_name".into()),
                Token::AtConstrained,
                Token::Newline,
                Token::Eof,
            ]
        );
    }
}
