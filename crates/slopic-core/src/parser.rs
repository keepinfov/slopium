use crate::diagnostic::{codes, Applicability, CompileResult, Diagnostic, Span};
use crate::lexer::{Token, TokenKind};

#[derive(Clone, Debug, PartialEq)]
pub enum SExprKind {
    Atom(String),
    /// A text literal's bytes (see `lexer::TokenKind::String`).
    String(Vec<u8>),
    List(Vec<SExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SExpr {
    pub kind: SExprKind,
    pub span: Span,
}

/// Deepest `(` nesting the parser will descend into.
///
/// Every later pass walks the same tree recursively - including the lossless
/// syntax tree's own `Drop` - so this single limit is what keeps the whole
/// front end off the stack guard page. The bound is calibrated against a 2 MiB
/// stack (the default for non-main threads, which is what the language server
/// and `cargo test` run on) with unoptimized frames: that survives ~768 levels
/// and dies by ~1024, so 256 leaves roughly a threefold margin. Hand-written
/// programs nest an order of magnitude less than this.
pub const MAX_NESTING_DEPTH: usize = 256;

pub fn parse(file: &str, tokens: &[Token]) -> CompileResult<Vec<SExpr>> {
    let mut parser = Parser {
        file,
        tokens,
        cursor: 0,
        depth: 0,
        reported_nesting: false,
        diagnostics: Vec::new(),
    };
    let mut forms = Vec::new();
    while parser.cursor < tokens.len() {
        if matches!(tokens[parser.cursor].kind, TokenKind::RightParen) {
            let span = tokens[parser.cursor].span;
            parser.diagnostics.push(Diagnostic::error(
                codes::UNEXPECTED_CLOSE,
                file,
                span,
                "unexpected `)`",
            ));
            parser.cursor += 1;
            continue;
        }
        if let Some(form) = parser.form() {
            forms.push(form);
        }
    }
    if parser.diagnostics.is_empty() {
        Ok(forms)
    } else {
        Err(parser.diagnostics)
    }
}

struct Parser<'a> {
    file: &'a str,
    tokens: &'a [Token],
    cursor: usize,
    depth: usize,
    reported_nesting: bool,
    diagnostics: Vec<Diagnostic>,
}

impl Parser<'_> {
    /// Consume tokens through the `)` that closes the already-consumed `(`,
    /// without recursing, so an over-deep form still leaves the cursor at a
    /// sane place and the parser terminates.
    fn skip_balanced(&mut self) {
        let mut depth = 1usize;
        while depth > 0 {
            let Some(token) = self.tokens.get(self.cursor) else {
                return;
            };
            self.cursor += 1;
            match token.kind {
                TokenKind::LeftParen => depth += 1,
                TokenKind::RightParen => depth -= 1,
                _ => {}
            }
        }
    }

    fn form(&mut self) -> Option<SExpr> {
        let token = self.tokens.get(self.cursor)?.clone();
        self.cursor += 1;
        match token.kind {
            TokenKind::Atom(value) => Some(SExpr {
                kind: SExprKind::Atom(value),
                span: token.span,
            }),
            TokenKind::String(value) => Some(SExpr {
                kind: SExprKind::String(value),
                span: token.span,
            }),
            TokenKind::RightParen => {
                self.diagnostics.push(Diagnostic::error(
                    codes::UNEXPECTED_CLOSE,
                    self.file,
                    token.span,
                    "unexpected `)`",
                ));
                None
            }
            TokenKind::LeftParen => {
                if self.depth >= MAX_NESTING_DEPTH {
                    if !self.reported_nesting {
                        self.reported_nesting = true;
                        self.diagnostics.push(
                            Diagnostic::error(
                                codes::MAX_NESTING,
                                self.file,
                                token.span,
                                format!(
                                    "expression nesting is deeper than {MAX_NESTING_DEPTH} levels"
                                ),
                            )
                            .with_help(
                                "restructure the expression into named functions or bindings",
                            ),
                        );
                    }
                    self.skip_balanced();
                    return None;
                }
                self.depth += 1;
                let mut forms = Vec::new();
                while self.cursor < self.tokens.len()
                    && !matches!(self.tokens[self.cursor].kind, TokenKind::RightParen)
                {
                    if let Some(form) = self.form() {
                        forms.push(form);
                    }
                }
                self.depth -= 1;
                let Some(closing) = self.tokens.get(self.cursor) else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            codes::UNCLOSED_LIST,
                            self.file,
                            token.span,
                            "unclosed `(`",
                        )
                        .with_help("add a matching `)`")
                        .with_suggestion(
                            Span {
                                start: token.span.end,
                                end: token.span.end,
                                line: token.span.line,
                                column: token.span.column + 1,
                            },
                            ")",
                            "close this list",
                            Applicability::MaybeIncorrect,
                        ),
                    );
                    return None;
                };
                self.cursor += 1;
                Some(SExpr {
                    kind: SExprKind::List(forms),
                    span: token.span.join(closing.span),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    #[test]
    fn parses_nested_forms() {
        let tokens = lex("test.slp", "(+ 1 (* 2 3))").unwrap();
        let forms = parse("test.slp", &tokens).unwrap();
        let SExprKind::List(items) = &forms[0].kind else {
            panic!()
        };
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn rejects_nesting_past_the_depth_limit() {
        // Deep enough to overflow the stack if the descent were unbounded.
        let source = "(".repeat(MAX_NESTING_DEPTH * 200);
        let tokens = lex("deep.slp", &source).unwrap();
        let errors = parse("deep.slp", &tokens).unwrap_err();
        assert_eq!(
            errors
                .iter()
                .filter(|error| error.code == codes::MAX_NESTING)
                .count(),
            1,
            "the nesting limit should be reported exactly once"
        );
    }

    #[test]
    fn accepts_nesting_up_to_the_depth_limit() {
        let depth = MAX_NESTING_DEPTH - 1;
        let source = format!("{}{}", "(".repeat(depth), ")".repeat(depth));
        let tokens = lex("deep.slp", &source).unwrap();
        parse("deep.slp", &tokens).unwrap();
    }

    #[test]
    fn reports_unclosed_list() {
        let tokens = lex("test.slp", "(+ 1").unwrap();
        assert!(parse("test.slp", &tokens).unwrap_err()[0]
            .message
            .contains("unclosed"));
    }
}
