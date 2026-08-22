use crate::diagnostic::{CompileResult, Span};
use crate::{lexer, parser, reader};
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
                    if next.is_whitespace() || matches!(next, '(' | ')' | ';' | '"') {
                        break;
                    }
                    cursor += next.len_utf8();
                    column += 1;
                }
                SyntaxKind::Atom
            }
        };
        // An atom divides into the sigils it begins with and the name they
        // stand before, exactly as `reader::expand` divides it, so the layout
        // sees the same pieces the parser will (`D-149`).
        if kind == SyntaxKind::Atom {
            let mut piece_start = start;
            let mut piece_column = token_column;
            let pieces = reader::pieces(&source[start..cursor]);
            for (index, piece) in pieces.iter().enumerate() {
                let last = index + 1 == pieces.len();
                let piece_end = if last {
                    cursor
                } else {
                    piece_start + piece.len()
                };
                tokens.push(SyntaxToken {
                    kind,
                    text: (*piece).to_owned(),
                    span: Span {
                        start: piece_start,
                        end: piece_end,
                        line: token_line,
                        column: piece_column,
                    },
                });
                piece_start += piece.len();
                piece_column += piece.chars().count();
            }
            continue;
        }
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

/// One element of a form, with the trivia the layout has to place.
///
/// The lossless tree carries whitespace and comments as tokens; this carries
/// only the two facts a layout needs from them — whether the author left a
/// blank line above an element, and whether a comment sat at the end of an
/// element's line rather than above the next one.
#[derive(Clone, Debug)]
enum Item {
    Atom(String),
    Comment(String),
    List(Vec<Entry>),
}

#[derive(Clone, Debug)]
struct Entry {
    item: Item,
    blank_before: bool,
    trailing: Vec<String>,
    /// A sigil that may not be written short, because the list it heads holds
    /// nothing else: `(&T)` is the borrow `(& T)` and always was, so a lone
    /// borrowed type inside a parameter list keeps its parentheses (`D-149`).
    spelled_out: bool,
    /// An arm of a `match`: a pattern, and then a body. Nothing about the arm
    /// itself says so — its head is a list, exactly as a field list's is — so
    /// the form above it is what marks it.
    arm: bool,
}

/// An atom that binds to what follows it, so the two are never split across
/// two lines: `->` before a return type, `:` before a declared one, a field
/// keyword before the value it names, and `$` before the form it nests.
///
/// The layout knows nothing else about `$`: which grouping a human meant by one
/// is not recoverable from the tree, so it is carried through untouched and
/// only kept off the end of a line (`D-150`).
fn is_glue(item: &Item) -> bool {
    matches!(item, Item::Atom(text)
        if text == "->" || text == "$" || text.starts_with(':')
            || reader::sigil_of(text).is_some())
}

/// The list an abbreviation stands for: its sigil, and the one form it applies
/// to.
///
/// `(& x)`, `& x` and `&x` all reach the layout as this shape, and all three
/// leave it written `&x` — which is how the 752 sites that were written the
/// long way migrate (`D-149`).
fn abbreviated(entries: &[Entry]) -> Option<(String, &Entry)> {
    let [head, operand] = entries else {
        return None;
    };
    let Item::Atom(text) = &head.item else {
        return None;
    };
    let sigil = reader::sigil_for_head(text)?;
    let ordinary = !head.spelled_out
        && head.trailing.is_empty()
        && operand.trailing.is_empty()
        && !matches!(operand.item, Item::Comment(_));
    ordinary.then(|| (sigil.prefix(), operand))
}

