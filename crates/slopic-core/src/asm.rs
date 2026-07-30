//! The object interface: what a backend hands to the object writer.
//!
//! A backend used to produce assembly text and hand it to `as`. It now
//! produces a sequence of [`Item`]s — sections, labels, symbol attributes,
//! data, and instructions — and that one sequence is *both* rendered as the
//! same assembly text as before *and* encoded into a relocatable ELF object.
//!
//! One stream with two readings is the point. Text and machine code cannot
//! drift apart, because there is no second description of the program for them
//! to drift between (`D-025`). It is also why the object writer is not an
//! assembler: it never parses anything, and a backend cannot hand it a form it
//! has no encoding for without failing to compile.
//!
//! What lives here is everything neither architecture gets to decide on its
//! own: section identity, label scope, symbol binding, relocation bookkeeping,
//! and the layout pass that turns labels into addresses. What does not live
//! here is a single byte of instruction encoding.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// The sections a backend can emit into.
///
/// Deliberately closed. A backend that wants another one has to say so here,
/// where the ELF writer can see it, rather than by naming a string.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Section {
    Text,
    RoData,
    /// The empty marker that tells the linker this object does not want an
    /// executable stack. It holds no data and never will.
    GnuStack,
}

impl Section {
    /// The directive that opens this section, as the assembler spells it.
    fn directive(self) -> &'static str {
        match self {
            Section::Text => ".text",
            Section::RoData => ".section .rodata",
            Section::GnuStack => ".section .note.GNU-stack,\"\",@progbits",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Section::Text => ".text",
            Section::RoData => ".rodata",
            Section::GnuStack => ".note.GNU-stack",
        }
    }
}

/// A place a fixup can point at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Target {
    /// A name: a local label such as `.Lsl_fn_main_bb0`, a symbol this object
    /// defines, or one it expects the linker to find.
    Named(String),
    /// A reusable numeric label, referenced forward as `1f`. Assembly has
    /// scoped these since long before this compiler, and both backends use
    /// them for the two-instruction detours inside a single helper.
    Forward(u32),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Named(name) => f.write_str(name),
            Target::Forward(id) => write!(f, "{id}f"),
        }
    }
}

/// How the bytes at a fixup site relate to the address they refer to.
///
/// One variant per encoding, not per relocation: whether it ends up resolved
/// in place or handed to the linker is decided by [`Assembly::finish`], not by
/// the backend that recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixupKind {
    /// x86-64: a 32-bit displacement from the end of the instruction.
    Pc32,
    /// x86-64: the same displacement for a call, which the linker may route
    /// through a PLT entry.
    Plt32,
    /// AArch64: the 26-bit word displacement of `bl`.
    Call26,
    /// AArch64: the 26-bit word displacement of `b`.
    Jump26,
    /// AArch64: the 19-bit word displacement of `b.cond`, `cbz` and `cbnz`.
    CondBr19,
    /// AArch64: the 21-bit page displacement of `adrp`.
    AdrPage21,
    /// AArch64: the low 12 bits of an address, as an `add` immediate.
    AddLo12,
}

impl FixupKind {
    /// Whether a reference of this kind to a label in the same section can be
    /// worked out here.
    ///
    /// A displacement between two things in one section is known once both
    /// have addresses. A page number or a low-order slice of an address is
    /// not: it depends on where the linker puts the section.
    fn resolvable_in_place(self) -> bool {
        matches!(
            self,
            FixupKind::Pc32 | FixupKind::Call26 | FixupKind::Jump26 | FixupKind::CondBr19
        )
    }
}

/// A reference whose value is not known when the instruction is encoded.
#[derive(Clone, Debug)]
pub struct Fixup {
    /// Offset within [`Section::Text`] of the field to patch.
    pub offset: u64,
    pub kind: FixupKind,
    pub target: Target,
    pub addend: i64,
}

/// The buffer an instruction encodes itself into.
///
/// Instructions only ever land in `.text`, so there is no section to choose.
#[derive(Debug, Default)]
pub struct Code {
    bytes: Vec<u8>,
    fixups: Vec<Fixup>,
}

