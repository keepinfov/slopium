use crate::diagnostic::{CompileResult, Span};
use crate::{lexer, parser};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyntaxKind {
    LeftParen,
    RightParen,
    Atom,
    String,
    Comment,
    Whitespace,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SyntaxToken {
    pub kind: SyntaxKind,
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum SyntaxElement {
    Token(SyntaxToken),
    List(SyntaxNode),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SyntaxNode {
    pub span: Span,
    pub children: Vec<SyntaxElement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LosslessSyntax {
    pub tokens: Vec<SyntaxToken>,
    pub root: SyntaxNode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatOptions {
    pub indent_width: usize,
    pub preferred_width: usize,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            indent_width: 2,
            preferred_width: 88,
        }
    }
}

pub fn lex_lossless(source: &str) -> Vec<SyntaxToken> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let mut line = 1;
    let mut column = 1;
    while cursor < source.len() {
        let start = cursor;
        let token_line = line;
        let token_column = column;
        let ch = source[cursor..]
            .chars()
            .next()
            .expect("cursor is at a character boundary");
        let kind = match ch {
            '(' => {
                cursor += 1;
                column += 1;
                SyntaxKind::LeftParen
            }
            ')' => {
                cursor += 1;
                column += 1;
                SyntaxKind::RightParen
            }
            ';' => {
                cursor += 1;
                column += 1;
                while cursor < source.len() {
                    let next = source[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is at a character boundary");
                    if next == '\n' || next == '\r' {
                        break;
                    }
                    cursor += next.len_utf8();
                    column += 1;
                }
                SyntaxKind::Comment
            }
            '"' => {
                cursor += 1;
                column += 1;
                let mut escaped = false;
                while cursor < source.len() {
                    let next = source[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is at a character boundary");
                    cursor += next.len_utf8();
                    if next == '\n' {
                        line += 1;
                        column = 1;
                    } else {
                        column += 1;
                    }
                    if !escaped && next == '"' {
                        break;
                    }
                    escaped = !escaped && next == '\\';
                    if next != '\\' {
                        escaped = false;
                    }
                }
                SyntaxKind::String
            }
            value if value.is_whitespace() => {
                cursor += value.len_utf8();
                if value == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
                while cursor < source.len() {
                    let next = source[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is at a character boundary");
                    if !next.is_whitespace() {
                        break;
                    }
                    cursor += next.len_utf8();
                    if next == '\n' {
                        line += 1;
                        column = 1;
                    } else {
                        column += 1;
                    }
                }
                SyntaxKind::Whitespace
            }
            _ => {
                cursor += ch.len_utf8();
                column += 1;
                while cursor < source.len() {
                    let next = source[cursor..]
                        .chars()
                        .next()
                        .expect("cursor is at a character boundary");
                    if next.is_whitespace() || matches!(next, '(' | ')' | ';') {
                        break;
                    }
                    cursor += next.len_utf8();
                    column += 1;
                }
                SyntaxKind::Atom
            }
        };
        tokens.push(SyntaxToken {
            kind,
            text: source[start..cursor].to_owned(),
            span: Span {
                start,
                end: cursor,
                line: token_line,
                column: token_column,
            },
        });
    }
    tokens
}

pub fn parse_lossless(source: &str) -> LosslessSyntax {
    let tokens = lex_lossless(source);
    let root_span = Span {
        start: 0,
        end: source.len(),
        line: 1,
        column: 1,
    };
    let mut stack = vec![SyntaxNode {
        span: root_span,
        children: Vec::new(),
    }];
    // The tree this builds is dropped recursively, so its depth has to be
    // bounded even though the build itself is iterative. Past the limit,
    // parens are kept as plain tokens in the current node: `tokens` stays
    // lossless and only the grouping degrades, for input the parser rejects
    // anyway.
    let mut suppressed = 0usize;
    for token in &tokens {
        match token.kind {
            SyntaxKind::LeftParen if stack.len() > crate::parser::MAX_NESTING_DEPTH => {
                suppressed += 1;
                stack
                    .last_mut()
                    .expect("root node exists")
                    .children
                    .push(SyntaxElement::Token(token.clone()));
            }
            SyntaxKind::LeftParen => stack.push(SyntaxNode {
                span: token.span,
                children: vec![SyntaxElement::Token(token.clone())],
            }),
            SyntaxKind::RightParen if suppressed > 0 => {
                suppressed -= 1;
                stack
                    .last_mut()
                    .expect("root node exists")
                    .children
                    .push(SyntaxElement::Token(token.clone()));
            }
            SyntaxKind::RightParen if stack.len() > 1 => {
                let current = stack.last_mut().expect("root node exists");
                current.children.push(SyntaxElement::Token(token.clone()));
                current.span = current.span.join(token.span);
                let list = stack.pop().expect("list node exists");
                stack
                    .last_mut()
                    .expect("root node exists")
                    .children
                    .push(SyntaxElement::List(list));
            }
            _ => stack
                .last_mut()
                .expect("root node exists")
                .children
                .push(SyntaxElement::Token(token.clone())),
        }
    }
    while stack.len() > 1 {
        let mut list = stack.pop().expect("list node exists");
        if let Some(last) = list.children.last() {
            let end = match last {
                SyntaxElement::Token(token) => token.span,
                SyntaxElement::List(node) => node.span,
            };
            list.span = list.span.join(end);
        }
        stack
            .last_mut()
            .expect("root node exists")
            .children
            .push(SyntaxElement::List(list));
    }
    LosslessSyntax {
        tokens,
        root: stack.pop().expect("root node exists"),
    }
}

pub fn format_source(file: &str, source: &str, options: &FormatOptions) -> CompileResult<String> {
    let semantic_tokens = lexer::lex(file, source)?;
    parser::parse(file, &semantic_tokens)?;
    let syntax = parse_lossless(source);
    let tokens = syntax.tokens;
    let mut output = String::new();
    let mut depth = 0usize;
    let mut line_start = true;
    let mut pending_space = false;
    let mut pending_newlines = 0usize;
    let mut previous = None;

    for (index, token) in tokens.iter().enumerate() {
        if token.kind == SyntaxKind::Whitespace {
            let newlines = token.text.bytes().filter(|byte| *byte == b'\n').count();
            if newlines == 0 {
                pending_space = true;
            } else {
                pending_newlines = if newlines > 1 { 2 } else { 1 };
                pending_space = false;
            }
            continue;
        }

        if pending_newlines > 0 {
            while output.ends_with(' ') {
                output.pop();
            }
            if !output.is_empty() {
                let existing = output
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|byte| **byte == b'\n')
                    .count();
                for _ in existing..pending_newlines {
                    output.push('\n');
                }
            }
            line_start = true;
            pending_newlines = 0;
        }

        let needs_separator = pending_space
            && previous != Some(SyntaxKind::LeftParen)
            && token.kind != SyntaxKind::RightParen;
        let current_width = output
            .rsplit('\n')
            .next()
            .map_or(0, |line| line.chars().count());
        let token_width = token
            .text
            .lines()
            .next()
            .map_or(0, |line| line.chars().count());
        if !line_start
            && token.kind != SyntaxKind::RightParen
            && token.kind != SyntaxKind::Comment
            && current_width + usize::from(needs_separator) + token_width > options.preferred_width
        {
            while output.ends_with(' ') {
                output.pop();
            }
            output.push('\n');
            line_start = true;
            pending_space = false;
        }

        if line_start {
            let closes_current_list = token.kind == SyntaxKind::RightParen;
            let indent_depth = depth.saturating_sub(usize::from(closes_current_list));
            output.push_str(&" ".repeat(indent_depth * options.indent_width));
            line_start = false;
        } else {
            let separated_token = pending_space
                && previous != Some(SyntaxKind::LeftParen)
                && token.kind != SyntaxKind::RightParen;
            let trailing_comment =
                token.kind == SyntaxKind::Comment && previous.is_some() && !output.ends_with(' ');
            if separated_token || trailing_comment {
                output.push(' ');
            }
        }
        pending_space = false;

        output.push_str(&token.text);
        match token.kind {
            SyntaxKind::LeftParen => depth += 1,
            SyntaxKind::RightParen => depth = depth.saturating_sub(1),
            SyntaxKind::Comment => {
                let next_has_newline = tokens.get(index + 1).is_some_and(|next| {
                    next.kind == SyntaxKind::Whitespace && next.text.contains('\n')
                });
                if !next_has_newline {
                    output.push('\n');
                    line_start = true;
                }
            }
            _ => {}
        }
        previous = Some(token.kind);
    }

    while output.ends_with([' ', '\n', '\r', '\t']) {
        output.pop();
    }
    output.push('\n');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile_to_hir, CompileOptions};

    /// The bundled library is read by anyone who follows a diagnostic into it,
    /// and it is the one package `fmt` will never be run on, because it is not
    /// anybody's project. So it is checked here instead.
    #[test]
    fn the_bundled_library_is_canonically_formatted() {
        for package in slopium_std::TOOLCHAIN_PACKAGES {
            for (module, source) in package.modules {
                let path = slopium_std::toolchain_source_path(package.name, module);
                let formatted = format_source(&path, source, &FormatOptions::default())
                    .expect("a bundled module parses");
                assert_eq!(&formatted, source, "`{path}` is not canonically formatted");
            }
        }
    }

    fn remove_spans(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                fields.remove("span");
                for value in fields.values_mut() {
                    remove_spans(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    remove_spans(value);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn lossless_tokens_reconstruct_source() {
        let source = "(fn main () -> i32 ; comment\n  (println \"💥\\n\")\n  0)\n";
        let reconstructed = lex_lossless(source)
            .into_iter()
            .map(|token| token.text)
            .collect::<String>();
        assert_eq!(reconstructed, source);
        let syntax = parse_lossless(source);
        assert!(syntax
            .root
            .children
            .iter()
            .any(|child| matches!(child, SyntaxElement::List(_))));
    }

    #[test]
    fn a_doc_block_survives_the_formatter_unchanged() {
        // `;;` means documentation now (`D-134`), and the formatter has no
        // opinion about it: what hover reads is the bytes the author wrote, so
        // a pass that reflowed or re-indented one would change the sentence.
        let source = concat!(
            ";; The answer.\n",
            ";; Two lines of it.\n",
            "(fn answer () -> i64 42)\n",
            "(fn main () -> i32 0)\n",
        );
        let formatted = format_source("test.slp", source, &FormatOptions::default()).unwrap();
        assert_eq!(formatted, source);
    }

    #[test]
    fn formatting_is_idempotent_and_preserves_comments() {
        let source =
            "  (fn   main ()  -> i32 ; trailing\n\n (let message \"hello\")\n; leading\n  0 )";
        let formatted = format_source("test.slp", source, &FormatOptions::default()).unwrap();
        assert!(formatted.contains("; trailing"));
        assert!(formatted.contains("; leading"));
        assert_eq!(
            format_source("test.slp", &formatted, &FormatOptions::default()).unwrap(),
            formatted
        );
        let before = compile_to_hir("test.slp", source, &CompileOptions::default()).unwrap();
        let after = compile_to_hir("test.slp", &formatted, &CompileOptions::default()).unwrap();
        let mut before = serde_json::to_value(before).unwrap();
        let mut after = serde_json::to_value(after).unwrap();
        remove_spans(&mut before);
        remove_spans(&mut after);
        assert_eq!(before, after);
    }

    #[test]
    fn preferred_width_breaks_long_forms_stably() {
        let source = "(fn main () -> i64 (+ 111111111111111 (+ 222222222222222 (+ 333333333333333 (+ 444444444444444 555555555555555)))))";
        let formatted = format_source("test.slp", source, &FormatOptions::default()).unwrap();
        assert!(formatted.lines().all(|line| line.chars().count() <= 88));
        assert_eq!(
            format_source("test.slp", &formatted, &FormatOptions::default()).unwrap(),
            formatted
        );
    }
}
