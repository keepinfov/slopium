use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};
use crate::parser::{SExpr, SExprKind};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub enum Type {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F64,
    String,
    List(Box<Type>),
    Array {
        element: Box<Type>,
        length: usize,
    },
    Slice(Box<Type>),
    Ref {
        mutable: bool,
        inner: Box<Type>,
    },
    Named(String),
    Apply {
        name: String,
        args: Vec<Type>,
    },
    /// The type of a function value, written `(Fn (i64 i64) bool)` (`D-092`).
    ///
    /// Parameters are grouped in their own list so that every arity has one
    /// form: `(Fn () i64)` is nullary and nothing is counted from the right to
    /// find where the result starts.
    ///
    /// A value of this type is one machine word, like every other local, and
    /// since `D-101` that word is a pointer to an owned block rather than a
    /// code address: `[code, drop, clone, capture ...]`. So a `Fn` is not
    /// `Copy` — it owns whatever a `lambda` moved into it — and using one
    /// twice as an argument means borrowing it, exactly as it would for a
    /// `String`.
    Fn {
        params: Vec<Type>,
        result: Box<Type>,
    },
    /// A raw pointer to a scalar, written `(Ptr u16)` (`D-067`).
    ///
    /// The pointee is one of the eight integers, `bool` or `f64`, and that
    /// restriction is the whole reason this type costs so little: nothing
    /// owned can be reached through a pointer, so there is no aliasing rule
    /// for `unsafe` to turn off. What it turns off is narrower and more
    /// honest — memory reached this way is not proven to exist.
    ///
    /// A `Ptr` is a scalar like an integer: one machine word, `Copy`, no drop
    /// and no clone. In particular it is deliberately *not*
    /// `lowering::is_pointer_like`, which means "the word is the address of a
    /// block the compiler manages". This word is a value that happens to be
    /// an address, so `(& p)` takes the address of the slot holding it.
    Ptr(Box<Type>),
}

/// The width and signedness of an integer type (`D-107`).
///
/// The eight integer types are eight `Type` variants, but almost nothing wants
/// to name them one at a time: the backends and the optimizer want the two
/// numbers. Asking for those through one table is what keeps a `== Type::I32`
/// test from silently treating every new type as an `i64`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct IntKind {
    pub bits: u8,
    pub signed: bool,
}

impl IntKind {
    /// Whether a value of this type already fills a machine word, so that the
    /// canonical form of §2 costs nothing.
    pub fn is_full_width(&self) -> bool {
        self.bits == 64
    }

    /// The mask that keeps the bits this type has.
    pub fn mask(&self) -> i64 {
        if self.bits == 64 {
            -1
        } else {
            ((1i128 << self.bits) - 1) as i64
        }
    }

    /// Whether every canonical `source` word is already a canonical word of
    /// this type, so that a conversion between them costs nothing.
    ///
    /// A 64-bit target accepts anything, because the whole word *is* the value.
    /// Below that, a strictly narrower source is accepted when it cannot carry
    /// a bit pattern this type would read differently: an unsigned source is a
    /// small non-negative word whatever this type is, and a signed one is
    /// already extended as long as this type is signed too.
    pub fn accepts(self, source: IntKind) -> bool {
        if self.bits == 64 {
            return true;
        }
        match source.bits.cmp(&self.bits) {
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => source.signed == self.signed,
            std::cmp::Ordering::Less => !source.signed || self.signed,
        }
    }

    /// Reduces a 64-bit computation to this type's canonical machine word:
    /// sign-extended when signed, zero-extended when unsigned (`D-074`).
    pub fn canonicalize(&self, value: i64) -> i64 {
        if self.bits == 64 {
            return value;
        }
        let shift = 64 - u32::from(self.bits);
        if self.signed {
            (value << shift) >> shift
        } else {
            value & self.mask()
        }
    }
}

/// An integer literal as it was written: a magnitude, a sign, and whether it
/// was written as a bit pattern.
///
/// The three travel together because what a literal *means* depends on a type
/// the parser does not know (`D-107`). `255` is a `u8` and not an `i8`; `0xFF`
/// is both, and is `-1` at the second.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct IntLiteral {
    pub magnitude: u64,
    pub negative: bool,
    /// Written `0x` or `0b`, which `D-112` says is a bit pattern rather than a
    /// number.
    pub bits: bool,
}