impl Code {
    /// The offset the next byte will be written at.
    pub fn here(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn byte(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    pub fn extend(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn word(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Records that the field at `offset` refers to `target`.
    pub fn relocate(&mut self, offset: u64, kind: FixupKind, target: Target, addend: i64) {
        self.fixups.push(Fixup {
            offset,
            kind,
            target,
            addend,
        });
    }
}

/// An instruction of some architecture.
///
/// The three things the shared half needs from one: how it prints, how it
/// encodes, and — for the peephole both backends share — whether it is a plain
/// copy, expressed as the copy that would put things back.
pub trait Instruction: fmt::Display + PartialEq + Sized {
    /// Appends this instruction's machine code, recording any fixups it needs.
    ///
    /// The error is for a value that cannot be encoded at all — an immediate
    /// too wide for its field, say. It is an internal compiler error, not a
    /// user one: instruction selection is supposed to have ruled it out.
    fn encode(&self, code: &mut Code) -> Result<(), String>;

    /// The instruction that would undo this one, when it is a plain copy.
    ///
    /// `mov a, b` undoes `mov b, a`, so a copy immediately following its own
    /// mirror is dead. Returning `None` means "not a copy", which is the
    /// answer for everything that touches memory, sets flags, or has a side
    /// effect the peephole would otherwise delete.
    fn undo(&self) -> Option<Self>;
}

/// One emitted thing, in the order it was emitted.
#[derive(Clone, Debug, PartialEq)]
pub enum Item<I> {
    /// Everything after this point belongs to `.0`.
    Section(Section),
    /// A named label at the current position.
    Label(String),
    /// A reusable numeric label at the current position.
    Numeric(u32),
    /// `.globl name`
    Global(String),
    /// `.type name, @function`
    Function(String),
    /// `.size name, .-name`, closing the symbol opened by `name:`.
    Size(String),
    /// Raw bytes in the current section.
    Bytes(Vec<u8>),
    /// `.file index "path"`
    File {
        index: usize,
        path: String,
    },
    /// `.loc file line column`
    Loc {
        file: usize,
        line: usize,
        column: usize,
    },
    Instruction(I),
    /// A directive that changes how the assembler *reads* the text and says
    /// nothing about the program. `.intel_syntax noprefix` is the only one.
    Syntax(&'static str),
}

impl<I> Item<I> {
    /// Whether this item only attributes the instructions around it.
    ///
    /// Debug information is not allowed to change which instructions are
    /// emitted (`D-024`), so the peephole has to be able to look past it.
    fn is_location(&self) -> bool {
        matches!(self, Item::Loc { .. })
    }
}

/// The stream a backend builds.
#[derive(Debug)]
pub struct Assembly<I> {
    items: Vec<Item<I>>,
}

impl<I> Default for Assembly<I> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<I: Instruction> Assembly<I> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, item: Item<I>) {
        self.items.push(item);
    }

    pub fn instruction(&mut self, instruction: I) {
        self.items.push(Item::Instruction(instruction));
    }

    pub fn items(&self) -> &[Item<I>] {
        &self.items
    }

    /// Deletes a copy that puts a value straight back where it came from.
    ///
    /// Instruction selection is per-MIR-statement, so a result written to a
    /// register and immediately read again — the shape of `let x = ...`
    /// followed by `return x` — leaves a copy that undoes itself. Only
    /// adjacent instructions are considered, so a label or anything else in
    /// between blocks the rewrite, and a copy sets no flags, so removing one
    /// cannot change what a following branch sees.
    pub fn remove_redundant_copies(&mut self) {
        let mut kept: Vec<Item<I>> = Vec::with_capacity(self.items.len());
        for item in self.items.drain(..) {
            let undoes_previous = match &item {
                Item::Instruction(current) => kept
                    .iter()
                    .rev()
                    .find(|kept| !kept.is_location())
                    .and_then(|previous| match previous {
                        Item::Instruction(previous) => previous.undo(),
                        _ => None,
                    })
                    .is_some_and(|undo| &undo == current),
                _ => false,
            };
            if !undoes_previous {
                kept.push(item);
            }
        }
        self.items = kept;
    }

    /// Renders the stream as assembly text, exactly as it was written before
    /// there was anything else to render it as.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for item in &self.items {
            match item {
                Item::Section(section) => text.push_str(section.directive()),
                Item::Label(label) => {
                    text.push_str(label);
                    text.push(':');
                }
                Item::Numeric(id) => {
                    text.push_str(&id.to_string());
                    text.push(':');
                }
                Item::Global(name) => {
                    text.push_str(".globl ");
                    text.push_str(name);
                }
                Item::Function(name) => {
                    text.push_str(".type ");
                    text.push_str(name);
                    text.push_str(", @function");
                }
                Item::Size(name) => {
                    text.push_str(&format!(".size {name}, .-{name}"));
                }
                Item::Bytes(bytes) => {
                    text.push_str("  .byte ");
                    for (index, byte) in bytes.iter().enumerate() {
                        if index != 0 {
                            text.push_str(", ");
                        }
                        text.push_str(&byte.to_string());
                    }
                }
                Item::File { index, path } => {
                    text.push_str(&format!(".file {index} \"{}\"", quoted(path)));
                }
                Item::Loc { file, line, column } => {
                    text.push_str(&format!("  .loc {file} {line} {column}"));
                }
                Item::Instruction(instruction) => {
                    text.push_str("  ");
                    text.push_str(&instruction.to_string());
                }
                Item::Syntax(directive) => text.push_str(directive),
            }
            text.push('\n');
        }
        text
    }

    /// Lays the stream out: assigns every label an address, resolves what can
    /// be resolved, and leaves the rest for the linker.
    pub fn finish(&self) -> Result<Object, String> {
        let mut object = Layout::default().run(self)?;
        object.resolve()?;
        Ok(object)
    }
}

/// A symbol this object defines or expects.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    /// `None` for a symbol the linker has to find elsewhere.
    pub definition: Option<Definition>,
}

