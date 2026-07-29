use crate::ast::Type;
use crate::diagnostic::Span;
use crate::sema::{
    BindingId, TExpr, TExprKind, TMatchArm, TPattern, TypedFunction, TypedProgram, TypedTest,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub type LocalId = usize;
pub type BlockId = usize;

#[derive(Clone, Debug, Serialize)]
pub struct MirModule {
    pub functions: Vec<MirFunction>,
    pub tests: Vec<MirTest>,
    pub structs: Vec<MirStruct>,
    pub enums: Vec<MirEnum>,
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
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
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
    MirModule {
        functions: program.functions.iter().map(lower_function).collect(),
        tests: program
            .tests
            .iter()
            .enumerate()
            .map(|(index, test)| lower_test(index, test))
            .collect(),
        structs: program
            .structs
            .iter()
            .map(|item| MirStruct {
                name: item.name.clone(),
                fields: item.fields.clone(),
                emit: true,
            })
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

pub fn optimize(module: &mut MirModule) {
    for function in module
        .functions
        .iter_mut()
        .chain(module.tests.iter_mut().map(|test| &mut test.function))
    {
        for block in &mut function.blocks {
            let mut constants = HashMap::<LocalId, Constant>::new();
            let mut optimized = Vec::with_capacity(block.instructions.len());
            for instruction in block.instructions.drain(..) {
                let replacement = match &instruction {
                    Instruction::ConstInt { dst, value } => {
                        constants.insert(*dst, Constant::Int(*value));
                        None
                    }
                    Instruction::ConstBool { dst, value } => {
                        constants.insert(*dst, Constant::Bool(*value));
                        None
                    }
                    Instruction::ConstFloat { dst, bits } => {
                        constants.insert(*dst, Constant::Float(*bits));
                        None
                    }
                    Instruction::Assign { dst, src } if dst == src => {
                        continue;
                    }
                    Instruction::Assign { dst, src } => {
                        if let Some(value) = constants.get(src).copied() {
                            constants.insert(*dst, value);
                        } else {
                            constants.remove(dst);
                        }
                        None
                    }
                    Instruction::Binary {
                        dst,
                        op,
                        lhs,
                        rhs,
                        ty,
                    } => {
                        let folded = fold_binary(
                            *op,
                            ty,
                            constants.get(lhs).copied(),
                            constants.get(rhs).copied(),
                        );
                        if let Some(value) = folded {
                            constants.insert(*dst, value);
                            Some(value.instruction(*dst))
                        } else {
                            constants.remove(dst);
                            None
                        }
                    }
                    Instruction::StringNew { dst, .. }
                    | Instruction::AddressOf { dst, .. }
                    | Instruction::Call { dst, .. }
                    | Instruction::StructNew { dst, .. }
                    | Instruction::FieldLoad { dst, .. }
                    | Instruction::EnumNew { dst, .. }
                    | Instruction::EnumTag { dst, .. }
                    | Instruction::EnumFieldLoad { dst, .. } => {
                        constants.remove(dst);
                        None
                    }
                    Instruction::Drop { .. } | Instruction::Free { .. } => None,
                };
                optimized.push(replacement.unwrap_or(instruction));
            }
            block.instructions = optimized;
        }
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

#[derive(Clone, Copy)]
enum Constant {
    Int(i64),
    Bool(bool),
    Float(u64),
}

impl Constant {
    fn instruction(self, dst: LocalId) -> Instruction {
        match self {
            Constant::Int(value) => Instruction::ConstInt { dst, value },
            Constant::Bool(value) => Instruction::ConstBool { dst, value },
            Constant::Float(bits) => Instruction::ConstFloat { dst, bits },
        }
    }
}

fn fold_binary(
    op: BinaryOp,
    ty: &Type,
    lhs: Option<Constant>,
    rhs: Option<Constant>,
) -> Option<Constant> {
    if *ty == Type::F64 {
        let (Constant::Float(lhs), Constant::Float(rhs)) = (lhs?, rhs?) else {
            return None;
        };
        let lhs = f64::from_bits(lhs);
        let rhs = f64::from_bits(rhs);
        return Some(match op {
            BinaryOp::Add => Constant::Float((lhs + rhs).to_bits()),
            BinaryOp::Sub => Constant::Float((lhs - rhs).to_bits()),
            BinaryOp::Mul => Constant::Float((lhs * rhs).to_bits()),
            BinaryOp::Div => Constant::Float((lhs / rhs).to_bits()),
            BinaryOp::Less => Constant::Bool(lhs < rhs),
            BinaryOp::Greater => Constant::Bool(lhs > rhs),
            BinaryOp::Equal => Constant::Bool(lhs == rhs),
        });
    }
    let integer = |value: Constant| match value {
        Constant::Int(value) => Some(value),
        Constant::Bool(value) => Some(i64::from(value)),
        Constant::Float(_) => None,
    };
    let lhs = integer(lhs?)?;
    let rhs = integer(rhs?)?;
    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs).map(Constant::Int),
        BinaryOp::Sub => lhs.checked_sub(rhs).map(Constant::Int),
        BinaryOp::Mul => lhs.checked_mul(rhs).map(Constant::Int),
        BinaryOp::Div => lhs.checked_div(rhs).map(Constant::Int),
        BinaryOp::Less => Some(Constant::Bool(lhs < rhs)),
        BinaryOp::Greater => Some(Constant::Bool(lhs > rhs)),
        BinaryOp::Equal => Some(Constant::Bool(lhs == rhs)),
    }?;
    if *ty == Type::I32 {
        if let Constant::Int(value) = value {
            i32::try_from(value)
                .ok()
                .map(|value| Constant::Int(i64::from(value)))
        } else {
            Some(value)
        }
    } else {
        Some(value)
    }
}

fn lower_function(function: &TypedFunction) -> MirFunction {
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
    let value = builder.expr(&function.body);
    builder.drop_scope_except(0, value.as_ref().map(|value| value.local));
    if builder.blocks[builder.current].terminator.is_none() {
        builder.blocks[builder.current].terminator = Some(Terminator::Return(
            value
                .filter(|value| value.ty != Type::Unit)
                .map(|value| value.local),
        ));
    }
    builder.finish()
}

fn lower_test(index: usize, test: &TypedTest) -> MirTest {
    let function = TypedFunction {
        name: format!("__slop_test_{index}"),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Type::Bool,
        body: test.body.clone(),
        span: test.span,
    };
    MirTest {
        name: test.name.clone(),
        function: lower_function(&function),
        emit: true,
    }
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
    instructions: Vec<Instruction>,
    terminator: Option<Terminator>,
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
                instructions: Vec::new(),
                terminator: None,
            }],
            current: 0,
            bindings: HashMap::new(),
            live: BTreeMap::new(),
            scopes: Vec::new(),
            loop_targets: Vec::new(),
        }
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
                    instructions: block.instructions,
                    terminator: block.terminator.unwrap_or(Terminator::Unreachable),
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
        self.blocks[self.current].instructions.push(instruction);
    }

    fn block(&mut self) -> BlockId {
        let id = self.blocks.len();
        self.blocks.push(BuilderBlock {
            instructions: Vec::new(),
            terminator: None,
        });
        id
    }

    fn expr(&mut self, expr: &TExpr) -> Option<Value> {
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
                } else {
                    self.emit(Instruction::Call {
                        dst,
                        callee: callee.clone(),
                        args: lowered.clone(),
                        arg_types: arg_types.clone(),
                        result: expr.ty.clone(),
                    });
                    if matches!(callee.as_str(), "print" | "println") {
                        for (local, ty) in lowered.into_iter().zip(arg_types) {
                            if ty == Type::String {
                                self.emit(Instruction::Drop { local, ty });
                            }
                        }
                    }
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
        for scope in self.scopes.iter().skip(target.scope_depth).rev() {
            for id in scope.iter().rev() {
                if !self.live.get(id).copied().unwrap_or(false) {
                    continue;
                }
                let local = self.bindings[id];
                let ty = self.locals[local].ty.clone();
                if !ty.is_copy() {
                    self.blocks[self.current]
                        .instructions
                        .push(Instruction::Drop { local, ty });
                }
            }
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
                    self.blocks[block]
                        .instructions
                        .push(Instruction::Drop { local, ty });
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

    fn consume_pattern(
        &mut self,
        pattern: &TPattern,
        value: LocalId,
        value_type: &Type,
        scope: usize,
    ) {
        match pattern {
            TPattern::Wildcard => {
                if !value_type.is_copy() {
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
                    let local = self.temp(field.ty.clone());
                    self.emit(Instruction::EnumFieldLoad {
                        dst: local,
                        base: value,
                        index,
                    });
                    self.consume_pattern(&field.pattern, local, &field.ty, scope);
                }
                self.emit(Instruction::Free { local: value });
            }
            TPattern::Struct { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    let local = self.temp(field.ty.clone());
                    self.emit(Instruction::FieldLoad {
                        dst: local,
                        base: value,
                        index,
                    });
                    self.consume_pattern(&field.pattern, local, &field.ty, scope);
                }
                self.emit(Instruction::Free { local: value });
            }
        }
    }

    fn lower_match(&mut self, expr: &TExpr, value: &TExpr, arms: &[TMatchArm]) -> Option<Value> {
        let scrutinee = self.expr(value)?;
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
                &value.ty,
                arm_block,
                next_check,
            );
            self.current = arm_block;
            self.live = base_live.clone();
            self.scopes.push(Vec::new());
            let scope = self.scopes.len() - 1;
            self.consume_pattern(&arm.pattern, scrutinee.local, &value.ty, scope);
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
                            self.blocks[*block]
                                .instructions
                                .push(Instruction::Drop { local, ty });
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
        let source = r#"(fn main () -> i32 (let text "hello") (println (& text)) 0)"#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let has_drop = mir.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
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
            .flat_map(|block| &block.instructions)
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
                    .instructions
                    .iter()
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
        let instructions = &mir.functions[0].blocks[0].instructions;
        assert!(instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::ConstInt { value: 42, .. })));
        assert!(!instructions
            .iter()
            .any(|inst| matches!(inst, Instruction::Binary { .. })));
    }
}
