use crate::ast::{Expr, ExprKind, Function, Pattern, PatternKind, Program, Type};
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
    pub type_params: Vec<String>,
    pub params: Vec<TypedParam>,
    pub return_type: Type,
    pub body: TExpr,
    pub span: Span,
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
    String(String),
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
    Break,
    Continue,
    Match {
        value: Box<TExpr>,
        arms: Vec<TMatchArm>,
    },
    Borrow {
        id: BindingId,
        mutable: bool,
    },
    Call {
        callee: String,
        args: Vec<TExpr>,
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
}

#[derive(Clone, Debug, Serialize)]
pub struct TMatchArm {
    pub pattern: TPattern,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CollectionKind {
    List,
    Array,
    Slice,
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
}

#[derive(Clone, Debug, Default)]
struct Scope {
    names: HashMap<String, BindingId>,
    loans: Vec<BindingId>,
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
            for id in scope.names.values() {
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
    analyze_with_options(file, program, &crate::LanguageItems::default(), true)
}

pub fn analyze_with_options(
    file: &str,
    program: &Program,
    language_items: &crate::LanguageItems,
    validate_entry_point: bool,
) -> CompileResult<TypedProgram> {
    Analyzer::new(file, program, language_items, validate_entry_point).analyze(program)
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
    loop_depth: usize,
    diagnostics: Vec<Diagnostic>,
    next_binding: BindingId,
    /// Where each binding was consumed, so a loop can point at the move that
    /// its next iteration would repeat.
    move_sites: HashMap<BindingId, Span>,
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
            loop_depth: 0,
            diagnostics: Vec::new(),
            next_binding: 0,
            move_sites: HashMap::new(),
            env: Environment::default(),
        }
    }

    fn analyze(mut self, program: &Program) -> CompileResult<TypedProgram> {
        self.collect_signatures(program);
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
            Type::Ref { mutable, inner } => Type::Ref {
                mutable: *mutable,
                inner: Box::new(self.normalize_type(inner, span)),
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
            };
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
            ExprKind::Int(value) => {
                let ty = match expected {
                    Some(Type::I32) => Type::I32,
                    _ => Type::I64,
                };
                if ty == Type::I32 && i32::try_from(*value).is_err() {
                    self.error(
                        expr.span,
                        format!("integer literal `{value}` does not fit in i32"),
                    );
                }
                self.typed(expr, ty, TExprKind::Int(*value))
            }
            ExprKind::Float(value) => self.typed(expr, Type::F64, TExprKind::Float(*value)),
            ExprKind::String(value) => {
                self.typed(expr, Type::String, TExprKind::String(value.clone()))
            }
            ExprKind::Var(name) => self.variable(expr, name, true),
            ExprKind::Let {
                name,
                mutable,
                value,
            } => {
                let value = self.expr(value, None);
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
                };
                if self.env.insert(name.clone(), binding).is_err() {
                    self.error(
                        expr.span,
                        format!("`{name}` is already defined in this scope"),
                    );
                }
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
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => self.if_expr(expr, condition, then_expr, else_expr, expected),
            ExprKind::Loop { body } => self.loop_expr(expr, None, body),
            ExprKind::While { condition, body } => self.loop_expr(expr, Some(condition), body),
            ExprKind::Break => {
                if self.loop_depth == 0 {
                    self.error(expr.span, "`break` can only be used inside a loop");
                }
                self.typed(expr, Type::Unit, TExprKind::Break)
            }
            ExprKind::Continue => {
                if self.loop_depth == 0 {
                    self.error(expr.span, "`continue` can only be used inside a loop");
                }
                self.typed(expr, Type::Unit, TExprKind::Continue)
            }
            ExprKind::Match { value, arms } => self.match_expr(expr, value, arms, expected),
            ExprKind::Borrow { mutable, value } => self.borrow(expr, *mutable, value),
            ExprKind::Try(value) => self.try_expr(expr, value),
            ExprKind::Call { callee, args } => self.call(expr, callee, args, expected),
        };

