use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};
use crate::parser::{SExpr, SExprKind};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Type {
    Unit,
    Bool,
    I32,
    I64,
    F64,
    String,
    List(Box<Type>),
    Array { element: Box<Type>, length: usize },
    Slice(Box<Type>),
    Ref { mutable: bool, inner: Box<Type> },
    Named(String),
    Apply { name: String, args: Vec<Type> },
}

impl Type {
    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            Type::Unit | Type::Bool | Type::I32 | Type::I64 | Type::F64 | Type::Ref { .. }
        )
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, Type::I32 | Type::I64)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => f.write_str("unit"),
            Type::Bool => f.write_str("bool"),
            Type::I32 => f.write_str("i32"),
            Type::I64 => f.write_str("i64"),
            Type::F64 => f.write_str("f64"),
            Type::String => f.write_str("String"),
            Type::List(inner) => write!(f, "List<{inner}>"),
            Type::Array { element, length } => write!(f, "Array<{element}, {length}>"),
            Type::Slice(inner) => write!(f, "Slice<{inner}>"),
            Type::Ref { mutable, inner } => {
                if *mutable {
                    write!(f, "&mut {inner}")
                } else {
                    write!(f, "&{inner}")
                }
            }
            Type::Named(name) => f.write_str(name),
            Type::Apply { name, args } => {
                write!(f, "({name}")?;
                for argument in args {
                    write!(f, " {argument}")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub exports: Vec<ExportDecl>,
    pub takes: Vec<TakeDecl>,
    pub functions: Vec<Function>,
    pub tests: Vec<Test>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportDecl {
    pub items: Vec<ImportItem>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct TakeDecl {
    pub module: String,
    pub items: Vec<ImportItem>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportItem {
    pub path: String,
    pub alias: String,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Function {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Test {
    pub name: String,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct StructDecl {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<Param>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumDecl {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Param>,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum ExprKind {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Var(String),
    Let {
        name: String,
        mutable: bool,
        value: Box<Expr>,
    },
    Set {
        name: String,
        value: Box<Expr>,
    },
    Do(Vec<Expr>),
    If {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Loop {
        body: Box<Expr>,
    },
    While {
        condition: Box<Expr>,
        body: Box<Expr>,
    },
    Break,
    Continue,
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Borrow {
        mutable: bool,
        value: Box<Expr>,
    },
    Try(Box<Expr>),
    Call {
        callee: String,
        args: Vec<Expr>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub enum PatternKind {
    Wildcard,
    Binding(String),
    Bool(bool),
    Int(i64),
    Enum {
        path: String,
        fields: Vec<Pattern>,
    },
    Struct {
        path: String,
        fields: Vec<(String, Pattern)>,
    },
}

pub fn build_program(file: &str, forms: &[SExpr]) -> CompileResult<Program> {
    let mut builder = AstBuilder {
        file,
        diagnostics: Vec::new(),
    };
    let mut program = Program {
        exports: Vec::new(),
        takes: Vec::new(),
        functions: Vec::new(),
        tests: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
    };

    for form in forms {
        let Some(items) = builder.list(form, "top-level declaration") else {
            continue;
        };
        let Some(kind) = items.first().and_then(atom) else {
            builder.error(form.span, "declaration must begin with a name");
            continue;
        };
        match kind {
            "export" => {
                if let Some(export) = builder.export_decl(form.span, items) {
                    program.exports.push(export);
                }
            }
            "take" => {
                if let Some(take) = builder.take_decl(form.span, items) {
                    program.takes.push(take);
                }
            }
            "fn" => {
                if let Some(function) = builder.function(form.span, items) {
                    program.functions.push(function);
                }
            }
            "test" => {
                if let Some(test) = builder.test(form.span, items) {
                    program.tests.push(test);
                }
            }
            "struct" => {
                if let Some(decl) = builder.struct_decl(form.span, items) {
                    program.structs.push(decl);
                }
            }
            "enum" => {
                if let Some(decl) = builder.enum_decl(form.span, items) {
                    program.enums.push(decl);
                }
            }
            other => builder.error(
                items[0].span,
                format!("unknown top-level declaration `{other}`"),
            ),
        }
    }

    if builder.diagnostics.is_empty() {
        Ok(program)
    } else {
        Err(builder.diagnostics)
    }
}

struct AstBuilder<'a> {
    file: &'a str,
    diagnostics: Vec<Diagnostic>,
}

impl AstBuilder<'_> {
    fn export_decl(&mut self, span: Span, items: &[SExpr]) -> Option<ExportDecl> {
        if items.len() < 2 {
            self.error(
                span,
                "export syntax is `(export Name (path:Name :as Alias) ...)`",
            );
            return None;
        }
        let mut exported = Vec::new();
        for item in &items[1..] {
            if let Some(item) = self.import_item(item) {
                exported.push(item);
            }
        }
        Some(ExportDecl {
            items: exported,
            span,
        })
    }

    fn take_decl(&mut self, span: Span, items: &[SExpr]) -> Option<TakeDecl> {
        if items.len() < 3 {
            self.error(
                span,
                "take syntax is `(take module Name (other :as Alias) ...)`",
            );
            return None;
        }
        let module = self.required_atom(&items[1], "module path")?.to_owned();
        let mut imported = Vec::new();
        for item in &items[2..] {
            if let Some(item) = self.import_item(item) {
                imported.push(item);
            }
        }
        Some(TakeDecl {
            module,
            items: imported,
            span,
        })
    }

    fn import_item(&mut self, form: &SExpr) -> Option<ImportItem> {
        match &form.kind {
            SExprKind::Atom(path) => Some(ImportItem {
                path: path.clone(),
                alias: path.rsplit(':').next().unwrap_or(path).to_owned(),
                span: form.span,
            }),
            SExprKind::List(parts) if parts.len() == 3 && atom(&parts[1]) == Some(":as") => {
                Some(ImportItem {
                    path: self.required_atom(&parts[0], "imported path")?.to_owned(),
                    alias: self.required_atom(&parts[2], "import alias")?.to_owned(),
                    span: form.span,
                })
            }
            _ => {
                self.error(form.span, "expected `Name` or `(qualified:Name :as Alias)`");
                None
            }
        }
    }

    fn function(&mut self, span: Span, items: &[SExpr]) -> Option<Function> {
        if items.len() < 6 {
            self.error(
                span,
                "function syntax is `(fn name ((arg type) ...) -> type body...)`",
            );
            return None;
        }
        let name = self.required_atom(&items[1], "function name")?.to_owned();
        let has_generics = items.len() >= 7
            && self.type_params_if_present(&items[2]).is_some()
            && matches!(items[3].kind, SExprKind::List(_));
        let (type_params, params_index) = if has_generics {
            (self.type_params(&items[2])?, 3)
        } else {
            (Vec::new(), 2)
        };
        let params = self.params(&items[params_index])?;
        if atom(&items[params_index + 1]) != Some("->") {
            self.error(
                items[params_index + 1].span,
                "expected `->` before the return type",
            );
            return None;
        }
        let return_type = self.ty(&items[params_index + 2])?;
        let body = self.body(&items[params_index + 3..], span)?;
        Some(Function {
            name,
            type_params,
            params,
            return_type,
            body,
            span,
        })
    }

    fn test(&mut self, span: Span, items: &[SExpr]) -> Option<Test> {
        if items.len() < 3 {
            self.error(span, "test syntax is `(test \"name\" expression...)`");
            return None;
        }
        let SExprKind::String(name) = &items[1].kind else {
            self.error(items[1].span, "test name must be a string literal");
            return None;
        };
        Some(Test {
            name: name.clone(),
            body: self.body(&items[2..], span)?,
            span,
        })
    }

    fn struct_decl(&mut self, span: Span, items: &[SExpr]) -> Option<StructDecl> {
        if !matches!(items.len(), 3 | 4) {
            self.error(
                span,
                "struct syntax is `(struct Name (T ...) ((field type) ...))`",
            );
            return None;
        }
        let has_generics = items.len() == 4;
        Some(StructDecl {
            name: self.required_atom(&items[1], "struct name")?.to_owned(),
            type_params: if has_generics {
                self.type_params(&items[2])?
            } else {
                Vec::new()
            },
            fields: self.params(&items[if has_generics { 3 } else { 2 }])?,
            span,
        })
    }

    fn enum_decl(&mut self, span: Span, items: &[SExpr]) -> Option<EnumDecl> {
        if items.len() < 3 {
            self.error(
                span,
                "enum syntax is `(enum Name Variant (Variant ((field type) ...)) ...)`",
            );
            return None;
        }
        let name = self.required_atom(&items[1], "enum name")?.to_owned();
        let has_generics = items.len() >= 4 && self.type_params_if_present(&items[2]).is_some();
        let type_params = if has_generics {
            self.type_params(&items[2])?
        } else {
            Vec::new()
        };
        let mut variants = Vec::new();
        for item in &items[if has_generics { 3 } else { 2 }..] {
            match &item.kind {
                SExprKind::Atom(variant) => variants.push(EnumVariant {
                    name: variant.clone(),
                    fields: Vec::new(),
                    span: item.span,
                }),
                SExprKind::List(parts) if !parts.is_empty() => {
                    let Some(variant_name) = self.required_atom(&parts[0], "variant name") else {
                        continue;
                    };
                    let fields = if parts.len() == 1 {
                        Vec::new()
                    } else if parts.len() == 2 {
                        self.params(&parts[1])?
                    } else {
                        self.error(item.span, "variant has too many parts");
                        continue;
                    };
                    variants.push(EnumVariant {
                        name: variant_name.to_owned(),
                        fields,
                        span: item.span,
                    });
                }
                _ => self.error(item.span, "invalid enum variant"),
            }
        }
        Some(EnumDecl {
            name,
            type_params,
            variants,
            span,
        })
    }

    fn params(&mut self, form: &SExpr) -> Option<Vec<Param>> {
        let items = self.list(form, "parameter or field list")?;
        let mut params = Vec::new();
        for item in items {
            let Some(pair) = self.list(item, "name/type pair") else {
                continue;
            };
            if pair.len() != 2 {
                self.error(item.span, "expected `(name type)`");
                continue;
            }
            let Some(name) = self.required_atom(&pair[0], "name") else {
                continue;
            };
            let Some(ty) = self.ty(&pair[1]) else {
                continue;
            };
            params.push(Param {
                name: name.to_owned(),
                ty,
                span: item.span,
            });
        }
        Some(params)
    }

    fn type_params_if_present(&self, form: &SExpr) -> Option<Vec<String>> {
        let SExprKind::List(items) = &form.kind else {
            return None;
        };
        if items.is_empty() {
            return None;
        }
        items
            .iter()
            .map(|item| atom(item).map(str::to_owned))
            .collect()
    }

    fn type_params(&mut self, form: &SExpr) -> Option<Vec<String>> {
        let Some(parameters) = self.type_params_if_present(form) else {
            self.error(form.span, "generic parameter list must contain names");
            return None;
        };
        let mut seen = std::collections::HashSet::new();
        for parameter in &parameters {
            if !seen.insert(parameter.clone()) {
                self.error(
                    form.span,
                    format!("generic parameter `{parameter}` is declared more than once"),
                );
            }
        }
        Some(parameters)
    }

    fn ty(&mut self, form: &SExpr) -> Option<Type> {
        match &form.kind {
            SExprKind::Atom(name) => Some(match name.as_str() {
                "unit" => Type::Unit,
                "bool" => Type::Bool,
                "i32" => Type::I32,
                "i64" => Type::I64,
                "f64" => Type::F64,
                "String" => Type::String,
                name => Type::Named(name.to_owned()),
            }),
            SExprKind::List(parts)
                if parts.len() == 2 && matches!(atom(&parts[0]), Some("List")) =>
            {
                Some(Type::List(Box::new(self.ty(&parts[1])?)))
            }
            SExprKind::List(parts)
                if parts.len() == 3 && matches!(atom(&parts[0]), Some("Array")) =>
            {
                let length = self
                    .required_atom(&parts[2], "array length")?
                    .parse::<usize>()
                    .ok();
                let Some(length) = length else {
                    self.error(parts[2].span, "array length must be a non-negative integer");
                    return None;
                };
                Some(Type::Array {
                    element: Box::new(self.ty(&parts[1])?),
                    length,
                })
            }
            SExprKind::List(parts)
                if parts.len() == 2 && matches!(atom(&parts[0]), Some("Slice")) =>
            {
                Some(Type::Slice(Box::new(self.ty(&parts[1])?)))
            }
            SExprKind::List(parts)
                if parts.len() == 2 && matches!(atom(&parts[0]), Some("&" | "&mut")) =>
            {
                Some(Type::Ref {
                    mutable: atom(&parts[0]) == Some("&mut"),
                    inner: Box::new(self.ty(&parts[1])?),
                })
            }
            SExprKind::List(parts) if !parts.is_empty() && atom(&parts[0]).is_some() => {
                let name = atom(&parts[0]).expect("checked above").to_owned();
                let mut args = Vec::new();
                for argument in &parts[1..] {
                    args.push(self.ty(argument)?);
                }
                Some(Type::Apply { name, args })
            }
            _ => {
                self.error(form.span, "invalid type");
                None
            }
        }
    }

    fn body(&mut self, forms: &[SExpr], span: Span) -> Option<Expr> {
        if forms.is_empty() {
            self.error(span, "body cannot be empty");
            return None;
        }
        let mut expressions = Vec::new();
        for form in forms {
            if let Some(expr) = self.expr(form) {
                expressions.push(expr);
            }
        }
        if expressions.len() == 1 {
            expressions.pop()
        } else {
            Some(Expr {
                kind: ExprKind::Do(expressions),
                span,
            })
        }
    }

    fn expr(&mut self, form: &SExpr) -> Option<Expr> {
        let kind = match &form.kind {
            SExprKind::String(value) => ExprKind::String(value.clone()),
            SExprKind::Atom(value) => {
                if value == "true" {
                    ExprKind::Bool(true)
                } else if value == "false" {
                    ExprKind::Bool(false)
                } else if let Ok(number) = value.parse::<i64>() {
                    ExprKind::Int(number)
                } else if value.contains('.') {
                    match value.parse::<f64>() {
                        Ok(number) => ExprKind::Float(number),
                        Err(_) => ExprKind::Var(value.clone()),
                    }
                } else {
                    ExprKind::Var(value.clone())
                }
            }
            SExprKind::List(items) if items.is_empty() => ExprKind::Unit,
            SExprKind::List(items) => {
                let Some(head) = items.first().and_then(atom) else {
                    self.error(form.span, "call head must be a name");
                    return None;
                };
                match head {
                    "let" => return self.let_expr(form.span, items),
                    "set" => {
                        if items.len() != 3 {
                            self.error(form.span, "`set` expects a name and a value");
                            return None;
                        }
                        ExprKind::Set {
                            name: self.required_atom(&items[1], "binding name")?.to_owned(),
                            value: Box::new(self.expr(&items[2])?),
                        }
                    }
                    "do" => {
                        let mut body = Vec::new();
                        for item in &items[1..] {
                            body.push(self.expr(item)?);
                        }
                        ExprKind::Do(body)
                    }
                    "if" => {
                        if items.len() != 4 {
                            self.error(form.span, "`if` expects condition, then, and else");
                            return None;
                        }
                        ExprKind::If {
                            condition: Box::new(self.expr(&items[1])?),
                            then_expr: Box::new(self.expr(&items[2])?),
                            else_expr: Box::new(self.expr(&items[3])?),
                        }
                    }
                    "loop" => {
                        if items.len() < 2 {
                            self.error(form.span, "`loop` expects a body");
                            return None;
                        }
                        ExprKind::Loop {
                            body: Box::new(self.body(&items[1..], form.span)?),
                        }
                    }
                    "while" => {
                        if items.len() < 3 {
                            self.error(form.span, "`while` expects a condition and body");
                            return None;
                        }
                        ExprKind::While {
                            condition: Box::new(self.expr(&items[1])?),
                            body: Box::new(self.body(&items[2..], form.span)?),
                        }
                    }
                    "break" => {
                        if items.len() != 1 {
                            self.error(form.span, "`break` does not accept a value");
                            return None;
                        }
                        ExprKind::Break
                    }
                    "continue" => {
                        if items.len() != 1 {
                            self.error(form.span, "`continue` does not accept a value");
                            return None;
                        }
                        ExprKind::Continue
                    }
                    "match" => return self.match_expr(form.span, items),
                    "try" => {
                        if items.len() != 2 {
                            self.error(form.span, "`try` expects one Result expression");
                            return None;
                        }
                        ExprKind::Try(Box::new(self.expr(&items[1])?))
                    }
                    "&" | "&mut" => {
                        if items.len() != 2 {
                            self.error(form.span, format!("`{head}` expects one value"));
                            return None;
                        }
                        ExprKind::Borrow {
                            mutable: head == "&mut",
                            value: Box::new(self.expr(&items[1])?),
                        }
                    }
                    callee => {
                        let mut args = Vec::new();
                        for item in &items[1..] {
                            args.push(self.expr(item)?);
                        }
                        ExprKind::Call {
                            callee: callee.to_owned(),
                            args,
                        }
                    }
                }
            }
        };
        Some(Expr {
            kind,
            span: form.span,
        })
    }

    fn let_expr(&mut self, span: Span, items: &[SExpr]) -> Option<Expr> {
        let (mutable, name_index, value_index) = if items.get(1).and_then(atom) == Some("mut") {
            (true, 2, 3)
        } else {
            (false, 1, 2)
        };
        if items.len() != value_index + 1 {
            self.error(
                span,
                "`let` syntax is `(let name value)` or `(let mut name value)`",
            );
            return None;
        }
        Some(Expr {
            kind: ExprKind::Let {
                name: self
                    .required_atom(&items[name_index], "binding name")?
                    .to_owned(),
                mutable,
                value: Box::new(self.expr(&items[value_index])?),
            },
            span,
        })
    }

    fn match_expr(&mut self, span: Span, items: &[SExpr]) -> Option<Expr> {
        if items.len() < 3 {
            self.error(span, "`match` expects a value and at least one arm");
            return None;
        }
        let value = Box::new(self.expr(&items[1])?);
        let mut arms = Vec::new();
        for item in &items[2..] {
            let Some(parts) = self.list(item, "match arm") else {
                continue;
            };
            if parts.len() != 2 {
                self.error(item.span, "match arm syntax is `(pattern expression)`");
                continue;
            }
            let pattern = self.pattern(&parts[0])?;
            let body = self.expr(&parts[1])?;
            arms.push(MatchArm {
                pattern,
                body,
                span: item.span,
            });
        }
        Some(Expr {
            kind: ExprKind::Match { value, arms },
            span,
        })
    }

    fn pattern(&mut self, form: &SExpr) -> Option<Pattern> {
        if let SExprKind::List(items) = &form.kind {
            if let Some(path) = items.first().and_then(atom) {
                if path.contains(':') {
                    let mut fields = Vec::new();
                    for item in &items[1..] {
                        fields.push(self.pattern(item)?);
                    }
                    return Some(Pattern {
                        kind: PatternKind::Enum {
                            path: path.to_owned(),
                            fields,
                        },
                        span: form.span,
                    });
                }
                if items.len() > 1 {
                    if (items.len() - 1) % 2 != 0 {
                        self.error(
                            form.span,
                            "struct pattern fields use `:field pattern` pairs",
                        );
                        return None;
                    }
                    let mut fields = Vec::new();
                    for pair in items[1..].chunks(2) {
                        let field = self.required_atom(&pair[0], "struct pattern field")?;
                        let Some(field) = field.strip_prefix(':') else {
                            self.error(pair[0].span, "struct pattern field must start with `:`");
                            return None;
                        };
                        fields.push((field.to_owned(), self.pattern(&pair[1])?));
                    }
                    return Some(Pattern {
                        kind: PatternKind::Struct {
                            path: path.to_owned(),
                            fields,
                        },
                        span: form.span,
                    });
                }
            }
        }
        let atom_value = match &form.kind {
            SExprKind::Atom(value) => Some(value.as_str()),
            SExprKind::List(items) if items.len() == 1 => atom(&items[0]),
            _ => None,
        };
        let Some(value) = atom_value else {
            self.error(
                form.span,
                "patterns are bindings, `_`, literals, enum variants, or struct patterns",
            );
            return None;
        };
        let kind = match value {
            "_" => PatternKind::Wildcard,
            "true" => PatternKind::Bool(true),
            "false" => PatternKind::Bool(false),
            value if value.contains(':') => PatternKind::Enum {
                path: value.to_owned(),
                fields: Vec::new(),
            },
            value => match value.parse::<i64>() {
                Ok(value) => PatternKind::Int(value),
                Err(_) => PatternKind::Binding(value.to_owned()),
            },
        };
        Some(Pattern {
            kind,
            span: form.span,
        })
    }

    fn required_atom<'a>(&mut self, form: &'a SExpr, purpose: &str) -> Option<&'a str> {
        let value = atom(form);
        if value.is_none() {
            self.error(form.span, format!("expected {purpose}"));
        }
        value
    }

    fn list<'a>(&mut self, form: &'a SExpr, purpose: &str) -> Option<&'a [SExpr]> {
        if let SExprKind::List(items) = &form.kind {
            Some(items)
        } else {
            self.error(form.span, format!("expected {purpose}"));
            None
        }
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::error(
            codes::INVALID_SYNTAX,
            self.file,
            span,
            message,
        ));
    }
}

fn atom(form: &SExpr) -> Option<&str> {
    match &form.kind {
        SExprKind::Atom(value) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser};

    #[test]
    fn builds_function_and_test() {
        let source = "(fn add ((a i64) (b i64)) -> i64 (+ a b))\n(test \"add\" (= (add 1 2) 3))";
        let tokens = lexer::lex("test.slp", source).unwrap();
        let forms = parser::parse("test.slp", &tokens).unwrap();
        let program = build_program("test.slp", &forms).unwrap();
        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.tests.len(), 1);
        assert_eq!(program.functions[0].params.len(), 2);
    }
}
