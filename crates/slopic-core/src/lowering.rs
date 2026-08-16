//! What every backend decides the same way.
//!
//! Two backends emit different instructions for the same MIR, but they must
//! agree on everything that is not an instruction: what a function is called in
//! the object file, which runtime helper releases a given type, how wide an
//! aggregate is, and what a builtin call actually does. None of that is a
//! property of the machine, and a second backend that re-derived it would be
//! free to derive it differently — a linker error at best, and a type dropped
//! by the wrong helper at worst.
//!
//! So it lives here, and the backends read it.

use crate::ast::Type;
use crate::mir::{Instruction, LocalId, MirExtern, MirFunction, MirModule};

/// The object-file name of a Slopium function.
///
/// Module paths contain colons, which an assembler reads as a label
/// terminator, so the name is hex-encoded rather than escaped. The encoding is
/// total: every byte becomes two hex digits, so distinct names cannot collide.
pub fn function_symbol(name: &str, is_test: bool) -> String {
    let prefix = if is_test { "sl_test" } else { "sl_fn" };
    let encoded = name
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}_{encoded}")
}

/// The declaration behind a callee, when the callee is an `extern`.
pub fn extern_declaration<'a>(module: &'a MirModule, callee: &str) -> Option<&'a MirExtern> {
    module
        .externs
        .iter()
        .find(|declaration| declaration.name == callee)
}

/// What the linker is asked for when this callee is called.
///
/// A Slopium function is hex-encoded under `sl_fn_`; a C function is asked for
/// by the name C gave it, because that is the only name it has. Both backends
/// come here rather than deciding it twice (`D-073`).
pub fn call_symbol(module: &MirModule, callee: &str) -> String {
    match extern_declaration(module, callee) {
        Some(declaration) => declaration.symbol.clone(),
        None => function_symbol(callee, false),
    }
}

/// Where the pointer sits inside a borrowed `String` or `Slice`.
///
/// `SlString` is `{len, cap, ptr}` and `SlSlice` is `{len, elem_size, ptr}`
/// (`runtime/slop_rt.c`), so both keep their length first and their pointer two
/// words in. This is an ABI fact rather than a convenience, which is why it is
/// written down once and tested rather than spelled twice in two backends.
pub const RUNTIME_POINTER_OFFSET: i64 = 16;
/// Where the length sits inside a borrowed `Slice`.
pub const RUNTIME_LENGTH_OFFSET: i64 = 0;

/// Where one machine word of an `extern` call's argument list comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternWord {
    /// The local's own value: a scalar, or a pointer C is given as-is.
    Value(LocalId),
    /// The word `offset` bytes into what the local points at.
    Indirect { base: LocalId, offset: i64 },
}

/// Which of the two argument sequences a word is placed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternClass {
    Integer,
    Float,
}

/// A call's arguments, one machine word at a time, in the order the target's
/// argument sequences take them.
///
/// A Slopium callee takes each local whole, so this is the identity on its
/// arguments. An `extern` is where the vocabulary of `D-065` turns into words:
/// a borrowed `String` becomes the `const char *` inside it, and a borrowed
/// `Slice` becomes the pointer and the length C expects as two consecutive
/// arguments.
///
/// The words are handed to the backend's ordinary calling-convention code
/// rather than to the builtin plan, because only the former knows about the
/// float sequence and the stack.
pub fn call_words(
    module: &MirModule,
    callee: &str,
    args: &[LocalId],
    arg_types: &[Type],
) -> Vec<(ExternWord, ExternClass)> {
    match extern_declaration(module, callee) {
        Some(declaration) => extern_arguments(declaration, args),
        None => value_words(args, arg_types),
    }
}

/// A call's arguments where each one is a whole machine word.
///
/// Every Slopium callee takes them this way, and an indirect call has no name
/// to look an `extern` up by — which is the same statement twice, because a
/// function value can only ever point at a Slopium function (`D-092`).
pub fn value_words(args: &[LocalId], arg_types: &[Type]) -> Vec<(ExternWord, ExternClass)> {
    args.iter()
        .zip(arg_types)
        .map(|(arg, ty)| {
            let class = if *ty == Type::F64 {
                ExternClass::Float
            } else {
                ExternClass::Integer
            };
            (ExternWord::Value(*arg), class)
        })
        .collect()
}