/// Folds each sigil into the form it stands before, so that the layout has one
/// shape to lay out however the source spelled it.
///
/// A sigil that opens a list with nothing after its operand is left alone:
/// there it is the head of the form itself, and `(& x)` is the borrow it has
/// always been. This is `reader::head_opens_the_list` read off the entries
/// rather than off the tokens, and the two have to agree or `fmt` writes a
/// program back as a different one.
fn glue_sigils(entries: Vec<Entry>, in_list: bool) -> Vec<Entry> {
    let mut folded: Vec<Entry> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate().rev() {
        let opens = in_list
            && index == 0
            && folded
                .iter()
                .filter(|entry| !matches!(entry.item, Item::Comment(_)))
                .count()
                <= 1;
        let sigil = match &entry.item {
            Item::Atom(text) if !opens => reader::sigil_of(text)
                .and_then(|sigil| match sigil.expansion {
                    reader::Expands::Around(head) => Some(head),
                    _ => None,
                })
                .filter(|_| entry.trailing.is_empty()),
            _ => None,
        };
        let Some(head) = sigil else {
            folded.push(entry);
            continue;
        };
        // A sigil whose operand is `$` applies to everything after it rather
        // than to the `$`, so the two stay the separate tokens they are: the
        // layout has no shape to fold them into (`D-150`).
        let takes = folded.last().is_some_and(|next| {
            !matches!(next.item, Item::Comment(_))
                && !matches!(&next.item, Item::Atom(text) if text == "$")
        });
        if !takes {
            folded.push(entry);
            continue;
        }
        let mut operand = folded.pop().expect("the operand was just measured");
        let trailing = std::mem::take(&mut operand.trailing);
        operand.blank_before = false;
        folded.push(Entry {
            item: Item::List(vec![
                Entry {
                    item: Item::Atom(head.to_owned()),
                    blank_before: false,
                    trailing: Vec::new(),
                    spelled_out: false,
                    arm: false,
                },
                operand,
            ]),
            blank_before: entry.blank_before,
            trailing,
            spelled_out: false,
            arm: false,
        });
    }
    folded.reverse();
    folded
}

fn entries_of(children: &[SyntaxElement], in_list: bool) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut newlines = 0usize;
    let mut started = false;
    let push = |entries: &mut Vec<Entry>, item: Item, newlines: usize, started: bool| {
        entries.push(Entry {
            item,
            blank_before: started && newlines >= 2,
            trailing: Vec::new(),
            spelled_out: false,
            arm: false,
        });
    };
    for child in children {
        match child {
            SyntaxElement::Token(token) => match token.kind {
                SyntaxKind::Whitespace => {
                    newlines += token.text.bytes().filter(|byte| *byte == b'\n').count();
                    continue;
                }
                SyntaxKind::LeftParen | SyntaxKind::RightParen => continue,
                SyntaxKind::Comment => {
                    let text = token.text.trim_end().to_owned();
                    match entries.last_mut() {
                        Some(previous) if newlines == 0 => previous.trailing.push(text),
                        _ => push(&mut entries, Item::Comment(text), newlines, started),
                    }
                }
                SyntaxKind::Atom | SyntaxKind::String => {
                    push(
                        &mut entries,
                        Item::Atom(token.text.clone()),
                        newlines,
                        started,
                    );
                }
            },
            SyntaxElement::List(node) => {
                let inner = entries_of(&node.children, true);
                push(&mut entries, Item::List(inner), newlines, started);
            }
        }
        newlines = 0;
        started = true;
    }
    let mut entries = glue_sigils(entries, in_list);
    if in_list {
        spell_out_a_lone_borrow(&mut entries);
    }
    mark_arms(&mut entries);
    entries
}

/// Keeps the parentheses on a borrow that is all its list holds.
///
/// `((& T))` is a parameter list of one borrowed type and `(&T)` is a borrow of
/// `T`: the reader reads a head sigil whose operand ends the list as the list
/// itself, so this is the one place the short spelling says something else.
fn spell_out_a_lone_borrow(entries: &mut [Entry]) {
    let mut written = entries
        .iter_mut()
        .filter(|entry| !matches!(entry.item, Item::Comment(_)));
    let Some(only) = written.next() else {
        return;
    };
    if written.next().is_some() {
        return;
    }
    let Item::List(inner) = &mut only.item else {
        return;
    };
    if abbreviated(inner).is_some() {
        inner[0].spelled_out = true;
    }
}