impl IntLiteral {
    /// The canonical machine word this literal denotes at `kind`, or `None`
    /// when it does not fit there.
    pub fn at(self, kind: IntKind) -> Option<i64> {
        let width = u32::from(kind.bits);
        if self.negative {
            // A written minus sign means a number whatever the radix, and no
            // unsigned type has one. `-128` is an `i8` and `-129` is not, so
            // the magnitude reaches the bound and stops — which is also how
            // `-9223372036854775808` gets written at all.
            if !kind.signed || self.magnitude > (1u64 << (width - 1)) {
                return None;
            }
            return Some((self.magnitude as i64).wrapping_neg());
        }
        if self.bits {
            // A pattern is accepted whenever it fits in the width, and is then
            // read at that width.
            if width < 64 && self.magnitude >= (1u64 << width) {
                return None;
            }
            return Some(kind.canonicalize(self.magnitude as i64));
        }
        let limit = match (kind.signed, width) {
            (false, 64) => u64::MAX,
            (false, _) => (1u64 << width) - 1,
            (true, _) => (1u64 << (width - 1)) - 1,
        };
        if self.magnitude > limit {
            return None;
        }
        Some(self.magnitude as i64)
    }
}

impl fmt::Display for IntLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        if self.bits {
            write!(f, "{:#x}", self.magnitude)
        } else {
            write!(f, "{}", self.magnitude)
        }
    }
}

impl Type {
    pub fn is_copy(&self) -> bool {
        matches!(
            self,
            Type::Unit | Type::Bool | Type::F64 | Type::Ref { .. } | Type::Ptr(_)
        ) || self.is_integer()
    }