fn extern_arguments(declaration: &MirExtern, args: &[LocalId]) -> Vec<(ExternWord, ExternClass)> {
    let mut words = Vec::new();
    for (index, ty) in declaration.params.iter().enumerate() {
        let Some(&arg) = args.get(index) else {
            break;
        };
        match ty {
            Type::F64 => words.push((ExternWord::Value(arg), ExternClass::Float)),
            Type::Ref { inner, .. } if matches!(inner.as_ref(), Type::String) => {
                words.push((
                    ExternWord::Indirect {
                        base: arg,
                        offset: RUNTIME_POINTER_OFFSET,
                    },
                    ExternClass::Integer,
                ));
            }
            Type::Ref { inner, .. } if matches!(inner.as_ref(), Type::Slice(_)) => {
                words.push((
                    ExternWord::Indirect {
                        base: arg,
                        offset: RUNTIME_POINTER_OFFSET,
                    },
                    ExternClass::Integer,
                ));
                words.push((
                    ExternWord::Indirect {
                        base: arg,
                        offset: RUNTIME_LENGTH_OFFSET,
                    },
                    ExternClass::Integer,
                ));
            }
            _ => words.push((ExternWord::Value(arg), ExternClass::Integer)),
        }
    }
    words
}

/// Words a function value carries before its first capture (`D-101`).
///
/// The code address, the helper that frees the block, and the helper that
/// copies it. The block is laid out as an ordinary struct, so those two are the
/// ones both backends already generate for every struct — which is why a
/// closure costs no new code generation at all.
pub const CLOSURE_HEADER: usize = 3;

pub fn struct_drop_symbol(name: &str) -> String {
    format!("sl_drop_struct_{}", encoded(name))
}

pub fn struct_clone_symbol(name: &str) -> String {
    format!("sl_clone_struct_{}", encoded(name))
}

pub fn enum_drop_symbol(name: &str) -> String {
    format!("sl_drop_enum_{}", encoded(name))
}

pub fn enum_clone_symbol(name: &str) -> String {
    format!("sl_clone_enum_{}", encoded(name))
}

fn encoded(name: &str) -> String {
    name.bytes().map(|byte| format!("{byte:02x}")).collect()
}

/// The helper that releases a value of this type, or `None` when the type owns
/// nothing and dropping it is a no-op.
pub fn drop_function(module: &MirModule, ty: &Type) -> Option<String> {
    match ty {
        Type::String => Some("sl_rt_string_drop".to_owned()),
        Type::List(_) | Type::Array { .. } => Some("sl_rt_list_drop".to_owned()),
        Type::Slice(_) => Some("sl_rt_slice_drop".to_owned()),
        Type::Named(inner) if module.structs.iter().any(|item| &item.name == inner) => {
            Some(struct_drop_symbol(inner))
        }
        Type::Named(inner) if module.enums.iter().any(|item| &item.name == inner) => {
            Some(enum_drop_symbol(inner))
        }
        // Every `Fn` reaches the same shim, which reads the helper out of the
        // block and jumps to it (`D-101`). The static type cannot name the
        // helper the way a struct's can: two closures of one type capture
        // different things, so the block is the only thing that knows.
        Type::Fn { .. } => Some("sl_rt_closure_drop".to_owned()),
        _ => None,
    }
}

/// The helper that copies a value of this type, or `None` when the value is its
/// own copy.
pub fn clone_function(module: &MirModule, ty: &Type) -> Option<String> {
    match ty {
        Type::String => Some("sl_rt_string_clone".to_owned()),
        Type::List(_) | Type::Array { .. } => Some("sl_rt_list_clone".to_owned()),
        Type::Slice(_) => Some("sl_rt_slice_clone".to_owned()),
        Type::Named(inner) if module.structs.iter().any(|item| &item.name == inner) => {
            Some(struct_clone_symbol(inner))
        }
        Type::Named(inner) if module.enums.iter().any(|item| &item.name == inner) => {
            Some(enum_clone_symbol(inner))
        }
        Type::Fn { .. } => Some("sl_rt_closure_clone".to_owned()),
        _ => None,
    }
}