/// How many of an arm's entries are its head: the pattern, and the `when` and
/// the guard when it has one. What follows is the body.
fn arm_head(entries: &[Entry]) -> usize {
    match entries.get(1) {
        Some(Entry {
            item: Item::Atom(name),
            ..
        }) if name == "when" && entries.len() > 3 => 3,
        _ => 1,
    }
}

/// Marks the arms of every `match` in a form's own children.
fn mark_arms(entries: &mut [Entry]) {
    let is_match = matches!(entries.first(),
        Some(Entry { item: Item::Atom(name), .. }) if name == "match");
    for (index, entry) in entries.iter_mut().enumerate() {
        entry.arm = is_match && index > 1 && matches!(entry.item, Item::List(_));
    }
}

/// The form on one line, or `None` when it holds a comment — a comment ends a
/// line by definition, so nothing containing one has a flat shape.
fn flat(item: &Item) -> Option<String> {
    match item {
        Item::Atom(text) => Some(text.clone()),
        Item::Comment(_) => None,
        Item::List(entries) => {
            if must_break(entries) {
                return None;
            }
            if let Some((sigil, operand)) = abbreviated(entries) {
                return Some(format!("{sigil}{}", flat(&operand.item)?));
            }
            let mut parts = Vec::with_capacity(entries.len());
            for entry in entries {
                if !entry.trailing.is_empty() {
                    return None;
                }
                parts.push(flat(&entry.item)?);
            }
            Some(format!("({})", parts.join(" ")))
        }
    }
}

/// An argument with no structure under it: an atom, or a list of atoms. These
/// are what a form packs several of onto a line instead of breaking one per
/// line, which is what keeps an `export` of sixteen names three lines long.
fn is_simple(item: &Item) -> bool {
    match item {
        Item::Atom(_) => true,
        Item::Comment(_) => false,
        Item::List(entries) => entries
            .iter()
            .all(|entry| entry.trailing.is_empty() && matches!(entry.item, Item::Atom(_))),
    }
}

/// A type's parameter list: `(T)`, `(K V)`. Nothing else beside a name is a
/// list of bare atoms, so this needs no keyword to recognise it.
fn is_parameter_list(item: &Item) -> bool {
    matches!(item, Item::List(entries) if !entries.is_empty()) && is_simple(item)
}

fn width_of(text: &str) -> usize {
    text.chars().count()
}

/// How many argument groups stay beside the head when a form breaks.
///
/// A declaration's first line is its signature and a body is what follows it,
/// which is the one thing a layout cannot work out from the shape of the tree.
/// Everything absent from this table is laid out by where its arguments fit.
fn head_line_groups(head: &str, groups: &[Group]) -> Option<usize> {
    let arrow = groups
        .iter()
        .position(|group| matches!(&group.lead().item, Item::Atom(text) if text == "->"));
    match head {
        "fn" | "lambda" => Some(arrow.map_or(groups.len().min(2), |index| index + 1)),
        "extern" => Some(1),
        "let" | "set" | "const" => Some(
            groups
                .iter()
                .rposition(|group| !is_glue(&group.lead().item))
                .unwrap_or(0),
        ),
        "if" | "while" | "when" | "match" | "test" => Some(1),
        // A type's parameter list belongs beside its name, and its fields or
        // its variants are what follows.
        "struct" | "enum" => Some(match groups.get(1) {
            Some(group) if is_parameter_list(&group.lead().item) => 2,
            _ => 1,
        }),
        "do" | "loop" | "defer" | "unsafe" => Some(0),
        _ => None,
    }
}

/// One or two entries laid out as a unit: `-> i64` and `: i64` are two.
struct Group<'a> {
    entries: &'a [Entry],
}