    /// Whether this is a type a raw pointer may point at, and a volatile
    /// access may carry (`D-067`).
    ///
    /// The eight integers, `bool` and `f64` — everything that is one machine
    /// word and owns nothing. A `String` or a `List` behind a pointer would
    /// be an ownership question, and refusing the type is how that question
    /// is kept from arising at all.
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::Bool | Type::F64) || self.is_integer()
    }

    pub fn is_integer(&self) -> bool {
        self.int_kind().is_some()
    }

    /// The width and signedness of an integer type, or `None` for everything
    /// else. The single place the eight types are spelled out.
    pub fn int_kind(&self) -> Option<IntKind> {
        let (bits, signed) = match self {
            Type::I8 => (8, true),
            Type::I16 => (16, true),
            Type::I32 => (32, true),
            Type::I64 => (64, true),
            Type::U8 => (8, false),
            Type::U16 => (16, false),
            Type::U32 => (32, false),
            Type::U64 => (64, false),
            _ => return None,
        };
        Some(IntKind { bits, signed })
    }

    /// The width and signedness a *conversion* treats this type as having.
    ///
    /// An integer's own, and `u64`'s for a raw pointer: an address is a 64-bit
    /// unsigned quantity on both targets, so `(as u8 p)` is a narrowing and has
    /// to truncate like any other (`D-067`, `D-113`).
    ///
    /// Asking `int_kind` here instead answers `None` for a pointer, which sends
    /// the conversion down the "nothing to do" path and leaves a `u8` holding a
    /// whole address — a value outside its own type, which every comparison and
    /// every arithmetic operation downstream then reads at 64 bits.
    pub fn conversion_kind(&self) -> Option<IntKind> {
        match self {
            Type::Ptr(_) => Some(IntKind {
                bits: 64,
                signed: false,
            }),
            _ => self.int_kind(),
        }
    }

    /// What a borrow borrows, or the type itself when it is not one.
    ///
    /// One level, because that is all the language can build: a `(& (& T))` has
    /// no spelling. `clone` uses it to cross the borrow it is handed (`D-091`),
    /// and lowering uses it to pick the glue for what is behind one.
    pub fn strip_ref(&self) -> &Type {
        match self {
            Type::Ref { inner, .. } => inner,
            other => other,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unit => f.write_str("unit"),
            Type::Bool => f.write_str("bool"),
            Type::I8 => f.write_str("i8"),
            Type::I16 => f.write_str("i16"),
            Type::I32 => f.write_str("i32"),
            Type::I64 => f.write_str("i64"),
            Type::U8 => f.write_str("u8"),
            Type::U16 => f.write_str("u16"),
            Type::U32 => f.write_str("u32"),
            Type::U64 => f.write_str("u64"),
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
            // Not the source spelling, for the same reason `List<T>` is not:
            // this rendering is also the mangling, because
            // `sema::generic_instance_name` builds an instance name out of it.
            // It only has to be injective, and hex-encoding in
            // `lowering::function_symbol` makes any of it a legal symbol.
            Type::Fn { params, result } => {
                f.write_str("Fn(")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{param}")?;
                }
                write!(f, ") -> {result}")
            }
            Type::Ptr(inner) => write!(f, "Ptr<{inner}>"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Program {
    pub exports: Vec<ExportDecl>,
    pub takes: Vec<TakeDecl>,
    pub functions: Vec<Function>,
    pub externs: Vec<ExternDecl>,
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

/// A function this package does not define, reached by its C symbol.
///
/// The symbol is written out because it is not derivable: a C name is whatever
/// the library chose, and the Slopium name is whatever reads well here. The
/// rest is a function declaration without a body, and is checked like one
/// (`D-065`).
#[derive(Clone, Debug, Serialize)]
pub struct ExternDecl {
    /// The unmangled symbol the linker resolves.
    pub symbol: String,
    /// The name calls in this package use.
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub span: Span,
    /// The span of the symbol literal, for diagnostics that are about the C
    /// side rather than the declaration as a whole.
    pub symbol_span: Span,
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

/// Which of the two short-circuiting forms this is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum LogicalOp {
    And,
    Or,
}

impl LogicalOp {
    pub fn name(self) -> &'static str {
        match self {
            LogicalOp::And => "and",
            LogicalOp::Or => "or",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum ExprKind {
    Unit,
    Bool(bool),
    Int(IntLiteral),
    Float(f64),
    /// A text literal's bytes (see `lexer::TokenKind::String`).
    String(Vec<u8>),
    /// A bare name: a local, or a top-level `fn` used as a value (`D-092`).
    ///
    /// `resolved` is what the name would mean as a top-level item in the module
    /// this expression was written in — filled in by `package.rs`, which is the
    /// only place that knows the imports, and **advisory**: a name that resolves
    /// to nothing in particular still gets one, and no diagnostic is raised for
    /// it here.
    ///
    /// Sema consults the environment first and reaches for `resolved` only when
    /// the name is not a local, so a local always wins and a `(let count 0)`
    /// beside a `(fn count ...)` keeps meaning what it did.
    Var {
        name: String,
        resolved: Option<String>,
    },
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
    /// A block that permits raw-pointer operations (`D-067`).
    ///
    /// It is a `do` with a permission, not a second type system: the value and
    /// type of the block are the last expression's, and nothing inside it is
    /// checked differently except that a volatile access, a `ptr-offset` and a
    /// conversion to or from a pointer are allowed to appear at all.
    ///
    /// What it does *not* turn off is bounds checks and overflow checks
    /// (`D-031`). A program in an `unsafe` block still cannot index a list past
    /// its end; it buys a pointer, not permission to skip a check.
    Unsafe(Vec<Expr>),
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
    /// `(and a b ...)` and `(or a b ...)` — forms, not calls (`D-106`).
    ///
    /// A call evaluates its arguments, and short-circuiting is the entire
    /// point: `(and (holds map key) (trust (lookup map key)))` must not look
    /// the key up when the map does not hold it. So they join `if` here.
    ///
    /// They stop at `sema`, which types every operand against `bool` and folds
    /// them into nested `If`s — `(and a b)` is `(if a b false)` and `(or a b)`
    /// is `(if a true b)`. Typing the operands directly is what keeps the
    /// diagnostic honest: a desugar in this file would type the second operand
    /// against the *caller's* expectation and then complain about the
    /// synthesized constant, at a span nobody wrote.
    Logical {
        op: LogicalOp,
        operands: Vec<Expr>,
    },
    /// `(as i64 value)` — a widening between two named numeric types.
    ///
    /// A form rather than a call, so the target type is parsed as a type and
    /// not as a variable that happens to spell one (`D-090`).
    Convert {
        target: Type,
        value: Box<Expr>,
    },
    Call {
        callee: String,
        args: Vec<Expr>,
    },
    /// `(lambda (captures ...) ((parameter type) ...) -> result body)`
    /// (`D-102`).
    ///
    /// A `fn` with the name dropped and one list changed: in a declaration the
    /// second list is what the function is parameterised over, and here it is
    /// what the function closes over. Each capture is the name of a binding in
    /// the enclosing scope, and naming it there moves it in — which is why it
    /// is written rather than inferred, since every other move in the language
    /// is written too.
    Lambda {
        captures: Vec<Capture>,
        params: Vec<Param>,
        result: Type,
        body: Box<Expr>,
    },
}

/// One name a `lambda` closes over.
#[derive(Clone, Debug, Serialize)]
pub struct Capture {
    pub name: String,
    pub span: Span,
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
    Int(IntLiteral),
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
        externs: Vec::new(),
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
            "extern" => {
                if let Some(declaration) = builder.extern_decl(form.span, items) {
                    program.externs.push(declaration);
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

    fn extern_decl(&mut self, span: Span, items: &[SExpr]) -> Option<ExternDecl> {
        const SYNTAX: &str = "extern syntax is `(extern \"symbol\" (name (arg type) ...) -> type)`";
        if items.len() != 5 {
            self.error(span, SYNTAX);
            return None;
        }
        let SExprKind::String(symbol) = &items[1].kind else {
            self.error(items[1].span, "the C symbol must be a string literal");
            return None;
        };
        if symbol.is_empty() {
            self.error(items[1].span, "the C symbol cannot be empty");
            return None;
        }
        let signature = self.list(&items[2], "extern signature")?;
        let Some((head, rest)) = signature.split_first() else {
            self.error(items[2].span, SYNTAX);
            return None;
        };
        let name = self.required_atom(head, "extern name")?.to_owned();
        // `(name (T) (value T))` is the generic `fn` shape. There is nothing a
        // type parameter could be instantiated to here — the C vocabulary is
        // closed (`D-065`) — so say that instead of complaining that `(T)` is
        // not a `(name type)` pair.
        if let Some(first) = rest.first() {
            if matches!(&first.kind, SExprKind::List(items)
                if items.len() == 1 && atom(&items[0]).is_some())
            {
                self.error(
                    first.span,
                    "an `extern` cannot be generic: the C boundary has a closed set of types",
                );
                return None;
            }
        }
        let params = self.param_pairs(rest)?;
        if atom(&items[3]) != Some("->") {
            self.error(items[3].span, "expected `->` before the return type");
            return None;
        }
        let return_type = self.ty(&items[4])?;
        let symbol = self.text_literal(symbol, items[1].span, "a C symbol")?;
        Some(ExternDecl {
            symbol,
            name,
            params,
            return_type,
            span,
            symbol_span: items[1].span,
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
        let name = self.text_literal(name, items[1].span, "a test name")?;
        Some(Test {
            name,
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

    /// The names a `lambda` closes over, which are names and nothing else: a
    /// capture is a binding that already exists, so there is no type to give it
    /// and no expression to evaluate.
    fn captures(&mut self, form: &SExpr) -> Option<Vec<Capture>> {
        let items = self.list(form, "capture list")?;
        let mut captures = Vec::new();
        for item in items {
            let Some(name) = self.required_atom(item, "captured name") else {
                continue;
            };
            captures.push(Capture {
                name: name.to_owned(),
                span: item.span,
            });
        }
        Some(captures)
    }

    fn params(&mut self, form: &SExpr) -> Option<Vec<Param>> {
        let items = self.list(form, "parameter or field list")?;
        self.param_pairs(items)
    }

    /// The `(name type)` pairs themselves, without the list that holds them.
    ///
    /// An `extern` keeps its name and its parameters in one list, so it has the
    /// pairs but no list of its own to hand to `params`.
    fn param_pairs(&mut self, items: &[SExpr]) -> Option<Vec<Param>> {
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
                "i8" => Type::I8,
                "i16" => Type::I16,
                "i32" => Type::I32,
                "i64" => Type::I64,
                "u8" => Type::U8,
                "u16" => Type::U16,
                "u32" => Type::U32,
                "u64" => Type::U64,
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
                // The same literal grammar as everywhere else: an array of
                // `0x100` is a length written the way a program that cares
                // about a power of two writes one.
                let length = match numeric_atom(self.required_atom(&parts[2], "array length")?) {
                    NumericAtom::Integer(length) if !length.negative => {
                        usize::try_from(length.magnitude).ok()
                    }
                    _ => None,
                };
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
                if parts.len() == 2 && matches!(atom(&parts[0]), Some("Ptr")) =>
            {
                // The pointee is checked in `sema`, not here: this is where a
                // type is spelled, and `(Ptr String)` is a well-formed
                // spelling of a type the language refuses.
                Some(Type::Ptr(Box::new(self.ty(&parts[1])?)))
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
            // Before the generic catch-all, and matching on the head alone
            // rather than on the whole shape: `D-088` wrote `(Fn i64 i64)`
            // while naming a feature it was refusing, so that is the spelling
            // someone will try, and it deserves the correct form in a message
            // rather than "unknown generic type `Fn`" from two passes later.
            SExprKind::List(parts) if matches!(parts.first().and_then(atom), Some("Fn")) => {
                let Some(SExprKind::List(params)) = parts.get(1).map(|part| &part.kind) else {
                    self.error(
                        form.span,
                        "a function type is written `(Fn (parameter ...) result)`",
                    );
                    return None;
                };
                if parts.len() != 3 {
                    self.error(
                        form.span,
                        "a function type is written `(Fn (parameter ...) result)`",
                    );
                    return None;
                }
                let mut types = Vec::new();
                for param in params {
                    types.push(self.ty(param)?);
                }
                Some(Type::Fn {
                    params: types,
                    result: Box::new(self.ty(&parts[2])?),
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
                } else {
                    match numeric_atom(value) {
                        NumericAtom::Integer(number) => ExprKind::Int(number),
                        NumericAtom::Float(number) => ExprKind::Float(number),
                        NumericAtom::Malformed => {
                            self.error(form.span, format!("`{value}` is not a number"));
                            return None;
                        }
                        NumericAtom::Name => ExprKind::Var {
                            name: value.clone(),
                            resolved: None,
                        },
                    }
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
                    "unsafe" => {
                        let mut body = Vec::new();
                        for item in &items[1..] {
                            body.push(self.expr(item)?);
                        }
                        ExprKind::Unsafe(body)
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
                    "and" | "or" => {
                        let op = if head == "and" {
                            LogicalOp::And
                        } else {
                            LogicalOp::Or
                        };
                        // At least two, because `(and x)` is `x` spelled longer
                        // and `(and)` is a puzzle rather than a program.
                        // Refusing them now stays compatible with allowing them
                        // after the freeze; the reverse would not be.
                        if items.len() < 3 {
                            self.error(
                                form.span,
                                format!("`{head}` expects at least two operands"),
                            );
                            return None;
                        }
                        let mut operands = Vec::new();
                        for item in &items[1..] {
                            operands.push(self.expr(item)?);
                        }
                        ExprKind::Logical { op, operands }
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
                    "lambda" => {
                        if items.len() < 6 || atom(&items[3]) != Some("->") {
                            self.error(
                                form.span,
                                "lambda syntax is \
                                 `(lambda (capture ...) ((arg type) ...) -> type body...)`",
                            );
                            return None;
                        }
                        let captures = self.captures(&items[1])?;
                        ExprKind::Lambda {
                            captures,
                            params: self.params(&items[2])?,
                            result: self.ty(&items[4])?,
                            body: Box::new(self.body(&items[5..], form.span)?),
                        }
                    }
                    "match" => return self.match_expr(form.span, items),
                    "try" => {
                        if items.len() != 2 {
                            self.error(form.span, "`try` expects one Result expression");
                            return None;
                        }
                        ExprKind::Try(Box::new(self.expr(&items[1])?))
                    }
                    "as" => {
                        if items.len() != 3 {
                            self.error(form.span, "`as` expects a target type and a value");
                            return None;
                        }
                        ExprKind::Convert {
                            target: self.ty(&items[1])?,
                            value: Box::new(self.expr(&items[2])?),
                        }
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
            // A malformed number here is the sharpest edge the literal parser
            // closes: this used to fall through to `Binding`, so `0xZZ` in an
            // arm bound a variable named `0xZZ`, matched everything, and made
            // every arm below it unreachable without a word being said.
            value => match numeric_atom(value) {
                NumericAtom::Integer(value) => PatternKind::Int(value),
                NumericAtom::Float(_) | NumericAtom::Malformed => {
                    self.error(form.span, format!("`{value}` is not a pattern"));
                    return None;
                }
                NumericAtom::Name => PatternKind::Binding(value.to_owned()),
            },
        };
        Some(Pattern {
            kind,
            span: form.span,
        })
    }

    /// A string literal in a position that is genuinely *text*.
    ///
    /// A text literal holds bytes, and most of them are a value the program
    /// carries. Two are not: a C symbol is a name the linker resolves and a
    /// test name is something the harness prints, so both have to be readable
    /// as text and neither may be an arbitrary payload.
    fn text_literal(&mut self, bytes: &[u8], span: Span, purpose: &str) -> Option<String> {
        match std::str::from_utf8(bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(_) => {
                self.error(span, format!("{purpose} must be text, not arbitrary bytes"));
                None
            }
        }
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

/// What an atom that might be a number turned out to be.
pub enum NumericAtom {
    Integer(IntLiteral),
    Float(f64),
    /// It began like a number and is not one. Distinguishing this from a name
    /// is the whole point of the type: `0xZZ` used to become a *variable* in an
    /// expression and a catch-all *binding* in a pattern, and neither said a
    /// word about the literal being wrong.
    Malformed,
    /// It never looked like a number at all.
    Name,
}

/// Reads an integer or float literal, in every base the language has
/// (`D-106`).
///
/// **A hexadecimal or binary literal is a bit pattern; a decimal one is a
/// number.** So `0xFFFF_FFFF_FFFF_FFFF` is `-1` and `0x8000_0000_0000_0000` is
/// the smallest `i64`, while the same values in decimal are refused for being
/// out of range. A mask is not a magnitude, and requiring one to be written as
/// a negative decimal is how `core:float` ended up taking the sign bit off a
/// double by adding `2^62` to it twice.
///
/// What this does *not* decide is whether a literal fits, because since
/// `D-107` that depends on a type only `sema` knows: `255` is a `u8` and not
/// an `i8`, and `0xFF` is both. So the magnitude, the sign and the radix all
/// travel forward and the only check here is that the digits fit in 64 bits.
///
/// An `_` may appear only *between* digits. `_1` is a name, and `1_` and
/// `0x_ff` are malformed: the separator groups digits and does not blur the
/// line between a number and a name.
pub fn numeric_atom(text: &str) -> NumericAtom {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if !digits.starts_with(|ch: char| ch.is_ascii_digit()) {
        return NumericAtom::Name;
    }

    let (radix, body) = match digits.get(..2) {
        Some("0x" | "0X") => (16, &digits[2..]),
        Some("0b" | "0B") => (2, &digits[2..]),
        _ => (10, digits),
    };
    let Some(body) = strip_separators(body) else {
        return NumericAtom::Malformed;
    };

    if radix == 10 && body.contains(['.', 'e', 'E']) {
        // A float keeps the range and the spelling Rust gives it; `D-106` adds
        // separators to integer literals and says nothing about these.
        return match text.parse::<f64>() {
            Ok(number) => NumericAtom::Float(number),
            Err(_) => NumericAtom::Malformed,
        };
    }

    let Ok(magnitude) = u64::from_str_radix(&body, radix) else {
        return NumericAtom::Malformed;
    };
    NumericAtom::Integer(IntLiteral {
        magnitude,
        negative,
        bits: radix != 10,
    })
}

/// Removes the `_`s from a literal, or refuses one that is not between digits.
fn strip_separators(body: &str) -> Option<String> {
    if body.is_empty() || body.starts_with('_') || body.ends_with('_') {
        return None;
    }
    if body.contains("__") {
        return None;
    }
    Some(body.replace('_', ""))
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