#[derive(Clone, Copy, Debug)]
pub struct Definition {
    pub section: Section,
    pub offset: u64,
    pub size: u64,
}

/// A laid-out object: bytes, symbols, and the references left for the linker.
#[derive(Debug, Default)]
pub struct Object {
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    /// Global symbols, defined and undefined, in the order they were named.
    pub symbols: Vec<Symbol>,
    /// References into `.text` the linker still has to fill in.
    pub relocations: Vec<Relocation>,
    /// The symbols declared `@function`, which is every symbol these backends
    /// define, but the object writer should not have to assume that.
    pub functions: HashSet<String>,
    /// Where every label landed, kept for tests and diagnostics.
    labels: HashMap<String, (Section, u64)>,
    fixups: Vec<Fixup>,
    forward: Vec<(u32, u64)>,
}

/// A reference the linker has to fill in.
#[derive(Clone, Debug)]
pub struct Relocation {
    pub offset: u64,
    pub kind: FixupKind,
    /// What the reference names: a symbol by index into [`Object::symbols`],
    /// or a section, for a local label the linker has no name for.
    pub against: Against,
    pub addend: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Against {
    Symbol(usize),
    Section(Section),
}

#[derive(Default)]
struct Layout {
    object: Object,
    section: Option<Section>,
    globals: Vec<String>,
    open: HashMap<String, u64>,
}

impl Layout {
    fn run<I: Instruction>(mut self, assembly: &Assembly<I>) -> Result<Object, String> {
        let mut code = Code::default();
        for item in assembly.items() {
            match item {
                Item::Section(section) => self.section = Some(*section),
                Item::Label(label) => {
                    let (section, offset) = self.position(&code)?;
                    self.object.labels.insert(label.clone(), (section, offset));
                    self.open.insert(label.clone(), offset);
                }
                Item::Numeric(id) => {
                    let (section, offset) = self.position(&code)?;
                    if section != Section::Text {
                        return Err(format!("numeric label {id} outside .text"));
                    }
                    self.object.forward.push((*id, offset));
                }
                Item::Global(name) => self.globals.push(name.clone()),
                Item::Function(name) => {
                    self.object.functions.insert(name.clone());
                }
                Item::Size(name) => {
                    let (section, end) = self.position(&code)?;
                    let start = self
                        .open
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("`.size {name}` before `{name}:`"))?;
                    self.object.symbols.push(Symbol {
                        name: name.clone(),
                        definition: Some(Definition {
                            section,
                            offset: start,
                            size: end - start,
                        }),
                    });
                }
                Item::Bytes(bytes) => match self.section {
                    Some(Section::RoData) => self.object.rodata.extend_from_slice(bytes),
                    Some(Section::Text) => code.extend(bytes),
                    _ => return Err("data emitted outside a section that holds any".into()),
                },
                // Line tables are the assembler's to build from these, and the
                // object writer does not build them (`D-028`).
                Item::File { .. } | Item::Loc { .. } | Item::Syntax(_) => {}
                Item::Instruction(instruction) => {
                    if self.section != Some(Section::Text) {
                        return Err("instruction outside .text".into());
                    }
                    instruction
                        .encode(&mut code)
                        .map_err(|error| format!("cannot encode `{instruction}`: {error}"))?;
                }
            }
        }
        self.object.text = code.bytes;
        self.object.fixups = code.fixups;
        // A `.globl` with no `.size` still has to reach the symbol table.
        for name in &self.globals {
            if self
                .object
                .symbols
                .iter()
                .any(|symbol| &symbol.name == name)
            {
                continue;
            }
            let definition = self
                .object
                .labels
                .get(name)
                .map(|(section, offset)| Definition {
                    section: *section,
                    offset: *offset,
                    size: 0,
                });
            self.object.symbols.push(Symbol {
                name: name.clone(),
                definition,
            });
        }
        Ok(self.object)
    }

