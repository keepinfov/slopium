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

/// A sigil: a prefix standing before the one form it applies to.
pub struct Sigil {
    /// The characters that are written.
    pub text: &'static str,
    /// The head of the list it expands to, or `None` for a reserved row.
    pub expansion: Option<&'static str>,
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
        expansion: Some("&mut"),
        means: "an exclusive borrow",
    },
    Sigil {
        text: "&",
        expansion: Some("&"),
        means: "a shared borrow",
    },
    Sigil {
        text: "'",
        expansion: None,
        means: "quotation, for the macros this language has not built yet",
    },
    Sigil {
        text: "`",
        expansion: None,
        means: "quasiquotation, for the macros this language has not built yet",
    },
    Sigil {
        text: ",",
        expansion: None,
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
    SIGILS.iter().find(|sigil| sigil.expansion == Some(head))
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
            if sigil_of(text).is_some_and(|sigil| sigil.expansion.is_some()) {
                cursor += 1;
            } else {
                break;
            }
        }
        match self.tokens.get(cursor).map(|token| &token.kind) {
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
    fn element(&mut self, head: bool) {
        let token = self.tokens[self.cursor].clone();
        let sigil = match &token.kind {
            TokenKind::Atom(text) => sigil_of(text),
            _ => None,
        };
        match (&token.kind, sigil) {
            (_, Some(sigil)) if sigil.expansion.is_none() => {
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
            }
            (_, Some(sigil)) if !head => self.abbreviation(sigil),
            (TokenKind::LeftParen, _) => {
                // Past the parser's own limit the file is refused anyway, and
                // recursing beside it is what the limit exists to prevent.
                if self.depth >= MAX_NESTING_DEPTH {
                    self.copy_balanced();
                    return;
                }
                self.out.push(token);
                self.cursor += 1;
                self.depth += 1;
                let mut first = true;
                while self.cursor < self.tokens.len()
                    && !matches!(self.tokens[self.cursor].kind, TokenKind::RightParen)
                {
                    let head = first && self.head_opens_the_list();
                    self.element(head);
                    first = false;
                }
                self.depth -= 1;
                if let Some(close) = self.tokens.get(self.cursor) {
                    self.out.push(close.clone());
                    self.cursor += 1;
                }
            }
            _ => {
                self.out.push(token);
                self.cursor += 1;
            }
        }
    }

    /// Expands a run of sigils and the one form they stand before.
    ///
    /// The run is taken whole rather than one sigil at a time, so `&&value`
    /// costs the reader no stack: what nests is the output, which the parser
    /// bounds already.
    fn abbreviation(&mut self, first: &'static Sigil) {
        let mut run = vec![(first, self.tokens[self.cursor].span)];
        self.cursor += 1;
        while let Some(token) = self.tokens.get(self.cursor) {
            let TokenKind::Atom(text) = &token.kind else {
                break;
            };
            let Some(sigil) = sigil_of(text) else {
                break;
            };
            if sigil.expansion.is_none() {
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
            return;
        }
        for (sigil, span) in &run {
            self.out.push(Token {
                kind: TokenKind::LeftParen,
                span: *span,
            });
            self.out.push(Token {
                kind: TokenKind::Atom(
                    sigil
                        .expansion
                        .expect("a reserved sigil never reaches an expansion")
                        .to_owned(),
                ),
                span: *span,
            });
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