/// Bytes to allocate for a struct: one word per field, never zero.
pub fn struct_size(module: &MirModule, name: &str) -> usize {
    module
        .structs
        .iter()
        .find(|item| item.name == name)
        .map(|item| item.fields.len() * 8)
        .unwrap_or(0)
        .max(8)
}

/// Bytes to allocate for an enum value with `field_count` payload words: the
/// tag plus the payload, never zero.
pub fn enum_size(field_count: usize) -> usize {
    ((field_count + 1) * 8).max(8)
}

/// Bytes an enum's clone helper allocates: enough for its widest variant, since
/// the helper sees the tag only at run time.
pub fn enum_clone_size(module: &MirModule, name: &str) -> usize {
    module
        .enums
        .iter()
        .find(|item| item.name == name)
        .map(|item| {
            item.variants
                .iter()
                .map(|variant| enum_size(variant.fields.len()))
                .max()
                .unwrap_or(8)
        })
        .unwrap_or(8)
        .max(8)
}

/// Whether a borrow of this type is a borrow of a slice, which the collection
/// builtins reach through a different runtime entry point than a list.
pub fn reference_is_slice(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Ref { inner, .. } if matches!(inner.as_ref(), Type::Slice(_))
    )
}

/// A value that is represented by a pointer, so borrowing it copies the pointer
/// rather than taking the address of the slot holding it.
///
/// `Type::Ptr` is deliberately absent (`D-067`). A raw pointer's word is a
/// value that happens to be an address, the way an integer's word is a value,
/// so `(& p)` takes the address of the slot holding it. Listing it here would
/// make a borrow of a pointer alias whatever the pointer points at.
pub fn is_pointer_like(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String
            | Type::List(_)
            | Type::Array { .. }
            | Type::Slice(_)
            | Type::Named(_)
            | Type::Fn { .. }
    )
}

/// The width of one machine access, for the narrow loads and stores a raw
/// pointer reaches memory through (`D-067`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessSize {
    Byte,
    Half,
    Word,
    Double,
}

impl AccessSize {
    pub fn bytes(self) -> u32 {
        match self {
            AccessSize::Byte => 1,
            AccessSize::Half => 2,
            AccessSize::Word => 4,
            AccessSize::Double => 8,
        }
    }

    /// AArch64's name for a load of this width, and the opcode beside it.
    ///
    /// The mnemonic and the encoding are returned by two functions over one
    /// table so they cannot drift apart: printing `ldrb` while assembling
    /// `ldr` is exactly the bug this width was added to fix.
    pub fn load_mnemonic(self) -> &'static str {
        match self {
            AccessSize::Byte => "ldrb",
            AccessSize::Half => "ldrh",
            AccessSize::Word | AccessSize::Double => "ldr",
        }
    }

    pub fn store_mnemonic(self) -> &'static str {
        match self {
            AccessSize::Byte => "strb",
            AccessSize::Half => "strh",
            AccessSize::Word | AccessSize::Double => "str",
        }
    }

    /// Whether the transfer register is an `X` rather than a `W`.
    ///
    /// Only the eight-byte access uses the wide register; `ldrb`, `ldrh` and
    /// the four-byte `ldr` all write a `W`, which zeroes the top half and is
    /// what makes every narrow load zero-extending.
    pub fn is_wide(self) -> bool {
        matches!(self, AccessSize::Double)
    }
}

/// How many bytes a volatile access to this type touches, or `None` for a type
/// no pointer may point at.
///
/// Both backends and the verifier ask *this* function rather than deciding for
/// themselves. Two encoders that each answer "how wide is a `bool`" separately
/// are two encoders that will eventually disagree, which is the drift `D-025`
/// exists to prevent — and a disagreement here does not crash, it writes over
/// the neighbouring device register.
pub fn access_size(ty: &Type) -> Option<AccessSize> {
    if let Some(kind) = ty.int_kind() {
        return Some(match kind.bits {
            8 => AccessSize::Byte,
            16 => AccessSize::Half,
            32 => AccessSize::Word,
            _ => AccessSize::Double,
        });
    }
    match ty {
        // A `bool` is one byte because that is what a device register holding
        // a flag is. Making it a word would read three bytes that are not the
        // program's.
        Type::Bool => Some(AccessSize::Byte),
        // A double is an ordinary 64-bit value everywhere but the two ABI
        // boundaries, so a volatile `f64` is the plain eight-byte access.
        Type::F64 => Some(AccessSize::Double),
        _ => None,
    }
}