impl Group<'_> {
    fn lead(&self) -> &Entry {
        &self.entries[0]
    }

    fn blank_before(&self) -> bool {
        self.lead().blank_before
    }

    fn ends_open(&self) -> bool {
        let last = self.entries.last().expect("a group holds an entry");
        !last.trailing.is_empty() || matches!(last.item, Item::Comment(_))
    }

    fn flat(&self) -> Option<String> {
        let mut parts = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            if !entry.trailing.is_empty() {
                return None;
            }
            parts.push(flat(&entry.item)?);
        }
        Some(parts.join(" "))
    }

    fn render(&self, indent: usize, options: &FormatOptions, closers: usize) -> String {
        let mut out = String::new();
        let last = self.entries.len() - 1;
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            let column = indent + width_of(&out);
            let closers = if index == last && entry.trailing.is_empty() {
                closers
            } else {
                0
            };
            match &entry.item {
                Item::List(inner) if entry.arm && inner.len() > arm_head(inner) + 1 => {
                    out.push_str(&render_broken(inner, column, options, true, closers));
                }
                item => out.push_str(&render_item(item, column, options, closers)),
            }
            for comment in &entry.trailing {
                out.push(' ');
                out.push_str(comment);
            }
        }
        out
    }
}

fn group(entries: &[Entry]) -> Vec<Group<'_>> {
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let mut width = 1;
        if is_glue(&entries[index].item)
            && entries[index].trailing.is_empty()
            && index + 1 < entries.len()
        {
            width = 2;
        }
        groups.push(Group {
            entries: &entries[index..index + width],
        });
        index += width;
    }
    groups
}

/// A body begins on its own line, however short it is.
///
/// Everywhere else the two shapes of `D-143` decide alone — a form fits, or it
/// does not. These seven are the exception because what follows their head
/// line is a sequence of statements rather than an argument, and a reader
/// looking for what a loop does is looking down the left margin. `if` and
/// `struct` is absent on purpose: a record of one field is a line, and `if` is
/// absent until it takes a fourth argument, because with three it is the
/// expression `(if flag 1 0)` is and reads as an argument. So is `lambda`,
/// which is an argument far more often than it is a declaration.
fn must_break(entries: &[Entry]) -> bool {
    let Some(head) = entries.first() else {
        return false;
    };
    let Item::Atom(name) = &head.item else {
        return false;
    };
    if !head.trailing.is_empty() {
        return true;
    }
    let groups = group(&entries[1..]);
    // An `if` with a fourth argument is a branch whose second arm is a
    // sequence, which is a body by any other name. With three it is the
    // conditional expression `(if flag 1 0)` is, and reads as an argument.
    if name == "if" {
        return groups.len() > 3;
    }
    if !matches!(
        name.as_str(),
        "fn" | "test" | "when" | "while" | "loop" | "do" | "match" | "enum"
    ) {
        return false;
    }
    let kept = head_line_groups(name, &groups).unwrap_or(0);
    match groups.len().checked_sub(kept) {
        // A body that is a single literal is not a body: `(fn com1-data () ->
        // u16 0x3F8)` names a constant, and putting the constant on a line of
        // its own says there is something to read there.
        Some(1) => !matches!(groups[kept].lead().item, Item::Atom(_)),
        Some(more) => more > 1,
        None => false,
    }
}

/// Lays one form out at `indent`, knowing how many closing parens will follow
/// it on the same line.
///
/// A form's last line ends with its own `)` and every ancestor's that closes
/// with it, so the width it has to fit in is the preferred width less that run.
/// Measuring without it is what let a line reach 92 columns while every
/// decision on the way down believed it had room (`D-149`).
fn render_item(item: &Item, indent: usize, options: &FormatOptions, closers: usize) -> String {
    match item {
        Item::Atom(text) => text.clone(),
        Item::Comment(text) => text.clone(),
        Item::List(entries) => {
            if entries.is_empty() {
                return "()".to_owned();
            }
            if let Some(one_line) = flat(item) {
                if indent + width_of(&one_line) + closers <= options.preferred_width {
                    return one_line;
                }
            }
            if let Some((sigil, operand)) = abbreviated(entries) {
                return format!(
                    "{sigil}{}",
                    render_item(&operand.item, indent + width_of(&sigil), options, closers)
                );
            }
            render_broken(entries, indent, options, false, closers)
        }
    }
}

