use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    LeftParen,
    RightParen,
    Atom(String),
    /// A text literal's *bytes*, not its characters.
    ///
    /// A Slopium `String` is a length and a byte buffer (`D-079`), and `\xNN`
    /// writes one byte for any `NN` (`D-106`). A Rust `String` could not hold
    /// the result — `\xFF` is not a character — so the value is bytes from here
    /// all the way to the object file, where it always was.
    String(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Appends a source character as the bytes it is written in.
///
/// The source is UTF-8, so a character outside ASCII already *is* several
/// bytes, and a text literal holding one keeps exactly those. That is what
/// makes `(len "é")` answer 2 both before this patch and after it.
fn push_char(value: &mut Vec<u8>, ch: char) {
    let mut buffer = [0; 4];
    value.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
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
                let mut value: Vec<u8> = Vec::new();
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
                                    'n' => value.push(b'\n'),
                                    'r' => value.push(b'\r'),
                                    't' => value.push(b'\t'),
                                    '"' => value.push(b'"'),
                                    '\\' => value.push(b'\\'),
                                    // A Slopium string may hold a NUL (`D-079`)
                                    // and until now had no way to write one.
                                    '0' => value.push(0),
                                    // Exactly one byte, `\x00` through `\xFF`.
                                    // Not a code point: a payload is bytes, and
                                    // a `\xFF` that arrived as two of them
                                    // would be a different string than the one
                                    // written.
                                    'x' => {
                                        let mut digits = String::new();
                                        for _ in 0..2 {
                                            let Some((_, digit)) = chars.peek().copied() else {
                                                break;
                                            };
                                            if !digit.is_ascii_hexdigit() {
                                                break;
                                            }
                                            chars.next();
                                            end += digit.len_utf8();
                                            column += 1;
                                            digits.push(digit);
                                        }
                                        match u8::from_str_radix(&digits, 16) {
                                            Ok(byte) if digits.len() == 2 => value.push(byte),
                                            _ => diagnostics.push(
                                                Diagnostic::error(
                                                    codes::UNKNOWN_ESCAPE,
                                                    file,
                                                    Span {
                                                        start: esc_idx,
                                                        end,
                                                        line,
                                                        column: column - 1,
                                                    },
                                                    "`\\x` takes exactly two hexadecimal digits",
                                                )
                                                .with_help("a byte is written `\\x00` to `\\xff`"),
                                            ),
                                        }
                                    }
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
                                            .with_help("supported escapes are \\n, \\r, \\t, \\0, \\xNN, \\\", and \\\\"),
                                        );
                                        push_char(&mut value, other);
                                    }
                                }
                            }
                        }
                        '\n' => {
                            value.push(b'\n');
                            line += 1;
                            column = 1;
                        }
                        other => push_char(&mut value, other),
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
            // `|` ends a token wherever it appears, so `(c d|)` is a name and a
            // closer. A closer built from a character a name may contain could
            // not work at all — `<` and `*` are ordinary names, which is the
            // trap `D-106` refused when it declined `(& a b)`.
            //
            // `|)` is one token: the closer that ends every list a declaration
            // left open (`D-151`). It is lexed here rather than read as a `|`
            // beside a `)` because the two are one character apart on purpose —
            // `| )` is a name and a paren, and nothing else in the language
            // depends on whitespace.
            '|' if matches!(chars.peek(), Some((_, ')'))) => {
                chars.next();
                tokens.push(Token {
                    kind: TokenKind::Atom("|)".to_owned()),
                    span: Span {
                        start,
                        end: start + 2,
                        line,
                        column,
                    },
                });
                column += 2;
            }
            _ => {
                let mut end = start + ch.len_utf8();
                while let Some((idx, next)) = chars.peek().copied() {
                    // A text literal opens a token wherever it appears, which
                    // is what makes `&"literal"` the borrow it looks like
                    // (`D-149`). Before the sigils nothing could be written
                    // immediately before a `"`, so nothing depended on an atom
                    // swallowing one.
                    if next.is_whitespace() || matches!(next, '(' | ')' | ';' | '"' | '|') {
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