/// Which panic trampolines the trapping arithmetic in a set of functions can
/// actually reach.
///
/// The two trap messages and their trampolines used to be emitted by every
/// program, whether or not it could trap. A program with no division carried a
/// `"division by zero"` string it could never reach. This says what is really
/// used, so a backend emits only that. It lives here because both backends must
/// answer it identically — a string one emits and the other omits would be a
/// difference the cross-backend suite has to chase (`D-025`).
///
/// Integer add, subtract and multiply check for overflow; integer division and
/// remainder check for a zero divisor *and* for the most-negative-over-`-1`
/// overflow, so they reach both. A shift checks its count, which is neither of
/// those things and says so in its own words. Bitwise operations, float
/// arithmetic and comparisons trap on nothing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrapUsage {
    pub overflow: bool,
    pub div_zero: bool,
    /// A shift by a negative amount or by the operand width or more. It is not
    /// an overflow — no value was too large — and reusing the overflow message
    /// for it would misdescribe the only bug a driver author writes here.
    pub shift: bool,
}

impl TrapUsage {
    fn complete(self) -> bool {
        self.overflow && self.div_zero && self.shift
    }
}

pub fn trap_usage<'a>(functions: impl Iterator<Item = &'a MirFunction>) -> TrapUsage {
    use crate::mir::BinaryOp;
    let mut usage = TrapUsage::default();
    for function in functions {
        for instruction in function
            .blocks
            .iter()
            .flat_map(|block| block.instructions())
        {
            if let Instruction::Binary { op, ty, .. } = instruction {
                if !ty.is_integer() {
                    continue;
                }
                match op {
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => usage.overflow = true,
                    BinaryOp::Div | BinaryOp::Rem => {
                        usage.overflow = true;
                        usage.div_zero = true;
                    }
                    BinaryOp::Shl | BinaryOp::Shr => usage.shift = true,
                    BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Less
                    | BinaryOp::Greater
                    | BinaryOp::LessEqual
                    | BinaryOp::GreaterEqual
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual => {}
                }
            }
        }
        if usage.complete() {
            break;
        }
    }
    usage
}