        if let Some(expected) = expected {
            if typed.ty != *expected {
                self.error(
                    expr.span,
                    format!("expected `{expected}`, found `{}`", typed.ty),
                );
            }
        }
        if !matches!(typed.ty, Type::Ref { .. } | Type::Slice(_)) {
            if let TExprKind::Call { args, .. } | TExprKind::GenericCall { args, .. } = &typed.kind
            {
                for argument in args {
                    if let TExprKind::Borrow { id, .. } = &argument.kind {
                        self.env.discharge_loan(*id);
                    }
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
            TExprKind::Call { callee, args } if callee == "clone" => args
                .first()
                .and_then(|argument| self.reference_loan(argument))
                .map(|(origin, _)| (origin, false)),
            _ => None,
        }
    }

    fn variable(&mut self, expr: &Expr, name: &str, consume: bool) -> TExpr {
        let Some(id) = self.env.lookup_id(name) else {
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
        if consume && !binding.ty.is_copy() && binding.state == OwnershipState::Available {
            if let Some(binding) = self.env.bindings.get_mut(&id) {
                binding.state = OwnershipState::Moved;
            }
            self.move_sites.insert(id, expr.span);
        }
        self.typed(expr, binding.ty, TExprKind::Var(id))
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

    fn loop_expr(&mut self, expr: &Expr, condition: Option<&Expr>, body: &Expr) -> TExpr {
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
        self.loop_depth += 1;
        self.env.push();
        let body = Box::new(self.expr(body, Some(&Type::Unit)));
        self.env.pop();
        self.loop_depth -= 1;

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
            None => self.typed(expr, Type::Unit, TExprKind::Loop { body }),
        }
    }

    fn borrow(&mut self, expr: &Expr, mutable: bool, value: &Expr) -> TExpr {
        let ExprKind::Var(name) = &value.kind else {
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
            if binding.state != OwnershipState::Available {
                self.error(
                    value.span,
                    format!("cannot mutably borrow `{name}` more than once"),
                );
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
            return self.typed(expr, Type::Unit, TExprKind::Unit);
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
        let enum_name = match &value.ty {
            Type::Named(name)
                if self
                    .variants
                    .values()
                    .any(|variant| &variant.enum_name == name)
                    || self.enum_instances.contains_key(name) =>
            {
                Some(name.clone())
            }
            _ => None,
        };
        let struct_name = match &value.ty {
            Type::Named(name)
                if self.structs.contains_key(name) || self.generated_structs.contains_key(name) =>
            {
                Some(name.clone())
            }
            _ => None,
        };
        if !matches!(value.ty, Type::Bool | Type::I32 | Type::I64)
            && enum_name.is_none()
            && struct_name.is_none()
        {
            self.error(
                value.span,
                format!("`match` does not support `{}`", value.ty),
            );
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
            let pattern = self.type_pattern(&arm.pattern, &value.ty);
            if pattern_irrefutable(&pattern) {
                wildcard = true;
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
                body,
                span: arm.span,
            });
        }
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
            || (value.ty == Type::Bool
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
        if !exhaustive {
            self.diagnostics.push(
                Diagnostic::error(codes::MATCH, self.file, expr.span, "non-exhaustive match")
                    .with_help(if value.ty == Type::Bool {
                        "cover both `true` and `false`, or add `_`"
                    } else if enum_name.is_some() {
                        "cover every enum variant, or add `_`"
                    } else {
                        "integer matches require a final `_` arm"
                    }),
            );
        }
        self.typed(
            expr,
            result_type.unwrap_or(Type::Unit),
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
                let binding = Binding {
                    id,
                    ty: expected.clone(),
                    mutable: false,
                    state: OwnershipState::Available,
                    definition: pattern.span,
                    borrowed_from: None,
                    owns_loan: false,
                };
                if self.env.insert(name.clone(), binding).is_err() {
                    self.error(pattern.span, format!("duplicate pattern binding `{name}`"));
                }
                TPattern::Binding(TPatternBinding {
                    id,
                    name: name.clone(),
                    ty: expected.clone(),
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
                if !matches!(expected, Type::I32 | Type::I64) {
                    self.error(
                        pattern.span,
                        format!("integer pattern does not match `{expected}`"),
                    );
                }
                if *expected == Type::I32 && i32::try_from(*value).is_err() {
                    self.error(
                        pattern.span,
                        format!("pattern `{value}` does not fit in i32"),
                    );
                }
                TPattern::Int(*value)
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
                if let Type::Named(expected_name) = expected {
                    if let Some((base, arguments)) = self.enum_instances.get(expected_name) {
                        if base == &info.enum_name {
                            let substitutions = info
                                .type_params
                                .iter()
                                .cloned()
                                .zip(arguments.iter().cloned())
                                .collect::<HashMap<_, _>>();
                            info.enum_name = expected_name.clone();
                            info.fields = info
                                .fields
                                .iter()
                                .map(|(field, ty)| {
                                    (field.clone(), substitute_type(ty, &substitutions))
                                })
                                .collect();
                        }
                    }
                }
                if expected != &Type::Named(info.enum_name.clone()) {
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
                let typed_fields = fields
                    .iter()
                    .zip(&info.fields)
                    .map(|(field, (_, ty))| TPatternField {
                        ty: ty.clone(),
                        pattern: self.type_pattern(field, ty),
                    })
                    .collect();
                TPattern::Enum {
                    enum_name: info.enum_name,
                    variant: info.variant,
                    tag: info.tag,
                    fields: typed_fields,
                }
            }
            PatternKind::Struct { path, fields } => {
                let expected_name = match expected {
                    Type::Named(name) => name.clone(),
                    _ => {
                        self.error(
                            pattern.span,
                            format!("struct pattern `{path}` does not match `{expected}`"),
                        );
                        "<error>".into()
                    }
                };
                if &expected_name != path {
                    self.error(
                        pattern.span,
                        format!("struct pattern `{path}` does not match `{expected}`"),
                    );
                }
                let declared_fields = self
                    .structs
                    .get(&expected_name)
                    .cloned()
                    .or_else(|| {
                        self.generated_structs
                            .get(&expected_name)
                            .map(|structure| structure.fields.clone())
                    })
                    .unwrap_or_else(|| {
                        self.error(pattern.span, format!("unknown struct `{path}`"));
                        Vec::new()
                    });
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
                let typed_fields = declared_fields
                    .iter()
                    .map(|(name, ty)| TPatternField {
                        ty: ty.clone(),
                        pattern: provided
                            .get(name)
                            .map_or(TPattern::Wildcard, |field| self.type_pattern(field, ty)),
                    })
                    .collect();
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
        if callee == "list" {
            return self.collection_literal(expr, callee, args);
        }
        if callee == "array" {
            return self.collection_literal(expr, callee, args);
        }
        if callee == "slice" {
            return self.slice_operation(expr, args);
        }
        if matches!(
            callee,
            "len" | "push" | "get" | "get-ref" | "pop" | "remove"
        ) {
            return self.list_operation(expr, callee, args);
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
        if matches!(callee, "+" | "-" | "*" | "/" | "<" | ">" | "=") {
            return self.operator_call(expr, callee, args, expected);
        }
        let Some(signature) = self.signatures.get(callee).cloned() else {
            self.error(expr.span, format!("unknown function `{callee}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
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
        let mut typed_args = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let template = signature.params.get(index);
            let concrete_expected = template.filter(|ty| !contains_parameter(ty, &parameters));
            let typed = self.expr(argument, concrete_expected);
            if let Some(template) = template {
                if let Err(message) =
                    unify_type(template, &typed.ty, &parameters, &mut substitutions)
                {
                    self.error(argument.span, message);
                }
            }
            typed_args.push(typed);
        }
        if let Some(expected) = expected {
            if let Err(message) =
                unify_type(&signature.result, expected, &parameters, &mut substitutions)
            {
                self.error(expr.span, message);
            }
        }
        let mut type_args = Vec::new();
        for parameter in &signature.type_params {
            if let Some(argument) = substitutions.get(parameter) {
                type_args.push(argument.clone());
            } else {
                self.error_with_code(
                    codes::GENERIC,
                    expr.span,
                    format!("cannot infer generic parameter `{parameter}` for `{callee}`"),
                );
                type_args.push(Type::Unit);
            }
        }
        let result = substitute_type(&signature.result, &substitutions);
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

    fn clone_call(&mut self, expr: &Expr, args: &[Expr]) -> TExpr {
        if args.len() != 1 {
            self.error(expr.span, "`clone` expects one argument");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let arg = match &args[0].kind {
            ExprKind::Var(name) => self.variable(&args[0], name, false),
            _ => self.expr(&args[0], None),
        };
        if matches!(arg.ty, Type::Ref { mutable: true, .. }) {
            self.error(arg.span, "cannot clone a mutable reference");
        }
        let result = arg.ty.clone();
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
            let ExprKind::Var(keyword) = &pair[0].kind else {
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
            let ExprKind::Var(keyword) = &pair[0].kind else {
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
        if let Some(Type::Named(expected_name)) = expected {
            if let Some((base, arguments)) = self.struct_instances.get(expected_name) {
                if base == name {
                    substitutions.extend(
                        info.type_params
                            .iter()
                            .cloned()
                            .zip(arguments.iter().cloned()),
                    );
                }
            }
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
            let partially_substituted = substitute_type(template, &substitutions);
            let expected_field = (!contains_parameter(&partially_substituted, &parameters))
                .then_some(&partially_substituted);
            let typed = self.expr(value, expected_field);
            if let Err(message) = unify_type(template, &typed.ty, &parameters, &mut substitutions) {
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
        let ExprKind::Var(base_name) = &args[0].kind else {
            self.error(
                args[0].span,
                "field access currently requires a named binding",
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let ExprKind::Var(field_name) = &args[1].kind else {
            self.error(args[1].span, "field name must be an identifier");
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(id) = self.env.lookup_id(base_name) else {
            self.error(args[0].span, format!("undefined variable `{base_name}`"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let binding = self.env.bindings.get(&id).cloned().expect("binding exists");
        let Type::Named(struct_name) = &binding.ty else {
            self.error(args[0].span, format!("`{base_name}` is not a struct"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
        let Some(fields) = self.structs.get(struct_name) else {
            self.error(args[0].span, format!("`{struct_name}` is not a struct"));
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        };
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
        if let Some(Type::Named(expected_name)) = expected {
            if let Some((base, arguments)) = self.enum_instances.get(expected_name) {
                if base == &info.enum_name {
                    substitutions.extend(
                        info.type_params
                            .iter()
                            .cloned()
                            .zip(arguments.iter().cloned()),
                    );
                }
            }
        }
        let mut fields = Vec::new();
        for (index, argument) in args.iter().enumerate() {
            let Some((_, template)) = info.fields.get(index) else {
                fields.push(self.expr(argument, None));
                continue;
            };
            let partially_substituted = substitute_type(template, &substitutions);
            let expected_field = (!contains_parameter(&partially_substituted, &parameters))
                .then_some(&partially_substituted);
            let typed = self.expr(argument, expected_field);
            if let Err(message) = unify_type(template, &typed.ty, &parameters, &mut substitutions) {
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

    fn collection_literal(&mut self, expr: &Expr, callee: &str, args: &[Expr]) -> TExpr {
        if args.is_empty() {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::NAME_OR_TYPE,
                    self.file,
                    expr.span,
                    format!("cannot infer the type of an empty {callee}"),
                )
                .with_help(format!("provide at least one element in `({callee} ...)`")),
            );
            let ty = if callee == "array" {
                Type::Array {
                    element: Box::new(Type::Unit),
                    length: 0,
                }
            } else {
                Type::List(Box::new(Type::Unit))
            };
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

    fn list_operation(&mut self, expr: &Expr, callee: &str, args: &[Expr]) -> TExpr {
        let expected_len = if matches!(callee, "push" | "get" | "get-ref" | "remove") {
            2
        } else {
            1
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
                        if matches!(callee, "push" | "pop" | "remove") {
                            "pass `(&mut list)`"
                        } else {
                            "pass `(& list)`"
                        },
                    ),
                );
                (false, Type::Unit, CollectionKind::List)
            }
        };
        if matches!(callee, "push" | "pop" | "remove") && kind != CollectionKind::List {
            self.error(
                args[0].span,
                format!("`{callee}` is only available for List"),
            );
        }
        if matches!(callee, "push" | "pop" | "remove") && !mutable {
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
        if args.len() != 2 {
            self.error(
                expr.span,
                format!("operator `{callee}` expects two arguments"),
            );
            return self.typed(expr, Type::Unit, TExprKind::Unit);
        }
        let numeric_expectation = match expected {
            Some(Type::I32 | Type::I64 | Type::F64) => expected,
            _ => None,
        };
        let left = self.expr(&args[0], numeric_expectation);
        let right = self.expr(&args[1], Some(&left.ty));
        let valid = match callee {
            "+" | "-" | "*" | "/" => left.ty.is_integer() || left.ty == Type::F64,
            "<" | ">" => left.ty.is_integer() || left.ty == Type::F64,
            // Scalars only, per `D-089`. Every other type is one machine word
            // in a local — `Type` has no aggregate variant — so a wider `=`
            // would compare handles rather than contents, and would hand an
            // unconstrained type parameter a capability `D-012` denies it.
            "=" => left.ty.is_integer() || matches!(left.ty, Type::F64 | Type::Bool),
            _ => false,
        };
        if !valid {
            let mut diagnostic = Diagnostic::error(
                codes::NAME_OR_TYPE,
                self.file,
                expr.span,
                format!("operator `{callee}` does not support `{}`", left.ty),
            );
            if callee == "=" && is_text(&left.ty) {
                diagnostic = diagnostic.with_help("compare text with `core:string:equals`");
            }
            self.diagnostics.push(diagnostic);
        }
        let result = if matches!(callee, "<" | ">" | "=") {
            Type::Bool
        } else {
            left.ty.clone()
        };
        self.typed(
            expr,
            result,
            TExprKind::Call {
                callee: callee.to_owned(),
                args: vec![left, right],
            },
        )
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
                for arm in arms {
                    self.materialize_pattern(&mut arm.pattern, arm.span);
                    self.materialize_typed_expr(&mut arm.body);
                }
            }
            TExprKind::Call { args, .. } | TExprKind::GenericCall { args, .. } => {
                for argument in args {
                    self.materialize_typed_expr(argument);
                }
            }
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
            TExprKind::Unit
            | TExprKind::Bool(_)
            | TExprKind::Int(_)
            | TExprKind::Float(_)
            | TExprKind::String(_)
            | TExprKind::Var(_)
            | TExprKind::Borrow { .. }
            | TExprKind::Break
            | TExprKind::Continue
            | TExprKind::Field { .. } => {}
        }
    }

    fn materialize_pattern(&mut self, pattern: &mut TPattern, span: Span) {
        match pattern {
            TPattern::Binding(binding) => {
                binding.ty = self.normalize_type(&binding.ty, span);
            }
            TPattern::Enum {
                enum_name, fields, ..
            } => {
                for field in fields {
                    field.ty = self.normalize_type(&field.ty, span);
                    self.materialize_pattern(&mut field.pattern, span);
                }
                let ty = self.normalize_type(&Type::Named(enum_name.clone()), span);
                if let Type::Named(name) = ty {
                    *enum_name = name;
                }
            }
            TPattern::Struct {
                struct_name,
                fields,
            } => {
                for field in fields {
                    field.ty = self.normalize_type(&field.ty, span);
                    self.materialize_pattern(&mut field.pattern, span);
                }
                let ty = self.normalize_type(&Type::Named(struct_name.clone()), span);
                if let Type::Named(name) = ty {
                    *struct_name = name;
                }
            }
            TPattern::Wildcard | TPattern::Bool(_) | TPattern::Int(_) => {}
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

    fn error_with_code(&mut self, code: &str, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, self.file, span, message));
    }
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

fn collect_variable_names(expr: &Expr, output: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Var(name) => {
            output.insert(name.clone());
        }
        ExprKind::Let { value, .. } => collect_variable_names(value, output),
        ExprKind::Set { name, value } => {
            output.insert(name.clone());
            collect_variable_names(value, output);
        }
        ExprKind::Do(expressions) => {
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
        ExprKind::Borrow { value, .. } | ExprKind::Try(value) => {
            collect_variable_names(value, output);
        }
        ExprKind::Call { args, .. } => {
            for argument in args {
                collect_variable_names(argument, output);
            }
        }
        ExprKind::Unit
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Break
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
        _ => false,
    }
}

const EXTERN_PARAMETER_HELP: &str =
    "an `extern` parameter is `i32`, `i64`, `f64`, `bool`, `(& String)` or `(& (Slice T))`";

const EXTERN_RESULT_HELP: &str =
    "an `extern` returns `unit`, `i32`, `i64`, `f64`, `bool` or an owned `String`";

/// Whether a parameter type is one the C boundary can carry (`D-065`).
///
/// The list is short because the language has no `u8`, no `char` and no
/// unsigned types, and it is closed on purpose: a type that is not here has no
/// agreed C spelling, and guessing one is how an FFI starts lying.
fn extern_parameter_is_expressible(ty: &Type) -> bool {
    match ty {
        Type::I32 | Type::I64 | Type::F64 | Type::Bool => true,
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
    matches!(
        ty,
        Type::Unit | Type::Bool | Type::I32 | Type::I64 | Type::F64 | Type::String
    )
}

fn contains_borrowed_type(ty: &Type) -> bool {
    match ty {
        Type::Ref { .. } | Type::Slice(_) => true,
        Type::List(inner) => contains_borrowed_type(inner),
        Type::Array { element, .. } => contains_borrowed_type(element),
        Type::Apply { args, .. } => args.iter().any(contains_borrowed_type),
        Type::Unit
        | Type::Bool
        | Type::I32
        | Type::I64
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
        ) => unify_type(left, right, parameters, substitutions),
        (
            Type::Array {
                element: left,
                length: left_length,
            },
            Type::Array {
                element: right,
                length: right_length,
            },
        ) if left_length == right_length => unify_type(left, right, parameters, substitutions),
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
                unify_type(left, right, parameters, substitutions)?;
            }
            Ok(())
        }
        _ if template == actual => Ok(()),
        _ => Err(format!("expected `{template}`, found `{actual}`")),
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
                specialize_expr(&mut arm.body, substitutions, queue);
            }
        }
        TExprKind::Call { args, .. }
        | TExprKind::StructInit { fields: args, .. }
        | TExprKind::EnumInit { fields: args, .. } => {
            for argument in args {
                specialize_expr(argument, substitutions, queue);
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
        TExprKind::Try { value, ok_type, .. } => {
            specialize_expr(value, substitutions, queue);
            *ok_type = substitute_type(ok_type, substitutions);
        }
        TExprKind::Unit
        | TExprKind::Bool(_)
        | TExprKind::Int(_)
        | TExprKind::Float(_)
        | TExprKind::String(_)
        | TExprKind::Var(_)
        | TExprKind::Borrow { .. }
        | TExprKind::Field { .. }
        | TExprKind::Break
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
