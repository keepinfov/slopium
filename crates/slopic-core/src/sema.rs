use crate::ast::{
    Annotation, Capture, Expr, ExprKind, Function, LogicalOp, Param, Pattern, PatternKind, Program,
    Type,
};
use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};
use serde::Serialize;
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};

pub type BindingId = u32;

#[derive(Clone, Debug, Serialize)]
pub struct TypedProgram {
    pub functions: Vec<TypedFunction>,
    pub externs: Vec<TypedExtern>,
    pub tests: Vec<TypedTest>,
    pub structs: Vec<TypedStruct>,
    pub enums: Vec<TypedEnum>,
}

/// A checked `extern`: the Slopium name calls use, and the C symbol they reach.
#[derive(Clone, Debug, Serialize)]
pub struct TypedExtern {
    pub name: String,
    pub symbol: String,
    pub params: Vec<Type>,
    pub result: Type,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedStruct {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<(String, Type)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedEnum {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<TypedVariant>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedVariant {
    pub name: String,
    pub tag: usize,
    pub fields: Vec<(String, Type)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedFunction {
    pub name: String,
    /// Whether this function was annotated `inline` (`D-122`).
    ///
    /// It is a hint and nothing else: everything that makes inlining sound is
    /// decided by `opt::inline`, and what this moves is the size at which a
    /// body stops being worth copying.
    pub inline: bool,
    pub type_params: Vec<String>,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: TExpr,
    pub span: Span,
}

/// One name a `lambda` moved into its environment.
#[derive(Clone, Debug, Serialize)]
pub struct TCapture {
    /// The binding the value was taken from, in the enclosing function.
    pub from: BindingId,
    /// The binding it is inside the body, which is a different one holding the
    /// same value under the same name.
    pub id: BindingId,
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedParam {
    pub id: BindingId,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypedTest {
    pub name: String,
    pub body: TExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct TExpr {
    pub kind: TExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum TExprKind {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// A text literal's bytes (see `lexer::TokenKind::String`).
    String(Vec<u8>),
    Var(BindingId),
    Let {
        id: BindingId,
        name: String,
        mutable: bool,
        value: Box<TExpr>,
    },
    Set {
        id: BindingId,
        value: Box<TExpr>,
    },
    Do(Vec<TExpr>),
    If {
        condition: Box<TExpr>,
        then_expr: Box<TExpr>,
        else_expr: Box<TExpr>,
    },
    Loop {
        body: Box<TExpr>,
    },
    While {
        condition: Box<TExpr>,
        body: Box<TExpr>,
    },
    Break(Option<Box<TExpr>>),
    Continue,
    Match {
        value: Box<TExpr>,
        arms: Vec<TMatchArm>,
    },
    Borrow {
        id: BindingId,
        mutable: bool,
    },
    /// A widening between two numeric types, from `(as i64 value)`.
    ///
    /// The source type is the value's own; the destination is this
    /// expression's. Only the pairs `D-090` allows survive sema.
    Convert {
        value: Box<TExpr>,
    },
    Call {
        callee: String,
        args: Vec<TExpr>,
    },
    /// A top-level `fn` named where a value is expected (`D-092`).
    ///
    /// `type_args` is empty for a concrete function and names the instance for
    /// a generic one, exactly as `GenericCall` does — `specialize_expr` reads
    /// it to enqueue the monomorphization, which is the only path to the queue.
    FnRef {
        name: String,
        type_args: Vec<Type>,
    },

    /// A call through a local of `Fn` type.
    ///
    /// The callee is a binding rather than a name, which is the whole
    /// difference from `Call`: there is no symbol until run time.
    CallValue {
        callee: BindingId,
        args: Vec<TExpr>,
    },
    /// A `lambda` and what it closes over (`D-102`).
    ///
    /// The body is lowered to a function of its own and the captures become
    /// the block that function is handed, so this is the last place the two are
    /// one expression.
    Lambda {
        captures: Vec<TCapture>,
        params: Vec<TypedParam>,
        result: Type,
        body: Box<TExpr>,
    },
    GenericCall {
        callee: String,
        type_args: Vec<Type>,
        args: Vec<TExpr>,
    },
    Try {
        value: Box<TExpr>,
        ok_type: Type,
        enum_name: String,
        ok_tag: usize,
        err_tag: usize,
    },
    StructInit {
        name: String,
        fields: Vec<TExpr>,
    },
    EnumInit {
        enum_name: String,
        variant: String,
        tag: usize,
        fields: Vec<TExpr>,
    },
    Field {
        base: BindingId,
        struct_name: String,
        index: usize,
    },
    /// A module-level `const` named where a value is expected (`D-121`).
    ///
    /// The value is already the literal the declaration held, so this node is
    /// the inlining and not a step before it. What the wrapper keeps is the
    /// name, for the same reason `FnRef` does: hover and go-to-definition
    /// follow a top-level reference, and a bare literal is not one.
    Const {
        name: String,
        value: Box<TExpr>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct TMatchArm {
    pub pattern: TPattern,
    /// The `when` condition, typed as `bool` (`D-121`).
    pub guard: Option<TExpr>,
    pub body: TExpr,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum TPattern {
    Wildcard,
    Binding(TPatternBinding),
    Bool(bool),
    Int(i64),
    Enum {
        enum_name: String,
        variant: String,
        tag: usize,
        fields: Vec<TPatternField>,
    },
    Struct {
        struct_name: String,
        fields: Vec<TPatternField>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct TPatternField {
    pub ty: Type,
    pub pattern: TPattern,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct TPatternBinding {
    pub id: BindingId,
    pub name: String,
    pub ty: Type,
}

#[derive(Clone, Debug)]
struct Signature {
    type_params: Vec<String>,
    params: Vec<Type>,
    result: Type,
    /// Whether this name is an `extern` rather than a `fn`.
    ///
    /// Calling one is an ordinary call, which is why they share a table. Taking
    /// one as a value is not: an `extern` argument expands to more than one
    /// machine word at the boundary (a borrowed `Slice` is pointer and length),
    /// so a `Fn` type would describe a shape the call does not have.
    is_extern: bool,
    deprecated: Option<Deprecation>,
}

/// What a `deprecated` annotation said about a declaration (`D-122`).
///
/// The message is optional because the annotation is: `(deprecated)` says the
/// name is going away and `(deprecated "use `parse-line`")` says where to go
/// instead, and a warning that names the declaration is worth having either
/// way.
#[derive(Clone, Debug)]
struct Deprecation {
    message: Option<String>,
}

/// The `deprecated` a declaration carries, if it carries one.
fn deprecation(annotations: &[Annotation]) -> Option<Deprecation> {
    annotations
        .iter()
        .find(|annotation| annotation.name == "deprecated")
        .map(|annotation| Deprecation {
            message: annotation.text().map(str::to_owned),
        })
}

#[derive(Clone, Debug)]
struct VariantInfo {
    enum_name: String,
    type_params: Vec<String>,
    variant: String,
    tag: usize,
    fields: Vec<(String, Type)>,
}

#[derive(Clone, Debug)]
struct GenericStructInfo {
    type_params: Vec<String>,
    fields: Vec<(String, Type)>,
}

#[derive(Clone, Debug)]
struct GenericEnumInfo {
    type_params: Vec<String>,
    variants: Vec<TypedVariant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnershipState {
    Available,
    Moved,
    SharedBorrowed(u32),
    MutBorrowed,
}

/// The borrow a `match` is looking through while its arms are typed.
#[derive(Clone, Copy, Debug)]
struct PatternBorrow {
    /// The binding the scrutinee's borrow came from, when it had one.
    origin: Option<BindingId>,
    /// Whether it is exclusive, which is what makes the names underneath it
    /// places rather than values that can only be read (`D-120`).
    mutable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectionKind {
    List,
    Array,
    Slice,
}

/// A loop being typed, and what it has been told to produce (`D-121`).
#[derive(Clone, Debug)]
struct LoopFrame {
    /// A `while` may end by its condition, where there is no value to hand
    /// back, so `(break value)` belongs to a `loop` alone.
    conditional: bool,
    /// The type every `break` in this loop agrees on, once one has been seen or
    /// the context asked for one.
    result: Option<Type>,
}

#[derive(Clone, Debug)]
struct Binding {
    id: BindingId,
    ty: Type,
    mutable: bool,
    state: OwnershipState,
    definition: Span,
    borrowed_from: Option<BindingId>,
    owns_loan: bool,
    /// Whether this name is a `lambda`'s capture, which the environment owns
    /// and therefore nothing may move back out (`D-102`).
    captured: bool,
    /// Whether this name is a field an exclusive pattern bound, and therefore a
    /// place `set` can write through (`D-120`).
    ///
    /// A `(&mut T)` parameter is not one: the function was handed a borrow and
    /// not the aggregate it came from, so there is no field here to name.
    place: bool,
}

#[derive(Clone, Debug, Default)]
struct Scope {
    names: HashMap<String, BindingId>,
    loans: Vec<BindingId>,
    /// Bindings a later `let` of the same name displaced (`D-121`).
    ///
    /// Shadowing takes the name away and not the value: the first `x` is still
    /// alive until the scope ends and is still dropped there, so the id has to
    /// outlive the entry in `names` that pointed at it.
    shadowed: Vec<BindingId>,
}

#[derive(Clone, Debug, Default)]
struct Environment {
    scopes: Vec<Scope>,
    bindings: HashMap<BindingId, Binding>,
}

impl Environment {
    fn push(&mut self) {
        self.scopes.push(Scope::default());
    }

    fn pop(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            // Each entry in `loans` is one outstanding borrow of that origin, so
            // release them one at a time. Overwriting the state instead would
            // cancel loans still held by enclosing scopes.
            for id in scope.loans {
                self.release_loan(id);
            }
            for id in scope.names.values().chain(scope.shadowed.iter()) {
                self.bindings.remove(id);
            }
        }
    }

    fn insert(&mut self, name: String, binding: Binding) -> Result<(), ()> {
        let Some(scope) = self.scopes.last_mut() else {
            return Err(());
        };
        if scope.names.contains_key(&name) {
            return Err(());
        }
        scope.names.insert(name, binding.id);
        self.bindings.insert(binding.id, binding);
        Ok(())
    }

    /// Binds `name`, displacing whatever it named in this scope (`D-121`).
    ///
    /// This is `let` alone. A rebind in the same scope was refused until
    /// v0.9.1 while a rebind in a nested one was not, which was a split nobody
    /// had decided; it is one rule now, and the rule is the permissive one
    /// because allowing this later would have been compatible and forbidding
    /// it later would not. A parameter, a capture and a pattern binding still
    /// go through `insert`, because each of those is a list the author wrote
    /// twice rather than a name they reused.
    fn rebind(&mut self, name: String, binding: Binding) {
        let Some(scope) = self.scopes.last_mut() else {
            return;
        };
        if let Some(previous) = scope.names.insert(name, binding.id) {
            scope.shadowed.push(previous);
        }
        self.bindings.insert(binding.id, binding);
    }

    fn name_of(&self, id: BindingId) -> Option<String> {
        self.scopes.iter().rev().find_map(|scope| {
            scope
                .names
                .iter()
                .find(|(_, candidate)| **candidate == id)
                .map(|(name, _)| name.clone())
        })
    }

    fn lookup_id(&self, name: &str) -> Option<BindingId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).copied())
    }

    fn add_loan(&mut self, id: BindingId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.loans.push(id);
        }
    }

    /// Release a loan before its scope ends, forgetting the record so that
    /// `pop` does not release the same borrow a second time.
    fn discharge_loan(&mut self, id: BindingId) {
        let recorded = self.scopes.iter_mut().rev().find_map(|scope| {
            scope
                .loans
                .iter()
                .rposition(|loan| *loan == id)
                .map(|index| {
                    scope.loans.remove(index);
                })
        });
        if recorded.is_some() {
            self.release_loan(id);
        }
    }

    fn release_loan(&mut self, id: BindingId) {
        if let Some(binding) = self.bindings.get_mut(&id) {
            binding.state = match binding.state {
                OwnershipState::SharedBorrowed(count) if count > 1 => {
                    OwnershipState::SharedBorrowed(count - 1)
                }
                OwnershipState::SharedBorrowed(_) | OwnershipState::MutBorrowed => {
                    OwnershipState::Available
                }
                state => state,
            };
        }
    }

    fn release_dead_references(&mut self, live_names: &HashSet<String>) {
        let visible = self
            .scopes
            .iter()
            .flat_map(|scope| scope.names.iter())
            .map(|(name, id)| (name.clone(), *id))
            .collect::<Vec<_>>();
        let dead = visible
            .iter()
            .filter_map(|(name, id)| {
                let binding = self.bindings.get(id)?;
                (binding.borrowed_from.is_some() && !live_names.contains(name)).then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in dead {
            let Some(binding) = self.bindings.get(&id).cloned() else {
                continue;
            };
            let Some(origin) = binding.borrowed_from else {
                continue;
            };
            if binding.owns_loan {
                let successor = visible.iter().find_map(|(name, other)| {
                    let candidate = self.bindings.get(other)?;
                    (*other != id
                        && live_names.contains(name)
                        && candidate.borrowed_from == Some(origin))
                    .then_some(*other)
                });
                if let Some(successor) = successor {
                    if let Some(candidate) = self.bindings.get_mut(&successor) {
                        candidate.owns_loan = true;
                    }
                } else {
                    self.discharge_loan(origin);
                }
            }
            if let Some(binding) = self.bindings.get_mut(&id) {
                binding.borrowed_from = None;
                binding.owns_loan = false;
            }
        }
    }
}

pub fn analyze(file: &str, program: &Program) -> CompileResult<TypedProgram> {
    analyze_with_options(
        file,
        program,
        &crate::LanguageItems::default(),
        true,
        &mut Vec::new(),
    )
}

/// Checks a program, appending to `warnings` what it has to say about one that
/// compiles (`D-122`).
///
/// The sink is a parameter rather than a field of the result because a warning
/// belongs to the *compilation* and not to the program: which of them a run
/// reports depends on what it was asked to build, and that is a decision only
/// the caller can make.
pub fn analyze_with_options(
    file: &str,
    program: &Program,
    language_items: &crate::LanguageItems,
    validate_entry_point: bool,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<TypedProgram> {
    Analyzer::new(file, program, language_items, validate_entry_point).analyze(program, warnings)
}

struct Analyzer<'a> {
    file: &'a str,
    signatures: HashMap<String, Signature>,
    structs: HashMap<String, Vec<(String, Type)>>,
    generic_structs: HashMap<String, GenericStructInfo>,
    generic_enums: HashMap<String, GenericEnumInfo>,
    generated_structs: HashMap<String, TypedStruct>,
    generated_enums: HashMap<String, TypedEnum>,
    struct_instances: HashMap<String, (String, Vec<Type>)>,
    enum_instances: HashMap<String, (String, Vec<Type>)>,
    normalizing_types: HashSet<String>,
    variants: HashMap<String, VariantInfo>,
    declared_types: HashSet<String>,
    type_arities: HashMap<String, usize>,
    active_type_params: HashSet<String>,
    language_items: crate::LanguageItems,
    validate_entry_point: bool,
    current_return_type: Option<Type>,
    /// One frame per loop being typed, innermost last (`D-121`).
    ///
    /// It replaces the depth counter a bare `break` needed, because a `break`
    /// with a value has to know two more things: whether the loop it leaves can
    /// produce one at all, and what the loop's earlier breaks already decided
    /// it is.
    loops: Vec<LoopFrame>,
    /// The module-level constants, by canonical name (`D-121`).
    ///
    /// Each is the literal the declaration held, already typed, so a use is a
    /// clone of it rather than a lookup anything downstream performs.
    consts: HashMap<String, TExpr>,
    /// The `deprecated` a `const` carries, kept beside `consts` rather than in
    /// it: a use of an ordinary constant should not pay a word for it.
    deprecated_consts: HashMap<String, Deprecation>,
    /// One entry per `when` guard being typed, holding the first binding id
    /// that did not exist when the guard started (`D-121`).
    ///
    /// A guard runs before its arm is taken, so moving out of a name the guard
    /// found already there would consume a value the next arm still matches
    /// against. A name the guard *made* is its own, which is what the floor
    /// distinguishes: ids only ever go up.
    guards: Vec<BindingId>,
    /// How many `unsafe` blocks enclose the expression being typed (`D-067`).
    ///
    /// A raw-pointer operation is refused at zero. It is a depth rather than a
    /// flag because the blocks nest, and it is reset to zero — not decremented
    /// — while a `lambda` body is typed: the body is a separate function that
    /// can be called from anywhere, so the permission its `lambda` was written
    /// inside of does not travel into it.
    unsafe_depth: usize,
    diagnostics: Vec<Diagnostic>,
    /// What the compiler has to say about a program that compiles anyway.
    warnings: Vec<Diagnostic>,
    next_binding: BindingId,
    /// Where each binding was consumed, so a loop can point at the move that
    /// its next iteration would repeat.
    move_sites: HashMap<BindingId, Span>,
    /// Set while the arms of a `match` whose scrutinee is a borrow are typed
    /// (`D-099`), holding the borrow's origin and whether it is exclusive.
    ///
    /// Every binding a pattern makes underneath this is a borrow of the field it
    /// names rather than the field itself, at every depth, which is why it is
    /// state on the analyzer and not an argument: `type_pattern` recurses
    /// through struct and enum fields and the answer never changes on the way
    /// down.
    pattern_borrow: Option<PatternBorrow>,
    /// How many aggregate patterns deep `type_pattern` currently is.
    ///
    /// A name bound at depth zero is the whole scrutinee and names no field, so
    /// it is a borrow like any other and never a place (`D-120`).
    pattern_depth: usize,
    /// The environments a `lambda` body is written inside of, while it is being
    /// typed.
    ///
    /// The body cannot see them — that is what `D-102` is — but a name it
    /// forgot to capture is looked up here anyway, so that the diagnostic can
    /// tell "you did not capture this" from "there is no such name".
    enclosing: Vec<Environment>,
    env: Environment,
}

impl<'a> Analyzer<'a> {
    fn new(
        file: &'a str,
        program: &Program,
        language_items: &crate::LanguageItems,
        validate_entry_point: bool,
    ) -> Self {
        let mut declared_types = HashSet::new();
        declared_types.extend(program.structs.iter().map(|item| item.name.clone()));
        declared_types.extend(program.enums.iter().map(|item| item.name.clone()));
        let type_arities = program
            .structs
            .iter()
            .map(|item| (item.name.clone(), item.type_params.len()))
            .chain(
                program
                    .enums
                    .iter()
                    .map(|item| (item.name.clone(), item.type_params.len())),
            )
            .collect();
        let structs = program
            .structs
            .iter()
            .filter(|item| item.type_params.is_empty())
            .map(|item| {
                (
                    item.name.clone(),
                    item.fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone()))
                        .collect(),
                )
            })
            .collect();
        let generic_structs = program
            .structs
            .iter()
            .filter(|item| !item.type_params.is_empty())
            .map(|item| {
                (
                    item.name.clone(),
                    GenericStructInfo {
                        type_params: item.type_params.clone(),
                        fields: item
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.ty.clone()))
                            .collect(),
                    },
                )
            })
            .collect();
        let generic_enums = program
            .enums
            .iter()
            .filter(|item| !item.type_params.is_empty())
            .map(|item| {
                (
                    item.name.clone(),
                    GenericEnumInfo {
                        type_params: item.type_params.clone(),
                        variants: item
                            .variants
                            .iter()
                            .enumerate()
                            .map(|(tag, variant)| TypedVariant {
                                name: variant.name.clone(),
                                tag,
                                fields: variant
                                    .fields
                                    .iter()
                                    .map(|field| (field.name.clone(), field.ty.clone()))
                                    .collect(),
                            })
                            .collect(),
                    },
                )
            })
            .collect();
        let variants = program
            .enums
            .iter()
            .flat_map(|item| {
                item.variants.iter().enumerate().map(move |(tag, variant)| {
                    (
                        format!("{}:{}", item.name, variant.name),
                        VariantInfo {
                            enum_name: item.name.clone(),
                            type_params: item.type_params.clone(),
                            variant: variant.name.clone(),
                            tag,
                            fields: variant
                                .fields
                                .iter()
                                .map(|field| (field.name.clone(), field.ty.clone()))
                                .collect(),
                        },
                    )
                })
            })
            .collect();
        Self {
            file,
            signatures: HashMap::new(),
            structs,
            generic_structs,
            generic_enums,
            generated_structs: HashMap::new(),
            generated_enums: HashMap::new(),
            struct_instances: HashMap::new(),
            enum_instances: HashMap::new(),
            normalizing_types: HashSet::new(),
            variants,
            declared_types,
            type_arities,
            active_type_params: HashSet::new(),
            language_items: language_items.clone(),
            validate_entry_point,
            current_return_type: None,
            loops: Vec::new(),
            consts: HashMap::new(),
            deprecated_consts: HashMap::new(),
            guards: Vec::new(),
            unsafe_depth: 0,
            diagnostics: Vec::new(),
            warnings: Vec::new(),
            next_binding: 0,
            move_sites: HashMap::new(),
            pattern_borrow: None,
            pattern_depth: 0,
            enclosing: Vec::new(),
            env: Environment::default(),
        }
    }

    /// Types every module-level `const` before anything can name one
    /// (`D-121`).
    ///
    /// The value is a literal, so this is the only place it is ever typed and
    /// a use is a copy of the result. Order does not matter: a `const` cannot
    /// mention another one, which is exactly the property that lets this be a
    /// single pass with no dependency between its entries.
    fn collect_consts(&mut self, program: &Program) {
        self.env.push();
        for constant in &program.consts {
            if self.signatures.contains_key(&constant.name) {
                self.error(
                    constant.span,
                    format!("`{}` is already a function", constant.name),
                );
                continue;
            }
            let written = constant.ty.as_ref().map(|ty| {
                self.validate_type(ty, constant.span);
                self.normalize_type(ty, constant.span)
            });
            let value = self.expr(&constant.value, written.as_ref());
            if let Some(deprecation) = deprecation(&constant.annotations) {
                self.deprecated_consts
                    .insert(constant.name.clone(), deprecation);
            }
            if self.consts.insert(constant.name.clone(), value).is_some() {
                self.error(
                    constant.span,
                    format!("`{}` is defined more than once", constant.name),
                );
            }
        }
        self.env.pop();
    }

    fn analyze(
        mut self,
        program: &Program,
        warnings: &mut Vec<Diagnostic>,
    ) -> CompileResult<TypedProgram> {
        self.collect_signatures(program);
        self.collect_consts(program);
        self.validate_declarations(program);
        let mut functions = Vec::new();
        for function in &program.functions {
            if let Some(typed) = self.function(function) {
                functions.push(typed);
            }
        }
        let mut tests = Vec::new();
        for test in &program.tests {
            self.env = Environment::default();
            self.env.push();
            let body = self.expr(&test.body, Some(&Type::Bool));
            self.env.pop();
            tests.push(TypedTest {
                name: test.name.clone(),
                body,
                span: test.span,
            });
        }

        let mut functions = match monomorphize_functions(functions, &mut tests) {
            Ok(functions) => functions,
            Err(message) => {
                self.error_with_code(codes::GENERIC, Span::default(), message);
                Vec::new()
            }
        };
        self.active_type_params.clear();
        for function in &mut functions {
            for parameter in &mut function.params {
                parameter.ty = self.normalize_type(&parameter.ty, parameter.span);
            }
            function.return_type = self.normalize_type(&function.return_type, function.span);
            self.materialize_typed_expr(&mut function.body);
        }
        for test in &mut tests {
            self.materialize_typed_expr(&mut test.body);
        }
        let mut structs = Vec::new();
        for item in program
            .structs
            .iter()
            .filter(|item| item.type_params.is_empty())
        {
            let mut fields = Vec::new();
            for field in &item.fields {
                fields.push((
                    field.name.clone(),
                    self.normalize_type(&field.ty, field.span),
                ));
            }
            structs.push(TypedStruct {
                name: item.name.clone(),
                type_params: Vec::new(),
                fields,
            });
        }
        let mut enums = Vec::new();
        for item in program
            .enums
            .iter()
            .filter(|item| item.type_params.is_empty())
        {
            let mut variants = Vec::new();
            for (tag, variant) in item.variants.iter().enumerate() {
                let mut fields = Vec::new();
                for field in &variant.fields {
                    fields.push((
                        field.name.clone(),
                        self.normalize_type(&field.ty, field.span),
                    ));
                }
                variants.push(TypedVariant {
                    name: variant.name.clone(),
                    tag,
                    fields,
                });
            }
            enums.push(TypedEnum {
                name: item.name.clone(),
                type_params: Vec::new(),
                variants,
            });
        }
        structs.extend(self.generated_structs.values().cloned());
        enums.extend(self.generated_enums.values().cloned());
        structs.sort_by(|left, right| left.name.cmp(&right.name));
        enums.sort_by(|left, right| left.name.cmp(&right.name));

        let externs = program
            .externs
            .iter()
            .filter_map(|declaration| {
                let signature = self.signatures.get(&declaration.name)?;
                Some(TypedExtern {
                    name: declaration.name.clone(),
                    symbol: declaration.symbol.clone(),
                    params: signature.params.clone(),
                    result: signature.result.clone(),
                })
            })
            .collect();

        if self.diagnostics.is_empty() {
            warnings.append(&mut self.warnings);
            Ok(TypedProgram {
                functions,
                externs,
                tests,
                structs,
                enums,
            })
        } else {
            Err(self.diagnostics)
        }
    }

    fn collect_signatures(&mut self, program: &Program) {
        for function in &program.functions {
            if self.signatures.contains_key(&function.name) {
                self.error(
                    function.span,
                    format!("function `{}` is defined more than once", function.name),
                );
                continue;
            }
            self.active_type_params = function.type_params.iter().cloned().collect();
            let params = function
                .params
                .iter()
                .map(|param| self.normalize_type(&param.ty, param.span))
                .collect();
            let result = self.normalize_type(&function.return_type, function.span);
            self.signatures.insert(
                function.name.clone(),
                Signature {
                    type_params: function.type_params.clone(),
                    params,
                    result,
                    is_extern: false,
                    deprecated: deprecation(&function.annotations),
                },
            );
        }
        for declaration in &program.externs {
            if self.signatures.contains_key(&declaration.name) {
                self.error(
                    declaration.span,
                    format!("`{}` is defined more than once", declaration.name),
                );
                continue;
            }
            self.active_type_params.clear();
            let params = declaration
                .params
                .iter()
                .map(|param| self.normalize_type(&param.ty, param.span))
                .collect();
            let result = self.normalize_type(&declaration.return_type, declaration.span);
            // An `extern` shares the function namespace, so a call to one is an
            // ordinary call the moment its signature is here. It needs no
            // ownership rule of its own either: every type the C boundary
            // accepts is a scalar or a shared borrow (`D-065`), so there is no
            // argument a call site could move in the first place.
            self.signatures.insert(
                declaration.name.clone(),
                Signature {
                    type_params: Vec::new(),
                    params,
                    result,
                    is_extern: true,
                    deprecated: deprecation(&declaration.annotations),
                },
            );
        }
        if self.validate_entry_point {
            match self.signatures.get("main") {
                Some(signature)
                    if signature.params.is_empty()
                        && signature.type_params.is_empty()
                        && matches!(signature.result, Type::I32 | Type::I64 | Type::Unit) => {}
                Some(_) => self.error_with_code(
                    codes::ENTRY_POINT,
                    Span::default(),
                    "`main` must have signature `(fn main () -> i32 ...)`, `i64`, or `unit`",
                ),
                None if program.tests.is_empty() => self.error_with_code(
                    codes::ENTRY_POINT,
                    Span::default(),
                    "program does not define `main`",
                ),
                None => {}
            }
        }
    }

    fn validate_declarations(&mut self, program: &Program) {
        let mut names = HashSet::new();
        for decl in &program.structs {
            self.active_type_params = decl.type_params.iter().cloned().collect();
            if !names.insert(decl.name.clone()) {
                self.error(
                    decl.span,
                    format!("type `{}` is defined more than once", decl.name),
                );
            }
            self.validate_fields(&decl.fields);
        }
        for decl in &program.enums {
            self.active_type_params = decl.type_params.iter().cloned().collect();
            if !names.insert(decl.name.clone()) {
                self.error(
                    decl.span,
                    format!("type `{}` is defined more than once", decl.name),
                );
            }
            let mut variants = HashSet::new();
            for variant in &decl.variants {
                if !variants.insert(variant.name.clone()) {
                    self.error(
                        variant.span,
                        format!("variant `{}` is defined more than once", variant.name),
                    );
                }
                self.validate_fields(&variant.fields);
            }
        }
        for function in &program.functions {
            self.active_type_params = function.type_params.iter().cloned().collect();
            for param in &function.params {
                self.validate_type(&param.ty, param.span);
            }
            self.validate_type(&function.return_type, function.span);
            if contains_borrowed_type(&function.return_type) {
                self.error_with_code(
                    codes::OWNERSHIP,
                    function.span,
                    "borrowed values cannot be returned from functions",
                );
            }
        }
        self.active_type_params.clear();
        for declaration in &program.externs {
            for param in &declaration.params {
                self.validate_type(&param.ty, param.span);
                let ty = self.normalize_type(&param.ty, param.span);
                if !extern_parameter_is_expressible(&ty) {
                    let diagnostic = Diagnostic::error(
                        codes::NAME_OR_TYPE,
                        self.file,
                        param.span,
                        format!(
                            "`{ty}` cannot cross the C boundary as the parameter `{}`",
                            param.name
                        ),
                    )
                    .with_help(EXTERN_PARAMETER_HELP);
                    self.diagnostics.push(diagnostic);
                }
            }
            self.validate_type(&declaration.return_type, declaration.span);
            let result = self.normalize_type(&declaration.return_type, declaration.span);
            if !extern_result_is_expressible(&result) {
                let diagnostic = Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    declaration.span,
                    format!("`{result}` cannot cross the C boundary as a return type"),
                )
                .with_help(EXTERN_RESULT_HELP);
                self.diagnostics.push(diagnostic);
            }
        }
    }

    fn validate_fields(&mut self, fields: &[crate::ast::Param]) {
        let mut names = HashSet::new();
        for field in fields {
            if !names.insert(field.name.clone()) {
                self.error(
                    field.span,
                    format!("field `{}` is defined more than once", field.name),
                );
            }
            self.validate_type(&field.ty, field.span);
            if contains_borrowed_type(&field.ty) {
                self.error_with_code(
                    codes::OWNERSHIP,
                    field.span,
                    "borrowed values cannot be stored in aggregate fields",
                );
            }
        }
    }

    fn validate_type(&mut self, ty: &Type, span: Span) {
        match ty {
            Type::Named(name)
                if !self.declared_types.contains(name)
                    && !self.active_type_params.contains(name) =>
            {
                self.error(span, format!("unknown type `{name}`"));
            }
            Type::List(inner) | Type::Slice(inner) | Type::Ref { inner, .. } => {
                self.validate_type(inner, span);
            }
            Type::Array { element, length } => {
                if *length == 0 {
                    self.error(span, "array length must be greater than zero");
                }
                self.validate_type(element, span);
            }
            Type::Apply { name, args } => {
                match self.type_arities.get(name) {
                    Some(arity) if *arity == args.len() => {}
                    Some(arity) => self.error(
                        span,
                        format!(
                            "generic type `{name}` expects {arity} arguments, found {}",
                            args.len()
                        ),
                    ),
                    None => self.error(span, format!("unknown generic type `{name}`")),
                }
                for argument in args {
                    self.validate_type(argument, span);
                }
            }
            Type::Fn { params, result } => {
                for param in params {
                    self.validate_type(param, span);
                }
                self.validate_type(result, span);
            }
            // The pointee is refused here rather than at the access, so a
            // program that writes `(Ptr String)` is told about the type it
            // wrote instead of about the read it tried to do with it
            // (`D-067`). A type parameter is not scalar either, so `(Ptr T)`
            // in a generic is refused too: allowing it later is additive,
            // and monomorphizing one into a `(Ptr String)` is not.
            Type::Ptr(inner) => {
                self.validate_type(inner, span);
                if !inner.is_scalar() {
                    self.error(span, format!("a raw pointer cannot point at `{inner}`"));
                }
            }
            _ => {}
        }
    }

    fn normalize_type(&mut self, ty: &Type, span: Span) -> Type {
        match ty {
            Type::List(inner) => Type::List(Box::new(self.normalize_type(inner, span))),
            Type::Array { element, length } => Type::Array {
                element: Box::new(self.normalize_type(element, span)),
                length: *length,
            },
            Type::Slice(inner) => Type::Slice(Box::new(self.normalize_type(inner, span))),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.normalize_type(inner, span))),
            Type::Ref { mutable, inner } => Type::Ref {
                mutable: *mutable,
                inner: Box::new(self.normalize_type(inner, span)),
            },
            Type::Fn { params, result } => Type::Fn {
                params: params
                    .iter()
                    .map(|param| self.normalize_type(param, span))
                    .collect(),
                result: Box::new(self.normalize_type(result, span)),
            },
            Type::Apply { name, args } => {
                let args = args
                    .iter()
                    .map(|argument| self.normalize_type(argument, span))
                    .collect::<Vec<_>>();
                if args
                    .iter()
                    .any(|argument| contains_parameter(argument, &self.active_type_params))
                {
                    return Type::Apply {
                        name: name.clone(),
                        args,
                    };
                }
                let instance_name = generic_instance_name(name, &args);
                if self.structs.contains_key(&instance_name)
                    || self.generated_enums.contains_key(&instance_name)
                    || self.normalizing_types.contains(&instance_name)
                {
                    return Type::Named(instance_name);
                }
                if let Some(info) = self.generic_structs.get(name).cloned() {
                    if info.type_params.len() != args.len() {
                        self.error_with_code(
                            codes::GENERIC,
                            span,
                            format!(
                                "`{name}` expects {} type arguments, found {}",
                                info.type_params.len(),
                                args.len()
                            ),
                        );
                        return Type::Unit;
                    }
                    self.normalizing_types.insert(instance_name.clone());
                    let substitutions = info
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    let fields = info
                        .fields
                        .iter()
                        .map(|(field, ty)| {
                            let substituted = substitute_type(ty, &substitutions);
                            (field.clone(), self.normalize_type(&substituted, span))
                        })
                        .collect::<Vec<_>>();
                    if fields
                        .iter()
                        .any(|(_, field_type)| contains_borrowed_type(field_type))
                    {
                        self.error_with_code(
                            codes::OWNERSHIP,
                            span,
                            "borrowed values cannot be stored in aggregate fields",
                        );
                    }
                    self.normalizing_types.remove(&instance_name);
                    self.structs.insert(instance_name.clone(), fields.clone());
                    self.struct_instances
                        .insert(instance_name.clone(), (name.clone(), args));
                    self.generated_structs.insert(
                        instance_name.clone(),
                        TypedStruct {
                            name: instance_name.clone(),
                            type_params: Vec::new(),
                            fields,
                        },
                    );
                    return Type::Named(instance_name);
                }
                if let Some(info) = self.generic_enums.get(name).cloned() {
                    if info.type_params.len() != args.len() {
                        self.error_with_code(
                            codes::GENERIC,
                            span,
                            format!(
                                "`{name}` expects {} type arguments, found {}",
                                info.type_params.len(),
                                args.len()
                            ),
                        );
                        return Type::Unit;
                    }
                    self.normalizing_types.insert(instance_name.clone());
                    let substitutions = info
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    let variants = info
                        .variants
                        .iter()
                        .map(|variant| TypedVariant {
                            name: variant.name.clone(),
                            tag: variant.tag,
                            fields: variant
                                .fields
                                .iter()
                                .map(|(field, ty)| {
                                    let substituted = substitute_type(ty, &substitutions);
                                    (field.clone(), self.normalize_type(&substituted, span))
                                })
                                .collect(),
                        })
                        .collect::<Vec<_>>();
                    if variants.iter().any(|variant| {
                        variant
                            .fields
                            .iter()
                            .any(|(_, field_type)| contains_borrowed_type(field_type))
                    }) {
                        self.error_with_code(
                            codes::OWNERSHIP,
                            span,
                            "borrowed values cannot be stored in aggregate fields",
                        );
                    }
                    self.normalizing_types.remove(&instance_name);
                    self.enum_instances
                        .insert(instance_name.clone(), (name.clone(), args));
                    self.generated_enums.insert(
                        instance_name.clone(),
                        TypedEnum {
                            name: instance_name.clone(),
                            type_params: Vec::new(),
                            variants,
                        },
                    );
                    return Type::Named(instance_name);
                }
                self.error_with_code(
                    codes::GENERIC,
                    span,
                    format!("`{name}` is not a generic type"),
                );
                Type::Unit
            }
            _ => ty.clone(),
        }
    }

    fn function(&mut self, function: &Function) -> Option<TypedFunction> {
        self.active_type_params = function.type_params.iter().cloned().collect();
        self.env = Environment::default();
        self.env.push();
        let mut params = Vec::new();
        for param in &function.params {
            let parameter_type = self.normalize_type(&param.ty, param.span);
            let id = self.fresh_id();
            let binding = Binding {
                id,
                ty: parameter_type.clone(),
                mutable: false,
                state: OwnershipState::Available,
                definition: param.span,
                borrowed_from: None,
                owns_loan: false,
                captured: false,
                place: false,
            };
            self.refuse_function_shadow(&param.name, &parameter_type, param.span);
            if self.env.insert(param.name.clone(), binding).is_err() {
                self.error(
                    param.span,
                    format!("parameter `{}` is declared more than once", param.name),
                );
            }
            params.push(TypedParam {
                id,
                name: param.name.clone(),
                ty: parameter_type,
                span: param.span,
            });
        }
        let return_type = self.normalize_type(&function.return_type, function.span);
        self.current_return_type = Some(return_type.clone());
        let body = self.expr(&function.body, Some(&return_type));
        self.current_return_type = None;
        self.env.pop();
        Some(TypedFunction {
            name: function.name.clone(),
            inline: function
                .annotations
                .iter()
                .any(|annotation| annotation.name == "inline"),
            type_params: function.type_params.clone(),
            params,
            return_type,
            body,
            span: function.span,
        })
    }

    fn expr(&mut self, expr: &Expr, expected: Option<&Type>) -> TExpr {
        let typed = match &expr.kind {
            ExprKind::Unit => self.typed(expr, Type::Unit, TExprKind::Unit),
            ExprKind::Bool(value) => self.typed(expr, Type::Bool, TExprKind::Bool(*value)),
            // A literal takes its type from what is expected of it and falls
            // back to `i64`, and the range it has to fit is that type's rather
            // than the word's (`D-107`). Only `sema` can decide this, which is
            // why `numeric_atom` stopped trying to.
            ExprKind::Int(value) => {
                let ty = match expected.and_then(|ty| ty.int_kind().map(|_| ty.clone())) {
                    Some(ty) => ty,
                    None => Type::I64,
                };
                let kind = ty.int_kind().expect("the fallback is an integer type");
                let word = value.at(kind).unwrap_or_else(|| {
                    self.error(
                        expr.span,
                        format!("integer literal `{value}` does not fit in {ty}"),
                    );
                    0
                });
                self.typed(expr, ty, TExprKind::Int(word))
            }
            ExprKind::Float(value) => self.typed(expr, Type::F64, TExprKind::Float(*value)),
            ExprKind::String(value) => {
                self.typed(expr, Type::String, TExprKind::String(value.clone()))
            }
            ExprKind::Var { name, resolved } => {
                self.variable(expr, name, resolved.as_deref(), true, expected)
            }
            ExprKind::Let {
                name,
                mutable,
                ty,
                value,
            } => {
                // The written type is the value's expectation (`D-121`), which
                // is the whole of what a typed `let` is: `D-105` already makes
                // an expectation reach an empty container's element type, and
                // a `let` was the one place that had nothing to reach from.
                let written = ty.as_ref().map(|ty| self.normalize_type(ty, expr.span));
                let value = self.expr(value, written.as_ref());
                let (borrowed_from, owns_loan) = self
                    .reference_loan(&value)
                    .map_or((None, false), |(origin, owns)| (Some(origin), owns));
                if value.ty == Type::Unit || matches!(value.ty, Type::Ref { mutable: true, .. }) {
                    self.error(
                        expr.span,
                        format!("cannot bind a value of type `{}`", value.ty),
                    );
                }
                let id = self.fresh_id();
                let binding = Binding {
                    id,
                    ty: value.ty.clone(),
                    mutable: *mutable,
                    state: OwnershipState::Available,
                    definition: expr.span,
                    borrowed_from,
                    owns_loan,
                    captured: false,
                    place: false,
                };
                self.refuse_function_shadow(name, &value.ty, expr.span);
                self.env.rebind(name.clone(), binding);
                self.typed(
                    expr,
                    Type::Unit,
                    TExprKind::Let {
                        id,
                        name: name.clone(),
                        mutable: *mutable,
                        value: Box::new(value),
                    },
                )
            }
            ExprKind::Set { name, value } => self.set(expr, name, value),
            ExprKind::Do(expressions) => self.do_expr(expr, expressions, expected),
            // `unsafe` leaves nothing behind in the typed IR. The permission is
            // spent here, at the point the operations inside it are checked, so
            // the block is a `do` by the time anything downstream sees it and
            // neither MIR nor either backend learns the word (`D-067`).
            ExprKind::Unsafe(expressions) => {
                self.unsafe_depth += 1;
                let typed = self.do_expr(expr, expressions, expected);
                self.unsafe_depth -= 1;
                typed
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => self.if_expr(expr, condition, then_expr, else_expr, expected),
            ExprKind::Loop { body } => self.loop_expr(expr, None, body, expected),
            ExprKind::While { condition, body } => {
                self.loop_expr(expr, Some(condition), body, expected)
            }
            ExprKind::Break(value) => self.break_expr(expr, value.as_deref()),
            ExprKind::Continue => {
                if self.loops.is_empty() {
                    self.error(expr.span, "`continue` can only be used inside a loop");
                }
                self.typed(expr, Type::Unit, TExprKind::Continue)
            }
            ExprKind::Logical { op, operands } => self.logical(expr, *op, operands),
            ExprKind::Match { value, arms } => self.match_expr(expr, value, arms, expected),
            ExprKind::Borrow { mutable, value } => self.borrow(expr, *mutable, value),
            ExprKind::Try(value) => self.try_expr(expr, value),
            ExprKind::Convert { target, value } => self.convert(expr, target, value),
            ExprKind::Call { callee, args } => self.call(expr, callee, args, expected),
            ExprKind::Lambda {
                captures,
                params,
                result,
                body,
            } => self.lambda(expr, captures, params, result, body),
        };

        if let Some(expected) = expected {
            // An exclusive borrow is accepted where a shared one is asked for
            // (`D-120`). It costs no instruction — the two are the same word for
            // every type, because `AddressOf` decides what a borrow *is* by the
            // referent's shape and not by its mutability — and without it a
            // field bound by a `(&mut ...)` match could not be handed to
            // anything the library already has.
            if typed.ty != *expected && !typed.ty.weakens_to(expected) {
                self.error(
                    expr.span,
                    format!("expected `{expected}`, found `{}`", typed.ty),
                );
            }
        }
        if !matches!(typed.ty, Type::Ref { .. } | Type::Slice(_)) {
            // `CallValue` belongs here for the same reason the other two do,
            // and v0.7.0 left it out: a borrow handed to a function *value*
            // kept its loan for the rest of the scope, so a list could be
            // shown to a predicate or never mutated again but not both. One
            // call never noticed; `core:list:filter` is two.
            //
            // The argument is asked what loan it *carries* rather than whether
            // it is spelled `(& x)`, which is the same omission one level down:
            // `(get-ref (& items) index)` is a borrow of `items` and does not
            // look like one, so handing the element to a predicate used to hold
            // the list for the rest of the scope. A named borrow keeps its loan
            // — `reference_loan` says it does not own one — because it can be
            // used again after the call and the value behind it must stay put.
            if let TExprKind::Call { args, .. }
            | TExprKind::GenericCall { args, .. }
            | TExprKind::CallValue { args, .. } = &typed.kind
            {
                for origin in args
                    .iter()
                    .filter_map(|argument| self.reference_loan(argument))
                    .filter_map(|(origin, owns)| owns.then_some(origin))
                    .collect::<Vec<_>>()
                {
                    self.env.discharge_loan(origin);
                }
            }
        }
        typed
    }

    fn reference_loan(&self, value: &TExpr) -> Option<(BindingId, bool)> {
        match &value.kind {
            TExprKind::Borrow { id, .. } => Some((*id, true)),
            TExprKind::Var(id) => self
                .env
                .bindings
                .get(id)
                .and_then(|binding| binding.borrowed_from)
                .map(|origin| (origin, false)),
            TExprKind::Call { callee, args } if callee == "get-ref" => args
                .first()
                .and_then(|argument| self.reference_loan(argument))
                .map(|(origin, _)| (origin, true)),
            TExprKind::Call { callee, args } if callee == "slice" => args
                .first()
                .and_then(|argument| self.reference_loan(argument))
                .map(|(origin, _)| (origin, true)),
            // `clone` is deliberately absent: since `D-091` its result is owned
            // whatever it was handed, so it ends a loan rather than passing one
            // on.
            _ => None,
        }
    }

    fn variable(
        &mut self,
        expr: &Expr,
        name: &str,
        resolved: Option<&str>,
        consume: bool,
        expected: Option<&Type>,
    ) -> TExpr {
        let Some(id) = self.env.lookup_id(name) else {
            // A fallback, never a namespace merge (`D-092`): the environment is
            // consulted first, so no program that already compiles changes
            // meaning, and a name that was a variable stays one.
            //
            // `resolved` is what `package.rs` says the name means as a
            // top-level item, and it comes first because it is the one that
            // accounts for imports. The bare name is tried after it for the
            // sources that never went through the resolver.
            // A `const` is looked for the same way and first (`D-121`): the
            // two tables cannot hold one name, so the order between them
            // decides nothing, and putting the cheaper one in front keeps the
            // fallback readable.
            let constant = resolved
                .filter(|candidate| self.consts.contains_key(*candidate))
                .map(str::to_owned)
                .or_else(|| self.consts.contains_key(name).then(|| name.to_owned()));
            if let Some(constant) = constant {
                if let Some(deprecation) = self.deprecated_consts.get(&constant).cloned() {
                    self.deprecated_use(name, expr.span, &deprecation);
                }
                let mut value = self.consts[&constant].clone();
                // The literal is stamped with the span of the *use*, not of the
                // declaration. A line table that sent a debugger to the `const`
                // is the classic inlining confusion, and the use is where the
                // program is.
                value.span = expr.span;
                return TExpr {
                    ty: value.ty.clone(),
                    span: expr.span,
                    kind: TExprKind::Const {
                        name: constant,
                        value: Box::new(value),
                    },
                };
            }
            let candidate = resolved
                .filter(|candidate| self.signatures.contains_key(*candidate))
                .map(str::to_owned)
                .or_else(|| self.signatures.contains_key(name).then(|| name.to_owned()));
            if let Some(candidate) = candidate {
                return self.function_value(expr, &candidate, name, expected);
            }
            // A name the enclosing function has and this `lambda` did not
            // capture. It is one diagnostic and it names the fix, and the type
            // it hands back is the one that was asked for, so the mismatch that
            // would otherwise follow at the same span does not.
            if self
                .enclosing
                .iter()
                .any(|scope| scope.lookup_id(name).is_some())
            {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::NAME_OR_TYPE,
                        self.file,
                        expr.span,
                        format!("`{name}` is not captured by this lambda"),
                    )
                    .with_help(format!(
                        "name it in the capture list — `(lambda ({name} ...) ...)` — \
                         which moves it in"
                    )),
                );
                let ty = expected.cloned().unwrap_or(Type::Unit);
                return self.typed(expr, ty, TExprKind::Unit);
            }
            self.error(expr.span, format!("undefined variable `{name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(binding) = self.env.bindings.get(&id).cloned() else {
            self.error(expr.span, format!("undefined variable `{name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        match binding.state {
            OwnershipState::Moved => {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::OWNERSHIP,
                        self.file,
                        expr.span,
                        format!("use of moved value `{name}`"),
                    )
                    .with_label(binding.definition, "value was declared here")
                    .with_help("borrow the value or call `clone` before moving it"),
                );
            }
            OwnershipState::MutBorrowed if consume => {
                self.error(
                    expr.span,
                    format!("cannot use `{name}` while it is mutably borrowed"),
                );
            }
            OwnershipState::SharedBorrowed(_) if consume && !binding.ty.is_copy() => {
                self.error(
                    expr.span,
                    format!("cannot move `{name}` while it is borrowed"),
                );
            }
            _ => {}
        }
        // A capture belongs to the environment, which drops it (`D-102`). The
        // damage a move would do is not to this call but to the second one, so
        // it is refused by what the name is rather than by what state it is in.
        if consume && !binding.ty.is_copy() && binding.captured {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    expr.span,
                    format!("cannot move `{name}` out of the lambda that captured it"),
                )
                .with_label(binding.definition, "captured here")
                .with_help("borrow it or call `clone`; the capture stays where it is"),
            );
        }
        // A guard is asked whether this arm applies, and an arm that does not
        // apply leaves the value to the next one (`D-121`). So a guard reads,
        // and a move inside one is refused by where it is written rather than
        // by what state the binding is in — the same shape the capture above
        // uses, and for the same reason: the damage is to the arm that runs
        // afterwards.
        if consume && !binding.ty.is_copy() && self.guards.last().is_some_and(|floor| id < *floor) {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    expr.span,
                    format!("cannot move `{name}` inside a `when` guard"),
                )
                .with_label(binding.definition, "value was declared here")
                .with_help(
                    "a guard runs before its arm is taken, so the value has to still be \
                     there for the arms after it; borrow it or `clone` it",
                ),
            );
        }
        if consume && !binding.ty.is_copy() && binding.state == OwnershipState::Available {
            if let Some(binding) = self.env.bindings.get_mut(&id) {
                binding.state = OwnershipState::Moved;
            }
            self.move_sites.insert(id, expr.span);
        }
        self.typed(expr, binding.ty, TExprKind::Var(id))
    }

    /// A top-level `fn` named where a value is expected (`D-092`).
    ///
    /// A generic function needs its instance chosen here, because a value is
    /// the address of one monomorphized body and there is no later point that
    /// could pick it. The only evidence available is the expected type, so a
    /// generic function value is legal exactly where the context says which
    /// instance it is — and refused, by the diagnostic `generic_call` already
    /// uses, where it does not.
    fn function_value(
        &mut self,
        expr: &Expr,
        name: &str,
        written: &str,
        expected: Option<&Type>,
    ) -> TExpr {
        let signature = self
            .signatures
            .get(name)
            .cloned()
            .expect("the caller checked the name is a signature");
        if let Some(deprecation) = signature.deprecated.clone() {
            self.deprecated_use(written, expr.span, &deprecation);
        }
        if signature.is_extern {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("`{written}` is an `extern` and cannot be used as a value"),
                )
                .with_help(
                    "wrap it in a `fn` and take that instead: an `extern` argument may cross \
                     the boundary as more than one machine word, which a `Fn` type cannot say",
                ),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        if signature.type_params.is_empty() {
            let ty = Type::Fn {
                params: signature.params.clone(),
                result: Box::new(signature.result.clone()),
            };
            return self.typed(
                expr,
                ty,
                TExprKind::FnRef {
                    name: name.to_owned(),
                    type_args: Vec::new(),
                },
            );
        }
        let parameters = signature
            .type_params
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let template = Type::Fn {
            params: signature.params.clone(),
            result: Box::new(signature.result.clone()),
        };
        let mut substitutions = HashMap::<String, Type>::new();
        if let Some(expected) = expected {
            // An error here is not reported: a mismatch against the expected
            // type is the caller's to describe, and the missing-instance
            // diagnostic below is the one that names what went wrong.
            let _ = unify_type(
                &template,
                expected,
                &parameters,
                &mut substitutions,
                self.instances(),
            );
        }
        let mut type_args = Vec::new();
        for parameter in &signature.type_params {
            match substitutions.get(parameter) {
                Some(ty) => type_args.push(ty.clone()),
                None => {
                    self.error_with_code(
                        codes::GENERIC,
                        expr.span,
                        format!("cannot infer generic parameter `{parameter}` for `{written}`"),
                    );
                    return self.typed(expr, Type::Unit, TExprKind::Unit);
                }
            }
        }
        let ty = Type::Fn {
            params: signature
                .params
                .iter()
                .map(|param| substitute_type(param, &substitutions))
                .collect(),
            result: Box::new(substitute_type(&signature.result, &substitutions)),
        };
        let ty = self.normalize_type(&ty, expr.span);
        self.typed(
            expr,
            ty,
            TExprKind::FnRef {
                name: name.to_owned(),
                type_args,
            },
        )
    }

    fn set(&mut self, expr: &Expr, name: &str, value: &Expr) -> TExpr {
        let Some(id) = self.env.lookup_id(name) else {
            self.error(expr.span, format!("undefined variable `{name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let binding = self
            .env
            .bindings
            .get(&id)
            .cloned()
            .expect("binding id is valid");
        // Writing through a borrow (`D-120`). The name is a field an enclosing
        // `match` bound through a `(&mut ...)`, and `set` puts a new value in
        // that field and drops the one that was there. Nothing else that has a
        // borrow's type can be written: a shared borrow may not write at all,
        // and an exclusive one that names no field has nowhere to write to.
        if let Type::Ref { mutable, inner } = binding.ty.clone() {
            if !binding.place {
                let message = if mutable {
                    format!("cannot assign through the borrow `{name}`")
                } else {
                    format!("cannot assign through the shared borrow `{name}`")
                };
                let help = if mutable {
                    "match the aggregate and assign one of its fields"
                } else {
                    "read it with `clone`; to assign, match through `(&mut ...)`"
                };
                self.diagnostics.push(
                    Diagnostic::error(codes::OWNERSHIP, self.file, expr.span, message)
                        .with_label(binding.definition, "borrow bound here")
                        .with_help(help),
                );
            }
            let value = self.expr(value, Some(&inner));
            return self.typed(
                expr,
                Type::Unit,
                TExprKind::Set {
                    id,
                    value: Box::new(value),
                },
            );
        }
        if !binding.mutable {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    expr.span,
                    format!("cannot assign to immutable `{name}`"),
                )
                .with_label(binding.definition, "immutable binding declared here")
                .with_help(format!("declare it with `(let mut {name} ...)`")),
            );
        }
        if matches!(
            binding.state,
            OwnershipState::SharedBorrowed(_) | OwnershipState::MutBorrowed
        ) {
            self.error(
                expr.span,
                format!("cannot assign to `{name}` while it is borrowed"),
            );
        }
        let value = self.expr(value, Some(&binding.ty));
        if let Some(current) = self.env.bindings.get_mut(&id) {
            current.state = OwnershipState::Available;
        }
        self.typed(
            expr,
            Type::Unit,
            TExprKind::Set {
                id,
                value: Box::new(value),
            },
        )
    }

    fn do_expr(&mut self, expr: &Expr, expressions: &[Expr], expected: Option<&Type>) -> TExpr {
        self.env.push();
        let mut typed = Vec::new();
        for (index, item) in expressions.iter().enumerate() {
            let item_expected = if index + 1 == expressions.len() {
                expected
            } else {
                None
            };
            typed.push(self.expr(item, item_expected));
            let mut live_names = HashSet::new();
            for remaining in &expressions[index + 1..] {
                collect_variable_names(remaining, &mut live_names);
            }
            self.env.release_dead_references(&live_names);
        }
        self.env.pop();
        let ty = typed
            .last()
            .map(|item| item.ty.clone())
            .unwrap_or(Type::Unit);
        self.typed(expr, ty, TExprKind::Do(typed))
    }

    fn if_expr(
        &mut self,
        expr: &Expr,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        expected: Option<&Type>,
    ) -> TExpr {
        let condition = self.expr(condition, Some(&Type::Bool));
        let base = self.env.clone();

        self.env.push();
        let then_expr = self.expr(then_expr, expected);
        self.env.pop();
        let then_env = self.env.clone();

        self.env = base.clone();
        self.env.push();
        let else_expr = self.expr(else_expr, Some(&then_expr.ty));
        self.env.pop();
        let else_env = self.env.clone();

        self.env = base;
        for (id, binding) in &mut self.env.bindings {
            let left = then_env.bindings.get(id).map(|b| b.state);
            let right = else_env.bindings.get(id).map(|b| b.state);
            if left == Some(OwnershipState::Moved) || right == Some(OwnershipState::Moved) {
                binding.state = OwnershipState::Moved;
            }
        }
        let ty = then_expr.ty.clone();
        self.typed(
            expr,
            ty,
            TExprKind::If {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
        )
    }

    /// `and` and `or`, typed here and gone by the time anything else looks
    /// (`D-106`).
    ///
    /// Every operand is typed against `bool`, which is what makes `(and 1 2)`
    /// say "expected `bool`, found `i64`" at the `1` rather than complaining
    /// about a constant the compiler invented. What comes out is nested `If`s —
    /// `(and a b)` is `(if a b false)`, `(or a b)` is `(if a true b)` — so the
    /// short circuit is the branch that `if` already lowers to, and neither
    /// `mir` nor either backend learns anything.
    ///
    /// Folded from the right, so `(and a b c)` evaluates `a` first and stops at
    /// the first operand that answers.
    fn logical(&mut self, expr: &Expr, op: LogicalOp, operands: &[Expr]) -> TExpr {
        let mut typed = Vec::new();
        let base = self.env.clone();
        for (index, operand) in operands.iter().enumerate() {
            // Everything after the first runs only sometimes, so a move inside
            // one is a move on one path — the rule `if_expr` applies to its
            // branches, and the reason each gets its own scope.
            if index > 0 {
                self.env.push();
            }
            typed.push(self.expr(operand, Some(&Type::Bool)));
            if index > 0 {
                self.env.pop();
            }
        }
        // A binding moved in an operand that may not run is still moved: the
        // conservative answer is the only sound one, and it is what `if_expr`
        // reaches by merging both branches.
        let moved = self.env.clone();
        self.env = base;
        for (id, binding) in &mut self.env.bindings {
            if moved.bindings.get(id).map(|held| held.state) == Some(OwnershipState::Moved) {
                binding.state = OwnershipState::Moved;
            }
        }

        let constant = |kind: bool| TExpr {
            ty: Type::Bool,
            kind: TExprKind::Bool(kind),
            span: expr.span,
        };
        let mut folded = typed.pop().expect("`and`/`or` has at least two operands");
        while let Some(condition) = typed.pop() {
            let (then_expr, else_expr) = match op {
                LogicalOp::And => (folded, constant(false)),
                LogicalOp::Or => (constant(true), folded),
            };
            folded = TExpr {
                ty: Type::Bool,
                span: expr.span,
                kind: TExprKind::If {
                    condition: Box::new(condition),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                },
            };
        }
        folded
    }

    /// `(break)`, or `(break value)` in a `loop` (`D-121`).
    ///
    /// The value's type is the loop's, agreed between the breaks and whatever
    /// the context asked the loop for. A bare `break` agrees on `unit`, which
    /// is what every loop was before this, so nothing that compiled changes.
    fn break_expr(&mut self, expr: &Expr, value: Option<&Expr>) -> TExpr {
        let Some(frame) = self.loops.last().cloned() else {
            self.error(expr.span, "`break` can only be used inside a loop");
            let value = value.map(|value| Box::new(self.expr(value, None)));
            return self.typed(expr, Type::Unit, TExprKind::Break(value));
        };
        let Some(value) = value else {
            if !frame.conditional {
                self.agree_on_break_type(expr.span, &Type::Unit);
            }
            return self.typed(expr, Type::Unit, TExprKind::Break(None));
        };
        if frame.conditional {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    "a `while` cannot `break` with a value",
                )
                .with_help(
                    "a `while` ends when its condition is false, where there is no value to \
                     produce; write `loop` with the condition inside it",
                ),
            );
            let value = self.expr(value, None);
            return self.typed(expr, Type::Unit, TExprKind::Break(Some(Box::new(value))));
        }
        let value = self.expr(value, frame.result.as_ref());
        self.agree_on_break_type(expr.span, &value.ty);
        self.typed(expr, Type::Unit, TExprKind::Break(Some(Box::new(value))))
    }

    /// Records what this loop produces, or reports that its breaks disagree.
    ///
    /// The first `break` decides and the rest are typed against it, so the only
    /// disagreement that reaches here is a bare `break` beside a valued one —
    /// the case the expectation could not catch, because a bare one carries no
    /// expression to hang the diagnostic on.
    fn agree_on_break_type(&mut self, span: Span, ty: &Type) {
        let Some(frame) = self.loops.last_mut() else {
            return;
        };
        match &frame.result {
            None => frame.result = Some(ty.clone()),
            Some(previous) if previous == ty || ty.weakens_to(previous) => {}
            Some(previous) => {
                let previous = previous.clone();
                self.error(
                    span,
                    format!("this loop produces `{previous}`, and this `break` gives `{ty}`"),
                );
            }
        }
    }

    fn loop_expr(
        &mut self,
        expr: &Expr,
        condition: Option<&Expr>,
        body: &Expr,
        expected: Option<&Type>,
    ) -> TExpr {
        // A loop body runs an unknown number of times, so a value consumed by
        // one iteration is already gone when the next one starts. Record which
        // bindings were still owned on entry and reject any that the body moves
        // out; without this the second iteration reuses freed memory.
        let owned_on_entry = self
            .env
            .bindings
            .iter()
            .filter(|(_, binding)| binding.state == OwnershipState::Available)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        let condition =
            condition.map(|condition| Box::new(self.expr(condition, Some(&Type::Bool))));
        // A `loop` in value position starts out knowing what it owes, so the
        // first `(break (list))` has an element type to infer from — the same
        // reach `D-105` gave a call argument, one form further out.
        self.loops.push(LoopFrame {
            conditional: condition.is_some(),
            result: expected.filter(|_| condition.is_none()).cloned(),
        });
        self.env.push();
        let body = Box::new(self.expr(body, Some(&Type::Unit)));
        self.env.pop();
        let frame = self.loops.pop().expect("the frame was just pushed");
        let result = if frame.conditional {
            Type::Unit
        } else {
            frame.result.unwrap_or(Type::Unit)
        };

        let mut escaped = owned_on_entry
            .into_iter()
            .filter(|id| {
                self.env
                    .bindings
                    .get(id)
                    .is_some_and(|binding| binding.state == OwnershipState::Moved)
            })
            .filter_map(|id| self.move_sites.get(&id).map(|span| (id, *span)))
            .collect::<Vec<_>>();
        escaped.sort_by_key(|(_, span)| (span.start, span.end));
        for (id, span) in escaped {
            // Forget the site so an enclosing loop does not report the same
            // move again; the binding stays `Moved` either way.
            self.move_sites.remove(&id);
            let name = self.env.name_of(id).unwrap_or_else(|| "value".to_owned());
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    span,
                    format!("`{name}` is moved inside a loop body"),
                )
                .with_label(expr.span, "this loop can run more than once")
                .with_help(format!(
                    "move it before the loop, `clone` it inside the loop, or reassign `{name}` with `set` after the move"
                )),
            );
        }

        match condition {
            Some(condition) => self.typed(expr, Type::Unit, TExprKind::While { condition, body }),
            None => self.typed(expr, result, TExprKind::Loop { body }),
        }
    }

    fn borrow(&mut self, expr: &Expr, mutable: bool, value: &Expr) -> TExpr {
        let ExprKind::Var { name, .. } = &value.kind else {
            self.error(value.span, "only named bindings can be borrowed");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(id) = self.env.lookup_id(name) else {
            self.error(value.span, format!("undefined variable `{name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let binding = self.env.bindings.get(&id).cloned().expect("binding exists");
        if binding.state == OwnershipState::Moved {
            self.error(value.span, format!("cannot borrow moved value `{name}`"));
        }
        if mutable && !binding.mutable {
            self.error(
                value.span,
                format!("cannot mutably borrow immutable `{name}`"),
            );
        }
        let next_state = if mutable {
            // Two exclusive borrows and an exclusive one over a shared one are
            // the same refusal and not the same mistake, so they do not get the
            // same sentence. A `match` through a `(&mut ...)` is what made the
            // second reachable often enough to be worth telling apart.
            match binding.state {
                OwnershipState::Available | OwnershipState::Moved => {}
                OwnershipState::MutBorrowed => self.error(
                    value.span,
                    format!("cannot mutably borrow `{name}` more than once"),
                ),
                OwnershipState::SharedBorrowed(_) => self.error(
                    value.span,
                    format!("cannot mutably borrow `{name}` while it is shared-borrowed"),
                ),
            }
            OwnershipState::MutBorrowed
        } else {
            match binding.state {
                OwnershipState::Available => OwnershipState::SharedBorrowed(1),
                OwnershipState::SharedBorrowed(count) => OwnershipState::SharedBorrowed(count + 1),
                OwnershipState::MutBorrowed => {
                    self.error(
                        value.span,
                        format!("cannot share-borrow mutably borrowed `{name}`"),
                    );
                    OwnershipState::MutBorrowed
                }
                OwnershipState::Moved => OwnershipState::Moved,
            }
        };
        if let Some(current) = self.env.bindings.get_mut(&id) {
            current.state = next_state;
        }
        self.env.add_loan(id);
        self.typed(
            expr,
            Type::Ref {
                mutable,
                inner: Box::new(binding.ty),
            },
            TExprKind::Borrow { id, mutable },
        )
    }

    fn try_expr(&mut self, expr: &Expr, value: &Expr) -> TExpr {
        let value = self.expr(value, None);
        let Some(result_name) = self.language_items.result.clone() else {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                "`try` requires a `result` language item",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(ok_path) = self.language_items.result_ok.clone() else {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                "`try` requires a `result-ok` language item",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(err_path) = self.language_items.result_err.clone() else {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                "`try` requires a `result-err` language item",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Type::Named(instance_name) = &value.ty else {
            self.error(
                expr.span,
                format!("`try` expects Result, found `{}`", value.ty),
            );
            // A generic body's `(Result T E)` is the one case where the
            // refusal still knows what the form would have produced, so it
            // recovers as that rather than as `unit` and the one cause reports
            // once instead of three times (`D-095`).
            let recovered = match &value.ty {
                Type::Apply { name, args } if name == &result_name && args.len() == 2 => {
                    args[0].clone()
                }
                _ => Type::Unit,
            };
            return self.typed(expr, recovered, TExprKind::Unit);
        };
        let instance_name = instance_name.clone();
        let Some((base, arguments)) = self.enum_instances.get(&instance_name).cloned() else {
            self.error(
                expr.span,
                format!("`try` expects Result, found `{}`", value.ty),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        if base != result_name || arguments.len() != 2 {
            self.error(
                expr.span,
                format!("`try` expects `{result_name}`, found `{}`", value.ty),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let ok_type = arguments[0].clone();
        let error_type = arguments[1].clone();
        let Some(Type::Named(return_name)) = self.current_return_type.as_ref() else {
            self.error(
                expr.span,
                "`try` can only be used in a function returning Result",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some((return_base, return_arguments)) = self.enum_instances.get(return_name) else {
            self.error(
                expr.span,
                "`try` can only be used in a function returning Result",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        if return_base != &result_name || return_arguments.get(1) != Some(&error_type) {
            self.error(
                expr.span,
                format!(
                    "`try` error type `{error_type}` is incompatible with function return type `{}`",
                    self.current_return_type.as_ref().unwrap_or(&Type::Unit)
                ),
            );
        }
        let Some(ok) = self.variants.get(&ok_path).cloned() else {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                format!("result-ok language item `{ok_path}` does not exist"),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(err) = self.variants.get(&err_path).cloned() else {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                format!("result-err language item `{err_path}` does not exist"),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        if ok.enum_name != result_name
            || err.enum_name != result_name
            || ok.fields.len() != 1
            || err.fields.len() != 1
        {
            self.error_with_code(
                codes::STANDARD_LIBRARY,
                expr.span,
                "Result language items must identify one-field Ok and Err variants",
            );
        }
        self.typed(
            expr,
            ok_type.clone(),
            TExprKind::Try {
                value: Box::new(value),
                ok_type,
                enum_name: instance_name,
                ok_tag: ok.tag,
                err_tag: err.tag,
            },
        )
    }

    fn match_expr(
        &mut self,
        expr: &Expr,
        value: &Expr,
        arms: &[crate::ast::MatchArm],
        expected: Option<&Type>,
    ) -> TExpr {
        let value = self.expr(value, None);
        // A `match` looks through a borrow of either kind (`D-099`, `D-120`), in
        // which case everything below asks about what is behind it and every
        // binding a pattern makes is a borrow of the same kind. Through an
        // exclusive one those bindings are also places, which is the whole of
        // what field assignment is.
        let borrowed = matches!(value.ty, Type::Ref { .. });
        let exclusive = matches!(value.ty, Type::Ref { mutable: true, .. });
        let scrutinee = value.ty.strip_ref().clone();
        let enum_name = self.enum_of_type(&scrutinee);
        let struct_name = match &scrutinee {
            Type::Named(name)
                if self.structs.contains_key(name) || self.generated_structs.contains_key(name) =>
            {
                Some(name.clone())
            }
            Type::Apply { name, .. } if self.generic_structs.contains_key(name) => {
                Some(name.clone())
            }
            _ => None,
        };
        if enum_name.is_none() && struct_name.is_none() {
            if borrowed {
                // Through a borrow the scalar cases have no meaning worth
                // giving them: there is nothing to take apart, and the only
                // thing anyone wants is the value itself.
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::NAME_OR_TYPE,
                        self.file,
                        value.span,
                        format!(
                            "`match` through a borrow needs an enum or a struct, found `{}`",
                            value.ty
                        ),
                    )
                    .with_help("read a borrowed scalar with `clone` and match the value"),
                );
            } else if !matches!(scrutinee, Type::Bool | Type::I32 | Type::I64) {
                self.error(
                    value.span,
                    format!("`match` does not support `{}`", value.ty),
                );
            }
        }
        let outer_borrow = self.pattern_borrow.take();
        if borrowed {
            self.pattern_borrow = Some(PatternBorrow {
                origin: self.reference_loan(&value).map(|(origin, _)| origin),
                mutable: exclusive,
            });
        }
        let base = self.env.clone();
        let mut typed_arms = Vec::new();
        let mut arm_environments = Vec::new();
        let mut result_type = expected.cloned();
        let mut seen = HashSet::new();
        let mut wildcard = false;
        for arm in arms {
            self.env = base.clone();
            self.env.push();
            let pattern = self.type_pattern(&arm.pattern, &scrutinee);
            // A guarded arm proves nothing (`D-121`). It is tested last, its
            // condition can be false, and so the value it did not take is
            // still the next arm's business: it counts toward neither the
            // wildcard nor the variants seen, and two arms with one pattern
            // and different guards are not a duplicate.
            let guarded = arm.guard.is_some();
            let guard = arm.guard.as_ref().map(|guard| {
                self.guards.push(self.next_binding);
                let guard = self.expr(guard, Some(&Type::Bool));
                self.guards.pop();
                guard
            });
            if pattern_irrefutable(&pattern) && !guarded {
                wildcard = true;
            }
            if guarded {
                let body = self.expr(&arm.body, result_type.as_ref());
                self.env.pop();
                if result_type.is_none() {
                    result_type = Some(body.ty.clone());
                }
                arm_environments.push(self.env.clone());
                typed_arms.push(TMatchArm {
                    pattern,
                    guard,
                    body,
                    span: arm.span,
                });
                continue;
            }
            match &pattern {
                TPattern::Bool(pattern) => {
                    seen.insert(format!("bool:{pattern}"));
                }
                TPattern::Enum {
                    enum_name,
                    variant,
                    fields,
                    ..
                } if fields
                    .iter()
                    .all(|field| pattern_irrefutable(&field.pattern)) =>
                {
                    seen.insert(format!("enum:{enum_name}:{variant}"));
                }
                _ => {}
            }
            let pattern_key = format!("{pattern:?}");
            if !seen.insert(format!("pattern:{pattern_key}")) && !pattern_irrefutable(&pattern) {
                self.error(arm.pattern.span, "duplicate match pattern");
            }
            let body = self.expr(&arm.body, result_type.as_ref());
            self.env.pop();
            if result_type.is_none() {
                result_type = Some(body.ty.clone());
            }
            arm_environments.push(self.env.clone());
            typed_arms.push(TMatchArm {
                pattern,
                guard,
                body,
                span: arm.span,
            });
        }
        self.pattern_borrow = outer_borrow;
        self.env = base;
        if !arm_environments.is_empty() {
            for (id, binding) in &mut self.env.bindings {
                if arm_environments.iter().any(|env| {
                    env.bindings
                        .get(id)
                        .is_some_and(|item| item.state == OwnershipState::Moved)
                }) {
                    binding.state = OwnershipState::Moved;
                }
            }
        }
        let exhaustive = wildcard
            || (scrutinee == Type::Bool
                && seen.contains("bool:true")
                && seen.contains("bool:false"))
            || enum_name.as_ref().is_some_and(|enum_name| {
                let base = self
                    .enum_instances
                    .get(enum_name)
                    .map_or(enum_name.as_str(), |(base, _)| base.as_str());
                self.variants
                    .values()
                    .filter(|variant| variant.enum_name == base)
                    .all(|variant| seen.contains(&format!("enum:{enum_name}:{}", variant.variant)))
            });
        // A scrutinee the analyzer could make no sense of has already been
        // reported, and every arm's pattern with it. Saying the match is also
        // non-exhaustive is a second diagnostic for the first one's cause, and
        // the help it reaches for — a final `_` arm — is advice for an integer
        // match the author is not writing.
        let understood = enum_name.is_some()
            || struct_name.is_some()
            || matches!(scrutinee, Type::Bool | Type::I32 | Type::I64);
        if !exhaustive && understood {
            self.diagnostics.push(
                Diagnostic::error(codes::MATCH, self.file, expr.span, "non-exhaustive match")
                    .with_help(if scrutinee == Type::Bool {
                        "cover both `true` and `false`, or add `_`"
                    } else if enum_name.is_some() {
                        "cover every enum variant, or add `_`"
                    } else {
                        "integer matches require a final `_` arm"
                    }),
            );
        }
        let result_type = result_type.unwrap_or(Type::Unit);
        // The loan a borrowed scrutinee took, released the way a call releases
        // the one an argument took, and for the same reason: a value shown to a
        // `match` and never mutated again is one thing, and a value that can
        // never be mutated again because it was once matched is another. The
        // arms have been typed and their bindings popped, so nothing derived
        // from it is still live. A result that is itself a borrow keeps it.
        if borrowed && !matches!(result_type, Type::Ref { .. } | Type::Slice(_)) {
            if let TExprKind::Borrow { id, .. } = &value.kind {
                self.env.discharge_loan(*id);
            }
        }
        self.typed(
            expr,
            result_type,
            TExprKind::Match {
                value: Box::new(value),
                arms: typed_arms,
            },
        )
    }

    fn type_pattern(&mut self, pattern: &Pattern, expected: &Type) -> TPattern {
        match &pattern.kind {
            PatternKind::Wildcard => TPattern::Wildcard,
            PatternKind::Binding(name) => {
                let id = self.fresh_id();
                // Through a borrow the pattern names a field it does not own,
                // so what it binds is a borrow of that field (`D-099`), and it
                // is exclusive exactly when the scrutinee's borrow was
                // (`D-120`). The type is the same for every field type,
                // including one that is `Copy`: whether `T` is `Copy` is not
                // known inside a generic body, and a binding's type has to be.
                let ty = match self.pattern_borrow {
                    Some(borrow) => Type::Ref {
                        mutable: borrow.mutable,
                        inner: Box::new(expected.clone()),
                    },
                    None => expected.clone(),
                };
                // A name is a place when it is a field of an aggregate this
                // `match` took apart through an exclusive borrow. At depth zero
                // it is the scrutinee itself, which names no field and so has
                // nowhere for `set` to write.
                let place = self
                    .pattern_borrow
                    .is_some_and(|borrow| borrow.mutable && self.pattern_depth > 0);
                let binding = Binding {
                    id,
                    ty: ty.clone(),
                    mutable: false,
                    state: OwnershipState::Available,
                    definition: pattern.span,
                    borrowed_from: self.pattern_borrow.and_then(|borrow| borrow.origin),
                    owns_loan: false,
                    captured: false,
                    place,
                };
                self.refuse_function_shadow(name, &ty, pattern.span);
                if self.env.insert(name.clone(), binding).is_err() {
                    self.error(pattern.span, format!("duplicate pattern binding `{name}`"));
                }
                TPattern::Binding(TPatternBinding {
                    id,
                    name: name.clone(),
                    ty,
                })
            }
            PatternKind::Bool(value) => {
                if *expected != Type::Bool {
                    self.error(
                        pattern.span,
                        format!("boolean pattern does not match `{expected}`"),
                    );
                }
                TPattern::Bool(*value)
            }
            PatternKind::Int(value) => {
                let word = match expected.int_kind() {
                    Some(kind) => value.at(kind).unwrap_or_else(|| {
                        self.error(
                            pattern.span,
                            format!("pattern `{value}` does not fit in {expected}"),
                        );
                        0
                    }),
                    None => {
                        self.error(
                            pattern.span,
                            format!("integer pattern does not match `{expected}`"),
                        );
                        0
                    }
                };
                TPattern::Int(word)
            }
            PatternKind::Enum { path, fields } => {
                let mut info = if let Some(info) = self.variants.get(path).cloned() {
                    info
                } else {
                    self.error(pattern.span, format!("unknown enum variant `{path}`"));
                    VariantInfo {
                        enum_name: "<error>".into(),
                        type_params: Vec::new(),
                        variant: path.clone(),
                        tag: 0,
                        fields: Vec::new(),
                    }
                };
                // What the payloads are, and what to record the variant under.
                // A `Named` scrutinee is an instance and names itself; an
                // `Apply` one is a generic body's `(Option T)`, whose payloads
                // substitute to parameters and whose instance is not chosen
                // until materialization, so the base name stands in until then
                // (`D-095`).
                let arguments = match expected {
                    Type::Named(expected_name) => self
                        .enum_instances
                        .get(expected_name)
                        .filter(|(base, _)| base == &info.enum_name)
                        .map(|(_, arguments)| (Some(expected_name.clone()), arguments.clone())),
                    Type::Apply { name, args } if name == &info.enum_name => {
                        Some((None, args.clone()))
                    }
                    _ => None,
                };
                if let Some((instance, arguments)) = arguments {
                    let substitutions = info
                        .type_params
                        .iter()
                        .cloned()
                        .zip(arguments)
                        .collect::<HashMap<_, _>>();
                    if let Some(instance) = instance {
                        info.enum_name = instance;
                    }
                    info.fields = info
                        .fields
                        .iter()
                        .map(|(field, ty)| (field.clone(), substitute_type(ty, &substitutions)))
                        .collect();
                }
                let belongs = match expected {
                    Type::Apply { name, .. } => name == &info.enum_name,
                    other => other == &Type::Named(info.enum_name.clone()),
                };
                if !belongs {
                    self.error(
                        pattern.span,
                        format!("variant `{path}` does not belong to `{expected}`"),
                    );
                }
                if fields.len() != info.fields.len() {
                    self.error(
                        pattern.span,
                        format!(
                            "pattern `{path}` expects {} fields, found {}",
                            info.fields.len(),
                            fields.len()
                        ),
                    );
                }
                self.pattern_depth += 1;
                let typed_fields = fields
                    .iter()
                    .zip(&info.fields)
                    .map(|(field, (_, ty))| TPatternField {
                        ty: ty.clone(),
                        pattern: self.type_pattern(field, ty),
                    })
                    .collect();
                self.pattern_depth -= 1;
                TPattern::Enum {
                    enum_name: info.enum_name,
                    variant: info.variant,
                    tag: info.tag,
                    fields: typed_fields,
                }
            }
            PatternKind::Struct { path, fields } => {
                // What the pattern names, and the fields it therefore has. A
                // plain struct names itself; a monomorphized instance is
                // written with the base name the declaration used and carries
                // its arguments in the instance table; a generic body's
                // `(Pair T)` has neither and substitutes from the application
                // (`D-095`).
                let named = match expected {
                    Type::Named(name) => {
                        self.instances()
                            .application_of(name)
                            .map_or(name.as_str(), |(base, _)| base.as_str())
                            == path
                    }
                    Type::Apply { name, .. } => name == path,
                    _ => false,
                };
                if !named {
                    self.error(
                        pattern.span,
                        format!("struct pattern `{path}` does not match `{expected}`"),
                    );
                }
                let declared_fields = match expected {
                    Type::Apply { name, args } => self.generic_structs.get(name).map(|info| {
                        let substitutions = info
                            .type_params
                            .iter()
                            .cloned()
                            .zip(args.iter().cloned())
                            .collect::<HashMap<_, _>>();
                        info.fields
                            .iter()
                            .map(|(field, ty)| (field.clone(), substitute_type(ty, &substitutions)))
                            .collect::<Vec<_>>()
                    }),
                    Type::Named(name) => self.structs.get(name).cloned().or_else(|| {
                        self.generated_structs
                            .get(name)
                            .map(|structure| structure.fields.clone())
                    }),
                    _ => None,
                };
                let declared_fields = declared_fields.unwrap_or_else(|| {
                    if named {
                        self.error(pattern.span, format!("unknown struct `{path}`"));
                    }
                    Vec::new()
                });
                let expected_name = match expected {
                    Type::Named(name) => name.clone(),
                    _ => path.clone(),
                };
                let mut provided = HashMap::<String, &Pattern>::new();
                for (name, field) in fields {
                    if provided.insert(name.clone(), field).is_some() {
                        self.error(
                            field.span,
                            format!("struct pattern field `{name}` appears more than once"),
                        );
                    }
                }
                for name in provided.keys() {
                    if !declared_fields.iter().any(|(field, _)| field == name) {
                        self.error(pattern.span, format!("unknown field `{path}.{name}`"));
                    }
                }
                self.pattern_depth += 1;
                let typed_fields = declared_fields
                    .iter()
                    .map(|(name, ty)| TPatternField {
                        ty: ty.clone(),
                        pattern: provided
                            .get(name)
                            .map_or(TPattern::Wildcard, |field| self.type_pattern(field, ty)),
                    })
                    .collect();
                self.pattern_depth -= 1;
                TPattern::Struct {
                    struct_name: expected_name,
                    fields: typed_fields,
                }
            }
        }
    }

    fn call(&mut self, expr: &Expr, callee: &str, args: &[Expr], expected: Option<&Type>) -> TExpr {
        if callee == "clone" {
            return self.clone_call(expr, args);
        }
        if callee == "." {
            return self.field_access(expr, args);
        }
        if callee == "list" || callee == "array" {
            return self.collection_literal(expr, callee, args, expected);
        }
        if callee == "slice" {
            return self.slice_operation(expr, args);
        }
        if matches!(
            callee,
            "len" | "push" | "get" | "get-ref" | "pop" | "remove" | "replace"
        ) {
            return self.list_operation(expr, callee, args);
        }
        if matches!(callee, "volatile-read" | "volatile-write" | "ptr-offset") {
            return self.pointer_operation(expr, callee, args);
        }
        if self.structs.contains_key(callee) {
            return self.struct_init(expr, callee, args, expected);
        }
        if self.generic_structs.contains_key(callee) {
            return self.struct_init(expr, callee, args, expected);
        }
        if self.variants.contains_key(callee) {
            return self.enum_init(expr, callee, args, expected);
        }
        if is_operator(callee) {
            return self.operator_call(expr, callee, args, expected);
        }
        let Some(signature) = self.signatures.get(callee).cloned() else {
            // The mirror of the fallback in `variable`, and in the same
            // direction: the signature table is consulted first, so a local
            // never takes a call away from a `fn` of the same name (`D-092`).
            if self.env.lookup_id(callee).is_some() {
                return self.call_value(expr, callee, args);
            }
            self.error(expr.span, format!("unknown function `{callee}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        // Before the arity check and before `generic_call` takes over, so that
        // every call to the name warns exactly once however it is checked.
        if let Some(deprecation) = signature.deprecated.clone() {
            self.deprecated_use(callee, expr.span, &deprecation);
        }
        if args.len() != signature.params.len() {
            self.error(
                expr.span,
                format!(
                    "`{callee}` expects {} arguments, found {}",
                    signature.params.len(),
                    args.len()
                ),
            );
        }
        if !signature.type_params.is_empty() {
            return self.generic_call(expr, callee, args, expected, &signature);
        }
        let mut typed_args = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            typed_args.push(self.expr(arg, signature.params.get(index)));
        }
        self.typed(
            expr,
            signature.result,
            TExprKind::Call {
                callee: callee.to_owned(),
                args: typed_args,
            },
        )
    }

    /// A `lambda`, and the moves that fill its environment (`D-102`).
    ///
    /// The captures are read in the enclosing scope, so every ownership rule
    /// that applies to handing a value to a function applies to handing one to
    /// a `lambda`: a moved binding cannot be captured and a borrowed one
    /// cannot be moved. The body is then typed against an environment holding
    /// the captures and the parameters and *nothing else*, which is what makes
    /// a name it forgot to capture a diagnostic rather than a silent move.
    fn lambda(
        &mut self,
        expr: &Expr,
        captures: &[Capture],
        params: &[Param],
        result: &Type,
        body: &Expr,
    ) -> TExpr {
        let mut taken = Vec::new();
        for capture in captures {
            if self.env.lookup_id(&capture.name).is_none() {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::NAME_OR_TYPE,
                        self.file,
                        capture.span,
                        format!("`{}` is not a value in scope", capture.name),
                    )
                    .with_help("a capture names a binding the lambda is written inside of"),
                );
                continue;
            }
            let site = Expr {
                kind: ExprKind::Var {
                    name: capture.name.clone(),
                    resolved: None,
                },
                span: capture.span,
            };
            let value = self.variable(&site, &capture.name, None, true, None);
            let TExprKind::Var(from) = value.kind else {
                continue;
            };
            // The environment is an aggregate that outlives the frame it was
            // built in, so a borrow in it is the thing `SL0300` refuses
            // everywhere else — and a `Ref` is `Copy`, so nothing else would
            // have stopped it. Found by probing, not by a test.
            if contains_borrowed_type(&value.ty) {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::OWNERSHIP,
                        self.file,
                        capture.span,
                        format!(
                            "cannot capture `{}`, which is a `{}`",
                            capture.name, value.ty
                        ),
                    )
                    .with_help(
                        "a closure can outlive the frame it was written in, so it holds \
                         values and not borrows: capture what the borrow points at, or \
                         `clone` it first",
                    ),
                );
            }
            taken.push((capture, from, value.ty));
        }

        // The body sees an environment of its own. Swapping the whole thing is
        // what enforces `D-102`: there is no enclosing scope to fall through
        // to, so an uncaptured name is undefined rather than free.
        self.enclosing.push(std::mem::take(&mut self.env));
        let outer_pattern_borrow = self.pattern_borrow.take();
        let outer_return_type = self.current_return_type.take();
        // The permission does not cross into the body (`D-067`). A `lambda`
        // written inside an `unsafe` block is still a function value that can
        // be called from anywhere, so its body has to ask for the permission
        // itself — the same reason the environment is swapped rather than
        // pushed.
        let outer_unsafe_depth = std::mem::take(&mut self.unsafe_depth);
        self.env.push();

        let mut typed_captures = Vec::new();
        for (capture, from, ty) in taken {
            let id = self.fresh_id();
            let binding = Binding {
                id,
                ty: ty.clone(),
                mutable: false,
                state: OwnershipState::Available,
                definition: capture.span,
                borrowed_from: None,
                owns_loan: false,
                captured: true,
                place: false,
            };
            if self.env.insert(capture.name.clone(), binding).is_err() {
                self.error(
                    capture.span,
                    format!("`{}` is captured more than once", capture.name),
                );
            }
            typed_captures.push(TCapture {
                from,
                id,
                name: capture.name.clone(),
                ty,
            });
        }

        let mut typed_params = Vec::new();
        for param in params {
            // The written type, not the normalized one: normalizing rewrites a
            // generic instance to the name monomorphization gave it, which is
            // not a type anything wrote and not one this recognizes. The same
            // order a declaration is checked in.
            self.validate_type(&param.ty, param.span);
            let parameter_type = self.normalize_type(&param.ty, param.span);
            let id = self.fresh_id();
            let binding = Binding {
                id,
                ty: parameter_type.clone(),
                mutable: false,
                state: OwnershipState::Available,
                definition: param.span,
                borrowed_from: None,
                owns_loan: false,
                captured: false,
                place: false,
            };
            self.refuse_function_shadow(&param.name, &parameter_type, param.span);
            if self.env.insert(param.name.clone(), binding).is_err() {
                self.error(
                    param.span,
                    format!("parameter `{}` is declared more than once", param.name),
                );
            }
            typed_params.push(TypedParam {
                id,
                name: param.name.clone(),
                ty: parameter_type,
                span: param.span,
            });
        }

        self.validate_type(result, expr.span);
        let result_type = self.normalize_type(result, expr.span);
        // The same refusal a declaration gets, for the same reason: a borrow
        // returned from a call outlives what it points at. A `lambda` is
        // checked here because it is not a declaration and never passes the
        // loop that checks those.
        if contains_borrowed_type(&result_type) {
            self.error_with_code(
                codes::OWNERSHIP,
                expr.span,
                "borrowed values cannot be returned from functions",
            );
        }
        self.current_return_type = Some(result_type.clone());
        let typed_body = self.expr(body, Some(&result_type));
        self.env.pop();
        self.env = self.enclosing.pop().unwrap_or_default();
        self.pattern_borrow = outer_pattern_borrow;
        self.current_return_type = outer_return_type;
        self.unsafe_depth = outer_unsafe_depth;

        let ty = Type::Fn {
            params: typed_params.iter().map(|param| param.ty.clone()).collect(),
            result: Box::new(result_type.clone()),
        };
        self.typed(
            expr,
            ty,
            TExprKind::Lambda {
                captures: typed_captures,
                params: typed_params,
                result: result_type,
                body: Box::new(typed_body),
            },
        )
    }

    /// A call through a local of `Fn` type, or of a borrow of one.
    ///
    /// The callee is read without consuming it, which is what lets a parameter
    /// be called as often as its body likes. Since `D-101` a function value is
    /// owned, so *passing* one twice is two moves and the answer is the one the
    /// language already has — take it by borrow, and call through that.
    fn call_value(&mut self, expr: &Expr, callee: &str, args: &[Expr]) -> TExpr {
        let id = self
            .env
            .lookup_id(callee)
            .expect("the caller checked the name is bound");
        let Some(binding) = self.env.bindings.get(&id).cloned() else {
            self.error(expr.span, format!("undefined variable `{callee}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Type::Fn { params, result } = binding.ty.strip_ref().clone() else {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("`{callee}` is a `{}`, not a function", binding.ty),
                )
                .with_label(binding.definition, "value was declared here"),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        // Written at v0.7.2 against the day a function value stopped being
        // `Copy`, and unreachable until `D-101` made that day this one: a `Fn`
        // handed to something else is gone, and calling it afterwards is the
        // use this notices.
        if binding.state == OwnershipState::Moved {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    expr.span,
                    format!("use of moved value `{callee}`"),
                )
                .with_label(binding.definition, "value was declared here"),
            );
        }
        if args.len() != params.len() {
            self.error(
                expr.span,
                format!(
                    "`{callee}` expects {} arguments, found {}",
                    params.len(),
                    args.len()
                ),
            );
        }
        // Each argument is typed against the parameter and nothing more, which
        // is exactly what an ordinary call does: the expected type is what
        // reports a mismatch, and checking it again here said the same thing
        // twice at the same span.
        let mut typed_args = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            typed_args.push(self.expr(arg, params.get(index)));
        }
        self.typed(
            expr,
            *result,
            TExprKind::CallValue {
                callee: id,
                args: typed_args,
            },
        )
    }

    /// The type parameters nothing has decided yet.
    ///
    /// This is the set a template is checked against before it is handed to an
    /// argument as an expectation, and it is not the same as the whole
    /// parameter set: a parameter that has been substituted is gone, and what
    /// stands in its place may be spelled with the *enclosing* function's
    /// parameters, which are names this call has no say over. Checking against
    /// every parameter instead made the two happen to collide — a `K`
    /// substituted by a caller's `K` looked unbound — and a call inside a
    /// generic body then lost an expectation it had (`D-104`).
    fn unbound(
        &self,
        parameters: &HashSet<String>,
        substitutions: &HashMap<String, Type>,
    ) -> HashSet<String> {
        parameters
            .iter()
            .filter(|parameter| !substitutions.contains_key(*parameter))
            .cloned()
            .collect()
    }

    fn generic_call(
        &mut self,
        expr: &Expr,
        callee: &str,
        args: &[Expr],
        expected: Option<&Type>,
        signature: &Signature,
    ) -> TExpr {
        if args.len() != signature.params.len() {
            self.error(
                expr.span,
                format!(
                    "`{callee}` expects {} arguments, found {}",
                    signature.params.len(),
                    args.len()
                ),
            );
        }
        let parameters = signature
            .type_params
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let mut substitutions = HashMap::<String, Type>::new();
        // What the call site expects of the result is read *before* the
        // arguments, because it is what tells them what they are. `(fold map
        // (list) step)` is the case: the accumulator's type is the result's,
        // nothing about an empty list says what it holds, and the answer is
        // already written where the call is (`D-104`).
        if let Some(expected) = expected {
            if let Err(message) = unify_type(
                &signature.result,
                expected,
                &parameters,
                &mut substitutions,
                self.instances(),
            ) {
                self.error(expr.span, message);
            }
        }
        let mut typed_args = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let template = signature.params.get(index);
            // A parameter is only unknown while it is still unbound. Once the
            // result — or an earlier argument — has said what `T` is, the
            // template with `T` substituted is a type the argument can be
            // written against, even inside a generic body where what `T`
            // became is the caller's own parameter.
            let unbound = self.unbound(&parameters, &substitutions);
            let filled = template
                .map(|ty| substitute_type(ty, &substitutions))
                .filter(|ty| !contains_parameter(ty, &unbound))
                // Normalized only once it is bound: an unbound parameter is
                // not a type, and normalizing one would instantiate a generic
                // under the parameter's own name.
                .map(|ty| self.normalize_type(&ty, argument.span));
            let typed = self.expr(argument, filled.as_ref());
            if let Some(template) = template {
                if let Err(message) = unify_type(
                    template,
                    &typed.ty,
                    &parameters,
                    &mut substitutions,
                    self.instances(),
                ) {
                    self.error(argument.span, message);
                }
            }
            typed_args.push(typed);
        }
        let mut type_args = Vec::new();
        for parameter in &signature.type_params {
            if let Some(argument) = substitutions.get(parameter) {
                type_args.push(argument.clone());
            } else {
                // The fix is a place to write the type down, and since v0.9.1
                // there is one at every binding (`D-121`). Naming it here is
                // why the typed `let` moved this snapshot: a diagnostic that
                // knows the answer should say it.
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::GENERIC,
                        self.file,
                        expr.span,
                        format!("cannot infer generic parameter `{parameter}` for `{callee}`"),
                    )
                    .with_help(
                        "write the type after the value — `(let name value : Type)` — or pass \
                         an argument that mentions it",
                    ),
                );
                type_args.push(Type::Unit);
            }
        }
        // Substitution answers what the result is; normalization answers what
        // it is *called*. A generic function returning `(Box T)` substitutes to
        // `(Box i64)`, and every type written down at a concrete call site has
        // already been rewritten to the name monomorphization gave it, so
        // without this the two spellings of one type do not compare equal.
        // Inside a generic body the arguments are still parameters and
        // `normalize_type` leaves the `Apply` alone, which is what
        // `specialize_expr` needs it to do.
        let result = substitute_type(&signature.result, &substitutions);
        let result = self.normalize_type(&result, expr.span);
        self.typed(
            expr,
            result,
            TExprKind::GenericCall {
                callee: callee.to_owned(),
                type_args,
                args: typed_args,
            },
        )
    }

    /// Types `(as target value)` against the table of allowed pairs.
    ///
    /// The table is one row — `i32` to `i64` — and it is a table rather than a
    /// rule so that v0.8 freezes a list a later version can add a row to
    /// (`D-090`). A conversion that is not in it is refused by name, because
    /// the alternative is a language where `as` means "trust me".
    /// `(as T value)`, which since `D-107` is a table rather than a single row.
    ///
    /// Every integer converts to every integer, by one rule: **the source's
    /// signedness extends and the target's width truncates**. So
    /// `(as u64 (i8 -1))` is every bit set and `(as u8 (i8 -1))` is `255`.
    /// Nothing else converts — `D-090` says a conversion is a form and never
    /// implicit, and widening the integer axis does not weaken that for `f64`
    /// or `bool`.
    fn convert(&mut self, expr: &Expr, target: &Type, value: &Expr) -> TExpr {
        let target = self.normalize_type(target, expr.span);
        // A literal takes the target's type directly, so `(as u64 …)` can be
        // written above 2^63 and `(as u8 300)` is refused rather than quietly
        // truncated. Only a literal: an expectation carried into anything else
        // would be a second, implicit conversion.
        let expected = match (&value.kind, &target) {
            (ExprKind::Int(_), target) if target.is_integer() => Some(target.clone()),
            // An address is written as a literal — `(as (Ptr u16) 0xB8000)` —
            // and it is a `u64` that is being written, so the literal is
            // checked against a `u64`'s range rather than an `i64`'s.
            (ExprKind::Int(_), Type::Ptr(_)) => Some(Type::U64),
            _ => None,
        };
        let value = self.expr(value, expected.as_ref());
        let crosses_pointer = matches!(target, Type::Ptr(_)) || matches!(value.ty, Type::Ptr(_));
        // A pointer converts to and from any integer, and to another pointer.
        // The address is a `u64` on both targets, so the arithmetic of the
        // conversion is an integer's and nothing here is a second rule
        // (`D-113`); what is new is only that one side may be an address.
        let legal = if crosses_pointer {
            let sides_are_words = |ty: &Type| ty.is_integer() || matches!(ty, Type::Ptr(_));
            sides_are_words(&value.ty) && sides_are_words(&target)
        } else {
            value.ty.is_integer() && target.is_integer()
        };
        if crosses_pointer && legal {
            self.require_unsafe(expr.span, "a conversion to or from a raw pointer");
        }
        if !legal {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("`as` cannot convert `{}` to `{target}`", value.ty),
                )
                .with_help(
                    "`as` converts between the integer types; the source's signedness \
                     extends and the target's width truncates",
                ),
            );
        }
        self.typed(
            expr,
            target,
            TExprKind::Convert {
                value: Box::new(value),
            },
        )
    }

    fn clone_call(&mut self, expr: &Expr, args: &[Expr]) -> TExpr {
        if args.len() != 1 {
            self.error(expr.span, "`clone` expects one argument");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let arg = match &args[0].kind {
            ExprKind::Var { name, resolved } => {
                self.variable(&args[0], name, resolved.as_deref(), false, None)
            }
            _ => self.expr(&args[0], None),
        };
        // `clone` crosses a borrow (`D-091`). Returning the borrow it was handed
        // made the call a no-op, and left the language with no way at all to
        // turn a `(& String)` into a `String`.
        //
        // It crosses an exclusive one too since `D-120`: a field bound by a
        // `(&mut ...)` match is read the same way a shared one is, and
        // `(set count (+ (clone count) 1))` is the shape every counter in the
        // library now has. Refusing it was right while nothing could hold an
        // exclusive borrow long enough to read it.
        let borrowed = matches!(arg.ty, Type::Ref { .. });
        let result = arg.ty.strip_ref().clone();
        // Only an *owned* scalar has nothing to clone (`D-100`). A borrowed one
        // has everything to clone: `(+ r 0)` and `(println-i64 r)` refuse a
        // `(& i64)` too, so this is the only way to read one, and refusing it
        // here is what left a borrowed scalar unreadable by any means.
        if !borrowed && is_nothing_to_clone(&result) {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("`clone` has nothing to copy out of `{result}`"),
                )
                .with_help("a scalar is copied by using it; drop the `clone`"),
            );
        }
        self.typed(
            expr,
            result,
            TExprKind::Call {
                callee: "clone".into(),
                args: vec![arg],
            },
        )
    }

    fn struct_init(
        &mut self,
        expr: &Expr,
        name: &str,
        args: &[Expr],
        expected: Option<&Type>,
    ) -> TExpr {
        if let Some(info) = self.generic_structs.get(name).cloned() {
            return self.generic_struct_init(expr, name, args, expected, &info);
        }
        let fields = self.structs.get(name).cloned().unwrap_or_default();
        if args.len() != fields.len() * 2 {
            self.error(
                expr.span,
                format!(
                    "`{name}` expects {} named fields as `:field value` pairs",
                    fields.len()
                ),
            );
        }
        let mut provided = HashMap::new();
        for pair in args.chunks(2) {
            if pair.len() != 2 {
                break;
            }
            let ExprKind::Var { name: keyword, .. } = &pair[0].kind else {
                self.error(pair[0].span, "field name must use `:name` syntax");
                continue;
            };
            let Some(field_name) = keyword.strip_prefix(':') else {
                self.error(pair[0].span, "field name must begin with `:`");
                continue;
            };
            if provided.insert(field_name.to_owned(), &pair[1]).is_some() {
                self.error(
                    pair[0].span,
                    format!("field `{field_name}` is provided twice"),
                );
            }
        }
        let mut typed_fields = Vec::new();
        for (field_name, field_type) in &fields {
            if let Some(value) = provided.get(field_name) {
                typed_fields.push(self.expr(value, Some(field_type)));
            } else {
                self.error(
                    expr.span,
                    format!("missing field `{field_name}` in `{name}`"),
                );
                typed_fields.push(self.typed(expr, field_type.clone(), TExprKind::Unit));
            }
        }
        self.typed(
            expr,
            Type::Named(name.to_owned()),
            TExprKind::StructInit {
                name: name.to_owned(),
                fields: typed_fields,
            },
        )
    }

    fn generic_struct_init(
        &mut self,
        expr: &Expr,
        name: &str,
        args: &[Expr],
        expected: Option<&Type>,
        info: &GenericStructInfo,
    ) -> TExpr {
        if args.len() != info.fields.len() * 2 {
            self.error(
                expr.span,
                format!(
                    "`{name}` expects {} named fields as `:field value` pairs",
                    info.fields.len()
                ),
            );
        }
        let mut provided = HashMap::new();
        for pair in args.chunks(2) {
            if pair.len() != 2 {
                break;
            }
            let ExprKind::Var { name: keyword, .. } = &pair[0].kind else {
                self.error(pair[0].span, "field name must use `:name` syntax");
                continue;
            };
            let Some(field_name) = keyword.strip_prefix(':') else {
                self.error(pair[0].span, "field name must begin with `:`");
                continue;
            };
            if provided.insert(field_name.to_owned(), &pair[1]).is_some() {
                self.error(
                    pair[0].span,
                    format!("field `{field_name}` is provided twice"),
                );
            }
        }
        let parameters = info.type_params.iter().cloned().collect::<HashSet<_>>();
        let mut substitutions = HashMap::new();
        if let Some(arguments) = self.expected_arguments(expected, name) {
            substitutions.extend(info.type_params.iter().cloned().zip(arguments));
        }
        let mut typed_fields = Vec::new();
        for (field_name, template) in &info.fields {
            let Some(value) = provided.get(field_name) else {
                self.error(
                    expr.span,
                    format!("missing field `{field_name}` in `{name}`"),
                );
                typed_fields.push(self.typed(expr, Type::Unit, TExprKind::Unit));
                continue;
            };
            let substituted = substitute_type(template, &substitutions);
            let unbound = self.unbound(&parameters, &substitutions);
            let partially_substituted = (!contains_parameter(&substituted, &unbound))
                .then(|| self.normalize_type(&substituted, value.span));
            let expected_field = partially_substituted.as_ref();
            let typed = self.expr(value, expected_field);
            if let Err(message) = unify_type(
                template,
                &typed.ty,
                &parameters,
                &mut substitutions,
                self.instances(),
            ) {
                self.error(value.span, message);
            }
            typed_fields.push(typed);
        }
        let type_args = self.generic_arguments(name, &info.type_params, &substitutions, expr.span);
        let ty = self.normalize_type(
            &Type::Apply {
                name: name.to_owned(),
                args: type_args,
            },
            expr.span,
        );
        let instance_name = match &ty {
            Type::Named(instance_name) => instance_name.clone(),
            Type::Apply { name, args } => generic_instance_name(name, args),
            _ => return self.typed(expr, Type::Unit, TExprKind::Unit),
        };
        self.typed(
            expr,
            ty,
            TExprKind::StructInit {
                name: instance_name,
                fields: typed_fields,
            },
        )
    }

    fn field_access(&mut self, expr: &Expr, args: &[Expr]) -> TExpr {
        if args.len() != 2 {
            self.error(expr.span, "field access syntax is `(. binding field)`");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let ExprKind::Var {
            name: base_name, ..
        } = &args[0].kind
        else {
            self.error(
                args[0].span,
                "field access currently requires a named binding",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let ExprKind::Var {
            name: field_name, ..
        } = &args[1].kind
        else {
            self.error(args[1].span, "field name must be an identifier");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(id) = self.env.lookup_id(base_name) else {
            self.error(args[0].span, format!("undefined variable `{base_name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let binding = self.env.bindings.get(&id).cloned().expect("binding exists");
        // A generic body sees `(Pair T)` rather than an instance, so the
        // fields come from the generic declaration with the application's
        // arguments substituted in (`D-095`). The name recorded here reaches
        // only the LSP — MIR indexes the field — and the base name is the
        // better answer for it anyway, because that is where the source is.
        let (struct_name, fields) = match &binding.ty {
            Type::Named(struct_name) => match self.structs.get(struct_name) {
                Some(fields) => (struct_name.clone(), fields.clone()),
                None => {
                    self.error(args[0].span, format!("`{struct_name}` is not a struct"));
                    return self.typed(expr, Type::Unit, TExprKind::Unit);
                }
            },
            Type::Apply {
                name,
                args: arguments,
            } if self.generic_structs.contains_key(name) => {
                let info = self.generic_structs[name].clone();
                let substitutions = info
                    .type_params
                    .iter()
                    .cloned()
                    .zip(arguments.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let fields = info
                    .fields
                    .iter()
                    .map(|(field, ty)| (field.clone(), substitute_type(ty, &substitutions)))
                    .collect::<Vec<_>>();
                (name.clone(), fields)
            }
            _ => {
                self.error(args[0].span, format!("`{base_name}` is not a struct"));
                return self.typed(expr, Type::Unit, TExprKind::Unit);
            }
        };
        let struct_name = &struct_name;
        let Some((index, (_, field_type))) = fields
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == field_name)
        else {
            self.error(
                args[1].span,
                format!("`{struct_name}` has no field `{field_name}`"),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let field_type = field_type.clone();
        if !field_type.is_copy() {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::OWNERSHIP,
                    self.file,
                    expr.span,
                    format!("moving owned field `{field_name}` requires pattern destructuring"),
                )
                .with_help("store a Copy field or pass the whole struct by ownership"),
            );
        }
        self.typed(
            expr,
            field_type,
            TExprKind::Field {
                base: id,
                struct_name: struct_name.clone(),
                index,
            },
        )
    }

    fn enum_init(
        &mut self,
        expr: &Expr,
        path: &str,
        args: &[Expr],
        expected: Option<&Type>,
    ) -> TExpr {
        let info = self.variants[path].clone();
        if !info.type_params.is_empty() {
            return self.generic_enum_init(expr, path, args, expected, &info);
        }
        if args.len() != info.fields.len() {
            self.error(
                expr.span,
                format!(
                    "`{path}` expects {} payload values, found {}",
                    info.fields.len(),
                    args.len()
                ),
            );
        }
        let mut fields = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            fields.push(self.expr(arg, info.fields.get(index).map(|(_, ty)| ty)));
        }
        self.typed(
            expr,
            Type::Named(info.enum_name.clone()),
            TExprKind::EnumInit {
                enum_name: info.enum_name,
                variant: info.variant,
                tag: info.tag,
                fields,
            },
        )
    }

    fn generic_enum_init(
        &mut self,
        expr: &Expr,
        path: &str,
        args: &[Expr],
        expected: Option<&Type>,
        info: &VariantInfo,
    ) -> TExpr {
        if args.len() != info.fields.len() {
            self.error(
                expr.span,
                format!(
                    "`{path}` expects {} payload values, found {}",
                    info.fields.len(),
                    args.len()
                ),
            );
        }
        let parameters = info.type_params.iter().cloned().collect::<HashSet<_>>();
        let mut substitutions = HashMap::new();
        if let Some(arguments) = self.expected_arguments(expected, &info.enum_name) {
            substitutions.extend(info.type_params.iter().cloned().zip(arguments));
        }
        let mut fields = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let Some((_, template)) = info.fields.get(index) else {
                fields.push(self.expr(argument, None));
                continue;
            };
            let substituted = substitute_type(template, &substitutions);
            let unbound = self.unbound(&parameters, &substitutions);
            let partially_substituted = (!contains_parameter(&substituted, &unbound))
                .then(|| self.normalize_type(&substituted, argument.span));
            let expected_field = partially_substituted.as_ref();
            let typed = self.expr(argument, expected_field);
            if let Err(message) = unify_type(
                template,
                &typed.ty,
                &parameters,
                &mut substitutions,
                self.instances(),
            ) {
                self.error(argument.span, message);
            }
            fields.push(typed);
        }
        let type_args = self.generic_arguments(
            &info.enum_name,
            &info.type_params,
            &substitutions,
            expr.span,
        );
        let ty = self.normalize_type(
            &Type::Apply {
                name: info.enum_name.clone(),
                args: type_args,
            },
            expr.span,
        );
        let instance_name = match &ty {
            Type::Named(instance_name) => instance_name.clone(),
            Type::Apply { name, args } => generic_instance_name(name, args),
            _ => return self.typed(expr, Type::Unit, TExprKind::Unit),
        };
        self.typed(
            expr,
            ty,
            TExprKind::EnumInit {
                enum_name: instance_name,
                variant: info.variant.clone(),
                tag: info.tag,
                fields,
            },
        )
    }

    fn generic_arguments(
        &mut self,
        name: &str,
        parameters: &[String],
        substitutions: &HashMap<String, Type>,
        span: Span,
    ) -> Vec<Type> {
        parameters
            .iter()
            .map(|parameter| {
                substitutions.get(parameter).cloned().unwrap_or_else(|| {
                    self.error_with_code(
                        codes::GENERIC,
                        span,
                        format!("cannot infer generic parameter `{parameter}` for `{name}`"),
                    );
                    Type::Unit
                })
            })
            .collect()
    }

    fn collection_literal(
        &mut self,
        expr: &Expr,
        callee: &str,
        args: &[Expr],
        expected: Option<&Type>,
    ) -> TExpr {
        if args.is_empty() {
            // An empty literal has no element to read a type off, so the
            // expected type is the only thing that can say what it holds — the
            // same rule `(Option:None)` follows, carried by the same plumbing
            // (`D-096`). Without it neither `map` nor `filter` can be written,
            // because both must return an empty list when handed one.
            let inferred = match expected {
                Some(Type::List(_)) if callee == "list" => expected.cloned(),
                Some(Type::Array { length: 0, .. }) if callee == "array" => expected.cloned(),
                _ => None,
            };
            if let Some(ty) = inferred {
                return self.typed(
                    expr,
                    ty,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: Vec::new(),
                    },
                );
            }
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("cannot infer the type of an empty {callee}"),
                )
                .with_help(format!(
                    "provide at least one element in `({callee} ...)`, or write it where the \
                     expected type says what it holds"
                )),
            );
            // Recover as whatever was expected, so the one cause reports once.
            // Inventing a `unit` element used to add "expected `List<i64>`,
            // found `List<unit>`" underneath every empty literal.
            let ty = expected.cloned().unwrap_or(if callee == "array" {
                Type::Array {
                    element: Box::new(Type::Unit),
                    length: 0,
                }
            } else {
                Type::List(Box::new(Type::Unit))
            });
            return self.typed(
                expr,
                ty,
                TExprKind::Call {
                    callee: callee.into(),
                    args: Vec::new(),
                },
            );
        }
        let first = self.expr(&args[0], None);
        let element_type = first.ty.clone();
        if contains_borrowed_type(&element_type) {
            self.error_with_code(
                codes::OWNERSHIP,
                expr.span,
                "borrowed values cannot be stored in collections",
            );
        }
        let mut typed = vec![first];
        for arg in &args[1..] {
            typed.push(self.expr(arg, Some(&element_type)));
        }
        self.typed(
            expr,
            if callee == "array" {
                Type::Array {
                    element: Box::new(element_type),
                    length: args.len(),
                }
            } else {
                Type::List(Box::new(element_type))
            },
            TExprKind::Call {
                callee: callee.into(),
                args: typed,
            },
        )
    }

    fn slice_operation(&mut self, expr: &Expr, args: &[Expr]) -> TExpr {
        if args.len() != 3 {
            self.error(
                expr.span,
                format!("`slice` expects 3 arguments, found {}", args.len()),
            );
            return self.typed(expr, Type::Slice(Box::new(Type::Unit)), TExprKind::Unit);
        }
        let collection = self.expr(&args[0], None);
        let element = match &collection.ty {
            Type::Ref { inner, .. } => match inner.as_ref() {
                Type::List(element) => element.as_ref().clone(),
                Type::Array { element, .. } => element.as_ref().clone(),
                other => {
                    self.error(
                        args[0].span,
                        format!("`slice` expects `&List<T>` or `&Array<T, N>`, found `&{other}`"),
                    );
                    Type::Unit
                }
            },
            other => {
                self.error(
                    args[0].span,
                    format!("`slice` expects a borrowed collection, found `{other}`"),
                );
                Type::Unit
            }
        };
        let start = self.expr(&args[1], Some(&Type::I64));
        let end = self.expr(&args[2], Some(&Type::I64));
        self.typed(
            expr,
            Type::Slice(Box::new(element)),
            TExprKind::Call {
                callee: "slice".into(),
                args: vec![collection, start, end],
            },
        )
    }

    /// Types the three raw-pointer builtins (`D-067`).
    ///
    /// `(volatile-read p)` answers the pointee, `(volatile-write p v)` answers
    /// `unit`, and `(ptr-offset p n)` answers another pointer `n` elements
    /// along. All three ask for the permission, and all three refuse to say
    /// anything else about a program that did not have a pointer to begin
    /// with: the first diagnostic is about the type, not about the word.
    fn pointer_operation(&mut self, expr: &Expr, callee: &str, args: &[Expr]) -> TExpr {
        let expected_len = if callee == "volatile-read" { 1 } else { 2 };
        if args.len() != expected_len {
            self.error(
                expr.span,
                format!(
                    "`{callee}` expects {expected_len} argument(s), found {}",
                    args.len()
                ),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        self.require_unsafe(expr.span, format!("`{callee}`"));
        let pointer = self.expr(&args[0], None);
        let Type::Ptr(pointee) = pointer.ty.clone() else {
            self.error(
                args[0].span,
                format!("`{callee}` expects a raw pointer, found `{}`", pointer.ty),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let pointee = pointee.as_ref().clone();
        match callee {
            "volatile-read" => self.typed(
                expr,
                pointee,
                TExprKind::Call {
                    callee: callee.into(),
                    args: vec![pointer],
                },
            ),
            "volatile-write" => {
                // The value is typed against the pointee, so a `u8` register
                // takes a `u8` and `D-090`'s "never implicit" is not weakened
                // by the value happening to be a literal.
                let before = self.diagnostics.len();
                let value = self.expr(&args[1], Some(&pointee));
                // Only when the expectation did not already say so: it reports
                // the mismatch for everything that carries one, and two
                // diagnostics about one wrong argument is one too many. This
                // is the fallback for a value whose type nothing else checked.
                if value.ty != pointee && self.diagnostics.len() == before {
                    self.error(
                        args[1].span,
                        format!(
                            "`volatile-write` through a `{}` expects a `{pointee}`, found `{}`",
                            pointer.ty, value.ty
                        ),
                    );
                }
                self.typed(
                    expr,
                    Type::Unit,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: vec![pointer, value],
                    },
                )
            }
            _ => {
                // The count is a `u64` because an address is: the arithmetic
                // stays unsigned end to end, and it traps on overflow like any
                // other (`D-031`). A program that wants to go backwards
                // computes the base it wants.
                let count = self.expr(&args[1], Some(&Type::U64));
                if count.ty != Type::U64 {
                    self.error(
                        args[1].span,
                        format!("`ptr-offset` expects a `u64` count, found `{}`", count.ty),
                    );
                }
                let ty = pointer.ty.clone();
                self.typed(
                    expr,
                    ty,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: vec![pointer, count],
                    },
                )
            }
        }
    }

    fn list_operation(&mut self, expr: &Expr, callee: &str, args: &[Expr]) -> TExpr {
        let expected_len = match callee {
            "replace" => 3,
            "push" | "get" | "get-ref" | "remove" => 2,
            _ => 1,
        };
        if args.len() != expected_len {
            self.error(
                expr.span,
                format!(
                    "`{callee}` expects {expected_len} argument(s), found {}",
                    args.len()
                ),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let list = self.expr(&args[0], None);
        // A string is not a collection — it has no element type and nothing
        // else in this family applies to it — but its byte length is the one
        // thing the library cannot compute for itself, because the boundary
        // hands C a pointer and no length (`D-079`).
        if callee == "len"
            && matches!(&list.ty, Type::Ref { inner, .. } if inner.as_ref() == &Type::String)
        {
            return self.typed(
                expr,
                Type::I64,
                TExprKind::Call {
                    callee: callee.into(),
                    args: vec![list],
                },
            );
        }
        let (mutable, element, kind) = match &list.ty {
            Type::Ref { mutable, inner } => match inner.as_ref() {
                Type::List(element) => (*mutable, element.as_ref().clone(), CollectionKind::List),
                Type::Array { element, .. } => {
                    (*mutable, element.as_ref().clone(), CollectionKind::Array)
                }
                Type::Slice(element) => (*mutable, element.as_ref().clone(), CollectionKind::Slice),
                other => {
                    self.error(
                        args[0].span,
                        format!("expected a collection reference, found `&{other}`"),
                    );
                    (false, Type::Unit, CollectionKind::List)
                }
            },
            other => {
                self.diagnostics.push(
                    Diagnostic::error(
                        codes::NAME_OR_TYPE,
                        self.file,
                        args[0].span,
                        format!("`{callee}` expects a borrowed List, found `{other}`"),
                    )
                    .with_help(
                        if matches!(callee, "push" | "pop" | "remove" | "replace") {
                            "pass `(&mut list)`"
                        } else {
                            "pass `(& list)`"
                        },
                    ),
                );
                (false, Type::Unit, CollectionKind::List)
            }
        };
        if matches!(callee, "push" | "pop" | "remove" | "replace") && kind != CollectionKind::List {
            self.error(
                args[0].span,
                format!("`{callee}` is only available for List"),
            );
        }
        if matches!(callee, "push" | "pop" | "remove" | "replace") && !mutable {
            self.error(args[0].span, format!("`{callee}` requires `&mut List`"));
        }
        let mut typed = vec![list];
        match callee {
            "push" => {
                typed.push(self.expr(&args[1], Some(&element)));
                self.typed(
                    expr,
                    Type::Unit,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            "get" => {
                if !element.is_copy() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            codes::OWNERSHIP,
                            self.file,
                            expr.span,
                            format!("`get` cannot copy a List element of type `{element}`"),
                        )
                        .with_help(
                            "use `(get-ref (& list) index)` to borrow it, or `remove` to move it",
                        ),
                    );
                }
                typed.push(self.expr(&args[1], Some(&Type::I64)));
                self.typed(
                    expr,
                    element,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            "get-ref" => {
                typed.push(self.expr(&args[1], Some(&Type::I64)));
                self.typed(
                    expr,
                    Type::Ref {
                        mutable: false,
                        inner: Box::new(element),
                    },
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            "pop" => {
                let Some(option) = self.language_items.option.clone() else {
                    self.error_with_code(
                        codes::STANDARD_LIBRARY,
                        expr.span,
                        "`pop` requires the `option` language item",
                    );
                    return self.typed(expr, Type::Unit, TExprKind::Unit);
                };
                let result = self.normalize_type(
                    &Type::Apply {
                        name: option,
                        args: vec![element],
                    },
                    expr.span,
                );
                self.typed(
                    expr,
                    result,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            "remove" => {
                typed.push(self.expr(&args[1], Some(&Type::I64)));
                self.typed(
                    expr,
                    element,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            // The only write to an element a list has. It is a swap and not an
            // assignment because the element already there is owned: an
            // assignment would have to drop it, and returning it says instead
            // that the caller decides, which is the same shape as `remove`
            // (`D-103`).
            "replace" => {
                typed.push(self.expr(&args[1], Some(&Type::I64)));
                typed.push(self.expr(&args[2], Some(&element)));
                self.typed(
                    expr,
                    element,
                    TExprKind::Call {
                        callee: callee.into(),
                        args: typed,
                    },
                )
            }
            "len" => self.typed(
                expr,
                Type::I64,
                TExprKind::Call {
                    callee: callee.into(),
                    args: typed,
                },
            ),
            _ => unreachable!(),
        }
    }

    fn operator_call(
        &mut self,
        expr: &Expr,
        callee: &str,
        args: &[Expr],
        expected: Option<&Type>,
    ) -> TExpr {
        let spec = OPERATORS
            .iter()
            .find(|spec| spec.name == callee)
            .expect("`call` only routes names this table holds");
        if !spec.arity.accepts(args.len()) {
            self.error(
                expr.span,
                format!("operator `{callee}` expects {}", spec.arity.describe()),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let numeric_expectation = match expected {
            Some(ty) if ty.is_integer() || *ty == Type::F64 => expected,
            _ => None,
        };
        let left = self.expr(&args[0], numeric_expectation);
        // The right operand takes the left one's type, including for a shift:
        // `Instruction::Binary` records one operand type and both backends
        // select from it, so a count of a different width would make that field
        // a half-truth. `D-090` says a conversion is written, and a shift is not
        // the operator that gets an exception.
        let right = (args.len() == 2).then(|| self.expr(&args[1], Some(&left.ty)));
        if !spec.domain.accepts(&left.ty) {
            let mut diagnostic = Diagnostic::error(
                codes::NAME_OR_TYPE,
                self.file,
                expr.span,
                format!("operator `{callee}` does not support `{}`", left.ty),
            );
            // `D-089`'s help, and `!=` inherits it by sharing this line rather
            // than by a second claim that the two behave alike.
            if matches!(spec.domain, Domain::Scalar) && is_text(&left.ty) {
                diagnostic = diagnostic.with_help("compare text with `core:string:equals`");
            }
            self.diagnostics.push(diagnostic);
        }
        if spec.shift {
            self.check_shift_amount(callee, &left.ty, args.get(1));
        }
        // Negation on an unsigned type can only answer for zero, so it is a
        // mistake rather than a value that traps. Refusing it here says so with
        // the type in hand, where the runtime could only say "overflow".
        if callee == "-" && args.len() == 1 {
            if let Some(kind) = left.ty.int_kind() {
                if !kind.signed {
                    self.diagnostics.push(
                        Diagnostic::error(
                            codes::NAME_OR_TYPE,
                            self.file,
                            expr.span,
                            format!("`{}` is unsigned and has no negation", left.ty),
                        )
                        .with_help("subtract from zero if that is what you meant, and it traps"),
                    );
                }
            }
        }
        let result = if spec.answers_bool {
            Type::Bool
        } else {
            left.ty.clone()
        };
        let mut typed = vec![left];
        typed.extend(right);
        self.typed(
            expr,
            result,
            TExprKind::Call {
                callee: callee.to_owned(),
                args: typed,
            },
        )
    }

    /// Refuses a shift by a literal amount the type has no room for.
    ///
    /// Only a literal: everything else is a value, and a value is checked where
    /// every other trapping input is, at run time. But when the count is
    /// written down the compiler knows, and a diagnostic naming the width beats
    /// a panic that names none.
    fn check_shift_amount(&mut self, callee: &str, ty: &Type, amount: Option<&Expr>) {
        let Some(amount) = amount else { return };
        let ExprKind::Int(written) = amount.kind else {
            return;
        };
        // The bound is the type's own width, not the machine word's: a `u8`
        // shifted by 8 has nothing left, and traps for the same reason an
        // `i64` shifted by 64 does (`D-107`).
        let width = u64::from(ty.int_kind().map_or(64, |kind| kind.bits));
        if !written.negative && written.magnitude < width {
            return;
        }
        self.diagnostics.push(
            Diagnostic::error(
                codes::NAME_OR_TYPE,
                self.file,
                amount.span,
                format!("`{callee}` by {written} leaves nothing of `{ty}`"),
            )
            .with_help(format!(
                "a shift amount is between 0 and {}; anything else traps",
                width - 1
            )),
        );
    }

    fn materialize_typed_expr(&mut self, expression: &mut TExpr) {
        expression.ty = self.normalize_type(&expression.ty, expression.span);
        match &mut expression.kind {
            TExprKind::Let { value, .. } | TExprKind::Set { value, .. } => {
                self.materialize_typed_expr(value);
            }
            TExprKind::Do(items) => {
                for item in items {
                    self.materialize_typed_expr(item);
                }
            }
            TExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.materialize_typed_expr(condition);
                self.materialize_typed_expr(then_expr);
                self.materialize_typed_expr(else_expr);
            }
            TExprKind::Loop { body } => self.materialize_typed_expr(body),
            TExprKind::While { condition, body } => {
                self.materialize_typed_expr(condition);
                self.materialize_typed_expr(body);
            }
            TExprKind::Match { value, arms } => {
                self.materialize_typed_expr(value);
                // The scrutinee is materialized first, so it is the thing that
                // knows which instance an arm matches on. A pattern written in
                // a generic body only ever recorded the base name (`D-095`).
                let scrutinee = value.ty.clone();
                for arm in arms {
                    self.materialize_pattern(&mut arm.pattern, &scrutinee, arm.span);
                    if let Some(guard) = &mut arm.guard {
                        self.materialize_typed_expr(guard);
                    }
                    self.materialize_typed_expr(&mut arm.body);
                }
            }
            TExprKind::Call { args, .. }
            | TExprKind::GenericCall { args, .. }
            | TExprKind::CallValue { args, .. } => {
                for argument in args {
                    self.materialize_typed_expr(argument);
                }
            }
            // A `lambda` written in a generic body carries its parameters and
            // its captures as types of their own, and every one of them can
            // mention the enclosing function's parameters.
            TExprKind::Lambda {
                captures,
                params,
                result,
                body,
            } => {
                for capture in captures {
                    capture.ty = self.normalize_type(&capture.ty, expression.span);
                }
                for param in params {
                    param.ty = self.normalize_type(&param.ty, param.span);
                }
                *result = self.normalize_type(result, expression.span);
                self.materialize_typed_expr(body);
            }
            TExprKind::FnRef { type_args, .. } => {
                for argument in type_args {
                    *argument = self.normalize_type(argument, expression.span);
                }
            }
            TExprKind::Convert { value } => self.materialize_typed_expr(value),
            TExprKind::Try {
                value,
                ok_type,
                enum_name,
                ..
            } => {
                self.materialize_typed_expr(value);
                *ok_type = self.normalize_type(ok_type, expression.span);
                if let Type::Named(name) = &value.ty {
                    *enum_name = name.clone();
                }
            }
            TExprKind::StructInit { name, fields } => {
                for field in fields {
                    self.materialize_typed_expr(field);
                }
                if let Type::Named(instance) = &expression.ty {
                    *name = instance.clone();
                }
            }
            TExprKind::EnumInit {
                enum_name, fields, ..
            } => {
                for field in fields {
                    self.materialize_typed_expr(field);
                }
                if let Type::Named(instance) = &expression.ty {
                    *enum_name = instance.clone();
                }
            }
            TExprKind::Break(value) => {
                if let Some(value) = value {
                    self.materialize_typed_expr(value);
                }
            }
            TExprKind::Const { value, .. } => self.materialize_typed_expr(value),
            TExprKind::Unit
            | TExprKind::Bool(_)
            | TExprKind::Int(_)
            | TExprKind::Float(_)
            | TExprKind::String(_)
            | TExprKind::Var(_)
            | TExprKind::Borrow { .. }
            | TExprKind::Continue
            | TExprKind::Field { .. } => {}
        }
    }

    /// Settle a pattern's types, and the aggregate it names, against the
    /// materialized type of what it matches.
    ///
    /// The name comes from the scrutinee rather than from the pattern, because
    /// a pattern written inside a generic body knows only the base name and
    /// the instance does not exist until here. For concrete code the two agree
    /// — the pattern already recorded the instance the scrutinee has.
    fn materialize_pattern(&mut self, pattern: &mut TPattern, scrutinee: &Type, span: Span) {
        match pattern {
            TPattern::Binding(binding) => {
                binding.ty = self.normalize_type(&binding.ty, span);
            }
            TPattern::Enum {
                enum_name, fields, ..
            } => {
                for field in fields {
                    field.ty = self.normalize_type(&field.ty, span);
                    let field_type = field.ty.clone();
                    self.materialize_pattern(&mut field.pattern, &field_type, span);
                }
                if let Type::Named(name) = self.normalize_type(scrutinee, span) {
                    *enum_name = name;
                }
            }
            TPattern::Struct {
                struct_name,
                fields,
            } => {
                for field in fields {
                    field.ty = self.normalize_type(&field.ty, span);
                    let field_type = field.ty.clone();
                    self.materialize_pattern(&mut field.pattern, &field_type, span);
                }
                if let Type::Named(name) = self.normalize_type(scrutinee, span) {
                    *struct_name = name;
                }
            }
            TPattern::Wildcard | TPattern::Bool(_) | TPattern::Int(_) => {}
        }
    }

    /// The type arguments an expected type supplies for a named generic
    /// aggregate, whether it arrived as a monomorphized instance or as an
    /// application a generic body has not instantiated yet (`D-095`).
    fn expected_arguments(&self, expected: Option<&Type>, base_name: &str) -> Option<Vec<Type>> {
        match expected? {
            Type::Named(instance) => self
                .instances()
                .application_of(instance)
                .filter(|(base, _)| base == base_name)
                .map(|(_, arguments)| arguments.clone()),
            Type::Apply { name, args } if name == base_name => Some(args.clone()),
            _ => None,
        }
    }

    /// The two instance tables, for the free functions that need to see
    /// through a monomorphized name to the application it came from.
    fn instances(&self) -> Instances<'_> {
        Instances {
            enums: &self.enum_instances,
            structs: &self.struct_instances,
        }
    }

    /// The variants of the enum a value of this type has, with the enum's type
    /// parameters already substituted, and the base name to record them under.
    ///
    /// A `Named` is a monomorphized instance and its variants are already
    /// concrete. An `Apply` is a generic body's view of one — `(Option T)` —
    /// where the variants exist but their payloads are still parameters
    /// (`D-095`).
    fn enum_of_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Named(name)
                if self
                    .variants
                    .values()
                    .any(|variant| &variant.enum_name == name)
                    || self.enum_instances.contains_key(name) =>
            {
                Some(name.clone())
            }
            Type::Apply { name, .. } if self.generic_enums.contains_key(name) => Some(name.clone()),
            _ => None,
        }
    }

    fn typed(&self, expr: &Expr, ty: Type, kind: TExprKind) -> TExpr {
        TExpr {
            kind,
            ty,
            span: expr.span,
        }
    }

    fn fresh_id(&mut self) -> BindingId {
        let id = self.next_binding;
        self.next_binding += 1;
        id
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.error_with_code(codes::NAME_OR_TYPE, span, message);
    }

    /// A use of a declaration somebody marked `deprecated` (`D-122`).
    ///
    /// The span is the use rather than the declaration, because the use is
    /// what the reader has to change, and the annotation's message — when it
    /// carries one — is a note rather than the message, so that every one of
    /// these warnings reads the same at its first line.
    fn deprecated_use(&mut self, name: &str, span: Span, deprecation: &Deprecation) {
        // The name a module resolver made canonical is not the name anybody
        // wrote, and the span already says which module this is. So the
        // message carries the last segment, which is the text under the
        // caret in every spelling but the qualified one.
        let written = name.rsplit(':').next().unwrap_or(name);
        let mut warning = Diagnostic::warning(
            codes::DEPRECATED,
            self.file,
            span,
            format!("`{written}` is deprecated"),
        );
        if let Some(message) = &deprecation.message {
            warning = warning.with_note(message.clone());
        }
        self.warnings.push(warning);
    }

    /// Refuses a local of `Fn` type that has the name of a top-level `fn`.
    ///
    /// Both lookups are fallbacks, so the collision has a winner already: the
    /// signature table is consulted first, and the local would silently never
    /// be the thing called. `STATUS.md` carries "a `fn` may silently shadow a
    /// builtin" as a standing debt, and `D-092` says this must not become a
    /// second one of the same shape — so it is named rather than resolved.
    fn refuse_function_shadow(&mut self, name: &str, ty: &Type, span: Span) {
        if !matches!(ty, Type::Fn { .. }) || !self.signatures.contains_key(name) {
            return;
        }
        self.diagnostics.push(
            Diagnostic::error(
                codes::NAME_OR_TYPE,
                self.file,
                span,
                format!("`{name}` is already a function, so it cannot also name a function value"),
            )
            .with_help(format!(
                "rename the binding: `({name} ...)` would call the `fn`, not this value"
            )),
        );
    }

    fn error_with_code(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, self.file, span, message));
    }

    /// Refuses a raw-pointer operation written outside `unsafe` (`D-067`).
    ///
    /// Every one of them asks — the volatile read and write, `ptr-offset`, and
    /// a conversion to or from a pointer — so that auditing what a program can
    /// do to memory it does not own is a search for one word. Letting the
    /// address arithmetic out would be the cheaper rule and the wrong one to
    /// pick first: loosening this after the freeze is additive, and tightening
    /// it is not.
    fn require_unsafe(&mut self, span: Span, what: impl std::fmt::Display) {
        if self.unsafe_depth > 0 {
            return;
        }
        self.diagnostics.push(
            Diagnostic::error(
                codes::UNSAFE_REQUIRED,
                self.file,
                span,
                format!("{what} outside an `unsafe` block"),
            )
            .with_help(
                "write it inside `(unsafe ...)`: the compiler cannot prove a raw \
                 address points at anything, and the word is where a reader is told so",
            ),
        );
    }
}

/// Whether `clone` would have nothing to do for this type.
///
/// A scalar is copied by being used, so a `clone` of one is silence dressed as
/// a call, and `D-091` makes it an error rather than let it read as a deep
/// copy. Asked only of an argument that was **owned**: through a borrow the
/// same scalar is a load and the only reading of one the language has
/// (`D-100`).
fn is_nothing_to_clone(ty: &Type) -> bool {
    // `Fn` was here while a function value was a code address and copying one
    // was the no-op `D-091` refuses. Since `D-101` it owns its captures, so
    // `(clone f)` is a real copy — a second block holding a clone of each — and
    // it is what the diagnostic for using one twice tells the author to reach
    // for.
    matches!(
        ty,
        Type::Unit | Type::Bool | Type::I32 | Type::I64 | Type::F64
    )
}

/// Whether a refused `=` operand is text, owned or borrowed.
///
/// Only used to decide whether the diagnostic points at `core:string:equals`.
/// It is the case a user actually hits, and the one with an answer to give.
fn is_text(ty: &Type) -> bool {
    match ty {
        Type::String => true,
        Type::Ref { inner, .. } => is_text(inner),
        _ => false,
    }
}

/// How many operands an operator takes.
#[derive(Clone, Copy, Debug)]
enum Arity {
    One,
    Two,
    /// `-` alone: `(- a b)` subtracts and `(- a)` negates (`D-106`).
    OneOrTwo,
}

impl Arity {
    fn accepts(self, count: usize) -> bool {
        match self {
            Arity::One => count == 1,
            Arity::Two => count == 2,
            Arity::OneOrTwo => count == 1 || count == 2,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Arity::One => "one argument",
            Arity::Two => "two arguments",
            Arity::OneOrTwo => "one or two arguments",
        }
    }
}

/// Which types an operator is defined on.
#[derive(Clone, Copy, Debug)]
enum Domain {
    /// Every number: the integers and `f64`.
    Numeric,
    /// The integers alone. A remainder, a bit and a shift are statements about
    /// a pattern of bits, and an `f64` has none to speak of.
    Integer,
    Bool,
    /// What `=` accepts and nothing wider (`D-089`): every other type is one
    /// machine word in a local, so a wider comparison would answer about the
    /// handle while looking like it answered about the contents.
    Scalar,
}

impl Domain {
    fn accepts(self, ty: &Type) -> bool {
        match self {
            Domain::Numeric => ty.is_integer() || *ty == Type::F64,
            Domain::Integer => ty.is_integer(),
            Domain::Bool => *ty == Type::Bool,
            Domain::Scalar => ty.is_integer() || matches!(ty, Type::F64 | Type::Bool),
        }
    }
}

struct OperatorSpec {
    name: &'static str,
    arity: Arity,
    domain: Domain,
    /// Whether the result is a `bool` rather than the operand type.
    answers_bool: bool,
    /// Whether the second operand is a shift amount, and so is checked against
    /// the operand width when it is written down.
    shift: bool,
}

/// Every operator the language has, and what each accepts (`D-106`).
///
/// One table rather than four scattered `match`es, because the properties that
/// used to be spread across an arity check, a validity check and a result
/// choice are per-operator facts and disagreed the moment there were more than
/// seven of them.
const OPERATORS: &[OperatorSpec] = &[
    OperatorSpec {
        name: "+",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "-",
        arity: Arity::OneOrTwo,
        domain: Domain::Numeric,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "*",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "/",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "%",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "<",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: ">",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: "<=",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: ">=",
        arity: Arity::Two,
        domain: Domain::Numeric,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: "=",
        arity: Arity::Two,
        domain: Domain::Scalar,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: "!=",
        arity: Arity::Two,
        domain: Domain::Scalar,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: "not",
        arity: Arity::One,
        domain: Domain::Bool,
        answers_bool: true,
        shift: false,
    },
    OperatorSpec {
        name: "bit-and",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "bit-or",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "bit-xor",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "bit-not",
        arity: Arity::One,
        domain: Domain::Integer,
        answers_bool: false,
        shift: false,
    },
    OperatorSpec {
        name: "shl",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: true,
    },
    OperatorSpec {
        name: "shr",
        arity: Arity::Two,
        domain: Domain::Integer,
        answers_bool: false,
        shift: true,
    },
];

/// Whether a name in head position is an operator rather than a call.
pub fn is_operator(name: &str) -> bool {
    OPERATORS.iter().any(|spec| spec.name == name)
}

fn collect_variable_names(expr: &Expr, output: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Var { name, .. } => {
            output.insert(name.clone());
        }
        ExprKind::Let { value, .. } => collect_variable_names(value, output),
        ExprKind::Set { name, value } => {
            output.insert(name.clone());
            collect_variable_names(value, output);
        }
        // Both halves, and deliberately over-generous: this decides which
        // borrows may end early, and a name counted that is not really used
        // keeps a loan alive longer than it must, while a name missed releases
        // one that is still held.
        ExprKind::Lambda { captures, body, .. } => {
            for capture in captures {
                output.insert(capture.name.clone());
            }
            collect_variable_names(body, output);
        }
        ExprKind::Do(expressions) | ExprKind::Unsafe(expressions) => {
            for expression in expressions {
                collect_variable_names(expression, output);
            }
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_variable_names(condition, output);
            collect_variable_names(then_expr, output);
            collect_variable_names(else_expr, output);
        }
        ExprKind::Loop { body } => collect_variable_names(body, output),
        ExprKind::While { condition, body } => {
            collect_variable_names(condition, output);
            collect_variable_names(body, output);
        }
        ExprKind::Match { value, arms } => {
            collect_variable_names(value, output);
            for arm in arms {
                collect_variable_names(&arm.body, output);
            }
        }
        ExprKind::Borrow { value, .. } | ExprKind::Try(value) | ExprKind::Convert { value, .. } => {
            collect_variable_names(value, output);
        }
        ExprKind::Call { args, .. } => {
            for argument in args {
                collect_variable_names(argument, output);
            }
        }
        ExprKind::Logical { operands, .. } => {
            for operand in operands {
                collect_variable_names(operand, output);
            }
        }
        ExprKind::Break(value) => {
            if let Some(value) = value {
                collect_variable_names(value, output);
            }
        }
        ExprKind::Unit
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Continue => {}
    }
}

fn pattern_irrefutable(pattern: &TPattern) -> bool {
    match pattern {
        TPattern::Wildcard | TPattern::Binding(_) => true,
        TPattern::Struct { fields, .. } => fields
            .iter()
            .all(|field| pattern_irrefutable(&field.pattern)),
        TPattern::Bool(_) | TPattern::Int(_) | TPattern::Enum { .. } => false,
    }
}

fn contains_parameter(ty: &Type, parameters: &HashSet<String>) -> bool {
    match ty {
        Type::Named(name) => parameters.contains(name),
        Type::List(inner) | Type::Slice(inner) | Type::Ref { inner, .. } => {
            contains_parameter(inner, parameters)
        }
        Type::Array { element, .. } => contains_parameter(element, parameters),
        Type::Apply { args, .. } => args
            .iter()
            .any(|argument| contains_parameter(argument, parameters)),
        Type::Fn { params, result } => {
            params
                .iter()
                .any(|param| contains_parameter(param, parameters))
                || contains_parameter(result, parameters)
        }
        _ => false,
    }
}

const EXTERN_PARAMETER_HELP: &str =
    "an `extern` parameter is an integer type, `f64`, `bool`, `(Ptr T)`, `(& String)` \
     or `(& (Slice T))`";

const EXTERN_RESULT_HELP: &str =
    "an `extern` returns `unit`, an integer type, `f64`, `bool`, `(Ptr T)` or an owned `String`";

/// Whether a parameter type is one the C boundary can carry (`D-065`).
///
/// The list grew by the widths C actually has when `D-107` gave the language
/// eight integer types, and is still closed on purpose: a type that is not here
/// has no agreed C spelling, and guessing one is how an FFI starts lying. What
/// stays out is every aggregate — a `String` or a `Slice` crosses as a borrow
/// or not at all.
fn extern_parameter_is_expressible(ty: &Type) -> bool {
    match ty {
        Type::F64 | Type::Bool => true,
        _ if ty.is_integer() => true,
        // A `(Ptr T)` is C's `T *`, which is the one spelling in this table
        // that needs no agreeing about (`D-067`). It is `Copy` and it borrows
        // nothing, so it crosses under the same rule as the scalars above.
        Type::Ptr(_) => true,
        // A borrow is the only way a non-scalar crosses: an `extern` may not
        // take ownership, because the drop glue would then run where the
        // compiler cannot see it.
        Type::Ref {
            mutable: false,
            inner,
        } => {
            matches!(inner.as_ref(), Type::String | Type::Slice(_))
        }
        _ => false,
    }
}

/// Whether a return type is one the C boundary can carry (`D-065`).
///
/// A returned `String` is owned by the caller, so C must have allocated it
/// through `sl_rt_string_new`. Everything else is a scalar or nothing.
fn extern_result_is_expressible(ty: &Type) -> bool {
    ty.is_integer()
        || matches!(
            ty,
            Type::Unit | Type::Bool | Type::F64 | Type::String | Type::Ptr(_)
        )
}

fn contains_borrowed_type(ty: &Type) -> bool {
    match ty {
        Type::Ref { .. } | Type::Slice(_) => true,
        Type::List(inner) => contains_borrowed_type(inner),
        Type::Array { element, .. } => contains_borrowed_type(element),
        Type::Apply { args, .. } => args.iter().any(contains_borrowed_type),
        // A borrow in a function type is a parameter, not a stored value: the
        // function it describes takes one, and nothing about the value being
        // one machine word holds a loan open. `(Fn ((& String)) bool)` is a
        // legal field type for the same reason a `fn` taking a borrow is a
        // legal declaration.
        Type::Fn { .. } => false,
        // A raw pointer holds no loan. Its pointee is a scalar, so there is
        // nothing behind it whose lifetime a borrow could be tracking, which
        // is the whole reason `D-067` restricted the pointee in the first
        // place.
        Type::Ptr(_) => false,
        Type::Unit
        | Type::Bool
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::F64
        | Type::String
        | Type::Named(_) => false,
    }
}

fn unify_type(
    template: &Type,
    actual: &Type,
    parameters: &HashSet<String>,
    substitutions: &mut HashMap<String, Type>,
    instances: Instances<'_>,
) -> Result<(), String> {
    if let Type::Named(name) = template {
        if parameters.contains(name) {
            if let Some(previous) = substitutions.get(name) {
                return (previous == actual).then_some(()).ok_or_else(|| {
                    format!(
                        "generic parameter `{name}` was inferred as both `{previous}` and `{actual}`"
                    )
                });
            }
            substitutions.insert(name.clone(), actual.clone());
            return Ok(());
        }
    }
    match (template, actual) {
        (Type::List(left), Type::List(right))
        | (Type::Slice(left), Type::Slice(right))
        | (
            Type::Ref {
                mutable: false,
                inner: left,
            },
            Type::Ref {
                mutable: false,
                inner: right,
            },
        )
        | (
            Type::Ref {
                mutable: true,
                inner: left,
            },
            Type::Ref {
                mutable: true,
                inner: right,
            },
        )
        // The same weakening `weakens_to` allows, one level in: a generic
        // parameter inferred from a `(&mut T)` handed to a `(& K)` is `T`
        // (`D-120`).
        | (
            Type::Ref {
                mutable: false,
                inner: left,
            },
            Type::Ref {
                mutable: true,
                inner: right,
            },
        ) => unify_type(left, right, parameters, substitutions, instances),
        (
            Type::Array {
                element: left,
                length: left_length,
            },
            Type::Array {
                element: right,
                length: right_length,
            },
        ) if left_length == right_length => {
            unify_type(left, right, parameters, substitutions, instances)
        }
        (
            Type::Apply {
                name: left_name,
                args: left_args,
            },
            Type::Apply {
                name: right_name,
                args: right_args,
            },
        ) if left_name == right_name && left_args.len() == right_args.len() => {
            for (left, right) in left_args.iter().zip(right_args) {
                unify_type(left, right, parameters, substitutions, instances)?;
            }
            Ok(())
        }
        // Structural, and invariant in the parameters rather than contravariant:
        // the language has no subtyping, so a `(Fn (T) U)` template against a
        // concrete `Fn(i64) -> bool` is how `(map opt double)` learns what `T`
        // and `U` are.
        (
            Type::Fn {
                params: left_params,
                result: left_result,
            },
            Type::Fn {
                params: right_params,
                result: right_result,
            },
        ) if left_params.len() == right_params.len() => {
            for (left, right) in left_params.iter().zip(right_params) {
                unify_type(left, right, parameters, substitutions, instances)?;
            }
            unify_type(
                left_result,
                right_result,
                parameters,
                substitutions,
                instances,
            )
        }
        // A generic body writes `(Option T)`; by the time a call site has a
        // value, the application has already been instantiated and all that is
        // left of it is the name `Option$<i64>`, which is not parseable back
        // into one. The instance table is what connects the two (`D-095`).
        (Type::Apply { name, args }, Type::Named(instance)) => {
            match instances.application_of(instance) {
                Some((base, arguments)) if base == name && arguments.len() == args.len() => {
                    for (template, actual) in args.iter().zip(arguments) {
                        unify_type(template, actual, parameters, substitutions, instances)?;
                    }
                    Ok(())
                }
                _ => Err(format!("expected `{template}`, found `{actual}`")),
            }
        }
        _ if template == actual => Ok(()),
        _ => Err(format!("expected `{template}`, found `{actual}`")),
    }
}

/// The monomorphized aggregates in scope, keyed by the instance name they were
/// generated under. Unification is a free function and the two tables live on
/// `Sema`, so they travel together rather than as two arguments nobody would
/// keep in step.
#[derive(Clone, Copy)]
struct Instances<'a> {
    enums: &'a HashMap<String, (String, Vec<Type>)>,
    structs: &'a HashMap<String, (String, Vec<Type>)>,
}

impl Instances<'_> {
    /// The generic type and arguments an instance name was generated from.
    fn application_of(&self, instance: &str) -> Option<&(String, Vec<Type>)> {
        self.enums
            .get(instance)
            .or_else(|| self.structs.get(instance))
    }
}

fn substitute_type(ty: &Type, substitutions: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Named(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Type::List(inner) => Type::List(Box::new(substitute_type(inner, substitutions))),
        Type::Array { element, length } => Type::Array {
            element: Box::new(substitute_type(element, substitutions)),
            length: *length,
        },
        Type::Slice(inner) => Type::Slice(Box::new(substitute_type(inner, substitutions))),
        Type::Ref { mutable, inner } => Type::Ref {
            mutable: *mutable,
            inner: Box::new(substitute_type(inner, substitutions)),
        },
        Type::Apply { name, args } => Type::Apply {
            name: name.clone(),
            args: args
                .iter()
                .map(|argument| substitute_type(argument, substitutions))
                .collect(),
        },
        Type::Fn { params, result } => Type::Fn {
            params: params
                .iter()
                .map(|param| substitute_type(param, substitutions))
                .collect(),
            result: Box::new(substitute_type(result, substitutions)),
        },
        _ => ty.clone(),
    }
}

pub(crate) fn generic_instance_name(callee: &str, arguments: &[Type]) -> String {
    let encoded = arguments
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("{callee}$<{encoded}>")
}

fn monomorphize_functions(
    functions: Vec<TypedFunction>,
    tests: &mut [TypedTest],
) -> Result<Vec<TypedFunction>, String> {
    let templates = functions
        .iter()
        .filter(|function| !function.type_params.is_empty())
        .map(|function| (function.name.clone(), function.clone()))
        .collect::<HashMap<_, _>>();
    let mut output = functions
        .into_iter()
        .filter(|function| function.type_params.is_empty())
        .collect::<Vec<_>>();
    let mut queue = VecDeque::<(String, Vec<Type>)>::new();
    for function in &mut output {
        specialize_expr(&mut function.body, &HashMap::new(), &mut queue);
    }
    for test in tests {
        specialize_expr(&mut test.body, &HashMap::new(), &mut queue);
    }

    let mut generated = HashSet::<(String, Vec<Type>)>::new();
    while let Some((callee, arguments)) = queue.pop_front() {
        if !generated.insert((callee.clone(), arguments.clone())) {
            continue;
        }
        if generated.len() > 256 {
            return Err(
                "generic specialization exceeded 256 instances; possible recursive growth".into(),
            );
        }
        let Some(template) = templates.get(&callee) else {
            return Err(format!("missing generic template `{callee}`"));
        };
        if template.type_params.len() != arguments.len() {
            return Err(format!(
                "generic template `{callee}` expects {} arguments, found {}",
                template.type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = template
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut instance = template.clone();
        instance.name = generic_instance_name(&callee, &arguments);
        instance.type_params.clear();
        for parameter in &mut instance.params {
            parameter.ty = substitute_type(&parameter.ty, &substitutions);
        }
        instance.return_type = substitute_type(&instance.return_type, &substitutions);
        specialize_expr(&mut instance.body, &substitutions, &mut queue);
        output.push(instance);
    }
    Ok(output)
}

fn specialize_expr(
    expression: &mut TExpr,
    substitutions: &HashMap<String, Type>,
    queue: &mut VecDeque<(String, Vec<Type>)>,
) {
    expression.ty = substitute_type(&expression.ty, substitutions);
    match &mut expression.kind {
        TExprKind::Let { value, .. } | TExprKind::Set { value, .. } => {
            specialize_expr(value, substitutions, queue);
        }
        TExprKind::Do(items) => {
            for item in items {
                specialize_expr(item, substitutions, queue);
            }
        }
        TExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            specialize_expr(condition, substitutions, queue);
            specialize_expr(then_expr, substitutions, queue);
            specialize_expr(else_expr, substitutions, queue);
        }
        TExprKind::Loop { body } => specialize_expr(body, substitutions, queue),
        TExprKind::While { condition, body } => {
            specialize_expr(condition, substitutions, queue);
            specialize_expr(body, substitutions, queue);
        }
        TExprKind::Match { value, arms } => {
            specialize_expr(value, substitutions, queue);
            for arm in arms {
                specialize_pattern(&mut arm.pattern, substitutions);
                if let Some(guard) = &mut arm.guard {
                    specialize_expr(guard, substitutions, queue);
                }
                specialize_expr(&mut arm.body, substitutions, queue);
            }
        }
        TExprKind::Call { args, .. }
        | TExprKind::CallValue { args, .. }
        | TExprKind::StructInit { fields: args, .. }
        | TExprKind::EnumInit { fields: args, .. } => {
            for argument in args {
                specialize_expr(argument, substitutions, queue);
            }
        }
        // A generic function taken as a value: this is the only path to the
        // monomorphization queue, so without this arm the instance is named
        // and never generated, and the link fails rather than the compile.
        // `type_args` is emptied so a second pass over the same body is a
        // no-op, the way rewriting `GenericCall` into `Call` is for a call.
        TExprKind::Lambda {
            captures,
            params,
            result,
            body,
        } => {
            for capture in captures {
                capture.ty = substitute_type(&capture.ty, substitutions);
            }
            for param in params {
                param.ty = substitute_type(&param.ty, substitutions);
            }
            *result = substitute_type(result, substitutions);
            specialize_expr(body, substitutions, queue);
        }
        TExprKind::FnRef { name, type_args } => {
            if !type_args.is_empty() {
                let concrete = type_args
                    .iter()
                    .map(|argument| substitute_type(argument, substitutions))
                    .collect::<Vec<_>>();
                queue.push_back((name.clone(), concrete.clone()));
                *name = generic_instance_name(name, &concrete);
                type_args.clear();
            }
        }
        TExprKind::GenericCall {
            callee,
            type_args,
            args,
        } => {
            for argument in args.iter_mut() {
                specialize_expr(argument, substitutions, queue);
            }
            let concrete = type_args
                .iter()
                .map(|argument| substitute_type(argument, substitutions))
                .collect::<Vec<_>>();
            queue.push_back((callee.clone(), concrete.clone()));
            let ordinary_args = std::mem::take(args);
            expression.kind = TExprKind::Call {
                callee: generic_instance_name(callee, &concrete),
                args: ordinary_args,
            };
        }
        TExprKind::Convert { value } => specialize_expr(value, substitutions, queue),
        TExprKind::Try { value, ok_type, .. } => {
            specialize_expr(value, substitutions, queue);
            *ok_type = substitute_type(ok_type, substitutions);
        }
        TExprKind::Break(value) => {
            if let Some(value) = value {
                specialize_expr(value, substitutions, queue);
            }
        }
        TExprKind::Const { value, .. } => specialize_expr(value, substitutions, queue),
        TExprKind::Unit
        | TExprKind::Bool(_)
        | TExprKind::Int(_)
        | TExprKind::Float(_)
        | TExprKind::String(_)
        | TExprKind::Var(_)
        | TExprKind::Borrow { .. }
        | TExprKind::Field { .. }
        | TExprKind::Continue => {}
    }
}

fn specialize_pattern(pattern: &mut TPattern, substitutions: &HashMap<String, Type>) {
    match pattern {
        TPattern::Binding(binding) => {
            binding.ty = substitute_type(&binding.ty, substitutions);
        }
        TPattern::Enum { fields, .. } | TPattern::Struct { fields, .. } => {
            for field in fields {
                field.ty = substitute_type(&field.ty, substitutions);
                specialize_pattern(&mut field.pattern, substitutions);
            }
        }
        TPattern::Wildcard | TPattern::Bool(_) | TPattern::Int(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ast, lexer, parser};

    fn analyze_source(source: &str) -> CompileResult<TypedProgram> {
        let tokens = lexer::lex("test.slp", source)?;
        let forms = parser::parse("test.slp", &tokens)?;
        let program = ast::build_program("test.slp", &forms)?;
        analyze("test.slp", &program)
    }

    #[test]
    fn catches_use_after_move() {
        let source = r#"
            (fn consume ((value String)) -> unit ())
            (fn main () -> i32
              (let value "hello")
              (consume value)
              (consume value)
              0)
        "#;
        let errors = analyze_source(source).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("moved value")));
    }

    #[test]
    fn rejects_move_out_of_a_loop_body() {
        // The second iteration would re-consume the same value, so this must be
        // an error even though a single pass through the body is fine.
        let source = r#"
            (fn consume ((value String)) -> unit ())
            (fn main () -> i32
              (let mut i 0)
              (let s "hello")
              (while (< i 2)
                (do
                  (consume s)
                  (set i (+ i 1))))
              0)
        "#;
        let errors = analyze_source(source).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("moved inside a loop body")));
    }

    #[test]
    fn accepts_a_loop_that_reassigns_after_moving() {
        // Re-initialising the binding makes it owned again before the next
        // iteration, so the loop is sound and must keep compiling.
        let source = r#"
            (fn consume ((value String)) -> unit ())
            (fn main () -> i32
              (let mut i 0)
              (let mut s "hello")
              (while (< i 2)
                (do
                  (consume s)
                  (set s "next")
                  (set i (+ i 1))))
              0)
        "#;
        analyze_source(source).unwrap();
    }

    #[test]
    fn accepts_a_loop_that_owns_its_own_value() {
        let source = r#"
            (fn consume ((value String)) -> unit ())
            (fn main () -> i32
              (let mut i 0)
              (while (< i 2)
                (do
                  (let s "hello")
                  (consume s)
                  (set i (+ i 1))))
              0)
        "#;
        analyze_source(source).unwrap();
    }

    #[test]
    fn inner_scope_exit_keeps_an_outer_borrow_alive() {
        // Popping the inner scope must decrement the borrow count, not clear
        // it: `view` still points into the buffer that `push` would realloc.
        let source = r#"
            (fn show ((text (& String))) -> unit ())
            (fn main () -> i32
              (do
                (let mut values (list "one" "two" "three" "four"))
                (let view (slice (& values) 0 2))
                (do
                  (let second (get-ref (& values) 1))
                  (show second))
                (do (push (&mut values) "five"))
                (show (get-ref (& view) 0))
                0))
        "#;
        let errors = analyze_source(source).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("cannot mutably borrow")));
    }

    #[test]
    fn accepts_borrowed_string() {
        let source = r#"
            (fn show ((text (& String))) -> unit ())
            (fn main () -> i32
              (let value "hello")
              (show (& value))
              0)
        "#;
        analyze_source(source).unwrap();
    }

    #[test]
    fn catches_immutable_assignment() {
        let source = "(fn main () -> i32 (let x 1) (set x 2) 0)";
        let errors = analyze_source(source).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("immutable")));
    }

    #[test]
    fn catches_non_exhaustive_match() {
        let source = "(fn main () -> i32 (match true (true 0)))";
        let errors = analyze_source(source).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("non-exhaustive")));
    }
}