/// Locals whose frame address is handed to something else, and which therefore
/// cannot live in a register (`D-022`).
///
/// The builtins are read out of their own lowering plans rather than listed
/// again here. That list and the plans have to agree exactly — a local passed
/// by address but not pinned has no address to pass — and nothing in the type
/// system makes two copies of it agree, so there is only one.
pub fn address_taken(module: &MirModule, function: &MirFunction) -> Vec<bool> {
    let mut pinned = vec![false; function.locals.len()];
    let mut pin = |local: LocalId| {
        if let Some(entry) = pinned.get_mut(local) {
            *entry = true;
        }
    };
    for instruction in function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
    {
        match instruction {
            Instruction::AddressOf { src, .. } => {
                let scalar = function
                    .locals
                    .get(*src)
                    .is_some_and(|local| !is_pointer_like(&local.ty));
                if scalar {
                    pin(*src);
                }
            }
            Instruction::Call {
                dst,
                callee,
                args,
                arg_types,
                result,
            } => {
                let Some(steps) = builtin(module, *dst, callee, args, arg_types, result) else {
                    continue;
                };
                for step in &steps {
                    let Step::Invoke { arguments, .. } = step else {
                        continue;
                    };
                    for argument in arguments {
                        if let Argument::Address(local) = argument {
                            pin(*local);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    pinned
}

/// A value a builtin hands to the runtime, in the order the target's argument
/// registers take them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    /// The local's value.
    Value(LocalId),
    /// The address of the local's frame slot. Every local that appears here is
    /// reported by the backend's address-taken scan, so it has a slot (`D-022`).
    Address(LocalId),
    /// A constant, small enough for every target's immediate form.
    Immediate(i64),
    /// The address of a function, or a null pointer when the type needs no such
    /// helper. The runtime tests for null rather than calling through it.
    Function(Option<String>),
}

/// What happens once a builtin's arguments are in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tail {
    /// Call this runtime symbol.
    Call(String),
    /// Nothing. Cloning a type with no clone helper is a copy, and placing the
    /// argument already made it; the first argument is the result.
    FirstArgument,
}

/// One step of a builtin's lowering, in terms every backend can carry out.
///
/// "The result" is wherever the target returns an integer — `rax`, `x0` — which
/// is also where the caller of a builtin expects to find its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Place `arguments` in argument position, then `tail`.
    Invoke {
        arguments: Vec<Argument>,
        tail: Tail,
    },
    /// Copy the result into the builtin's destination.
    Save,
    /// Make the destination's value the result again, after the destination has
    /// been used as an argument.
    Restore,
    /// Replace the result with the word it points at.
    Load,
    /// Wrap the destination in the standard `Option`: a zero result becomes a
    /// freshly allocated `None`, anything else a `Some` holding the
    /// destination.
    WrapOption { some_tag: usize, none_tag: usize },
}

/// How to lower a call to `callee`, or `None` when it is an ordinary function
/// call and the target's calling convention applies.
///
/// The plan says what to do, not how: the backend decides which registers
/// arguments land in, how an address is formed, and how a branch is spelled.
/// What it does not decide is which runtime symbol runs, which is the part that
/// two backends must not disagree about.
pub fn builtin(
    module: &MirModule,
    dst: LocalId,
    callee: &str,
    args: &[LocalId],
    arg_types: &[Type],
    result: &Type,
) -> Option<Vec<Step>> {
    // An `extern` is an ordinary call, and it is checked here first because a
    // declaration is free to take a builtin's name in a package that never
    // reaches the builtin — the arms below match on the name alone.
    if extern_declaration(module, callee).is_some() {
        return None;
    }
    let call = |symbol: &str, arguments: Vec<Argument>| Step::Invoke {
        arguments,
        tail: Tail::Call(symbol.to_owned()),
    };
    let one = |symbol: &str| vec![call(symbol, vec![Argument::Value(args[0])])];

    let steps = match callee {
        // `clone` crosses a borrow (`D-091`), and a borrow of a pointer-shaped
        // value is that pointer rather than the address of a slot holding it,
        // so the glue to call is the one for what is behind the borrow.
        "clone" => vec![Step::Invoke {
            arguments: vec![Argument::Value(args[0])],
            tail: match clone_function(module, arg_types[0].strip_ref()) {
                Some(symbol) => Tail::Call(symbol),
                None => Tail::FirstArgument,
            },
        }],
        "list" | "array" => {
            let element = match result {
                Type::List(element) => element.as_ref(),
                Type::Array { element, .. } => element.as_ref(),
                _ => unreachable!("collection constructor must return List or Array"),
            };
            let mut steps = vec![
                call(
                    "sl_rt_list_new",
                    vec![
                        Argument::Immediate(8),
                        Argument::Function(drop_function(module, element)),
                        Argument::Function(clone_function(module, element)),
                    ],
                ),
                Step::Save,
            ];
            for arg in args {
                steps.push(call(
                    "sl_rt_list_push",
                    vec![Argument::Value(dst), Argument::Address(*arg)],
                ));
            }
            steps.push(Step::Restore);
            steps
        }
        "slice" => vec![call(
            "sl_rt_slice_new",
            args.iter().copied().map(Argument::Value).collect(),
        )],
        "len" => one(match &arg_types[0] {
            ty if reference_is_slice(ty) => "sl_rt_slice_len",
            Type::Ref { inner, .. } if inner.as_ref() == &Type::String => "sl_rt_string_len",
            _ => "sl_rt_list_len",
        }),
        "push" => vec![call(
            "sl_rt_list_push",
            vec![Argument::Value(args[0]), Argument::Address(args[1])],
        )],
        "get" | "get-ref" => {
            let entry = if reference_is_slice(&arg_types[0]) {
                "sl_rt_slice_get"
            } else {
                "sl_rt_list_get"
            };
            let mut steps = vec![call(
                entry,
                vec![Argument::Value(args[0]), Argument::Value(args[1])],
            )];
            // `get` copies the element out; `get-ref` yields the slot's address
            // and only dereferences it when the element is itself a pointer,
            // because then the reference is the pointer rather than the slot.
            let dereference = match callee {
                "get" => true,
                _ => matches!(
                    result,
                    Type::Ref { inner, .. } if matches!(
                        inner.as_ref(),
                        Type::String
                            | Type::List(_)
                            | Type::Array { .. }
                            | Type::Slice(_)
                            | Type::Named(_)
                    )
                ),
            };
            if dereference {
                steps.push(Step::Load);
            }
            steps
        }
        "pop" => {
            let Type::Named(option_name) = result else {
                unreachable!("pop must return Option<T>");
            };
            let option = module
                .enums
                .iter()
                .find(|item| &item.name == option_name)
                .expect("Option layout must be present");
            let tag = |name: &str| {
                option
                    .variants
                    .iter()
                    .find(|variant| variant.name == name)
                    .map(|variant| variant.tag)
                    .unwrap_or_else(|| panic!("Option must define {name}"))
            };
            vec![
                call(
                    "sl_rt_list_try_pop",
                    vec![Argument::Value(args[0]), Argument::Address(dst)],
                ),
                Step::WrapOption {
                    some_tag: tag("Some"),
                    none_tag: tag("None"),
                },
            ]
        }
        "remove" => vec![call(
            "sl_rt_list_remove",
            vec![Argument::Value(args[0]), Argument::Value(args[1])],
        )],
        // The element goes in by address like `push`'s and the old one comes
        // back in the result register like `remove`'s, because that is what it
        // is: the two halves of an update, done at one index and in one call.
        "replace" => vec![call(
            "sl_rt_list_replace",
            vec![
                Argument::Value(args[0]),
                Argument::Value(args[1]),
                Argument::Address(args[2]),
            ],
        )],
        _ => return None,
    };
    Some(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirEnum, MirVariant};

    fn module() -> MirModule {
        MirModule {
            functions: Vec::new(),
            externs: Vec::new(),
            tests: Vec::new(),
            structs: Vec::new(),
            enums: vec![MirEnum {
                name: "Option".into(),
                variants: vec![
                    MirVariant {
                        name: "None".into(),
                        tag: 0,
                        fields: Vec::new(),
                    },
                    MirVariant {
                        name: "Some".into(),
                        tag: 1,
                        fields: vec![("0".into(), Type::I64)],
                    },
                ],
                emit: true,
            }],
        }
    }

    #[test]
    fn an_ordinary_call_has_no_plan() {
        assert_eq!(
            builtin(&module(), 0, "user:helper", &[1], &[Type::I64], &Type::I64),
            None
        );
    }

    #[test]
    fn cloning_a_copy_type_calls_nothing() {
        let steps = builtin(&module(), 0, "clone", &[1], &[Type::I64], &Type::I64).unwrap();
        assert_eq!(
            steps,
            vec![Step::Invoke {
                arguments: vec![Argument::Value(1)],
                tail: Tail::FirstArgument,
            }]
        );
    }

    #[test]
    fn cloning_an_owning_type_calls_its_helper() {
        let steps = builtin(&module(), 0, "clone", &[1], &[Type::String], &Type::String).unwrap();
        assert_eq!(
            steps,
            vec![Step::Invoke {
                arguments: vec![Argument::Value(1)],
                tail: Tail::Call("sl_rt_string_clone".into()),
            }]
        );
    }

    /// The element's helpers are baked into the list at construction, so a list
    /// of owning values releases them and a list of scalars carries two nulls.
    #[test]
    fn a_collection_teaches_the_runtime_about_its_element() {
        let scalars = builtin(
            &module(),
            0,
            "list",
            &[1],
            &[Type::I64],
            &Type::List(Box::new(Type::I64)),
        )
        .unwrap();
        let Step::Invoke { arguments, .. } = &scalars[0] else {
            panic!("a collection starts by constructing the list");
        };
        assert_eq!(arguments[1], Argument::Function(None));
        assert_eq!(arguments[2], Argument::Function(None));

        let strings = builtin(
            &module(),
            0,
            "list",
            &[1],
            &[Type::String],
            &Type::List(Box::new(Type::String)),
        )
        .unwrap();
        let Step::Invoke { arguments, .. } = &strings[0] else {
            panic!("a collection starts by constructing the list");
        };
        assert_eq!(
            arguments[1],
            Argument::Function(Some("sl_rt_string_drop".into()))
        );
        assert_eq!(
            arguments[2],
            Argument::Function(Some("sl_rt_string_clone".into()))
        );
    }

    /// Every element is pushed by address, because the runtime copies a word
    /// out of the caller's frame rather than taking it in a register.
    #[test]
    fn every_element_of_a_collection_is_pushed_by_address() {
        let steps = builtin(
            &module(),
            7,
            "array",
            &[1, 2],
            &[Type::I64, Type::I64],
            &Type::Array {
                element: Box::new(Type::I64),
                length: 2,
            },
        )
        .unwrap();
        let pushes: Vec<&Vec<Argument>> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Invoke {
                    arguments,
                    tail: Tail::Call(symbol),
                } if symbol == "sl_rt_list_push" => Some(arguments),
                _ => None,
            })
            .collect();
        assert_eq!(
            pushes,
            vec![
                &vec![Argument::Value(7), Argument::Address(1)],
                &vec![Argument::Value(7), Argument::Address(2)],
            ]
        );
        assert_eq!(steps.last(), Some(&Step::Restore));
    }

    /// A borrowed slice and an owned list share every builtin's spelling and
    /// none of its runtime entry point.
    #[test]
    fn a_slice_reaches_different_runtime_entries_than_a_list() {
        let slice = Type::Ref {
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
            mutable: false,
        };
        let list = Type::Ref {
            inner: Box::new(Type::List(Box::new(Type::I64))),
            mutable: false,
        };
        for (callee, on_slice, on_list) in [
            ("len", "sl_rt_slice_len", "sl_rt_list_len"),
            ("get", "sl_rt_slice_get", "sl_rt_list_get"),
        ] {
            for (ty, expected) in [(&slice, on_slice), (&list, on_list)] {
                let steps = builtin(
                    &module(),
                    0,
                    callee,
                    &[1, 2],
                    std::slice::from_ref(ty),
                    &Type::I64,
                )
                .unwrap();
                let Step::Invoke {
                    tail: Tail::Call(symbol),
                    ..
                } = &steps[0]
                else {
                    panic!("{callee} calls the runtime");
                };
                assert_eq!(symbol, expected, "{callee} on {ty:?}");
            }
        }
    }

    #[test]
    fn pop_reads_the_option_tags_out_of_the_module() {
        let steps = builtin(
            &module(),
            3,
            "pop",
            &[1],
            &[Type::List(Box::new(Type::I64))],
            &Type::Named("Option".into()),
        )
        .unwrap();
        assert_eq!(
            steps.last(),
            Some(&Step::WrapOption {
                some_tag: 1,
                none_tag: 0
            })
        );
    }

    /// `len` picks its entry point from what is being measured. A string is not
    /// a collection, but its byte length is the one thing the library cannot
    /// work out for itself: the boundary hands C a pointer and no length.
    #[test]
    fn len_dispatches_on_what_is_measured() {
        let borrowed = |inner: Type| Type::Ref {
            inner: Box::new(inner),
            mutable: false,
        };
        for (ty, expected) in [
            (borrowed(Type::List(Box::new(Type::I64))), "sl_rt_list_len"),
            (
                borrowed(Type::Slice(Box::new(Type::I64))),
                "sl_rt_slice_len",
            ),
            (borrowed(Type::String), "sl_rt_string_len"),
        ] {
            let steps = builtin(&module(), 0, "len", &[1], &[ty], &Type::I64).unwrap();
            let Step::Invoke {
                tail: Tail::Call(symbol),
                ..
            } = &steps[0]
            else {
                panic!("len calls the runtime");
            };
            assert_eq!(symbol, expected);
        }
    }

    /// The hex encoding is what keeps a module path out of the assembler's way,
    /// and it has to be the same encoding everywhere a name is written.
    #[test]
    fn a_module_path_survives_symbol_encoding() {
        assert_eq!(function_symbol("a:b", false), "sl_fn_613a62");
        assert_eq!(function_symbol("a:b", true), "sl_test_613a62");
        assert_eq!(struct_drop_symbol("a:b"), "sl_drop_struct_613a62");
        assert_eq!(enum_clone_symbol("a:b"), "sl_clone_enum_613a62");
    }
}