/// A form that does not fit, in the four shapes it can take.
fn render_broken(
    entries: &[Entry],
    indent: usize,
    options: &FormatOptions,
    arm: bool,
    closers: usize,
) -> String {
    let head = &entries[0];
    let guard = arm && arm_head(entries) == 3;
    let groups = group(&entries[1..]);
    let mut out = String::from("(");
    let head_closers = if groups.is_empty() { closers + 1 } else { 0 };
    out.push_str(&render_item(&head.item, indent + 1, options, head_closers));
    for comment in &head.trailing {
        out.push(' ');
        out.push_str(comment);
    }
    let head_open = !head.trailing.is_empty();
    if groups.is_empty() {
        close(&mut out, indent, head_open, options);
        return out;
    }

    let body = indent + options.indent_width;
    let head_name = match &head.item {
        Item::Atom(text) if head.trailing.is_empty() => Some(text.as_str()),
        _ => None,
    };
    let table = head_name.and_then(|name| head_line_groups(name, &groups));
    // Packing is for a form that names a list of things — an `export`, a
    // `take`, a literal. A form headed by a list is a pattern and a body, and
    // filling those onto shared lines loses the one distinction it has.
    let packed = table.is_none()
        && head_name.is_some()
        && !arm
        && groups
            .iter()
            .all(|group| group.entries.len() == 1 && is_simple(&group.lead().item))
        && groups.iter().all(|group| group.lead().trailing.is_empty());

    if packed {
        let mut column = indent + width_of(&out);
        let last = groups.len() - 1;
        for (index, group) in groups.iter().enumerate() {
            let text = group.flat().expect("a simple group is flat");
            let tail = if index == last { closers + 1 } else { 0 };
            if column + 1 + width_of(&text) + tail > options.preferred_width {
                out.push('\n');
                out.push_str(&" ".repeat(body));
                column = body;
            } else {
                out.push(' ');
                column += 1;
            }
            out.push_str(&text);
            column += width_of(&text);
        }
        close(&mut out, indent, false, options);
        return out;
    }

    // Aligned under the head: only when every argument has a flat shape that
    // fits there, which is exactly when the alignment reads as one column and
    // not as a staircase.
    let aligned = table.is_none().then(|| {
        let column = indent + 1 + width_of(head_name?) + 1;
        let last = groups.len() - 1;
        groups
            .iter()
            .enumerate()
            .all(|(index, group)| {
                let tail = if index == last { closers + 1 } else { 0 };
                group
                    .flat()
                    .is_some_and(|text| column + width_of(&text) + tail <= options.preferred_width)
            })
            .then_some(column)
    });
    let (mut kept, continuation) = match (table, aligned.flatten()) {
        // An arm's guard is part of the question it answers, not part of the
        // answer, so `when` and what it tests stay on the pattern's line.
        _ if guard => (2, body),
        (Some(kept), _) => (kept, body),
        (None, Some(column)) => (1, column),
        // A call whose last argument is a whole `lambda` still reads as a
        // call, so an argument that fits beside the head stays beside it and
        // only what does not fit goes below.
        (None, None) => {
            let tail = if groups.len() == 1 { closers + 1 } else { 0 };
            let first = head_name.is_some_and(|_| !arm)
                && groups[0].flat().is_some_and(|text| {
                    indent + 1 + head_name.map_or(0, width_of) + 1 + width_of(&text) + tail
                        <= options.preferred_width
                });
            (usize::from(first), body)
        }
    };

    // A signature that does not fit its head line is not squeezed onto one:
    // the parameter list drops to the body indent, where it has the whole
    // width. A condition or a scrutinee is left where it was written, because
    // the head line is the only place either reads as the question being
    // asked.
    if matches!(head_name, Some("fn" | "lambda" | "extern")) {
        while kept > 1 {
            let mut width = indent + 1 + head_name.map_or(0, width_of);
            if kept == groups.len() {
                width += closers + 1;
            }
            let mut measured = true;
            for group in &groups[..kept] {
                match group.flat() {
                    Some(text) => width += 1 + width_of(&text),
                    None => measured = false,
                }
            }
            if !measured || width <= options.preferred_width {
                break;
            }
            kept -= 1;
        }
    }

    let mut open = head_open;
    let last = groups.len() - 1;
    for (index, group) in groups.iter().enumerate() {
        let tail = if index == last && !group.ends_open() {
            closers + 1
        } else {
            0
        };
        if index < kept && !open {
            out.push(' ');
            let column = indent + width_of(out.rsplit('\n').next().unwrap_or(""));
            out.push_str(&group.render(column, options, tail));
        } else {
            out.push('\n');
            if group.blank_before() && index >= kept {
                out.push('\n');
            }
            out.push_str(&" ".repeat(continuation));
            out.push_str(&group.render(continuation, options, tail));
        }
        open = group.ends_open();
    }
    close(&mut out, indent, open, options);
    out
}