    fn position(&self, code: &Code) -> Result<(Section, u64), String> {
        match self.section {
            Some(Section::Text) => Ok((Section::Text, code.here())),
            Some(Section::RoData) => Ok((Section::RoData, self.object.rodata.len() as u64)),
            Some(Section::GnuStack) => Err("nothing can be placed in .note.GNU-stack".into()),
            None => Err("nothing can be placed before the first section".into()),
        }
    }
}

impl Object {
    /// The offset a label landed at, for tests.
    pub fn label(&self, name: &str) -> Option<(Section, u64)> {
        self.labels.get(name).copied()
    }

    /// Turns fixups into either patched bytes or relocations.
    fn resolve(&mut self) -> Result<(), String> {
        let fixups = std::mem::take(&mut self.fixups);
        for fixup in fixups {
            let place = match &fixup.target {
                Target::Forward(id) => {
                    let offset = self
                        .forward
                        .iter()
                        .find(|(candidate, at)| candidate == id && *at > fixup.offset)
                        .map(|(_, at)| *at)
                        .ok_or_else(|| format!("no `{id}:` after the reference to `{id}f`"))?;
                    Some((Section::Text, offset))
                }
                Target::Named(name) => self.labels.get(name).copied(),
            };
            match place {
                // A displacement inside one section is arithmetic we can do.
                Some((Section::Text, offset))
                    if fixup.kind.resolvable_in_place() && is_local(&fixup.target) =>
                {
                    // The same arithmetic a linker would do: the target,
                    // plus the addend that says where the field is measured
                    // from, less the address of the field itself.
                    let displacement = offset as i64 + fixup.addend - fixup.offset as i64;
                    patch(&mut self.text, &fixup, displacement)?;
                }
                // A local label the linker has no name for is reached through
                // the section it sits in, offset and all.
                Some((section, offset)) if is_local(&fixup.target) => {
                    self.relocations.push(Relocation {
                        offset: fixup.offset,
                        kind: fixup.kind,
                        against: Against::Section(section),
                        addend: fixup.addend + offset as i64,
                    });
                }
                // Anything with a name of its own goes to the linker under it,
                // whether or not this object also defines it: a global may be
                // replaced at link time, and resolving it here would quietly
                // rule that out.
                _ => {
                    let Target::Named(name) = &fixup.target else {
                        return Err(format!("`{}` is not a defined label", fixup.target));
                    };
                    let index = self.symbol_index(name);
                    self.relocations.push(Relocation {
                        offset: fixup.offset,
                        kind: fixup.kind,
                        against: Against::Symbol(index),
                        addend: fixup.addend,
                    });
                }
            }
        }
        Ok(())
    }

