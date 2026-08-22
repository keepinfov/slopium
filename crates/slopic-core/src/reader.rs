//! The abbreviation table: a token standing for structure nobody typed.
//!
//! An abbreviation expands here, between the lexer and the parser, so nothing
//! downstream learns it was written — not `ast.rs`, not `sema`, not MIR, not
//! either backend (`D-149`). The tree stays uniform S-expressions and only the
//! characters get shorter, which is what `'x` for `(quote x)` has been in every
//! Lisp for sixty years.
//!
//! The table has rows nothing expands yet. They are refused by name rather than
//! left free, so that the macros `D-109` deferred inherit a mechanism instead of
//! claiming a character ad hoc.

use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};
use crate::lexer::{Token, TokenKind};
use crate::parser::MAX_NESTING_DEPTH;

/// What a row of the table stands for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expands {
    /// A list whose head is this name and whose one argument is the next form:
    /// `&x` for `(& x)`.
    Around(&'static str),
    /// A list opened where the sigil is written and closed where the form
    /// holding it closes: `(a $ b c)` for `(a (b c))`.
    Rest,
    /// The `)` of every list still open, back to the top level.
    Everything,
    /// Nothing yet. The row is held so that what it is for cannot be beaten to
    /// the character by something else.
    Nothing,
}

/// A sigil: a token standing for structure nobody typed.
pub struct Sigil {
    /// The characters that are written.
    pub text: &'static str,
    /// The structure it stands for.
    pub expansion: Expands,
    /// What the row is for, in a sentence a refusal can end with.
    pub means: &'static str,
}

/// Every sigil the reader knows, longest text first.
///
/// `&mut` is the one row of more than a character, and it is matched only as a
/// whole atom: `&mutable` is a borrow of `mutable` and not an exclusive borrow
/// of `able`.
pub const SIGILS: &[Sigil] = &[
    Sigil {
        text: "&mut",
        expansion: Expands::Around("&mut"),
        means: "an exclusive borrow",
    },
    Sigil {
        text: "&",
        expansion: Expands::Around("&"),
        means: "a shared borrow",
    },
    Sigil {
        text: "$",
        expansion: Expands::Rest,
        means: "the rest of a form, nested",
    },
    Sigil {
        text: "|)",
        expansion: Expands::Everything,
        means: "the close of every list a declaration left open",
    },
    Sigil {
        text: "'",
        expansion: Expands::Nothing,
        means: "quotation, for the macros this language has not built yet",
    },
    Sigil {
        text: "`",
        expansion: Expands::Nothing,
        means: "quasiquotation, for the macros this language has not built yet",
    },
    Sigil {
        text: ",",
        expansion: Expands::Nothing,
        means: "unquotation, for the macros this language has not built yet",
    },
];

/// The sigil an atom's text begins with, if any.
///
/// A row of one character matches as a prefix, so `&x` and `&(f x)` are a
/// sigil and its operand. A longer row matches only the whole text, because a
/// word-shaped sigil cannot be told from the beginning of a name.
pub fn sigil_prefix(text: &str) -> Option<&'static Sigil> {
    SIGILS.iter().find(|sigil| {
        if sigil.text.len() > 1 {
            text == sigil.text
        } else {
            text.starts_with(sigil.text)
        }
    })
}

impl Sigil {
    /// How the sigil is written before its operand.
    ///
    /// A sigil of one character binds to what follows with nothing between
    /// them. A word-shaped one keeps the space, because `&mut` is a sigil only
    /// as a whole atom and `&mutx` reads back as a shared borrow of `mutx`.
    pub fn prefix(&self) -> String {
        if self.text.len() > 1 {
            format!("{} ", self.text)
        } else {
            self.text.to_owned()
        }
    }
}

/// The sigil an atom *is*, if any.
pub fn sigil_of(text: &str) -> Option<&'static Sigil> {
    SIGILS.iter().find(|sigil| sigil.text == text)
}

