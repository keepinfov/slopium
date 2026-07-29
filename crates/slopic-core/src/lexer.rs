use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    LeftParen,
    RightParen,
    Atom(String),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub fn lex(file: &str, source: &str) -> CompileResult<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut chars = source.char_indices().peekable();
    let mut line = 1;
    let mut column = 1;

    while let Some((start, ch)) = chars.next() {
        let token_line = line;
        let token_column = column;
        match ch {
            '(' => {
                tokens.push(Token {
                    kind: TokenKind::LeftParen,
                    span: Span {
                        start,
                        end: start + 1,
                        line,
                        column,
                    },
                });
                column += 1;
            }
            ')' => {
                tokens.push(Token {
                    kind: TokenKind::RightParen,
                    span: Span {
                        start,
                        end: start + 1,
                        line,
                        column,
                    },
                });
                column += 1;
            }
            ';' => {
                column += 1;
                while let Some((_, next)) = chars.peek().copied() {
                    if next == '\n' {
                        break;
                    }
                    chars.next();
                    column += 1;
                }
            }
            '"' => {
                column += 1;
                let mut value = String::new();
                let mut end = start + 1;
                let mut terminated = false;
                while let Some((idx, next)) = chars.next() {
                    end = idx + next.len_utf8();
                    column += 1;
                    match next {
                        '"' => {
                            terminated = true;
                            break;
                        }
                        '\\' => {
                            if let Some((esc_idx, escaped)) = chars.next() {
                                end = esc_idx + escaped.len_utf8();
                                column += 1;
                                match escaped {
                                    'n' => value.push('\n'),
                                    'r' => value.push('\r'),
                                    't' => value.push('\t'),
                                    '"' => value.push('"'),
                                    '\\' => value.push('\\'),
                                    other => {
                                        diagnostics.push(
                                            Diagnostic::error(
                                                codes::UNKNOWN_ESCAPE,
                                                file,
                                                Span {
                                                    start: esc_idx,
                                                    end,
                                                    line,
                                                    column: column - 1,
                                                },
                                                format!("unknown string escape `\\{other}`"),
                                            )
                                            .with_help("supported escapes are \\n, \\r, \\t, \\\", and \\\\"),
                                        );
                                        value.push(other);
                                    }
                                }
                            }
                        }
                        '\n' => {
                            value.push('\n');
                            line += 1;
                            column = 1;
                        }
                        other => value.push(other),
                    }
                }
                if !terminated {
                    diagnostics.push(Diagnostic::error(
                        codes::UNTERMINATED_STRING,
                        file,
                        Span {
                            start,
                            end,
                            line: token_line,
                            column: token_column,
                        },
                        "unterminated string literal",
                    ));
                } else {
                    tokens.push(Token {
                        kind: TokenKind::String(value),
                        span: Span {
                            start,
                            end,
                            line: token_line,
                            column: token_column,
                        },
                    });
                }
            }
            ch if ch.is_whitespace() => {
                if ch == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
            }
            _ => {
                let mut end = start + ch.len_utf8();
                while let Some((idx, next)) = chars.peek().copied() {
                    if next.is_whitespace() || matches!(next, '(' | ')' | ';') {
                        break;
                    }
                    chars.next();
                    end = idx + next.len_utf8();
                }
                let atom = source[start..end].to_owned();
                column += atom.chars().count();
                tokens.push(Token {
                    kind: TokenKind::Atom(atom),
                    span: Span {
                        start,
                        end,
                        line: token_line,
                        column: token_column,
                    },
                });
            }
        }
    }

    if diagnostics.is_empty() {
        Ok(tokens)
    } else {
        Err(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_escaped_strings() {
        let tokens = lex("test.slp", "(println \"a\\n\") ; ignored\n42").unwrap();
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[2].kind, TokenKind::String("a\n".into()));
        assert_eq!(tokens[4].kind, TokenKind::Atom("42".into()));
    }

    #[test]
    fn reports_unterminated_string() {
        let error = lex("test.slp", "\"oops").unwrap_err();
        assert!(error[0].message.contains("unterminated"));
    }
}