/// A closing paren hugs the last argument, unless a comment is what the line
/// ends with — then it goes below, because anything after a `;` is the comment.
fn close(out: &mut String, indent: usize, open: bool, _options: &FormatOptions) {
    if open {
        out.push('\n');
        out.push_str(&" ".repeat(indent));
    }
    out.push(')');
}

fn render_entry(entry: &Entry, indent: usize, options: &FormatOptions) -> String {
    let mut out = render_item(&entry.item, indent, options, 0);
    for comment in &entry.trailing {
        out.push(' ');
        out.push_str(comment);
    }
    out
}

/// Lay a source file out again from the tree the lossless parse built.
///
/// The front half is what makes `fmt --check` a statement about whitespace: a
/// file that does not lex and parse is refused rather than rewritten, so the
/// output always describes the same program as the input.
pub fn format_source(file: &str, source: &str, options: &FormatOptions) -> CompileResult<String> {
    let semantic_tokens = reader::expand(file, &lexer::lex(file, source)?)?;
    parser::parse(file, &semantic_tokens)?;
    let syntax = parse_lossless(source);
    let entries = entries_of(&syntax.root.children, false);
    let mut output = String::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            output.push('\n');
            if entry.blank_before {
                output.push('\n');
            }
        }
        output.push_str(&render_entry(entry, 0, options));
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

    /// Every `.slp` the crate can reach: the bundled library, and the sources
    /// the `compile_fail` suite refuses for a reason other than their syntax.
    fn corpus() -> Vec<(String, String)> {
        let mut sources = Vec::new();
        for package in slopium_std::TOOLCHAIN_PACKAGES {
            for (module, source) in package.modules {
                sources.push((
                    slopium_std::toolchain_source_path(package.name, module),
                    (*source).to_owned(),
                ));
            }
        }
        let refused = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("compile_fail");
        let mut refused: Vec<_> = std::fs::read_dir(refused)
            .expect("the compile-fail suite is in the crate")
            .filter_map(|entry| {
                let path = entry.expect("a directory entry").path();
                (path.extension()? == "slp").then_some(path)
            })
            .collect();
        refused.sort();
        for path in refused {
            let source = std::fs::read_to_string(&path).expect("a refused source");
            let name = path.display().to_string();
            // A case that is refused *for* its syntax has no tree to lay out.
            if format_source(&name, &source, &FormatOptions::default()).is_ok() {
                sources.push((name, source));
            }
        }
        sources
    }

    /// What a layout may not change: the program the reader hands the parser,
    /// and the comments, in the order they were written.
    ///
    /// The tokens are compared *after* the reader expands them, because `fmt`
    /// writes `(& x)` as `&x` and those are one program spelled two ways
    /// (`D-149`). Comparing what was typed would refuse the rewrite; comparing
    /// what the reader makes of it is the assertion that was always meant, and
    /// `a_shape_tells_two_programs_apart` is what keeps it from being blind to
    /// everything else as well.
    fn shape(source: &str) -> (Vec<String>, Vec<String>) {
        let tokens = lexer::lex("shape.slp", source).expect("a corpus source lexes");
        let expanded = reader::expand("shape.slp", &tokens).expect("a corpus source expands");
        let program = expanded
            .iter()
            .map(|token| match &token.kind {
                lexer::TokenKind::LeftParen => "(".to_owned(),
                lexer::TokenKind::RightParen => ")".to_owned(),
                lexer::TokenKind::Atom(text) => text.clone(),
                lexer::TokenKind::String(bytes) => format!("{bytes:?}"),
            })
            .collect();
        let comments = lex_lossless(source)
            .into_iter()
            .filter(|token| token.kind == SyntaxKind::Comment)
            .map(|token| token.text.trim_end().to_owned())
            .collect();
        (program, comments)
    }

    /// `$` is neither written nor removed: which grouping a human meant by one
    /// is not in the tree, and guessing is where a formatter starts having
    /// opinions about structure rather than about layout (`D-150`).
    #[test]
    fn the_layout_neither_writes_a_dollar_nor_removes_one() {
        let options = FormatOptions::default();
        let nested = "(fn main () -> i32\n  (println-i64 $ len $ from-i64 12345)\n  0)\n";
        assert_eq!(format_source("test.slp", nested, &options).unwrap(), nested);
        let written = "(fn main () -> i32\n  (println-i64 (len (from-i64 12345)))\n  0)\n";
        assert_eq!(
            format_source("test.slp", written, &options).unwrap(),
            written
        );
    }

    #[test]
    fn a_dollar_never_ends_a_line() {
        let source = concat!(
            "(fn main () -> i32\n",
            "  (println-i64 $ enormously-long-conversion-name-here $ ",
            "another-long-name 1234567 7654321)\n",
            "  0)\n",
        );
        let formatted = format_source("test.slp", source, &FormatOptions::default()).unwrap();
        assert!(
            formatted
                .lines()
                .all(|line| !line.trim_end().ends_with('$')),
            "{formatted}"
        );
    }

    #[test]
    fn a_shape_tells_two_programs_apart() {
        let source = "(fn main () -> i32 (f x))\n";
        assert_ne!(shape(source), shape("(fn main () -> i32 (f y))\n"));
        assert_ne!(shape(source), shape("(fn main () -> i32 (f (g x)))\n"));
        assert_ne!(shape(source), shape("(fn main () -> i32 (f x)) ; note\n"));
        assert_ne!(shape(source), shape("(fn main () -> i32 (f \"x\"))\n"));
        // The one difference it is deliberately blind to, and the whole reason
        // it reads the expansion rather than the text.
        assert_eq!(
            shape("(fn main () -> i32 (f (& x)))\n"),
            shape("(fn main () -> i32 (f &x))\n")
        );
    }

    /// Left-trim every line: a different source, the same program, and no
    /// comment moved off the line it ends. A layout that reads the tree rather
    /// than the whitespace answers both with the same bytes.
    fn crushed(source: &str) -> Option<String> {
        if lex_lossless(source)
            .iter()
            .any(|token| token.kind == SyntaxKind::String && token.text.contains('\n'))
        {
            return None;
        }
        let mut out: String = source
            .lines()
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        out.push('\n');
        Some(out)
    }

    #[test]
    fn layout_is_a_function_of_the_tree_and_not_of_the_whitespace() {
        for (path, source) in corpus() {
            let Some(crushed) = crushed(&source) else {
                continue;
            };
            let options = FormatOptions::default();
            let once = format_source(&path, &crushed, &options).expect("a corpus source parses");
            let twice = format_source(&path, &once, &options).expect("formatted source parses");
            assert_eq!(once, twice, "`{path}` is not laid out idempotently");
            assert_eq!(
                once,
                format_source(&path, &source, &options).expect("a corpus source parses"),
                "`{path}` is laid out differently once its indentation is taken away"
            );
        }
    }

    #[test]
    fn a_layout_changes_nothing_but_the_whitespace() {
        for (path, source) in corpus() {
            let formatted = format_source(&path, &source, &FormatOptions::default())
                .expect("a corpus source parses");
            assert_eq!(
                shape(&formatted),
                shape(&source),
                "`{path}` came back a different program"
            );
            assert!(
                formatted.lines().all(|line| line.chars().count()
                    <= FormatOptions::default().preferred_width
                    || !line.contains(' ')),
                "`{path}` has a line past the preferred width with somewhere to break"
            );
        }
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