/// The sigil a list's head is the expansion of, if any.
///
/// This is what lets the formatter write `(& x)` back as `&x`: the layout
/// recognises the list an abbreviation stands for rather than remembering
/// which spelling arrived, so all three spellings leave as one.
pub fn sigil_for_head(head: &str) -> Option<&'static Sigil> {
    SIGILS
        .iter()
        .find(|sigil| matches!(sigil.expansion, Expands::Around(name) if name == head))
}

/// Divides an atom's text into the sigils it begins with and the name they
/// stand before, so that `&x` is two pieces and `&` alone stays one.
///
/// Both readers call this — the one that builds a tree and the lossless one
/// the formatter lays out — because a spelling the two disagreed about would be
/// a program `fmt` could change the meaning of.
pub fn pieces(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = text;
    loop {
        match sigil_prefix(rest) {
            Some(sigil) if sigil.text.len() < rest.len() => {
                pieces.push(&rest[..sigil.text.len()]);
                rest = &rest[sigil.text.len()..];
            }
            _ => {
                pieces.push(rest);
                return pieces;
            }
        }
    }
}

/// [`pieces`], applied to one lexed atom.
fn split_atom(token: &Token, out: &mut Vec<Token>) {
    let TokenKind::Atom(text) = &token.kind else {
        out.push(token.clone());
        return;
    };
    let mut start = token.span.start;
    let mut column = token.span.column;
    let pieces = pieces(text);
    for (index, piece) in pieces.iter().enumerate() {
        let last = index + 1 == pieces.len();
        out.push(Token {
            kind: TokenKind::Atom((*piece).to_owned()),
            span: Span {
                start,
                end: if last {
                    token.span.end
                } else {
                    start + piece.len()
                },
                line: token.span.line,
                column,
            },
        });
        start += piece.len();
        column += piece.chars().count();
    }
}

/// Rewrites every abbreviation into the form it stands for.
pub fn expand(file: &str, tokens: &[Token]) -> CompileResult<Vec<Token>> {
    let mut split = Vec::with_capacity(tokens.len());
    for token in tokens {
        split_atom(token, &mut split);
    }
    let mut reader = Reader {
        file,
        out: Vec::with_capacity(split.len()),
        tokens: split,
        cursor: 0,
        depth: 0,
        closing: None,
        diagnostics: Vec::new(),
    };
    while reader.cursor < reader.tokens.len() {
        reader.element(false);
    }
    if reader.diagnostics.is_empty() {
        Ok(reader.out)
    } else {
        Err(reader.diagnostics)
    }
}