    fn symbol_index(&mut self, name: &str) -> usize {
        if let Some(index) = self.symbols.iter().position(|symbol| symbol.name == name) {
            return index;
        }
        self.symbols.push(Symbol {
            name: name.to_owned(),
            definition: None,
        });
        self.symbols.len() - 1
    }
}

/// A path as the body of an assembler string literal.
///
/// Only the two characters that would end or continue the literal need
/// escaping; a path is bytes and the assembler passes the rest through.
fn quoted(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Whether a target is invisible to the linker.
///
/// `.L` is the assembler's own convention for a label that does not reach the
/// symbol table, and a numeric label never had a name to begin with.
fn is_local(target: &Target) -> bool {
    match target {
        Target::Forward(_) => true,
        Target::Named(name) => name.starts_with(".L"),
    }
}

/// Writes a resolved displacement into the field it belongs to.
fn patch(bytes: &mut [u8], fixup: &Fixup, value: i64) -> Result<(), String> {
    let at = fixup.offset as usize;
    let field = |width: usize| -> Result<std::ops::Range<usize>, String> {
        if at + width > bytes.len() {
            return Err(format!("fixup at {at} runs past the section"));
        }
        Ok(at..at + width)
    };
    match fixup.kind {
        FixupKind::Pc32 | FixupKind::Plt32 => {
            let displacement = i32::try_from(value)
                .map_err(|_| format!("displacement {value} does not fit in 32 bits"))?;
            let range = field(4)?;
            bytes[range].copy_from_slice(&displacement.to_le_bytes());
        }
        FixupKind::Call26 | FixupKind::Jump26 => {
            let words = word_displacement(value, 26)?;
            let range = field(4)?;
            let instruction = u32::from_le_bytes(bytes[range.clone()].try_into().unwrap());
            let patched = (instruction & !0x03ff_ffff) | (words & 0x03ff_ffff);
            bytes[range].copy_from_slice(&patched.to_le_bytes());
        }
        FixupKind::CondBr19 => {
            let words = word_displacement(value, 19)?;
            let range = field(4)?;
            let instruction = u32::from_le_bytes(bytes[range.clone()].try_into().unwrap());
            let patched = (instruction & !(0x0007_ffff << 5)) | ((words & 0x0007_ffff) << 5);
            bytes[range].copy_from_slice(&patched.to_le_bytes());
        }
        FixupKind::AdrPage21 | FixupKind::AddLo12 => {
            return Err("a page or low-order address field is the linker's to fill".into());
        }
    }
    Ok(())
}

/// A byte displacement as a signed count of instruction words, checked against
/// the field it has to fit in.
fn word_displacement(value: i64, bits: u32) -> Result<u32, String> {
    if value % 4 != 0 {
        return Err(format!("displacement {value} is not a whole instruction"));
    }
    let words = value / 4;
    let limit = 1i64 << (bits - 1);
    if words < -limit || words >= limit {
        return Err(format!(
            "displacement {value} is out of range for a {bits}-bit branch"
        ));
    }
    Ok(words as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in architecture: two instructions, one of which is a copy.
    #[derive(Clone, Debug, PartialEq)]
    enum Toy {
        Copy(u8, u8),
        Jump(Target),
        Call(String),
    }

    impl fmt::Display for Toy {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Toy::Copy(dst, src) => write!(f, "copy r{dst}, r{src}"),
                Toy::Jump(target) => write!(f, "jump {target}"),
                Toy::Call(name) => write!(f, "call {name}"),
            }
        }
    }

    impl Instruction for Toy {
        fn encode(&self, code: &mut Code) -> Result<(), String> {
            match self {
                Toy::Copy(dst, src) => {
                    code.byte(0x01);
                    code.byte(*dst);
                    code.byte(*src);
                    code.byte(0x00);
                }
                Toy::Jump(target) => {
                    let at = code.here();
                    code.word(0x0200_0000);
                    code.relocate(at, FixupKind::Jump26, target.clone(), 0);
                }
                Toy::Call(name) => {
                    let at = code.here();
                    code.word(0x0300_0000);
                    code.relocate(at, FixupKind::Call26, Target::Named(name.clone()), 0);
                }
            }
            Ok(())
        }

        fn undo(&self) -> Option<Self> {
            match self {
                Toy::Copy(dst, src) => Some(Toy::Copy(*src, *dst)),
                _ => None,
            }
        }
    }

    fn program(items: Vec<Item<Toy>>) -> Assembly<Toy> {
        let mut assembly = Assembly::new();
        for item in items {
            assembly.push(item);
        }
        assembly
    }

    #[test]
    fn text_is_rendered_the_way_the_assembler_reads_it() {
        let assembly = program(vec![
            Item::Syntax(".intel_syntax noprefix"),
            Item::Section(Section::RoData),
            Item::Label(".Lstr".into()),
            Item::Bytes(vec![104, 105, 0]),
            Item::Section(Section::Text),
            Item::Global("main".into()),
            Item::Function("main".into()),
            Item::Label("main".into()),
            Item::Loc {
                file: 1,
                line: 2,
                column: 3,
            },
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Size("main".into()),
            Item::Section(Section::GnuStack),
        ]);
        assert_eq!(
            assembly.to_text(),
            concat!(
                ".intel_syntax noprefix\n",
                ".section .rodata\n",
                ".Lstr:\n",
                "  .byte 104, 105, 0\n",
                ".text\n",
                ".globl main\n",
                ".type main, @function\n",
                "main:\n",
                "  .loc 1 2 3\n",
                "  copy r0, r1\n",
                ".size main, .-main\n",
                ".section .note.GNU-stack,\"\",@progbits\n",
            )
        );
    }

    #[test]
    fn a_copy_that_undoes_the_one_before_it_is_deleted() {
        let mut assembly = program(vec![
            Item::Section(Section::Text),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Instruction(Toy::Copy(1, 0)),
        ]);
        assembly.remove_redundant_copies();
        assert_eq!(assembly.items().len(), 2);
    }

    #[test]
    fn debug_information_does_not_hide_a_mirrored_copy() {
        // `D-024`: adding `.loc` may not change which instructions are
        // emitted, so the peephole has to look past one.
        let mut assembly = program(vec![
            Item::Section(Section::Text),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Loc {
                file: 1,
                line: 9,
                column: 1,
            },
            Item::Instruction(Toy::Copy(1, 0)),
        ]);
        assembly.remove_redundant_copies();
        let instructions = assembly
            .items()
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        assert_eq!(instructions, 1);
    }

    #[test]
    fn a_label_between_two_copies_blocks_the_rewrite() {
        let mut assembly = program(vec![
            Item::Section(Section::Text),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Label(".Lhere".into()),
            Item::Instruction(Toy::Copy(1, 0)),
        ]);
        assembly.remove_redundant_copies();
        let instructions = assembly
            .items()
            .iter()
            .filter(|item| matches!(item, Item::Instruction(_)))
            .count();
        assert_eq!(instructions, 2);
    }

    #[test]
    fn a_branch_inside_the_section_needs_no_linker() {
        let assembly = program(vec![
            Item::Section(Section::Text),
            Item::Instruction(Toy::Jump(Target::Named(".Lend".into()))),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Label(".Lend".into()),
        ]);
        let object = assembly.finish().unwrap();
        assert!(object.relocations.is_empty());
        assert_eq!(object.label(".Lend"), Some((Section::Text, 8)));
        let encoded = u32::from_le_bytes(object.text[0..4].try_into().unwrap());
        assert_eq!(encoded & 0x03ff_ffff, 2, "two words forward");
    }

    #[test]
    fn a_forward_numeric_label_is_the_next_one_and_not_an_earlier_one() {
        let assembly = program(vec![
            Item::Section(Section::Text),
            Item::Numeric(1),
            Item::Instruction(Toy::Jump(Target::Forward(1))),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Numeric(1),
        ]);
        let object = assembly.finish().unwrap();
        let encoded = u32::from_le_bytes(object.text[0..4].try_into().unwrap());
        assert_eq!(
            encoded & 0x03ff_ffff,
            2,
            "the `1:` that follows, not the one before"
        );
    }

    #[test]
    fn a_call_to_a_name_is_left_to_the_linker() {
        let assembly = program(vec![
            Item::Section(Section::Text),
            Item::Instruction(Toy::Call("sl_rt_alloc".into())),
        ]);
        let object = assembly.finish().unwrap();
        assert_eq!(object.relocations.len(), 1);
        let index = match object.relocations[0].against {
            Against::Symbol(index) => index,
            other => panic!("expected a symbol, got {other:?}"),
        };
        assert_eq!(object.symbols[index].name, "sl_rt_alloc");
        assert!(object.symbols[index].definition.is_none());
    }

    #[test]
    fn a_defined_global_is_still_the_linkers_to_bind() {
        // It may be replaced at link time. Resolving the call here would
        // quietly rule that out, and would differ from what `as` produces.
        let assembly = program(vec![
            Item::Section(Section::Text),
            Item::Global("helper".into()),
            Item::Label("helper".into()),
            Item::Instruction(Toy::Copy(0, 0)),
            Item::Size("helper".into()),
            Item::Label("caller".into()),
            Item::Instruction(Toy::Call("helper".into())),
        ]);
        let object = assembly.finish().unwrap();
        assert_eq!(object.relocations.len(), 1);
        assert_eq!(object.relocations[0].against, Against::Symbol(0));
        let helper = &object.symbols[0];
        assert_eq!(helper.definition.unwrap().size, 4);
    }

    #[test]
    fn a_local_label_in_another_section_is_reached_through_that_section() {
        let assembly = program(vec![
            Item::Section(Section::RoData),
            Item::Bytes(vec![0; 8]),
            Item::Label(".Lstr".into()),
            Item::Bytes(vec![104, 105, 0]),
            Item::Section(Section::Text),
            Item::Instruction(Toy::Jump(Target::Named(".Lstr".into()))),
        ]);
        let object = assembly.finish().unwrap();
        assert_eq!(object.relocations.len(), 1);
        assert_eq!(
            object.relocations[0].against,
            Against::Section(Section::RoData)
        );
        assert_eq!(object.relocations[0].addend, 8, "the label's own offset");
    }

    #[test]
    fn a_size_directive_measures_from_its_own_label() {
        let assembly = program(vec![
            Item::Section(Section::Text),
            Item::Global("f".into()),
            Item::Label("f".into()),
            Item::Instruction(Toy::Copy(0, 1)),
            Item::Instruction(Toy::Copy(1, 2)),
            Item::Size("f".into()),
        ]);
        let object = assembly.finish().unwrap();
        assert_eq!(object.symbols.len(), 1);
        let definition = object.symbols[0].definition.unwrap();
        assert_eq!((definition.offset, definition.size), (0, 8));
    }

    #[test]
    fn a_branch_beyond_its_field_is_an_error_and_not_a_wrong_address() {
        // Building a `.text` that large to prove it would cost a gigabyte of
        // test memory; the field check is where the answer comes from.
        let limit = 1i64 << 25;
        assert!(word_displacement(4 * (limit - 1), 26).is_ok());
        let error = word_displacement(4 * limit, 26).unwrap_err();
        assert!(error.contains("out of range"), "{error}");
        assert!(word_displacement(4 * -limit, 26).is_ok());
        assert!(word_displacement(4 * (-limit - 1), 26).is_err());
        let misaligned = word_displacement(6, 26).unwrap_err();
        assert!(misaligned.contains("whole instruction"), "{misaligned}");
    }
}
