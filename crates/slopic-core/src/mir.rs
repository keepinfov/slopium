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
        value: String,
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
    Free {
        local: LocalId,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Less,
    Greater,
    Equal,
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
        collect(lower_function(function));
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

#[derive(Clone, Copy)]
struct LoopTarget {
    continue_block: BlockId,
    break_block: BlockId,
    scope_depth: usize,
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
            TExprKind::Loop { body } => {
                self.lower_loop(None, body);
                None
            }
            TExprKind::While { condition, body } => {
                self.lower_loop(Some(condition), body);
                None
            }
            TExprKind::Break => {
                self.lower_loop_jump(false);
                None
            }
            TExprKind::Continue => {
                self.lower_loop_jump(true);
                None
            }
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
            TExprKind::Convert { value } => {
                let source = self.expr(value)?;
                let dst = self.temp(expr.ty.clone());
                self.emit(Instruction::Assign {
                    dst,
                    src: source.local,
                });
                Some(Value {
                    local: dst,
                    ty: expr.ty.clone(),
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
                    "<" => Some(BinaryOp::Less),
                    ">" => Some(BinaryOp::Greater),
                    "=" => Some(BinaryOp::Equal),
                    _ => None,
                };
                if let Some(op) = op {
                    self.emit(Instruction::Binary {
                        dst,
                        op,
                        lhs: lowered[0],
                        rhs: lowered[1],
                        ty: args[0].ty.clone(),
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

    fn lower_loop(&mut self, condition: Option<&TExpr>, body: &TExpr) {
        let condition_block = self.block();
        let body_block = self.block();
        let exit_block = self.block();
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
        });
        self.current = body_block;
        let value = self.expr(body);
        self.drop_temporary(value);
        if self.blocks[self.current].terminator.is_none() {
            self.blocks[self.current].terminator = Some(Terminator::Goto(condition_block));
        }
        self.loop_targets.pop();
        self.current = exit_block;
    }

    fn lower_loop_jump(&mut self, continuing: bool) {
        let Some(target) = self.loop_targets.last().copied() else {
            return;
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
                    self.consume_pattern(&field.pattern, local, &field.ty, scope, borrowed);
                }
                if !borrowed {
                    self.emit(Instruction::Free { local: value });
                }
            }
            TPattern::Struct { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    let local = self.pattern_field(value, index, &field.ty, false, borrowed);
                    self.consume_pattern(&field.pattern, local, &field.ty, scope, borrowed);
                }
                if !borrowed {
                    self.emit(Instruction::Free { local: value });
                }
            }
        }
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
            self.branch_pattern(
                &arm.pattern,
                scrutinee.local,
                &pattern_type,
                arm_block,
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
            if mir_pattern_irrefutable(&arm.pattern) {
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