struct Reader<'a> {
    file: &'a str,
    tokens: Vec<Token>,
    out: Vec<Token>,
    cursor: usize,
    depth: usize,
    /// The span of a `|)` being unwound, held until depth reaches zero.
    closing: Option<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl Reader<'_> {
    /// Whether the sigil under the cursor opens the list it is the head of.
    ///
    /// It does when the form it would take is all that follows it: the parens
    /// an abbreviation would add there are the ones already written, so `(& x)`
    /// is the borrow it has always been and not a call through one. With
    /// anything after that form the sigil abbreviates like any other, which is
    /// what makes a list of borrowed types — `(&T &T)`, the parameter list of a
    /// `Fn` — read as the two elements it looks like.
    fn head_opens_the_list(&self) -> bool {
        let mut cursor = self.cursor;
        while let Some(TokenKind::Atom(text)) = self.tokens.get(cursor).map(|token| &token.kind) {
            if sigil_of(text).is_some_and(|sigil| matches!(sigil.expansion, Expands::Around(_))) {
                cursor += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(cursor).map(|token| &token.kind) {
            // `$` makes everything after it one form, so the run's operand is
            // all that follows and the head opens the list after all:
            // `(& $ f x)` is `(& (f x))`.
            Some(TokenKind::Atom(text))
                if sigil_of(text).is_some_and(|sigil| sigil.expansion == Expands::Rest) =>
            {
                return true
            }
            Some(TokenKind::LeftParen) => {
                let mut depth = 0usize;
                loop {
                    match self.tokens.get(cursor).map(|token| &token.kind) {
                        Some(TokenKind::LeftParen) => depth += 1,
                        Some(TokenKind::RightParen) => depth -= 1,
                        Some(_) => {}
                        None => return false,
                    }
                    cursor += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Some(TokenKind::RightParen) | None => return false,
            Some(_) => cursor += 1,
        }
        matches!(
            self.tokens.get(cursor).map(|token| &token.kind),
            Some(TokenKind::RightParen)
        )
    }

    /// Copies one element to the output, expanding the abbreviation it begins
    /// with.
    ///
    /// `head` says the sigil opens the list rather than standing inside it,
    /// where there is nothing to abbreviate and `&` is the ordinary atom
    /// `(& value)` has always been written with.
    /// The answer is how many lists the element left open for the form around
    /// it to close, which is what `$` after a sigil produces.
    fn element(&mut self, head: bool) -> usize {
        let token = self.tokens[self.cursor].clone();
        let sigil = match &token.kind {
            TokenKind::Atom(text) => sigil_of(text),
            _ => None,
        };
        match (&token.kind, sigil) {
            (_, Some(sigil)) if sigil.expansion == Expands::Nothing => {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::RESERVED_SIGIL,
                        self.file,
                        token.span,
                        format!("`{}` is reserved", sigil.text),
                    )
                    .with_note(format!("it is held for {}", sigil.means)),
                );
                self.cursor += 1;
                0
            }
            // `|)` is read by the loop over the list it closes, so reaching one
            // here means there is no list open for it to close.
            (_, Some(sigil)) if sigil.expansion == Expands::Everything => {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::UNEXPECTED_CLOSE,
                        self.file,
                        token.span,
                        "unexpected `|)`",
                    )
                    .with_help(
                        "`|)` ends a declaration by closing every list it left open, and there \
                         is none open here",
                    ),
                );
                self.cursor += 1;
                0
            }
            // `$` nests what follows into the list around it, so it is read by
            // the loop over that list and never as an element of its own.
            // Reaching one here means there is no list for it to nest in.
            (_, Some(sigil)) if sigil.expansion == Expands::Rest => {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::ABBREVIATION,
                        self.file,
                        token.span,
                        "`$` nests the rest of the form it is written in, and there is none here",
                    )
                    .with_help(
                        "`$` stands between a form's head and the rest of it, as in \
                                `(println $ from-i64 42)`",
                    ),
                );
                self.cursor += 1;
                0
            }
            (_, Some(sigil)) if !head => self.abbreviation(sigil),
            (TokenKind::LeftParen, _) => {
                // Past the parser's own limit the file is refused anyway, and
                // recursing beside it is what the limit exists to prevent.
                if self.depth >= MAX_NESTING_DEPTH {
                    self.copy_balanced();
                    return 0;
                }
                self.out.push(token);
                self.cursor += 1;
                self.depth += 1;
                let mut first = true;
                let mut nested = 0usize;
                while self.cursor < self.tokens.len()
                    && self.closing.is_none()
                    && !matches!(self.tokens[self.cursor].kind, TokenKind::RightParen)
                {
                    if self.closes_everything() {
                        self.closing = Some(self.tokens[self.cursor].span);
                        self.cursor += 1;
                        break;
                    }
                    if self.nests_here() {
                        if self.nest(first) {
                            nested += 1;
                            first = true;
                        }
                        continue;
                    }
                    let head = first && self.head_opens_the_list();
                    nested += self.element(head);
                    first = false;
                }
                self.depth -= 1;
                // Every list `$` opened closes where this one does, innermost
                // first, and each ends at the last form written into it.
                let end = self.out.last().expect("the list opened with a paren").span;
                for _ in 0..nested {
                    self.out.push(Token {
                        kind: TokenKind::RightParen,
                        span: Span {
                            start: end.end,
                            end: end.end,
                            line: end.line,
                            column: end.column,
                        },
                    });
                }
                // A `|)` closes this list too, and every one outside it, with
                // a `)` whose span is the closer that was written.
                if let Some(span) = self.closing {
                    self.out.push(Token {
                        kind: TokenKind::RightParen,
                        span,
                    });
                    if self.depth == 0 {
                        self.closing = None;
                    }
                } else if let Some(close) = self.tokens.get(self.cursor) {
                    self.out.push(close.clone());
                    self.cursor += 1;
                }
                0
            }
            _ => {
                self.out.push(token);
                self.cursor += 1;
                0
            }
        }
    }

    /// Whether the token under the cursor is the row that closes everything.
    fn closes_everything(&self) -> bool {
        match &self.tokens[self.cursor].kind {
            TokenKind::Atom(text) => {
                sigil_of(text).is_some_and(|sigil| sigil.expansion == Expands::Everything)
            }
            _ => false,
        }
    }

    /// Whether the token under the cursor is the row that nests.
    fn nests_here(&self) -> bool {
        match &self.tokens[self.cursor].kind {
            TokenKind::Atom(text) => {
                sigil_of(text).is_some_and(|sigil| sigil.expansion == Expands::Rest)
            }
            _ => false,
        }
    }

    /// Opens the list `$` stands for, and says whether one was opened.
    ///
    /// `first` says `$` is where a form's head belongs, which is the one place
    /// it cannot go: the list it opens would have nothing to be the rest of.
    fn nest(&mut self, first: bool) -> bool {
        let span = self.tokens[self.cursor].span;
        self.cursor += 1;
        let empty = self
            .tokens
            .get(self.cursor)
            .is_none_or(|token| matches!(token.kind, TokenKind::RightParen));
        if first || empty {
            let message = if first {
                "`$` nests the rest of a form, and a form's first element is its head"
            } else {
                "`$` nests the rest of a form, and there is none here"
            };
            self.diagnostics.push(
                Diagnostic::error(codes::ABBREVIATION, self.file, span, message).with_help(
                    "`$` stands between a form's head and the rest of it, as in \
                     `(println $ from-i64 42)`",
                ),
            );
            return false;
        }
        self.out.push(Token {
            kind: TokenKind::LeftParen,
            span,
        });
        true
    }

    /// Expands a run of sigils and the one form they stand before.
    ///
    /// The run is taken whole rather than one sigil at a time, so `&&value`
    /// costs the reader no stack: what nests is the output, which the parser
    /// bounds already.
    fn abbreviation(&mut self, first: &'static Sigil) -> usize {
        let mut run = vec![(first, self.tokens[self.cursor].span)];
        self.cursor += 1;
        while let Some(token) = self.tokens.get(self.cursor) {
            let TokenKind::Atom(text) = &token.kind else {
                break;
            };
            let Some(sigil) = sigil_of(text) else {
                break;
            };
            if !matches!(sigil.expansion, Expands::Around(_)) {
                break;
            }
            run.push((sigil, token.span));
            self.cursor += 1;
        }
        let operand = self.tokens.get(self.cursor);
        if operand.is_none_or(|token| matches!(token.kind, TokenKind::RightParen)) {
            let span = run[0].1.join(run[run.len() - 1].1);
            let text = run[run.len() - 1].0.text;
            self.diagnostics.push(
                Diagnostic::error(
                    codes::ABBREVIATION,
                    self.file,
                    span,
                    format!("`{text}` stands before a form, and there is none here"),
                )
                .with_help(format!("write the form it applies to, as in `{text}value`")),
            );
            return 0;
        }
        for (sigil, span) in &run {
            self.out.push(Token {
                kind: TokenKind::LeftParen,
                span: *span,
            });
            self.out.push(Token {
                kind: TokenKind::Atom(match sigil.expansion {
                    Expands::Around(head) => head.to_owned(),
                    _ => unreachable!("only a prefix row reaches an expansion"),
                }),
                span: *span,
            });
        }
        // `$` is the operand: what the sigil applies to is the rest of the
        // form around it, so these lists close where that form closes and the
        // loop over it is what counts them.
        if self.nests_here() {
            return run.len();
        }
        self.element(false);
        // The synthesized list ends where its operand does: text that exists,
        // which is what a `compile_fail` snapshot asserts as a number.
        let end = self
            .out
            .last()
            .expect("the operand was copied to the output")
            .span;
        for _ in &run {
            self.out.push(Token {
                kind: TokenKind::RightParen,
                span: Span {
                    start: end.end,
                    end: end.end,
                    line: end.line,
                    column: end.column,
                },
            });
        }
        0
    }

    /// Copies tokens through the `)` that closes the already-seen `(`.
    fn copy_balanced(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.tokens.get(self.cursor) {
            match token.kind {
                TokenKind::LeftParen => depth += 1,
                TokenKind::RightParen => depth -= 1,
                _ => {}
            }
            self.out.push(token.clone());
            self.cursor += 1;
            if depth == 0 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn read(source: &str) -> String {
        let tokens = lex("test.slp", source).expect("the source lexes");
        let expanded = expand("test.slp", &tokens).expect("the source expands");
        expanded
            .iter()
            .map(|token| match &token.kind {
                TokenKind::LeftParen => "(".to_owned(),
                TokenKind::RightParen => ")".to_owned(),
                TokenKind::Atom(text) => text.clone(),
                TokenKind::String(_) => "\"…\"".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_sigil_takes_the_next_form_with_or_without_a_space() {
        assert_eq!(read("(f &x)"), "( f ( & x ) )");
        assert_eq!(read("(f & x)"), "( f ( & x ) )");
        assert_eq!(read("(f &(g x))"), "( f ( & ( g x ) ) )");
        assert_eq!(read("(f &mut x)"), "( f ( &mut x ) )");
        assert_eq!(read("(f &&x)"), "( f ( & ( & x ) ) )");
    }

    #[test]
    fn a_sigil_that_opens_a_list_is_the_atom_it_always_was() {
        assert_eq!(read("(& x)"), "( & x )");
        assert_eq!(read("(&mut x)"), "( &mut x )");
        assert_eq!(read("(& (f x))"), "( & ( f x ) )");
        // Written without the space, it still opens the list it heads.
        assert_eq!(read("(&x)"), "( & x )");
        // And a borrow of a borrow is the same form written twice.
        assert_eq!(read("(&&x)"), "( & ( & x ) )");
    }

    #[test]
    fn a_list_of_borrowed_types_is_a_list_and_not_a_borrow() {
        // The parameter list of `(Fn (&T &T) bool)`: the head sigil takes one
        // form and something follows it, so nothing there opens the list.
        assert_eq!(read("(&T &T)"), "( ( & T ) ( & T ) )");
        assert_eq!(read("(&T x)"), "( ( & T ) x )");
        assert_eq!(
            read("(&mut (List T) (index i64))"),
            "( ( &mut ( List T ) ) ( index i64 ) )"
        );
    }

    #[test]
    fn a_word_shaped_sigil_is_not_the_beginning_of_a_name() {
        assert_eq!(read("(f &mutable)"), "( f ( & mutable ) )");
    }

    #[test]
    fn the_expansion_spans_the_text_that_was_written() {
        let source = "(f &value)";
        let tokens = lex("test.slp", source).unwrap();
        let forms = parse("test.slp", &expand("test.slp", &tokens).unwrap()).unwrap();
        let crate::parser::SExprKind::List(items) = &forms[0].kind else {
            panic!("the form is a list")
        };
        let borrow = &items[1];
        assert_eq!(&source[borrow.span.start..borrow.span.end], "&value");
        assert_eq!(borrow.span.column, 4);
    }

    #[test]
    fn dollar_nests_the_rest_of_the_form_it_is_in() {
        assert_eq!(read("(a $ b c)"), "( a ( b c ) )");
        assert_eq!(read("(a $ b $ c d)"), "( a ( b ( c d ) ) )");
        assert_eq!(read("(a b $ c)"), "( a b ( c ) )");
        // A sigil after `$` heads the list `$` opened, so its operand ending
        // that list makes it the borrow it always was.
        assert_eq!(
            read("(note $ & disagreement)"),
            "( note ( & disagreement ) )"
        );
        assert_eq!(read("(f $ g &x y)"), "( f ( g ( & x ) y ) )");
        // Written without the space, exactly as `&` is.
        assert_eq!(read("(a $b c)"), "( a ( b c ) )");
    }

    #[test]
    fn a_sigil_whose_operand_is_the_rest_closes_where_the_form_does() {
        assert_eq!(read("(f & $ g h)"), "( f ( & ( g h ) ) )");
        assert_eq!(read("(f && $ g h)"), "( f ( & ( & ( g h ) ) ) )");
        assert_eq!(read("(& $ f x)"), "( & ( f x ) )");
        // The line the issue is written around, which is both rows at once.
        assert_eq!(
            read("(note $ & $ disagreement &(want) &(got))"),
            "( note ( & ( disagreement ( & ( want ) ) ( & ( got ) ) ) ) )"
        );
    }

    #[test]
    fn dollar_is_refused_where_it_nests_nothing() {
        for (source, expected) in [
            ("($ a b)", "a form's first element is its head"),
            ("(f $)", "there is none here"),
            ("$ a", "there is none here"),
        ] {
            let tokens = lex("test.slp", source).unwrap();
            let errors = expand("test.slp", &tokens).unwrap_err();
            assert_eq!(errors[0].code, codes::ABBREVIATION, "for `{source}`");
            assert!(
                errors[0].message.contains(expected),
                "`{source}` said `{}`",
                errors[0].message
            );
        }
    }

    #[test]
    fn a_nested_list_spans_from_the_dollar_to_its_last_element() {
        let source = "(println $ from-i64 42)";
        let tokens = lex("test.slp", source).unwrap();
        let forms = parse("test.slp", &expand("test.slp", &tokens).unwrap()).unwrap();
        let crate::parser::SExprKind::List(items) = &forms[0].kind else {
            panic!("the form is a list")
        };
        let nested = &items[1];
        assert_eq!(&source[nested.span.start..nested.span.end], "$ from-i64 42");
        assert_eq!(nested.span.column, 10);
    }

    /// The same assertion `&` gets, for the same reason: an abbreviation is one
    /// when the file it compiles to does not depend on it.
    #[test]
    fn nesting_with_a_dollar_emits_the_same_object() {
        let written = concat!(
            "(fn twice ((value i64)) -> i64 (* value 2))\n",
            "(fn main () -> i32\n",
            "  (as i32 (twice (twice (twice 1)))))\n",
        );
        let nested = concat!(
            "(fn twice ((value i64)) -> i64 (* value 2))\n",
            "(fn main () -> i32\n",
            "  (as i32 $ twice $ twice $ twice 1))\n",
        );
        let options = crate::CompileOptions::default();
        let long = crate::compile_to_object("same.slp", written, &options).expect("it compiles");
        let short = crate::compile_to_object("same.slp", nested, &options).expect("it compiles");
        assert_eq!(long, short);
    }

    #[test]
    fn one_token_closes_every_list_a_declaration_left_open() {
        assert_eq!(read("(a (b (c d|)"), "( a ( b ( c d ) ) )");
        assert_eq!(read("(a b|)"), "( a b )");
        assert_eq!(read("(a (b|)\n(c d)"), "( a ( b ) ) ( c d )");
        // It closes what `$` opened as well, because those are lists too.
        assert_eq!(read("(a $ b $ c d|)"), "( a ( b ( c d ) ) )");
    }

    #[test]
    fn a_closer_with_nothing_open_is_refused() {
        let tokens = lex("test.slp", "(f x)\n|)\n").unwrap();
        let errors = expand("test.slp", &tokens).unwrap_err();
        assert_eq!(errors[0].code, codes::UNEXPECTED_CLOSE);
        assert!(errors[0].message.contains("|)"));
    }

    #[test]
    fn a_bare_pipe_is_an_ordinary_name() {
        assert_eq!(read("(f | x)"), "( f | x )");
    }

    /// The closer is a spelling of the parens it stands for, so the object is
    /// the same file — the assertion `&` and `$` each get.
    #[test]
    fn closing_with_one_token_emits_the_same_object() {
        let written = concat!(
            "(fn describe ((value i64)) -> i64\n",
            "  (if (> value 0) (if (> value 10) 2 1) 0))\n",
            "(fn main () -> i32 (as i32 (describe 42)))\n",
        );
        let closed = concat!(
            "(fn describe ((value i64)) -> i64\n",
            "  (if (> value 0) (if (> value 10) 2 1) 0|)\n",
            "(fn main () -> i32 (as i32 (describe 42|)\n",
        );
        let options = crate::CompileOptions::default();
        let long = crate::compile_to_object("same.slp", written, &options).expect("it compiles");
        let short = crate::compile_to_object("same.slp", closed, &options).expect("it compiles");
        assert_eq!(long, short);
    }

    #[test]
    fn a_reserved_sigil_says_what_it_is_held_for() {
        let tokens = lex("test.slp", "(f 'x)").unwrap();
        let errors = expand("test.slp", &tokens).unwrap_err();
        assert_eq!(errors[0].code, codes::RESERVED_SIGIL);
        assert!(errors[0]
            .notes
            .iter()
            .any(|note| note.contains("quotation")));
    }

    #[test]
    fn a_sigil_with_no_operand_is_refused() {
        let tokens = lex("test.slp", "(f &)").unwrap();
        let errors = expand("test.slp", &tokens).unwrap_err();
        assert_eq!(errors[0].code, codes::ABBREVIATION);
    }

    /// The cheapest proof an abbreviation is one: the object is the same file.
    ///
    /// Everything downstream of the reader is handed the same tokens either
    /// way, so the assertion is not that the two programs behave alike but that
    /// they *are* one program — which is what makes the 752 sites `fmt` respelt
    /// a formatting change and not a rewrite.
    #[test]
    fn respelling_a_borrow_emits_the_same_object() {
        let long = concat!(
            "(struct Counter ((value i64)))\n",
            "(fn read ((counter (& Counter))) -> i64\n",
            "  (match counter ((Counter :value value) (clone value))))\n",
            "(fn bump ((counter (&mut Counter))) -> unit\n",
            "  (match counter ((Counter :value value) (set value (+ (clone value) 1)))))\n",
            "(fn main () -> i32\n",
            "  (let mut counter (Counter :value 0))\n",
            "  (bump (&mut counter))\n",
            "  (as i32 (read (& counter))))\n",
        );
        let short = long
            .replace("(& Counter)", "&Counter")
            .replace("(&mut Counter)", "&mut Counter")
            .replace("(bump (&mut counter))", "(bump &mut counter)")
            .replace("(read (& counter))", "(read &counter)");
        assert_ne!(long, short);
        let options = crate::CompileOptions::default();
        let written = crate::compile_to_object("same.slp", long, &options).expect("it compiles");
        let respelt = crate::compile_to_object("same.slp", &short, &options).expect("it compiles");
        assert_eq!(written, respelt);
    }

    #[test]
    fn nesting_past_the_parser_limit_is_left_for_the_parser() {
        let source = "(".repeat(MAX_NESTING_DEPTH * 4);
        let tokens = lex("deep.slp", &source).unwrap();
        let expanded = expand("deep.slp", &tokens).expect("the reader does not refuse depth");
        assert_eq!(expanded.len(), tokens.len());
    }
}
