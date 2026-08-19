use crate::ast::Type;
use crate::diagnostic::Span;
use crate::sema::{
    BindingId, TCapture, TExpr, TExprKind, TMatchArm, TPattern, TypedFunction, TypedParam,
    TypedProgram, TypedTest,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub type LocalId = usize;
pub type BlockId = usize;

#[derive(Clone, Debug, Serialize)]
pub struct MirModule {
    pub functions: Vec<MirFunction>,
    /// The C functions this module calls. A call to one is an ordinary
    /// `Instruction::Call`; what this table decides is that the symbol is not
    /// mangled and the arguments are expanded rather than passed whole
    /// (`D-073`).
    pub externs: Vec<MirExtern>,
    pub tests: Vec<MirTest>,
    pub structs: Vec<MirStruct>,
    pub enums: Vec<MirEnum>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirExtern {
    pub name: String,
    pub symbol: String,
    pub params: Vec<Type>,
    pub result: Type,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirStruct {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub emit: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirEnum {
    pub name: String,
    pub variants: Vec<MirVariant>,
    pub emit: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirVariant {
    pub name: String,
    pub tag: usize,
    pub fields: Vec<(String, Type)>,
}
#[derive(Clone, Debug, Serialize)]
pub struct MirTest {
    pub name: String,
    pub function: MirFunction,
    pub emit: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirFunction {
    pub name: String,
    pub emit: bool,
    /// The `inline` annotation, carried to the optimizer and nowhere else
    /// (`D-122`).
    pub inline_hint: bool,
    pub params: Vec<LocalId>,
    pub return_type: Type,
    pub locals: Vec<MirLocal>,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct MirLocal {
    pub ty: Type,
    pub name: Option<String>,
    pub is_param: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
    /// Source location of the construct that produced the terminator.
    pub terminator_span: Span,
}

impl BasicBlock {
    /// A block whose instructions have no source location.
    ///
    /// For passes and tests that synthesise blocks; lowering goes through
    /// `Builder::emit`, which attaches real spans.
    pub fn synthetic(instructions: Vec<Instruction>, terminator: Terminator) -> Self {
        Self {
            statements: instructions
                .into_iter()
                .map(|instruction| Statement {
                    instruction,
                    span: Span::default(),
                })
                .collect(),
            terminator,
            terminator_span: Span::default(),
        }
    }

    /// The instructions in this block, without their source locations.
    pub fn instructions(&self) -> impl Iterator<Item = &Instruction> {
        self.statements
            .iter()
            .map(|statement| &statement.instruction)
    }
}

/// One instruction together with the source it came from.
///
/// Code generation turns these spans into `.loc` directives, and the assembler
/// turns those into the DWARF line table, so a span that drifts moves a
/// breakpoint. In a package build the offsets are into the merged virtual
/// source while `line` and `column` stay file-local; `diagnostic::SourceMap`
/// resolves the first back to a file.
#[derive(Clone, Debug, Serialize)]
pub struct Statement {
    pub instruction: Instruction,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum Instruction {
    ConstInt {
        dst: LocalId,
        value: i64,
    },
    ConstFloat {
        dst: LocalId,
        bits: u64,
    },
    ConstBool {
        dst: LocalId,
        value: bool,
    },
    StringNew {
        dst: LocalId,
        /// The literal's bytes. Any byte: a Slopium `String` is a length and a
        /// buffer (`D-079`), and `\xNN` writes one of them (`D-106`).
        #[serde(serialize_with = "escaped_bytes")]
        value: Vec<u8>,
    },
    Assign {
        dst: LocalId,
        src: LocalId,
    },
    AddressOf {
        dst: LocalId,
        src: LocalId,
    },
    Binary {
        dst: LocalId,
        op: BinaryOp,
        lhs: LocalId,
        rhs: LocalId,
        ty: Type,
    },
    Call {
        dst: LocalId,
        callee: String,
        args: Vec<LocalId>,
        arg_types: Vec<Type>,
        result: Type,
    },
    /// The address of a top-level function, as a value (`D-092`).
    ///
    /// The symbol is already resolved through `lowering::call_symbol`, so both
    /// backends materialise it the same way they already do for the drop and
    /// clone helpers a `List` carries.
    FnAddr {
        dst: LocalId,
        symbol: String,
    },
    /// A call through a local holding a function address.
    ///
    /// Identical to `Call` except that the callee is a local rather than a
    /// name: there is no symbol until run time, so `arg_types` and `result`
    /// are the only description of the shape and the verifier checks them
    /// against the callee local's `Fn` type.
    CallValue {
        dst: LocalId,
        callee: LocalId,
        args: Vec<LocalId>,
        arg_types: Vec<Type>,
        result: Type,
    },
    Drop {
        local: LocalId,
        ty: Type,
    },
    StructNew {
        dst: LocalId,
        name: String,
        fields: Vec<LocalId>,
    },
    FieldLoad {
        dst: LocalId,
        base: LocalId,
        index: usize,
    },
    /// `dst = *src`, one machine word.
    ///
    /// The dereference the language spells `clone` (`D-100`). Only ever emitted
    /// for a borrow of something that is *not* pointer-like, because borrowing
    /// a pointer-shaped value copies the pointer and there is nothing to load.
    Load {
        dst: LocalId,
        src: LocalId,
    },
    /// `dst = base + index * 8`, the address of a struct field.
    ///
    /// What a pattern binds for a non-pointer-like field when the scrutinee is a
    /// borrow (`D-099`); the pointer-like case is a `FieldLoad`, because there
    /// the word in the slot already *is* the borrow.
    FieldAddr {
        dst: LocalId,
        base: LocalId,
        index: usize,
    },
    EnumNew {
        dst: LocalId,
        enum_name: String,
        tag: usize,
        fields: Vec<LocalId>,
    },
    EnumTag {
        dst: LocalId,
        base: LocalId,
    },
    EnumFieldLoad {
        dst: LocalId,
        base: LocalId,
        index: usize,
    },
    /// `dst = base + (index + 1) * 8`, the address of an enum payload slot.
    ///
    /// `FieldAddr`'s twin, off by the word the tag occupies — the same offset
    /// `EnumFieldLoad` reads through.
    EnumFieldAddr {
        dst: LocalId,
        base: LocalId,
        index: usize,
    },
    /// `base.index = src`, one machine word into a struct field (`D-120`).
    ///
    /// `FieldLoad`'s twin, and the whole of what assigning to a field is. The
    /// field is named rather than addressed because the name came from a
    /// pattern this function wrote, so the place is known here and a borrow
    /// keeps the representation it always had.
    ///
    /// A field is a machine word whatever it holds, so there is no width
    /// question and nothing here to canonicalise: the narrow memory in this
    /// compiler is still `VolatileLoad` and `VolatileStore` alone (`D-067`).
    FieldStore {
        base: LocalId,
        index: usize,
        src: LocalId,
    },
    /// `base.(index + 1) = src`, one machine word into an enum payload slot.
    ///
    /// `FieldStore` off by the word the tag occupies, exactly as
    /// `EnumFieldAddr` is `FieldAddr`.
    EnumFieldStore {
        base: LocalId,
        index: usize,
        src: LocalId,
    },
    Free {
        local: LocalId,
    },
    /// `dst = *(ty *) addr`, one volatile access of `ty`'s width (`D-067`).
    ///
    /// The narrowest memory this compiler touches. Everything else it loads is
    /// a machine word — a struct field, an enum payload, a borrow read — and
    /// this is the only place a byte or a half is read, because a device
    /// register is the only thing that has a width the program did not choose.
    ///
    /// The result is canonical in its word like any other integer (`D-113`),
    /// which for an unsigned type the zero-extending load already achieves.
    ///
    /// Neither backend may fold two of these, drop one whose result is unused,
    /// or move one past another. `opt::is_pure` says the first two and
    /// `opt::volatile_count` checks them; see `D-114` for why the third is
    /// where it is rather than in `verify.rs`.
    VolatileLoad {
        dst: LocalId,
        addr: LocalId,
        ty: Type,
    },
    /// `*(ty *) addr = src`, one volatile access of `ty`'s width (`D-067`).
    ///
    /// The first MIR instruction that writes to memory at all: every other
    /// store in this compiler is baked into `StructNew` and `EnumNew` inside
    /// the backends, or happens behind a runtime call.
    ///
    /// The value is canonical by invariant, so a narrow store truncates and
    /// does not check — truncating is what storing through a narrow pointer
    /// means.
    VolatileStore {
        addr: LocalId,
        src: LocalId,
        ty: Type,
    },
}

/// Writes a literal's bytes as an escaped string rather than a list of
/// numbers.
///
/// `--emit mir` is a debugging aid over an internal protocol (`D-002`), and an
/// array of 116 numbers where a program has `"hello"` is not one.
fn escaped_bytes<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&escape_bytes(bytes))
}

/// A literal's bytes, spelled the way the source would spell them.
pub fn escape_bytes(bytes: &[u8]) -> String {
    let mut text = String::new();
    for byte in bytes {
        match byte {
            b'\n' => text.push_str("\\n"),
            b'\r' => text.push_str("\\r"),
            b'\t' => text.push_str("\\t"),
            b'"' => text.push_str("\\\""),
            b'\\' => text.push_str("\\\\"),
            0x20..=0x7e => text.push(*byte as char),
            other => text.push_str(&format!("\\x{other:02x}")),
        }
    }
    text
}

/// What `Instruction::Binary` does to its two operands.
///
/// Every operator in the language is one of these, including the three unary
/// ones: `(- x)` is `Sub` from a zero, `(not b)` is `Equal` against `false`,
/// and `(bit-not x)` is `BitXor` against `-1` (`D-106`). A unary variant would
/// have bought a shape both backends and the verifier would have to learn for
/// operators that already have an exact binary spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    /// Remainder, truncated so that `a = (a / b) * b + (a % b)` holds for every
    /// pair, which is what `/` truncating toward zero forces (`D-106`).
    Rem,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Equal,
    NotEqual,
    BitAnd,
    BitOr,
    BitXor,
    /// Left shift. A bit operation, so it does **not** trap when bits leave the
    /// top — `(shl 1 63)` is `i64::MIN` and that is the answer. Only the count
    /// is checked, by [`BinaryOp::shifts`].
    Shl,
    /// Right shift, arithmetic on a signed type and logical on an unsigned one
    /// — one name whose meaning follows the operand type (`D-106`).
    Shr,
}

impl BinaryOp {
    /// Whether this operation compares and therefore produces a `bool`.
    pub fn compares(self) -> bool {
        matches!(
            self,
            BinaryOp::Less
                | BinaryOp::Greater
                | BinaryOp::LessEqual
                | BinaryOp::GreaterEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
        )
    }

    /// Whether this operation shifts, and so must check its count before it
    /// commits: x86-64 masks the count to five or six bits in hardware and
    /// AArch64 masks it modulo the width, so an unchecked shift by the width
    /// does not fault on either — it quietly answers two different things.
    pub fn shifts(self) -> bool {
        matches!(self, BinaryOp::Shl | BinaryOp::Shr)
    }

    /// Whether this operation is bitwise, which is the set that has no meaning
    /// on an `f64` and never traps.
    pub fn bitwise(self) -> bool {
        matches!(self, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor)
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum Terminator {
    Return(Option<LocalId>),
    Goto(BlockId),
    Branch {
        condition: LocalId,
        then_block: BlockId,
        else_block: BlockId,
    },
    /// Control flow cannot reach the end of this block. Lowering leaves a block
    /// unterminated only when it is dead, so `finish` seals those blocks here
    /// rather than letting an absent terminator stay representable.
    Unreachable,
}

pub fn lower(program: &TypedProgram) -> MirModule {
    let mut functions = Vec::new();
    let mut tests = Vec::new();
    // Lifted `lambda` bodies join the functions and their environments join the
    // structs, which is what makes a closure need nothing of either backend:
    // the block is a struct, so the clone and drop helpers generated for every
    // struct are its glue (`D-101`).
    let mut layouts = Vec::new();
    let mut collect = |lowered: Lowered| {
        functions.push(lowered.function);
        functions.extend(lowered.lifted);
        layouts.extend(lowered.layouts);
    };
    for function in &program.functions {
        let mut lowered = lower_function(function);
        // The hint belongs to the function somebody wrote, and to nothing a
        // `lambda` inside it was lifted into: an annotation is written on a
        // declaration, and a lifted body is not one.
        lowered.function.inline_hint = function.inline;
        collect(lowered);
    }
    for (index, test) in program.tests.iter().enumerate() {
        let (test, lowered) = lower_test(index, test);
        tests.push(test);
        functions.extend(lowered.lifted);
        layouts.extend(lowered.layouts);
    }
    MirModule {
        functions,
        externs: program
            .externs
            .iter()
            .map(|declaration| MirExtern {
                name: declaration.name.clone(),
                symbol: declaration.symbol.clone(),
                params: declaration.params.clone(),
                result: declaration.result.clone(),
            })
            .collect(),
        tests,
        structs: program
            .structs
            .iter()
            .map(|item| MirStruct {
                name: item.name.clone(),
                fields: item.fields.clone(),
                emit: true,
            })
            .chain(layouts)
            .collect(),
        enums: program
            .enums
            .iter()
            .map(|item| MirEnum {
                name: item.name.clone(),
                variants: item
                    .variants
                    .iter()
                    .map(|variant| MirVariant {
                        name: variant.name.clone(),
                        tag: variant.tag,
                        fields: variant.fields.clone(),
                    })
                    .collect(),
                emit: true,
            })
            .collect(),
    }
}

fn mir_pattern_irrefutable(pattern: &TPattern) -> bool {
    match pattern {
        TPattern::Wildcard | TPattern::Binding(_) => true,
        TPattern::Struct { fields, .. } => fields
            .iter()
            .all(|field| mir_pattern_irrefutable(&field.pattern)),
        TPattern::Bool(_) | TPattern::Int(_) | TPattern::Enum { .. } => false,
    }
}

/// Whether a borrow of this type has to be *read* to get the value out.
///
/// A borrow of a pointer-shaped value is that pointer, so it already holds
/// everything there is; a borrow of anything else is the address of a slot, and
/// the value is one load away. This is the same split `AddressOf` makes when it
/// creates the borrow, asked in the other direction.
fn reads_through_borrow(ty: &Type) -> bool {
    matches!(ty, Type::Ref { inner, .. } if !crate::lowering::is_pointer_like(inner))
}

/// One typed function, and everything lowering it produced beside it.
///
/// A `lambda` body becomes a function of its own and its environment becomes a
/// struct, and both are named after the function they were written in, so
/// neither needs a table to be found again.
struct Lowered {
    function: MirFunction,
    lifted: Vec<MirFunction>,
    layouts: Vec<MirStruct>,
}

fn lower_function(function: &TypedFunction) -> Lowered {
    let mut builder = Builder::new(
        function.name.clone(),
        function.return_type.clone(),
        function.span,
    );
    builder.scopes.push(Vec::new());
    for param in &function.params {
        let local = builder.local(param.ty.clone(), Some(param.name.clone()), true);
        builder.params.push(local);
        builder.bindings.insert(param.id, local);
        builder.live.insert(param.id, true);
        builder.scopes[0].push(param.id);
    }
    lower_body(builder, &function.body)
}

/// A `lambda` body as a function of its own (`D-102`).
///
/// The parameters are the ones written, and then the block, which is what
/// makes a named function usable as a function value without an adapter: it
/// has no such parameter and never reads the extra word.
fn lower_lambda(
    name: String,
    captures: &[TCapture],
    params: &[TypedParam],
    result: &Type,
    body: &TExpr,
    span: Span,
) -> Lowered {
    let mut builder = Builder::new(name, result.clone(), span);
    builder.scopes.push(Vec::new());
    for param in params {
        let local = builder.local(param.ty.clone(), Some(param.name.clone()), true);
        builder.params.push(local);
        builder.bindings.insert(param.id, local);
        builder.live.insert(param.id, true);
        builder.scopes[0].push(param.id);
    }
    // The block is typed `i64` rather than as the function it is: a parameter
    // of an owning type would be dropped when this body returns, and the block
    // belongs to whoever built it, not to a call of it.
    let block = builder.local(Type::I64, Some("closure".to_owned()), true);
    builder.params.push(block);
    // Each capture is read out of the block, and none of them is registered
    // live: the block owns them and its drop helper releases them, so a body
    // that also dropped one would free it twice.
    for (index, capture) in captures.iter().enumerate() {
        let local = builder.local(capture.ty.clone(), Some(capture.name.clone()), false);
        builder.emit(Instruction::FieldLoad {
            dst: local,
            base: block,
            index: crate::lowering::CLOSURE_HEADER + index,
        });
        builder.bindings.insert(capture.id, local);
    }
    lower_body(builder, body)
}

fn lower_body(mut builder: Builder, body: &TExpr) -> Lowered {
    let value = builder.expr(body);
    builder.drop_scope_except(0, value.as_ref().map(|value| value.local));
    if builder.blocks[builder.current].terminator.is_none() {
        builder.blocks[builder.current].terminator = Some(Terminator::Return(
            value
                .filter(|value| value.ty != Type::Unit)
                .map(|value| value.local),
        ));
    }
    let lifted = std::mem::take(&mut builder.lifted);
    let layouts = std::mem::take(&mut builder.layouts);
    Lowered {
        function: builder.finish(),
        lifted,
        layouts,
    }
}

fn lower_test(index: usize, test: &TypedTest) -> (MirTest, Lowered) {
    let function = TypedFunction {
        name: format!("__slop_test_{index}"),
        inline: false,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Type::Bool,
        body: test.body.clone(),
        span: test.span,
    };
    let lowered = lower_function(&function);
    (
        MirTest {
            name: test.name.clone(),
            function: lowered.function.clone(),
            emit: true,
        },
        lowered,
    )
}

#[derive(Clone, Debug)]
struct Value {
    local: LocalId,
    ty: Type,
    owned_temporary: bool,
}

/// A block under construction. `terminator` stays optional here because
/// lowering uses "has no terminator yet" to mean "this block is still open";
/// `Builder::finish` seals whatever is left into a real [`BasicBlock`].
struct BuilderBlock {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
    terminator_span: Span,
}

struct Builder {
    name: String,
    return_type: Type,
    span: Span,
    params: Vec<LocalId>,
    locals: Vec<MirLocal>,
    blocks: Vec<BuilderBlock>,
    current: BlockId,
    bindings: HashMap<BindingId, LocalId>,
    /// Where each name an exclusive pattern bound actually lives (`D-120`).
    ///
    /// A binding carries its type and nothing else, so the aggregate a field
    /// belongs to and the index it sits at are known while the pattern is being
    /// lowered and nowhere afterwards. `set` reads this to write the field
    /// rather than the name.
    places: HashMap<BindingId, Place>,
    /// Which owned bindings still hold a value.
    ///
    /// Ordered, not hashed: the merge points below iterate this map to decide
    /// where to insert `Drop`s, so hash order would leak into the emitted code
    /// and make builds irreproducible. `bindings` stays a `HashMap` because it
    /// is only ever point-queried.
    live: BTreeMap<BindingId, bool>,
    scopes: Vec<Vec<BindingId>>,
    loop_targets: Vec<LoopTarget>,
    /// Source location of the expression currently being lowered.
    ///
    /// `expr` maintains this around its recursion so that `emit` can attach a
    /// span without every one of its call sites having to pass one.
    current_span: Span,
    /// The `lambda` bodies lifted out of this function, and the layouts of the
    /// blocks that carry them (`D-101`, `D-102`).
    ///
    /// Both are named after the function they came out of, so the pass that
    /// decides which module emits what finds the owner by looking at the name,
    /// exactly as it does for the function itself.
    lifted: Vec<MirFunction>,
    layouts: Vec<MirStruct>,
}

/// The field a name bound through an exclusive borrow stands for (`D-120`).
#[derive(Clone, Copy)]
struct Place {
    base: LocalId,
    index: usize,
    enumeration: bool,
}

#[derive(Clone, Copy)]
struct LoopTarget {
    continue_block: BlockId,
    break_block: BlockId,
    scope_depth: usize,
    /// Where a `(break value)` writes what the loop produces (`D-121`).
    ///
    /// `None` for a `while` and for a `loop` of type `unit`, which is every
    /// loop written before v0.9.1.
    result: Option<LocalId>,
}

impl Builder {
    fn new(name: String, return_type: Type, span: Span) -> Self {
        Self {
            name,
            return_type,
            span,
            params: Vec::new(),
            locals: Vec::new(),
            blocks: vec![BuilderBlock {
                statements: Vec::new(),
                terminator: None,
                terminator_span: span,
            }],
            current: 0,
            bindings: HashMap::new(),
            places: HashMap::new(),
            live: BTreeMap::new(),
            scopes: Vec::new(),
            loop_targets: Vec::new(),
            current_span: span,
            lifted: Vec::new(),
            layouts: Vec::new(),
        }
    }

    /// Builds the block a function value is: the code address, the two helpers
    /// that free and copy it, then one word per capture (`D-101`).
    fn closure(&mut self, ty: Type, code: LocalId, captures: &[(LocalId, Type)]) -> LocalId {
        let layout = self.layout(captures);
        let drop = self.temp(Type::I64);
        self.emit(Instruction::FnAddr {
            dst: drop,
            symbol: crate::lowering::struct_drop_symbol(&layout),
        });
        let clone = self.temp(Type::I64);
        self.emit(Instruction::FnAddr {
            dst: clone,
            symbol: crate::lowering::struct_clone_symbol(&layout),
        });
        let mut fields = vec![code, drop, clone];
        fields.extend(captures.iter().map(|(local, _)| *local));
        let dst = self.temp(ty);
        self.emit(Instruction::StructNew {
            dst,
            name: layout,
            fields,
        });
        dst
    }

    /// The struct a block of these captures is laid out as, reusing one that
    /// already holds the same types.
    ///
    /// Sharing matters for more than size: the layout is where the drop and
    /// clone helpers come from, so two closures that capture the same shape
    /// generate one pair between them.
    fn layout(&mut self, captures: &[(LocalId, Type)]) -> String {
        let header = crate::lowering::CLOSURE_HEADER;
        if let Some(found) = self.layouts.iter().find(|item| {
            item.fields.len() == header + captures.len()
                && item.fields[header..]
                    .iter()
                    .zip(captures)
                    .all(|((_, held), (_, ty))| held == ty)
        }) {
            return found.name.clone();
        }
        let name = format!("{}$closure${}", self.name, self.layouts.len());
        let mut fields = vec![
            ("code".to_owned(), Type::I64),
            ("drop".to_owned(), Type::I64),
            ("clone".to_owned(), Type::I64),
        ];
        fields.extend(
            captures
                .iter()
                .enumerate()
                .map(|(index, (_, ty))| (format!("capture{index}"), ty.clone())),
        );
        self.layouts.push(MirStruct {
            name: name.clone(),
            fields,
            emit: true,
        });
        name
    }

    fn finish(self) -> MirFunction {
        MirFunction {
            name: self.name,
            emit: true,
            inline_hint: false,
            params: self.params,
            return_type: self.return_type,
            locals: self.locals,
            blocks: self
                .blocks
                .into_iter()
                .map(|block| BasicBlock {
                    statements: block.statements,
                    terminator: block.terminator.unwrap_or(Terminator::Unreachable),
                    terminator_span: block.terminator_span,
                })
                .collect(),
            entry: 0,
            span: self.span,
        }
    }

    fn local(&mut self, ty: Type, name: Option<String>, is_param: bool) -> LocalId {
        let id = self.locals.len();
        self.locals.push(MirLocal { ty, name, is_param });
        id
    }

    fn temp(&mut self, ty: Type) -> LocalId {
        self.local(ty, None, false)
    }

    /// The two operands a unary operator turns into.
    ///
    /// `(- x)` is `0 - x` and the order is the whole point; `(not b)` and
    /// `(bit-not x)` are commutative, so their constant goes on the right where
    /// it reads as an argument rather than as a subject.
    ///
    /// The float negation subtracts from **negative** zero. `0.0 - 0.0` is
    /// `+0.0` and `-(0.0)` is `-0.0`, and `core:float` reads the sign bit
    /// directly (`D-097`), so the positive zero would have been a wrong answer
    /// that only printing could see.
    /// Reduces a full machine word to `kind`'s canonical form, and answers with
    /// the local holding it.
    ///
    /// Everything here is at `i64`, deliberately: the shift amount is up to 56,
    /// which is inside the word's bound but outside a narrow type's, so a
    /// truncation written at the *target's* type would trip the shift-range
    /// trap it is meant to implement.
    fn canonicalize(&mut self, source: LocalId, kind: crate::ast::IntKind) -> LocalId {
        if kind.signed {
            let amount = self.temp(Type::I64);
            self.emit(Instruction::ConstInt {
                dst: amount,
                value: i64::from(64 - kind.bits),
            });
            let raised = self.temp(Type::I64);
            self.emit(Instruction::Binary {
                dst: raised,
                op: BinaryOp::Shl,
                lhs: source,
                rhs: amount,
                ty: Type::I64,
            });
            let lowered = self.temp(Type::I64);
            self.emit(Instruction::Binary {
                dst: lowered,
                op: BinaryOp::Shr,
                lhs: raised,
                rhs: amount,
                ty: Type::I64,
            });
            lowered
        } else {
            let mask = self.temp(Type::I64);
            self.emit(Instruction::ConstInt {
                dst: mask,
                value: kind.mask(),
            });
            let kept = self.temp(Type::I64);
            self.emit(Instruction::Binary {
                dst: kept,
                op: BinaryOp::BitAnd,
                lhs: source,
                rhs: mask,
                ty: Type::I64,
            });
            kept
        }
    }

    fn unary_operands(&mut self, callee: &str, ty: &Type, operand: LocalId) -> (LocalId, LocalId) {
        match callee {
            "-" if *ty == Type::F64 => {
                let zero = self.temp(Type::F64);
                self.emit(Instruction::ConstFloat {
                    dst: zero,
                    bits: (-0.0f64).to_bits(),
                });
                (zero, operand)
            }
            "-" => {
                let zero = self.temp(ty.clone());
                self.emit(Instruction::ConstInt {
                    dst: zero,
                    value: 0,
                });
                (zero, operand)
            }
            "not" => {
                let no = self.temp(Type::Bool);
                self.emit(Instruction::ConstBool {
                    dst: no,
                    value: false,
                });
                (operand, no)
            }
            // `bit-not` is `x ^ (every bit this type has)`, written as the
            // canonical word with all of them set: `-1` at any signed width,
            // because the operand is sign-extended and the bits above the type
            // have to flip with it, and the type's mask when it is unsigned,
            // because there they have to stay clear. That is what keeps the
            // result canonical, and is why the backends need no masking tail
            // on a bit operation (`D-107`).
            _ => {
                let ones = self.temp(ty.clone());
                self.emit(Instruction::ConstInt {
                    dst: ones,
                    value: ty.int_kind().map_or(-1, |kind| kind.canonicalize(-1)),
                });
                (operand, ones)
            }
        }
    }

    fn emit(&mut self, instruction: Instruction) {
        let block = self.current;
        self.emit_in(block, instruction);
    }

    /// Appends to a block that is not the current one. Used by the merge points
    /// that retroactively drop a binding on the branch that still owns it.
    fn emit_in(&mut self, block: BlockId, instruction: Instruction) {
        let span = self.current_span;
        self.blocks[block]
            .statements
            .push(Statement { instruction, span });
    }

    fn block(&mut self) -> BlockId {
        let id = self.blocks.len();
        self.blocks.push(BuilderBlock {
            statements: Vec::new(),
            terminator: None,
            terminator_span: self.current_span,
        });
        id
    }

    /// Lowers an expression, scoping [`Builder::current_span`] to it.
    ///
    /// The span must be restored afterwards, not merely set on entry: once a
    /// subexpression returns, later instructions belong to the enclosing
    /// expression again, and leaving the child's span in place would attribute
    /// them to the wrong line.
    fn expr(&mut self, expr: &TExpr) -> Option<Value> {
        let enclosing = std::mem::replace(&mut self.current_span, expr.span);
        let value = self.expr_kind(expr);
        self.current_span = enclosing;
        value
    }

    fn expr_kind(&mut self, expr: &TExpr) -> Option<Value> {
        match &expr.kind {
            TExprKind::Unit => None,
            TExprKind::Bool(value) => {
                let dst = self.temp(Type::Bool);
                self.emit(Instruction::ConstBool { dst, value: *value });
                Some(Value {
                    local: dst,
                    ty: Type::Bool,
                    owned_temporary: false,
                })
            }
            TExprKind::Int(value) => {
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::ConstInt { dst, value: *value });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: false,
                })
            }
            TExprKind::Float(value) => {
                let dst = self.temp(Type::F64);
                self.emit(Instruction::ConstFloat {
                    dst,
                    bits: value.to_bits(),
                });
                Some(Value {
                    local: dst,
                    ty: Type::F64,
                    owned_temporary: false,
                })
            }
            TExprKind::String(value) => {
                let dst = self.temp(Type::String);
                self.emit(Instruction::StringNew {
                    dst,
                    value: value.clone(),
                });
                Some(Value {
                    local: dst,
                    ty: Type::String,
                    owned_temporary: true,
                })
            }
            TExprKind::Var(id) => {
                let local = self.bindings[id];
                if !expr.ty.is_copy() {
                    self.live.insert(*id, false);
                }
                Some(Value {
                    local,
                    ty: expr.ty.clone(),
                    owned_temporary: !expr.ty.is_copy(),
                })
            }
            TExprKind::Let {
                id, name, value, ..
            } => {
                let value = self.expr(value)?;
                let dst = self.local(value.ty.clone(), Some(name.clone()), false);
                self.emit(Instruction::Assign {
                    dst,
                    src: value.local,
                });
                self.bindings.insert(*id, dst);
                self.live.insert(*id, true);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.push(*id);
                }
                None
            }
            TExprKind::Set { id, value } => {
                let value = self.expr(value)?;
                // Assigning to a field through an exclusive borrow (`D-120`).
                // The old word comes out first and is dropped only once the new
                // one is in the slot, so the field never briefly holds two
                // owners or none — `replace`'s discipline (`D-103`), one level
                // in. The name's own local is not touched: it is a borrow of
                // the field, and the field is what changed.
                if let Some(place) = self.places.get(id).copied() {
                    let old = (!value.ty.is_copy()).then(|| {
                        let old = self.temp(value.ty.clone());
                        self.emit(if place.enumeration {
                            Instruction::EnumFieldLoad {
                                dst: old,
                                base: place.base,
                                index: place.index,
                            }
                        } else {
                            Instruction::FieldLoad {
                                dst: old,
                                base: place.base,
                                index: place.index,
                            }
                        });
                        old
                    });
                    self.emit(if place.enumeration {
                        Instruction::EnumFieldStore {
                            base: place.base,
                            index: place.index,
                            src: value.local,
                        }
                    } else {
                        Instruction::FieldStore {
                            base: place.base,
                            index: place.index,
                            src: value.local,
                        }
                    });
                    if let Some(old) = old {
                        self.emit(Instruction::Drop {
                            local: old,
                            ty: value.ty.clone(),
                        });
                    }
                    return None;
                }
                let dst = self.bindings[id];
                if self.live.get(id).copied().unwrap_or(false) && !value.ty.is_copy() {
                    self.emit(Instruction::Drop {
                        local: dst,
                        ty: value.ty.clone(),
                    });
                }
                self.emit(Instruction::Assign {
                    dst,
                    src: value.local,
                });
                self.live.insert(*id, true);
                None
            }
            TExprKind::Do(expressions) => {
                self.scopes.push(Vec::new());
                let scope_index = self.scopes.len() - 1;
                let mut result = None;
                for (index, item) in expressions.iter().enumerate() {
                    let value = self.expr(item);
                    if index + 1 != expressions.len() {
                        self.drop_temporary(value);
                    } else {
                        result = value;
                    }
                }
                self.drop_scope_except(scope_index, result.as_ref().map(|value| value.local));
                self.scopes.pop();
                result
            }
            TExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => self.lower_if(expr, condition, then_expr, else_expr),
            TExprKind::Loop { body } => self.lower_loop(None, body, &expr.ty),
            TExprKind::While { condition, body } => {
                self.lower_loop(Some(condition), body, &Type::Unit)
            }
            TExprKind::Break(value) => {
                let value = value.as_ref().and_then(|value| self.expr(value));
                self.lower_loop_jump(false, value);
                None
            }
            TExprKind::Continue => {
                self.lower_loop_jump(true, None);
                None
            }
            TExprKind::Const { value, .. } => self.expr(value),
            TExprKind::Match { value, arms } => self.lower_match(expr, value, arms),
            TExprKind::Borrow { id, .. } => {
                let src = self.bindings[id];
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::AddressOf { dst, src });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: false,
                })
            }
            TExprKind::Try {
                value,
                ok_type,
                ok_tag,
                ..
            } => self.lower_try(value, ok_type, *ok_tag),
            // The only pair `D-090` allows is `i32` to `i64`, and an `i32` is
            // kept sign-extended in its full word everywhere (`D-074`), so the
            // widening is already done and the move is the whole conversion.
            // A pair that is not already extended needs an instruction here,
            // and the differential suite is what would catch its absence.
            // A conversion costs no instruction of its own. Every integer is
            // held canonical in a full machine word (`D-074`, `D-107`), so a
            // conversion is exactly the canonicalisation of the target's width
            // — a mask when the target is unsigned, a shift pair when it is
            // signed — and both are `Binary` shapes the backends already emit.
            // That is `D-112`'s call about unary operators made a second time:
            // no new MIR node means no chance of the two backends disagreeing
            // about what a conversion is.
            TExprKind::Convert { value } => {
                let source = self.expr(value)?;
                let target = expr.ty.clone();
                // `conversion_kind` rather than `int_kind`, so that a pointer
                // counts as the `u64` it is. A narrowing out of one truncates
                // like any other (`D-067`).
                let word = match (target.conversion_kind(), source.ty.conversion_kind()) {
                    (Some(to), Some(from)) if !to.accepts(from) => {
                        Some(self.canonicalize(source.local, to))
                    }
                    _ => None,
                };
                let dst = self.temp(target.clone());
                self.emit(Instruction::Assign {
                    dst,
                    src: word.unwrap_or(source.local),
                });
                Some(Value {
                    local: dst,
                    ty: target,
                    owned_temporary: false,
                })
            }
            TExprKind::Call { callee, args } | TExprKind::GenericCall { callee, args, .. } => {
                let callee = match &expr.kind {
                    TExprKind::GenericCall {
                        callee, type_args, ..
                    } => crate::sema::generic_instance_name(callee, type_args),
                    _ => callee.clone(),
                };
                let mut lowered = Vec::new();
                let mut arg_types = Vec::new();
                let mut cloned_temporaries = Vec::new();
                for arg in args {
                    let value = if callee == "clone" {
                        match &arg.kind {
                            TExprKind::Var(id) => Some(Value {
                                local: self.bindings[id],
                                ty: arg.ty.clone(),
                                owned_temporary: false,
                            }),
                            _ => self.expr(arg),
                        }
                    } else {
                        self.expr(arg)
                    };
                    if let Some(value) = value {
                        if callee == "clone" && value.owned_temporary {
                            cloned_temporaries.push(value.clone());
                        }
                        arg_types.push(value.ty);
                        lowered.push(value.local);
                    }
                }
                let dst = self.temp(expr.ty.clone());
                let op = match callee.as_str() {
                    "+" => Some(BinaryOp::Add),
                    "-" => Some(BinaryOp::Sub),
                    "*" => Some(BinaryOp::Mul),
                    "/" => Some(BinaryOp::Div),
                    "%" => Some(BinaryOp::Rem),
                    "<" => Some(BinaryOp::Less),
                    ">" => Some(BinaryOp::Greater),
                    "<=" => Some(BinaryOp::LessEqual),
                    ">=" => Some(BinaryOp::GreaterEqual),
                    "=" => Some(BinaryOp::Equal),
                    "!=" => Some(BinaryOp::NotEqual),
                    "bit-and" => Some(BinaryOp::BitAnd),
                    "bit-or" => Some(BinaryOp::BitOr),
                    "bit-xor" => Some(BinaryOp::BitXor),
                    // The three unary operators, each spelled as the binary one
                    // that already means it. `not` and `bit-not` are here
                    // rather than beside their siblings because the operand
                    // they need does not exist in the source (`D-106`).
                    "bit-not" => Some(BinaryOp::BitXor),
                    "not" => Some(BinaryOp::Equal),
                    "shl" => Some(BinaryOp::Shl),
                    "shr" => Some(BinaryOp::Shr),
                    _ => None,
                };
                if let Some(op) = op {
                    let ty = args[0].ty.clone();
                    // A unary operator arrives with one argument and leaves
                    // with two: the constant that makes it the binary
                    // operation it already was. Neither backend learns a
                    // shape, and the trap comes along for free — `0 - x`
                    // overflows at the smallest integer exactly as `(- 0 x)`
                    // did before this patch (`D-106`).
                    let (lhs, rhs) = if lowered.len() == 2 {
                        (lowered[0], lowered[1])
                    } else {
                        self.unary_operands(&callee.clone(), &ty, lowered[0])
                    };
                    self.emit(Instruction::Binary {
                        dst,
                        op,
                        lhs,
                        rhs,
                        ty,
                    });
                } else if callee == "volatile-read" {
                    self.emit(Instruction::VolatileLoad {
                        dst,
                        addr: lowered[0],
                        ty: expr.ty.clone(),
                    });
                } else if callee == "volatile-write" {
                    self.emit(Instruction::VolatileStore {
                        addr: lowered[0],
                        src: lowered[1],
                        ty: arg_types[1].clone(),
                    });
                } else if callee == "ptr-offset" {
                    // No new instruction: an offset is a multiply and an add on
                    // `Binary` shapes both backends already emit, which is the
                    // call `D-112` made for the unary operators and `D-113`
                    // made for conversions, made a third time. The arithmetic
                    // is `u64` because an address is, and it traps on overflow
                    // like any other (`D-031`).
                    let element = match &arg_types[0] {
                        Type::Ptr(pointee) => crate::lowering::access_size(pointee)
                            .map(|size| i64::from(size.bytes()))
                            .unwrap_or(1),
                        _ => 1,
                    };
                    // A pointer to a byte needs no scaling, and emitting the
                    // multiply anyway would put an overflow check that can
                    // never fire in front of every `u8` offset.
                    let scaled = if element == 1 {
                        lowered[1]
                    } else {
                        let size = self.temp(Type::U64);
                        self.emit(Instruction::ConstInt {
                            dst: size,
                            value: element,
                        });
                        let scaled = self.temp(Type::U64);
                        self.emit(Instruction::Binary {
                            dst: scaled,
                            op: BinaryOp::Mul,
                            lhs: lowered[1],
                            rhs: size,
                            ty: Type::U64,
                        });
                        scaled
                    };
                    self.emit(Instruction::Binary {
                        dst,
                        op: BinaryOp::Add,
                        lhs: lowered[0],
                        rhs: scaled,
                        ty: Type::U64,
                    });
                } else if callee == "clone" && arg_types.first().is_some_and(reads_through_borrow) {
                    // The dereference (`D-100`). A borrow of a pointer-shaped
                    // value is that pointer, so cloning one is the runtime glue
                    // below; a borrow of anything else is the address of a slot,
                    // and reading it is a load and not a call. Deciding here
                    // rather than in `lowering::builtin` is what makes the
                    // choice from the *concrete* type: a generic body reaches
                    // this only after specialization, and asking the generic
                    // `T` instead is what returned a pointer where an `i64` was
                    // asked for.
                    self.emit(Instruction::Load {
                        dst,
                        src: lowered[0],
                    });
                } else {
                    self.emit(Instruction::Call {
                        dst,
                        callee: callee.clone(),
                        args: lowered.clone(),
                        arg_types: arg_types.clone(),
                        result: expr.ty.clone(),
                    });
                }
                for value in cloned_temporaries {
                    self.drop_temporary(Some(value));
                }
                if expr.ty == Type::Unit {
                    None
                } else {
                    Some(Value {
                        local: dst,
                        ty: expr.ty.clone(),
                        owned_temporary: !expr.ty.is_copy(),
                    })
                }
            }
            TExprKind::FnRef { name, .. } => {
                // `type_args` is empty here: `specialize_expr` has already
                // folded a generic instance into the name.
                let code = self.temp(Type::I64);
                self.emit(Instruction::FnAddr {
                    dst: code,
                    // Not `call_symbol`, which exists to pick a C name for an
                    // `extern`: sema refuses an `extern` as a value, so the
                    // only symbol reachable here is a Slopium one.
                    symbol: crate::lowering::function_symbol(name, false),
                });
                // A named function captures nothing, and is boxed anyway
                // (`D-101`): the alternative is a static block holding a code
                // address, and this project's object writer has relocations in
                // `.text` and nowhere else.
                let dst = self.closure(expr.ty.clone(), code, &[]);
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: true,
                })
            }
            TExprKind::Lambda {
                captures,
                params,
                result,
                body,
            } => {
                // The captures are read here, in the function the `lambda` was
                // written in, and each one is a move: the block owns them from
                // now on, which is why `D-102` has them written down.
                let mut taken = Vec::new();
                for capture in captures {
                    let local = self.bindings[&capture.from];
                    if !capture.ty.is_copy() {
                        self.live.insert(capture.from, false);
                    }
                    taken.push((local, capture.ty.clone()));
                }
                let name = format!("{}$lambda${}", self.name, self.lifted.len());
                let lowered = lower_lambda(
                    name.clone(),
                    captures,
                    params,
                    result,
                    body,
                    self.current_span,
                );
                self.lifted.push(lowered.function);
                self.lifted.extend(lowered.lifted);
                self.layouts.extend(lowered.layouts);
                let code = self.temp(Type::I64);
                self.emit(Instruction::FnAddr {
                    dst: code,
                    symbol: crate::lowering::function_symbol(&name, false),
                });
                let dst = self.closure(expr.ty.clone(), code, &taken);
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: true,
                })
            }
            TExprKind::CallValue { callee, args } => {
                let mut lowered = Vec::new();
                let mut arg_types = Vec::new();
                for arg in args {
                    if let Some(value) = self.expr(arg) {
                        arg_types.push(value.ty);
                        lowered.push(value.local);
                    }
                }
                // The callee is the block, not the code: word 0 is what to
                // call, and the block itself goes along as a trailing argument
                // so the body can reach its captures. A named function has no
                // such parameter and ignores the extra word — which both
                // calling conventions allow, since the caller is the one that
                // lays out and cleans up an argument the callee never reads.
                let block = self.bindings[callee];
                let code = self.temp(Type::I64);
                self.emit(Instruction::FieldLoad {
                    dst: code,
                    base: block,
                    index: 0,
                });
                arg_types.push(self.locals[block].ty.clone());
                lowered.push(block);
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::CallValue {
                    dst,
                    callee: code,
                    args: lowered,
                    arg_types,
                    result: expr.ty.clone(),
                });
                if expr.ty == Type::Unit {
                    None
                } else {
                    Some(Value {
                        local: dst,
                        ty: expr.ty.clone(),
                        owned_temporary: !expr.ty.is_copy(),
                    })
                }
            }
            TExprKind::StructInit { name, fields } => {
                let mut lowered = Vec::new();
                for field in fields {
                    if let Some(value) = self.expr(field) {
                        lowered.push(value.local);
                    }
                }
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::StructNew {
                    dst,
                    name: name.clone(),
                    fields: lowered,
                });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: true,
                })
            }
            TExprKind::EnumInit {
                enum_name,
                tag,
                fields,
                ..
            } => {
                let mut lowered = Vec::new();
                for field in fields {
                    if let Some(value) = self.expr(field) {
                        lowered.push(value.local);
                    }
                }
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::EnumNew {
                    dst,
                    enum_name: enum_name.clone(),
                    tag: *tag,
                    fields: lowered,
                });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: true,
                })
            }
            TExprKind::Field { base, index, .. } => {
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::FieldLoad {
                    dst,
                    base: self.bindings[base],
                    index: *index,
                });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
                    owned_temporary: false,
                })
            }
        }
    }

    fn lower_loop(
        &mut self,
        condition: Option<&TExpr>,
        body: &TExpr,
        result_type: &Type,
    ) -> Option<Value> {
        let condition_block = self.block();
        let body_block = self.block();
        let exit_block = self.block();
        // A `loop` that produces something writes into one local on every
        // break edge and reads it once past the exit (`D-121`). There is no
        // new instruction in that: it is `Assign` and `Goto`, which is what a
        // `break` already was.
        let result = (*result_type != Type::Unit).then(|| self.temp(result_type.clone()));
        self.blocks[self.current].terminator = Some(Terminator::Goto(condition_block));

        self.current = condition_block;
        if let Some(condition) = condition {
            if let Some(condition) = self.expr(condition) {
                self.blocks[self.current].terminator = Some(Terminator::Branch {
                    condition: condition.local,
                    then_block: body_block,
                    else_block: exit_block,
                });
            } else {
                self.blocks[self.current].terminator = Some(Terminator::Goto(exit_block));
            }
        } else {
            self.blocks[self.current].terminator = Some(Terminator::Goto(body_block));
        }

        self.loop_targets.push(LoopTarget {
            continue_block: condition_block,
            break_block: exit_block,
            scope_depth: self.scopes.len(),
            result,
        });
        self.current = body_block;
        let value = self.expr(body);
        self.drop_temporary(value);
        if self.blocks[self.current].terminator.is_none() {
            self.blocks[self.current].terminator = Some(Terminator::Goto(condition_block));
        }
        self.loop_targets.pop();
        self.current = exit_block;
        result.map(|local| Value {
            local,
            ty: result_type.clone(),
            owned_temporary: !result_type.is_copy(),
        })
    }

    fn lower_loop_jump(&mut self, continuing: bool, value: Option<Value>) {
        let Some(target) = self.loop_targets.last().copied() else {
            return;
        };
        // The value leaves before the scopes it was standing in are dropped,
        // and the local it moved into is exempted from that walk the way
        // `drop_scope_except` exempts a block's result.
        let escaping = match (target.result, value) {
            (Some(result), Some(value)) => {
                self.emit(Instruction::Assign {
                    dst: result,
                    src: value.local,
                });
                Some(value.local)
            }
            (_, value) => {
                self.drop_temporary(value);
                None
            }
        };
        // Collect before emitting: `emit` needs `&mut self`, and the scope walk
        // holds an immutable borrow. Order is unchanged.
        let mut pending = Vec::new();
        for scope in self.scopes.iter().skip(target.scope_depth).rev() {
            for id in scope.iter().rev() {
                if !self.live.get(id).copied().unwrap_or(false) {
                    continue;
                }
                let local = self.bindings[id];
                if Some(local) == escaping {
                    continue;
                }
                let ty = self.locals[local].ty.clone();
                if !ty.is_copy() {
                    pending.push(Instruction::Drop { local, ty });
                }
            }
        }
        for instruction in pending {
            self.emit(instruction);
        }
        self.blocks[self.current].terminator = Some(Terminator::Goto(if continuing {
            target.continue_block
        } else {
            target.break_block
        }));
        self.current = self.block();
    }

    fn lower_try(&mut self, value: &TExpr, ok_type: &Type, ok_tag: usize) -> Option<Value> {
        let value = self.expr(value)?;
        let tag = self.temp(Type::I64);
        self.emit(Instruction::EnumTag {
            dst: tag,
            base: value.local,
        });
        let expected = self.temp(Type::I64);
        self.emit(Instruction::ConstInt {
            dst: expected,
            value: ok_tag as i64,
        });
        let is_ok = self.temp(Type::Bool);
        self.emit(Instruction::Binary {
            dst: is_ok,
            op: BinaryOp::Equal,
            lhs: tag,
            rhs: expected,
            ty: Type::I64,
        });
        let ok_block = self.block();
        let error_block = self.block();
        self.blocks[self.current].terminator = Some(Terminator::Branch {
            condition: is_ok,
            then_block: ok_block,
            else_block: error_block,
        });

        self.current = error_block;
        // Reverse binding order, matching `drop_scope_except`: bindings
        // declared later are dropped first.
        for (id, is_live) in self.live.clone().into_iter().rev() {
            if !is_live {
                continue;
            }
            let Some(local) = self.bindings.get(&id).copied() else {
                continue;
            };
            let ty = self.locals[local].ty.clone();
            if !ty.is_copy() && local != value.local {
                self.emit(Instruction::Drop { local, ty });
            }
        }
        self.blocks[self.current].terminator = Some(Terminator::Return(Some(value.local)));

        self.current = ok_block;
        let payload = self.temp(ok_type.clone());
        self.emit(Instruction::EnumFieldLoad {
            dst: payload,
            base: value.local,
            index: 0,
        });
        self.emit(Instruction::Free { local: value.local });
        Some(Value {
            local: payload,
            ty: ok_type.clone(),
            owned_temporary: !ok_type.is_copy(),
        })
    }

    fn lower_if(
        &mut self,
        expr: &TExpr,
        condition: &TExpr,
        then_expr: &TExpr,
        else_expr: &TExpr,
    ) -> Option<Value> {
        let condition = self.expr(condition)?;
        let then_block = self.block();
        let else_block = self.block();
        let merge_block = self.block();
        self.blocks[self.current].terminator = Some(Terminator::Branch {
            condition: condition.local,
            then_block,
            else_block,
        });

        let result = if expr.ty == Type::Unit {
            None
        } else {
            Some(self.temp(expr.ty.clone()))
        };
        let base_live = self.live.clone();

        self.current = then_block;
        self.live = base_live.clone();
        self.scopes.push(Vec::new());
        let then_scope = self.scopes.len() - 1;
        let then_value = self.expr(then_expr);
        if let (Some(result), Some(value)) = (result, then_value.as_ref()) {
            self.emit(Instruction::Assign {
                dst: result,
                src: value.local,
            });
        }
        self.drop_scope_except(then_scope, then_value.as_ref().map(|value| value.local));
        self.scopes.pop();
        let then_live = self.live.clone();
        self.blocks[self.current].terminator = Some(Terminator::Goto(merge_block));
        let then_end = self.current;

        self.current = else_block;
        self.live = base_live.clone();
        self.scopes.push(Vec::new());
        let else_scope = self.scopes.len() - 1;
        let else_value = self.expr(else_expr);
        if let (Some(result), Some(value)) = (result, else_value.as_ref()) {
            self.emit(Instruction::Assign {
                dst: result,
                src: value.local,
            });
        }
        self.drop_scope_except(else_scope, else_value.as_ref().map(|value| value.local));
        self.scopes.pop();
        let else_live = self.live.clone();
        self.blocks[self.current].terminator = Some(Terminator::Goto(merge_block));
        let else_end = self.current;

        self.live = base_live;
        for id in self.live.clone().keys().copied().rev().collect::<Vec<_>>() {
            let then_has = then_live.get(&id).copied().unwrap_or(false);
            let else_has = else_live.get(&id).copied().unwrap_or(false);
            if then_has != else_has {
                let local = self.bindings[&id];
                let ty = self.locals[local].ty.clone();
                if !ty.is_copy() {
                    let block = if then_has { then_end } else { else_end };
                    self.emit_in(block, Instruction::Drop { local, ty });
                }
            }
            self.live.insert(id, then_has && else_has);
        }
        self.current = merge_block;
        result.map(|local| Value {
            local,
            ty: expr.ty.clone(),
            owned_temporary: !expr.ty.is_copy(),
        })
    }

    fn branch_pattern(
        &mut self,
        pattern: &TPattern,
        value: LocalId,
        value_type: &Type,
        success: BlockId,
        failure: BlockId,
    ) {
        match pattern {
            TPattern::Wildcard | TPattern::Binding(_) => {
                self.blocks[self.current].terminator = Some(Terminator::Goto(success));
            }
            TPattern::Bool(pattern) => {
                let expected = self.temp(Type::Bool);
                self.emit(Instruction::ConstBool {
                    dst: expected,
                    value: *pattern,
                });
                self.branch_equal(value, expected, &Type::Bool, success, failure);
            }
            TPattern::Int(pattern) => {
                let expected = self.temp(value_type.clone());
                self.emit(Instruction::ConstInt {
                    dst: expected,
                    value: *pattern,
                });
                self.branch_equal(value, expected, value_type, success, failure);
            }
            TPattern::Enum { tag, fields, .. } => {
                let tag_value = self.temp(Type::I64);
                self.emit(Instruction::EnumTag {
                    dst: tag_value,
                    base: value,
                });
                let expected = self.temp(Type::I64);
                self.emit(Instruction::ConstInt {
                    dst: expected,
                    value: *tag as i64,
                });
                let fields_block = if fields
                    .iter()
                    .any(|field| !mir_pattern_irrefutable(&field.pattern))
                {
                    self.block()
                } else {
                    success
                };
                self.branch_equal(tag_value, expected, &Type::I64, fields_block, failure);
                if fields_block != success {
                    self.current = fields_block;
                    self.branch_pattern_fields(fields, value, true, success, failure);
                }
            }
            TPattern::Struct { fields, .. } => {
                self.branch_pattern_fields(fields, value, false, success, failure);
            }
        }
    }

    fn branch_pattern_fields(
        &mut self,
        fields: &[crate::sema::TPatternField],
        value: LocalId,
        enumeration: bool,
        success: BlockId,
        failure: BlockId,
    ) {
        let checks = fields
            .iter()
            .enumerate()
            .filter(|(_, field)| !mir_pattern_irrefutable(&field.pattern))
            .collect::<Vec<_>>();
        if checks.is_empty() {
            self.blocks[self.current].terminator = Some(Terminator::Goto(success));
            return;
        }
        for (position, (index, field)) in checks.iter().enumerate() {
            let field_value = self.temp(field.ty.clone());
            if enumeration {
                self.emit(Instruction::EnumFieldLoad {
                    dst: field_value,
                    base: value,
                    index: *index,
                });
            } else {
                self.emit(Instruction::FieldLoad {
                    dst: field_value,
                    base: value,
                    index: *index,
                });
            }
            let next = if position + 1 == checks.len() {
                success
            } else {
                self.block()
            };
            self.branch_pattern(&field.pattern, field_value, &field.ty, next, failure);
            if next != success {
                self.current = next;
            }
        }
    }

    fn branch_equal(
        &mut self,
        value: LocalId,
        expected: LocalId,
        ty: &Type,
        success: BlockId,
        failure: BlockId,
    ) {
        let condition = self.temp(Type::Bool);
        self.emit(Instruction::Binary {
            dst: condition,
            op: BinaryOp::Equal,
            lhs: value,
            rhs: expected,
            ty: ty.clone(),
        });
        self.blocks[self.current].terminator = Some(Terminator::Branch {
            condition,
            then_block: success,
            else_block: failure,
        });
    }

    /// Binds what an arm's pattern names, and disposes of what it does not.
    ///
    /// `borrowed` is the whole difference between taking a value apart and
    /// looking inside one (`D-099`): through a borrow nothing is freed, nothing
    /// is dropped, and each field is bound as a borrow of itself rather than
    /// moved out. The scrutinee belongs to someone else and is still theirs when
    /// the arm ends.
    fn consume_pattern(
        &mut self,
        pattern: &TPattern,
        value: LocalId,
        value_type: &Type,
        scope: usize,
        borrowed: bool,
    ) {
        match pattern {
            TPattern::Wildcard => {
                if !borrowed && !value_type.is_copy() {
                    self.emit(Instruction::Drop {
                        local: value,
                        ty: value_type.clone(),
                    });
                }
            }
            TPattern::Binding(binding) => {
                let local = self.local(binding.ty.clone(), Some(binding.name.clone()), false);
                self.emit(Instruction::Assign {
                    dst: local,
                    src: value,
                });
                self.bindings.insert(binding.id, local);
                self.live.insert(binding.id, true);
                self.scopes[scope].push(binding.id);
            }
            TPattern::Bool(_) | TPattern::Int(_) => {}
            TPattern::Enum { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    let local = self.pattern_field(value, index, &field.ty, true, borrowed);
                    self.record_place(&field.pattern, value, index, true);
                    self.consume_pattern(&field.pattern, local, &field.ty, scope, borrowed);
                }
                if !borrowed {
                    self.emit(Instruction::Free { local: value });
                }
            }
            TPattern::Struct { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    let local = self.pattern_field(value, index, &field.ty, false, borrowed);
                    self.record_place(&field.pattern, value, index, false);
                    self.consume_pattern(&field.pattern, local, &field.ty, scope, borrowed);
                }
                if !borrowed {
                    self.emit(Instruction::Free { local: value });
                }
            }
        }
    }

    /// Binds a pattern's names for a `when` guard, taking nothing apart
    /// (`D-121`).
    ///
    /// `consume_pattern` without the `Free`, without the drop bookkeeping and
    /// without the scope entries: a guard only reads — `sema` refuses a move
    /// inside one — so a field is the word already in the slot, and the
    /// aggregate is still whole whichever way the condition goes. The bindings
    /// are rebound for real in the arm's own block, so the locals made here
    /// are the guard's alone.
    fn bind_pattern_for_guard(&mut self, pattern: &TPattern, value: LocalId, borrowed: bool) {
        match pattern {
            TPattern::Wildcard | TPattern::Bool(_) | TPattern::Int(_) => {}
            TPattern::Binding(binding) => {
                let local = self.local(binding.ty.clone(), Some(binding.name.clone()), false);
                self.emit(Instruction::Assign {
                    dst: local,
                    src: value,
                });
                self.bindings.insert(binding.id, local);
            }
            TPattern::Enum { fields, .. } | TPattern::Struct { fields, .. } => {
                let enumeration = matches!(pattern, TPattern::Enum { .. });
                for (index, field) in fields.iter().enumerate() {
                    let local = self.pattern_field(value, index, &field.ty, enumeration, borrowed);
                    self.bind_pattern_for_guard(&field.pattern, local, borrowed);
                }
            }
        }
    }

    /// Remembers which field a name stands for, when the borrow it was bound
    /// through was an exclusive one (`D-120`).
    ///
    /// The aggregate and the index are in hand here and nowhere downstream,
    /// because a binding carries only its type. An exclusive binding at any
    /// other position — the whole scrutinee, or a name a shared pattern bound —
    /// records nothing, and `sema` has already refused to write through one.
    fn record_place(&mut self, pattern: &TPattern, base: LocalId, index: usize, enumeration: bool) {
        let TPattern::Binding(binding) = pattern else {
            return;
        };
        if !matches!(binding.ty, Type::Ref { mutable: true, .. }) {
            return;
        }
        self.places.insert(
            binding.id,
            Place {
                base,
                index,
                enumeration,
            },
        );
    }

    /// One field of an aggregate a pattern is taking apart, moved out of it or
    /// borrowed in place.
    ///
    /// Borrowing splits on the same question `AddressOf` asks: a pointer-shaped
    /// field *is* its own borrow, so the word in the slot is what to take, and
    /// anything else is borrowed by the address of the slot holding it.
    fn pattern_field(
        &mut self,
        base: LocalId,
        index: usize,
        field_type: &Type,
        enumeration: bool,
        borrowed: bool,
    ) -> LocalId {
        if !borrowed {
            let dst = self.temp(field_type.clone());
            self.emit(if enumeration {
                Instruction::EnumFieldLoad { dst, base, index }
            } else {
                Instruction::FieldLoad { dst, base, index }
            });
            return dst;
        }
        let dst = self.temp(Type::Ref {
            mutable: false,
            inner: Box::new(field_type.clone()),
        });
        self.emit(
            match (enumeration, crate::lowering::is_pointer_like(field_type)) {
                (true, true) => Instruction::EnumFieldLoad { dst, base, index },
                (true, false) => Instruction::EnumFieldAddr { dst, base, index },
                (false, true) => Instruction::FieldLoad { dst, base, index },
                (false, false) => Instruction::FieldAddr { dst, base, index },
            },
        );
        dst
    }

    fn lower_match(&mut self, expr: &TExpr, value: &TExpr, arms: &[TMatchArm]) -> Option<Value> {
        let scrutinee = self.expr(value)?;
        // Matching through a shared borrow (`D-099`). Branching needs no help:
        // an enum is a pointer, so a borrow of one is the same word, and the
        // tag and the fields a refutable pattern compares are read through it
        // either way. It is binding that differs, and freeing.
        let borrowed = matches!(value.ty, Type::Ref { .. });
        let pattern_type = value.ty.strip_ref().clone();
        let merge_block = self.block();
        let result = if expr.ty == Type::Unit {
            None
        } else {
            Some(self.temp(expr.ty.clone()))
        };
        let base_live = self.live.clone();
        let mut arm_states = Vec::new();
        for arm in arms {
            let arm_block = self.block();
            let next_check = self.block();
            // The guard is built before the arm block, so it starts from the
            // same live set the arm will: without this it would inherit the
            // previous arm's, which is a state no path into this guard has.
            self.live = base_live.clone();
            // A guarded arm is tested twice (`D-121`): the pattern first, and
            // then the condition in a block of its own. The names the guard
            // reads are bound there without taking the aggregate apart, so a
            // guard that answers `false` leaves the scrutinee exactly as the
            // next arm expects to find it — no `Free`, nothing live, nothing
            // to unwind.
            let entry = match &arm.guard {
                None => arm_block,
                Some(guard) => {
                    let guard_block = self.block();
                    let previous = self.current;
                    self.current = guard_block;
                    self.bind_pattern_for_guard(&arm.pattern, scrutinee.local, borrowed);
                    let condition = self.expr(guard);
                    self.blocks[self.current].terminator = Some(match condition {
                        Some(condition) => Terminator::Branch {
                            condition: condition.local,
                            then_block: arm_block,
                            else_block: next_check,
                        },
                        None => Terminator::Goto(next_check),
                    });
                    self.current = previous;
                    guard_block
                }
            };
            self.branch_pattern(
                &arm.pattern,
                scrutinee.local,
                &pattern_type,
                entry,
                next_check,
            );
            self.current = arm_block;
            self.live = base_live.clone();
            self.scopes.push(Vec::new());
            let scope = self.scopes.len() - 1;
            self.consume_pattern(
                &arm.pattern,
                scrutinee.local,
                &pattern_type,
                scope,
                borrowed,
            );
            let arm_value = self.expr(&arm.body);
            if let (Some(result), Some(value)) = (result, arm_value.as_ref()) {
                self.emit(Instruction::Assign {
                    dst: result,
                    src: value.local,
                });
            }
            self.drop_scope_except(scope, arm_value.as_ref().map(|value| value.local));
            self.scopes.pop();
            self.blocks[self.current].terminator = Some(Terminator::Goto(merge_block));
            arm_states.push((self.current, self.live.clone()));
            self.current = next_check;
            if mir_pattern_irrefutable(&arm.pattern) && arm.guard.is_none() {
                break;
            }
        }
        if self.blocks[self.current].terminator.is_none() {
            self.blocks[self.current].terminator = Some(Terminator::Goto(merge_block));
        }

        self.live = base_live;
        let ids = self.live.keys().copied().rev().collect::<Vec<_>>();
        for id in ids {
            let survives_every_arm = arm_states
                .iter()
                .all(|(_, state)| state.get(&id).copied().unwrap_or(false));
            if !survives_every_arm {
                for (block, state) in &arm_states {
                    if state.get(&id).copied().unwrap_or(false) {
                        let local = self.bindings[&id];
                        let ty = self.locals[local].ty.clone();
                        if !ty.is_copy() {
                            self.emit_in(*block, Instruction::Drop { local, ty });
                        }
                    }
                }
                self.live.insert(id, false);
            }
        }
        self.current = merge_block;
        result.map(|local| Value {
            local,
            ty: expr.ty.clone(),
            owned_temporary: !expr.ty.is_copy(),
        })
    }

    fn drop_temporary(&mut self, value: Option<Value>) {
        if let Some(value) = value {
            if value.owned_temporary && !value.ty.is_copy() {
                self.emit(Instruction::Drop {
                    local: value.local,
                    ty: value.ty,
                });
            }
        }
    }

    fn drop_scope_except(&mut self, scope_index: usize, except: Option<LocalId>) {
        let ids = self.scopes.get(scope_index).cloned().unwrap_or_default();
        for id in ids.into_iter().rev() {
            let Some(local) = self.bindings.get(&id).copied() else {
                continue;
            };
            let Some(ty) = self.locals.get(local).map(|local| local.ty.clone()) else {
                continue;
            };
            if self.live.get(&id).copied().unwrap_or(false)
                && !ty.is_copy()
                && Some(local) != except
            {
                self.emit(Instruction::Drop { local, ty });
                self.live.insert(id, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Instruction, LocalId};
    use crate::ast::Type;
    use crate::{compile_to_mir, CompileOptions};

    #[test]
    fn lowers_branch_to_basic_blocks() {
        let source = "(fn main () -> i32 (if true 1 0))";
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        assert!(mir.functions[0].blocks.len() >= 4);
    }

    #[test]
    fn inserts_string_drop() {
        let source = r#"(fn show ((text (& String))) -> unit ())
            (fn main () -> i32 (let text "hello") (show (& text)) 0)"#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let has_drop = mir
            .functions
            .iter()
            .flat_map(|function| function.blocks.iter())
            .flat_map(|block| block.instructions())
            .any(|inst| {
                matches!(
                    inst,
                    Instruction::Drop {
                        ty: Type::String,
                        ..
                    }
                )
            });
        assert!(has_drop);
    }

    /// Assigning an owning field is a store and a drop, in that order (`D-120`).
    ///
    /// The order is what the test is for: dropping the old value before the new
    /// one is in the slot leaves the field holding a word nobody owns, which is
    /// the failure `replace` was shaped to avoid one level down (`D-103`).
    #[test]
    fn assigning_an_owning_field_stores_before_it_drops_the_old_value() {
        let source = r#"
            (struct Holder ((text String)))
            (fn rename ((holder (&mut Holder))) -> unit
              (match holder
                ((Holder :text text) (set text "after"))))
            (fn main () -> i32 0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let rename = mir
            .functions
            .iter()
            .find(|function| function.name.ends_with("rename"))
            .expect("the function is lowered");
        let written = rename
            .blocks
            .iter()
            .flat_map(|block| block.instructions())
            .filter(|instruction| {
                matches!(
                    instruction,
                    Instruction::FieldStore { index: 0, .. }
                        | Instruction::Drop {
                            ty: Type::String,
                            ..
                        }
                )
            })
            .collect::<Vec<_>>();
        assert!(
            matches!(
                written.as_slice(),
                [
                    Instruction::FieldStore { index: 0, .. },
                    Instruction::Drop {
                        ty: Type::String,
                        ..
                    }
                ]
            ),
            "{written:?}"
        );
    }

    /// A scalar field is written and nothing is dropped, because there is
    /// nothing in the slot that owns anything.
    #[test]
    fn assigning_a_scalar_field_costs_one_store() {
        let source = r#"
            (struct Counter ((count i64)))
            (fn bump ((counter (&mut Counter))) -> unit
              (match counter
                ((Counter :count count) (set count 1))))
            (fn main () -> i32 0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let bump = mir
            .functions
            .iter()
            .find(|function| function.name.ends_with("bump"))
            .expect("the function is lowered");
        let instructions = bump
            .blocks
            .iter()
            .flat_map(|block| block.instructions())
            .collect::<Vec<_>>();
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| matches!(instruction, Instruction::FieldStore { .. }))
                .count(),
            1,
            "{instructions:?}"
        );
        assert!(
            !instructions
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Drop { .. })),
            "{instructions:?}"
        );
    }

    #[test]
    fn cloning_a_binding_keeps_both_owned_values_live_for_drop() {
        let source = r#"
            (struct Pair ((left String) (right String)))
            (fn main () -> i32
              (let pair (Pair :left "left" :right "right"))
              (let copied (clone pair))
              0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let pair_drops = mir.functions[0]
            .blocks
            .iter()
            .flat_map(|block| block.instructions())
            .filter(|instruction| {
                matches!(
                    instruction,
                    Instruction::Drop {
                        ty: Type::Named(name),
                        ..
                    } if name == "Pair"
                )
            })
            .count();
        assert_eq!(pair_drops, 2);
    }

    #[test]
    fn merge_drops_are_emitted_in_a_deterministic_order() {
        // Two owned bindings diverge across an `if`, so both are dropped on the
        // then-branch merge. The order used to follow `HashMap` iteration and
        // differed between runs of the same compiler on the same input, which
        // made builds irreproducible.
        let source = r#"
            (fn take ((s String)) -> i32 0)
            (fn main () -> i32
              (let a "aaa")
              (let b "bbb")
              (if true (do (take a) (take b) 0) 0))
        "#;
        let expected: Vec<LocalId> = {
            let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
            drop_order(&mir)
        };
        assert_eq!(expected.len(), 2, "both bindings are dropped at the merge");
        for _ in 0..16 {
            let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
            assert_eq!(
                drop_order(&mir),
                expected,
                "lowering must not depend on hash iteration order"
            );
        }
    }

    fn drop_order(mir: &crate::mir::MirModule) -> Vec<LocalId> {
        let main = mir
            .functions
            .iter()
            .find(|function| function.name.contains("main"))
            .expect("main was lowered");
        // The merge block is the one carrying more than one drop.
        main.blocks
            .iter()
            .map(|block| {
                block
                    .instructions()
                    .filter_map(|instruction| match instruction {
                        Instruction::Drop { local, .. } => Some(*local),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .find(|drops| drops.len() > 1)
            .unwrap_or_default()
    }

    #[test]
    fn release_optimizer_folds_constants() {
        let source = "(fn main () -> i32 (+ 20 22))";
        let mir = compile_to_mir(
            "test.slp",
            source,
            &CompileOptions {
                optimize: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let block = &mir.functions[0].blocks[0];
        assert!(block
            .instructions()
            .any(|inst| matches!(inst, Instruction::ConstInt { value: 42, .. })));
        assert!(!block
            .instructions()
            .any(|inst| matches!(inst, Instruction::Binary { .. })));
    }

    #[test]
    fn statements_carry_the_span_of_the_expression_that_produced_them() {
        // `20` and `22` sit on different lines, so their statements must report
        // different lines. This is the data a DWARF line table will consume.
        let source = "(fn main () -> i32\n  (+ 20\n     22))";
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let lines = mir.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(statement.instruction, Instruction::ConstInt { .. }))
            .map(|statement| statement.span.line)
            .collect::<Vec<_>>();
        assert_eq!(lines, vec![2, 3], "each constant keeps its own line");
    }

    #[test]
    fn merge_drops_inherit_the_enclosing_expression_span() {
        // Drops inserted at a branch merge have no expression of their own.
        // They must still land on the enclosing `if`, not at offset zero.
        let source = r#"
            (fn take ((s String)) -> i32 0)
            (fn main () -> i32
              (let a "aaa")
              (if true (do (take a) 0) 0))
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let main = mir
            .functions
            .iter()
            .find(|function| function.name.contains("main"))
            .expect("main was lowered");
        let drops = main
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(statement.instruction, Instruction::Drop { .. }))
            .collect::<Vec<_>>();
        assert!(!drops.is_empty(), "the string must be dropped");
        for statement in drops {
            assert!(
                statement.span.line > 1,
                "a merge drop lost its span: {:?}",
                statement.span
            );
        }
    }

    #[test]
    fn folding_preserves_the_original_span() {
        let source = "(fn main () -> i32\n  (+ 20 22))";
        let mir = compile_to_mir(
            "test.slp",
            source,
            &CompileOptions {
                optimize: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        let folded = mir.functions[0].blocks[0]
            .statements
            .iter()
            .find(|statement| {
                matches!(
                    statement.instruction,
                    Instruction::ConstInt { value: 42, .. }
                )
            })
            .expect("the sum was folded");
        assert_eq!(
            folded.span.line, 2,
            "the folded constant keeps the span of the expression it replaced"
        );
    }
}
