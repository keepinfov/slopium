//! Internal consistency checks for lowered MIR.
//!
//! The verifier exists to catch compiler bugs — a lowering mistake, a pass that
//! rewrites a block into an inconsistent state — before code generation turns
//! them into a miscompile that only shows up as wrong output at runtime.
//!
//! Failures are reported as `SL0700` diagnostics rather than panics. A v0.1.1
//! exit condition is that malformed source never panics the compiler, and a
//! panicking verifier would quietly weaken that guarantee for any input that
//! happened to trip a bug.
//!
//! Verification is not free, so it runs only in debug builds or when
//! `SLOPIUM_VERIFY_MIR=1` is set. See [`enabled`].

use crate::ast::Type;
use crate::cfg::{defs, successors, terminator_uses, uses, Cfg};
use crate::diagnostic::{codes, Diagnostic};
use crate::mir::{BinaryOp, Instruction, MirFunction, MirModule};

/// Whether MIR verification should run.
///
/// Debug builds always verify. Release builds opt in through
/// `SLOPIUM_VERIFY_MIR=1` so continuous integration can check the compiler it
/// actually ships.
pub fn enabled() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("SLOPIUM_VERIFY_MIR").is_some_and(|value| value == "1")
}

/// Verifies every function in `module`, returning one diagnostic per failure.
///
/// An empty result means the module passed. Callers should treat any result as
/// fatal: continuing to code generation with invalid MIR is what this is meant
/// to prevent.
pub fn verify_module(file: &str, module: &MirModule) -> Vec<Diagnostic> {
    verify_module_after(file, module, None)
}

fn verify_module_after(file: &str, module: &MirModule, phase: Option<&str>) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    for function in &module.functions {
        verify_function(file, function, phase, &mut errors);
    }
    for test in &module.tests {
        verify_function(file, &test.function, phase, &mut errors);
    }
    for function in module
        .functions
        .iter()
        .chain(module.tests.iter().map(|test| &test.function))
    {
        verify_extern_calls(file, module, function, phase, &mut errors);
    }
    errors
}

/// Checks every call that crosses into C.
///
/// Sema already refuses a declaration outside the vocabulary and an argument
/// that would move (`D-065`), but sema is one pass and this is the invariant
/// the backends actually rely on: `extern_arguments` reads a `String` or a
/// `Slice` through a fixed offset and hands the words to the C ABI, so a
/// mismatched or owned argument type here is a wrong call, not a type error.
fn verify_extern_calls(
    file: &str,
    module: &MirModule,
    function: &MirFunction,
    phase: Option<&str>,
    errors: &mut Vec<Diagnostic>,
) {
    let after = phase.map_or(String::new(), |phase| format!(" after {phase}"));
    let mut report = |message: String| {
        errors.push(Diagnostic::error(
            codes::INTERNAL,
            file,
            function.span,
            format!(
                "internal compiler error: MIR verification failed{after} in `{}`: {message}; \
                 this is a compiler bug",
                function.name
            ),
        ));
    };

    for (index, block) in function.blocks.iter().enumerate() {
        for (position, instruction) in block.instructions().enumerate() {
            let Instruction::Call {
                callee,
                arg_types,
                result,
                ..
            } = instruction
            else {
                continue;
            };
            let Some(declaration) = crate::lowering::extern_declaration(module, callee) else {
                continue;
            };
            let where_ = format!("block {index} instruction {position} calls extern `{callee}`");
            if arg_types.len() != declaration.params.len() {
                report(format!(
                    "{where_} with {} arguments but it is declared with {}",
                    arg_types.len(),
                    declaration.params.len()
                ));
                continue;
            }
            for (argument, (actual, declared)) in
                arg_types.iter().zip(&declaration.params).enumerate()
            {
                if actual != declared {
                    report(format!(
                        "{where_} passing `{actual:?}` as argument {argument}, declared \
                         `{declared:?}`"
                    ));
                }
                if !crossable_argument(actual) {
                    report(format!(
                        "{where_} passing `{actual:?}` as argument {argument}, which the C \
                         boundary cannot express, or which would move an owned value into C"
                    ));
                }
            }
            if result != &declaration.result {
                report(format!(
                    "{where_} taking `{result:?}` as the result, declared `{:?}`",
                    declaration.result
                ));
            }
        }
    }
}

/// The argument types an extern call may carry: the `D-065` vocabulary, which
/// is every type that is either `Copy` or already a borrow.
fn crossable_argument(ty: &Type) -> bool {
    match ty {
        Type::I32 | Type::I64 | Type::F64 | Type::Bool => true,
        Type::Ref {
            mutable: false,
            inner,
        } => {
            matches!(inner.as_ref(), Type::String | Type::Slice(_))
        }
        _ => false,
    }
}

fn verify_function(
    file: &str,
    function: &MirFunction,
    phase: Option<&str>,
    errors: &mut Vec<Diagnostic>,
) {
    let after = phase.map_or(String::new(), |phase| format!(" after {phase}"));
    let mut report = |message: String| {
        errors.push(Diagnostic::error(
            codes::INTERNAL,
            file,
            function.span,
            format!(
                "internal compiler error: MIR verification failed{after} in `{}`: {message}; \
                 this is a compiler bug",
                function.name
            ),
        ));
    };

    let blocks = function.blocks.len();
    let locals = function.locals.len();

    if blocks == 0 {
        report("the function has no blocks".into());
        return;
    }
    if function.entry >= blocks {
        report(format!(
            "entry block {} is out of range ({blocks} blocks)",
            function.entry
        ));
        return;
    }
    if function.entry != 0 {
        // Code generation emits blocks in index order and falls through into
        // the first one after storing parameters; it never jumps to the entry.
        // A pass that renumbers blocks must keep the entry at index zero.
        report(format!(
            "entry is block {}; code generation requires block 0",
            function.entry
        ));
    }

    // Parameters must be the leading locals, in order, and flagged as such.
    // Code generation relies on this when it stores incoming argument
    // registers into slots.
    for (index, param) in function.params.iter().enumerate() {
        if *param >= locals {
            report(format!(
                "parameter {index} refers to local {param}, out of range ({locals} locals)"
            ));
            continue;
        }
        if *param != index {
            report(format!(
                "parameter {index} is local {param}; parameters must be the leading locals"
            ));
        }
        if !function.locals[*param].is_param {
            report(format!("local {param} is a parameter but is not flagged"));
        }
    }

    for (index, block) in function.blocks.iter().enumerate() {
        for target in successors(&block.terminator) {
            if target >= blocks {
                report(format!(
                    "block {index} branches to block {target}, out of range ({blocks} blocks)"
                ));
            }
        }
        verify_block_operands(index, block, locals, &mut report);
    }

    let cfg = Cfg::new(function);
    verify_definedness(function, &cfg, &mut report);
}

fn verify_block_operands(
    index: usize,
    block: &crate::mir::BasicBlock,
    locals: usize,
    report: &mut impl FnMut(String),
) {
    let mut read = Vec::new();
    for (position, instruction) in block.instructions().enumerate() {
        read.clear();
        uses(instruction, &mut read);
        for local in read.iter().copied().chain(defs(instruction)) {
            if local >= locals {
                report(format!(
                    "block {index} instruction {position} refers to local {local}, \
                     out of range ({locals} locals)"
                ));
            }
        }
        verify_instruction_shape(index, position, instruction, report);
    }
    for local in terminator_uses(&block.terminator) {
        if local >= locals {
            report(format!(
                "block {index} terminator refers to local {local}, \
                 out of range ({locals} locals)"
            ));
        }
    }
}

fn verify_instruction_shape(
    index: usize,
    position: usize,
    instruction: &Instruction,
    report: &mut impl FnMut(String),
) {
    match instruction {
        Instruction::Call {
            callee,
            args,
            arg_types,
            ..
        } => {
            if args.len() != arg_types.len() {
                report(format!(
                    "block {index} instruction {position} calls `{callee}` with {} arguments \
                     but {} argument types",
                    args.len(),
                    arg_types.len()
                ));
            }
        }
        Instruction::Binary { op, ty, .. } => {
            // Comparisons produce a bool regardless of operand type; arithmetic
            // carries the operand type. Codegen selects integer or float
            // instructions from `ty`, so a wrong one is a silent miscompile.
            let comparison = matches!(op, BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal);
            let numeric = matches!(ty, Type::I32 | Type::I64 | Type::F64);
            if !comparison && !numeric {
                report(format!(
                    "block {index} instruction {position} performs arithmetic on non-numeric \
                     type `{ty:?}`"
                ));
            }
        }
        _ => {}
    }
}

/// Checks that every local read has at least one definition that can reach it.
///
/// This is a forward *may*-def dataflow (union across predecessors), not
/// must-def (intersection). The stricter check is deliberately not used: match
/// lowering terminates the trailing comparison block with `Goto(merge)` even
/// when the match is exhaustive, so the CFG contains a path to the merge that
/// never assigns the result temporary. Sema proves that path is dynamically
/// unreachable, but the CFG does not say so, and `boolean-score` in
/// `tests/projects/pass/aggregates-patterns` trips must-def for exactly this
/// reason.
///
/// [`verify_definite_initialization`] implements the strict version. Wiring it
/// in requires terminating that fallthrough block with
/// [`crate::mir::Terminator::Unreachable`], which changes emitted code and so belongs to a
/// later milestone.
fn verify_definedness(function: &MirFunction, cfg: &Cfg, report: &mut impl FnMut(String)) {
    check_initialization(function, cfg, Merge::Any, report);
}

/// The strict counterpart of [`verify_definedness`]: every read must be
/// preceded by a definition on *every* path, not merely some path.
///
/// Not wired into the compiler. Current match lowering violates it — see
/// [`verify_definedness`] — so it exists to be unit-tested against hand-built
/// MIR and to become the default once the match fallthrough is sealed with
/// [`crate::mir::Terminator::Unreachable`].
pub fn verify_definite_initialization(function: &MirFunction) -> Vec<String> {
    let cfg = Cfg::new(function);
    let mut errors = Vec::new();
    check_initialization(function, &cfg, Merge::All, &mut |message| {
        errors.push(message)
    });
    errors
}

/// How predecessor states combine at a join.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Merge {
    /// Union: a local counts as defined if any predecessor defines it.
    Any,
    /// Intersection: a local counts as defined only if all predecessors do.
    All,
}

fn check_initialization(
    function: &MirFunction,
    cfg: &Cfg,
    merge: Merge,
    report: &mut impl FnMut(String),
) {
    let blocks = function.blocks.len();
    let locals = function.locals.len();

    let mut entry_defined: Vec<Option<Vec<bool>>> = vec![None; blocks];
    let mut initial = vec![false; locals];
    for param in &function.params {
        if *param < locals {
            initial[*param] = true;
        }
    }
    entry_defined[function.entry] = Some(initial);

    let order = cfg.reverse_postorder().to_vec();
    // Both directions terminate: `Any` only ever adds members and `All` only
    // ever removes them, and the lattice is finite.
    let mut changed = true;
    while changed {
        changed = false;
        for &block in &order {
            let Some(mut defined) = entry_defined[block].clone() else {
                continue;
            };
            for instruction in function.blocks[block].instructions() {
                if let Some(local) = defs(instruction) {
                    if local < locals {
                        defined[local] = true;
                    }
                }
            }
            for target in successors(&function.blocks[block].terminator) {
                if target >= blocks {
                    continue;
                }
                match &mut entry_defined[target] {
                    Some(existing) => {
                        for (slot, incoming) in existing.iter_mut().zip(defined.iter()) {
                            let updated = match merge {
                                Merge::Any => *slot || *incoming,
                                Merge::All => *slot && *incoming,
                            };
                            if updated != *slot {
                                *slot = updated;
                                changed = true;
                            }
                        }
                    }
                    slot @ None => {
                        *slot = Some(defined.clone());
                        changed = true;
                    }
                }
            }
        }
    }

    let explanation = match merge {
        Merge::Any => "which is never defined",
        Merge::All => "before it is defined on every path",
    };
    let mut read = Vec::new();
    for &block in &order {
        let Some(mut defined) = entry_defined[block].clone() else {
            continue;
        };
        for (position, instruction) in function.blocks[block].instructions().enumerate() {
            read.clear();
            uses(instruction, &mut read);
            for local in read.iter().copied() {
                if local < locals && !defined[local] {
                    report(format!(
                        "block {block} instruction {position} reads local {local} {explanation}"
                    ));
                }
            }
            if let Some(local) = defs(instruction) {
                if local < locals {
                    defined[local] = true;
                }
            }
        }
        for local in terminator_uses(&function.blocks[block].terminator) {
            if local < locals && !defined[local] {
                report(format!(
                    "block {block} terminator reads local {local} {explanation}"
                ));
            }
        }
    }
}

/// The pipeline gate: verifies `module` when [`enabled`], naming the pass that
/// produced it so a failure points at the right stage.
///
/// Returns `Err` with every failure rather than only the first, because one
/// lowering mistake usually shows up as several broken invariants and seeing
/// them together identifies the cause faster.
pub fn check(file: &str, module: &MirModule, phase: &str) -> Result<(), Vec<Diagnostic>> {
    if !enabled() {
        return Ok(());
    }
    let errors = verify_module_after(file, module, Some(phase));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::{verify_definite_initialization, verify_module};
    use crate::ast::Type;
    use crate::mir::{
        BasicBlock, BinaryOp, Instruction, MirFunction, MirLocal, MirModule, Terminator,
    };
    use crate::{compile_to_mir, CompileOptions};

    fn module_with(function: MirFunction) -> MirModule {
        MirModule {
            functions: vec![function],
            externs: Vec::new(),
            tests: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
        }
    }

    fn probe(blocks: Vec<BasicBlock>, locals: usize) -> MirFunction {
        MirFunction {
            name: "probe".into(),
            emit: true,
            params: Vec::new(),
            return_type: Type::I64,
            locals: (0..locals)
                .map(|_| MirLocal {
                    ty: Type::I64,
                    name: None,
                    is_param: false,
                })
                .collect(),
            blocks,
            entry: 0,
            span: Default::default(),
        }
    }

    #[test]
    fn real_programs_verify() {
        let source = r#"
            (struct Pair ((left String) (right String)))
            (enum Shape Empty (Sized ((size i64))))
            (fn helper ((a i64)) -> i64
              (let b (+ a 2))
              (if (< a b) b a))
            (fn measure ((shape Shape)) -> i64
              (match shape
                ((Shape:Sized size) size)
                ((Shape:Empty) 0)))
            (fn main () -> i32
              (let pair (Pair :left "left" :right "right"))
              (let copied (clone pair))
              (let mut total 0)
              (while (< total 3) (set total (+ total 1)))
              (println total)
              (println (helper 3))
              (println (measure (Shape:Sized 7)))
              0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        assert_eq!(verify_module("test.slp", &mir), Vec::new());
    }

    #[test]
    fn an_extern_call_must_agree_with_its_declaration() {
        let source = r#"
            (extern "hal_add" (hal-add (a i64) (b i64)) -> i64)
            (fn main () -> i32 (println (hal-add 20 22)) 0)
        "#;
        let mut mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        assert_eq!(verify_module("test.slp", &mir), Vec::new());

        // An owned `String` is outside the vocabulary and would move into C,
        // which is exactly what the backends must never be handed (`D-065`).
        mir.externs[0].params[1] = Type::String;
        let errors = verify_module("test.slp", &mir);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("declared")),
            "a mismatched extern argument must be reported: {errors:?}"
        );
    }

    /// The verifier's real risk is a false positive on valid code, so check it
    /// against every shipped fixture rather than only hand-written snippets.
    /// This is what caught a must-init check rejecting `boolean-score`.
    #[test]
    fn the_shipped_fixture_corpus_verifies() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/projects");
        let Ok(categories) = std::fs::read_dir(&root) else {
            // Absent from packaged source snapshots; nothing to check there.
            return;
        };
        let mut checked = 0;
        let mut failures = Vec::new();
        for category in categories.flatten() {
            let Ok(projects) = std::fs::read_dir(category.path()) else {
                continue;
            };
            for project in projects.flatten() {
                let entry = project.path().join("src/main.slp");
                let Ok(source) = std::fs::read_to_string(&entry) else {
                    continue;
                };
                let name = entry.display().to_string();
                for optimize in [false, true] {
                    let options = CompileOptions {
                        optimize,
                        ..CompileOptions::default()
                    };
                    // Fixtures that need dependencies or are meant to fail
                    // simply do not lower here; skip them.
                    let Ok(mir) = compile_to_mir(&name, &source, &options) else {
                        continue;
                    };
                    checked += 1;
                    for error in verify_module(&name, &mir) {
                        failures.push(format!("{name} (optimize={optimize}): {}", error.message));
                    }
                }
            }
        }
        assert!(checked > 0, "no fixture lowered; the corpus path is wrong");
        assert!(
            failures.is_empty(),
            "the verifier rejected shipped fixtures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn optimized_programs_verify() {
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
        assert_eq!(verify_module("test.slp", &mir), Vec::new());
    }

    #[test]
    fn rejects_a_successor_out_of_range() {
        let mir = module_with(probe(
            vec![BasicBlock::synthetic(Vec::new(), Terminator::Goto(7))],
            1,
        ));
        let errors = verify_module("test.slp", &mir);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "SL0700");
        assert!(
            errors[0].message.contains("out of range"),
            "{:?}",
            errors[0]
        );
    }

    #[test]
    fn rejects_a_local_out_of_range() {
        let mir = module_with(probe(
            vec![BasicBlock::synthetic(
                vec![Instruction::Assign { dst: 0, src: 9 }],
                Terminator::Return(None),
            )],
            1,
        ));
        let errors = verify_module("test.slp", &mir);
        assert!(
            errors.iter().any(|error| error.message.contains("local 9")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_a_read_before_definition() {
        let mir = module_with(probe(
            vec![BasicBlock::synthetic(
                vec![Instruction::Assign { dst: 0, src: 1 }],
                Terminator::Return(Some(0)),
            )],
            2,
        ));
        let errors = verify_module("test.slp", &mir);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("never defined")),
            "{errors:?}"
        );
    }

    /// bb0 branches; only the then-branch defines _1, but the merge returns it.
    fn defined_on_one_branch_only() -> MirFunction {
        probe(
            vec![
                BasicBlock::synthetic(
                    vec![Instruction::ConstBool {
                        dst: 0,
                        value: true,
                    }],
                    Terminator::Branch {
                        condition: 0,
                        then_block: 1,
                        else_block: 2,
                    },
                ),
                BasicBlock::synthetic(
                    vec![Instruction::ConstInt { dst: 1, value: 1 }],
                    Terminator::Goto(3),
                ),
                BasicBlock::synthetic(Vec::new(), Terminator::Goto(3)),
                BasicBlock::synthetic(Vec::new(), Terminator::Return(Some(1))),
            ],
            2,
        )
    }

    #[test]
    fn a_local_defined_on_one_branch_passes_may_init() {
        // The compiler's own match lowering produces exactly this shape, so the
        // wired-in check must accept it. See `verify_definedness`.
        let errors = verify_module("test.slp", &module_with(defined_on_one_branch_only()));
        assert_eq!(errors, Vec::new());
    }

    #[test]
    fn a_local_defined_on_one_branch_fails_definite_initialization() {
        let errors = verify_definite_initialization(&defined_on_one_branch_only());
        assert!(
            errors
                .iter()
                .any(|error| error.contains("before it is defined on every path")),
            "the strict check must still catch it: {errors:?}"
        );
    }

    #[test]
    fn real_match_lowering_fails_definite_initialization_today() {
        // Documents why `verify_definite_initialization` is not wired in: an
        // exhaustive `match` leaves a CFG path to the merge that never assigns
        // the result. Sema proves it dynamically unreachable; the CFG does not.
        // When match lowering seals that block, this test should start failing
        // and the strict check can become the default.
        let source = r#"
            (fn boolean-score ((value bool)) -> i64
              (match value
                (true 1)
                (false 0)))
            (fn main () -> i32 0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let scored = mir
            .functions
            .iter()
            .find(|function| function.name.contains("boolean-score"))
            .expect("the function was lowered");
        assert!(
            !verify_definite_initialization(scored).is_empty(),
            "if this passes, seal the match fallthrough and wire the strict check in"
        );
        assert_eq!(
            verify_module("test.slp", &mir),
            Vec::new(),
            "the wired-in may-init check must still accept it"
        );
    }

    #[test]
    fn rejects_a_call_arity_mismatch() {
        let mir = module_with(probe(
            vec![BasicBlock::synthetic(
                vec![
                    Instruction::ConstInt { dst: 0, value: 1 },
                    Instruction::Call {
                        dst: 1,
                        callee: "helper".into(),
                        args: vec![0],
                        arg_types: vec![Type::I64, Type::I64],
                        result: Type::I64,
                    },
                ],
                Terminator::Return(Some(1)),
            )],
            2,
        ));
        let errors = verify_module("test.slp", &mir);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("argument types")),
            "{errors:?}"
        );
    }

    #[test]
    fn rejects_a_mismatched_parameter_list() {
        let mut function = probe(
            vec![BasicBlock::synthetic(Vec::new(), Terminator::Return(None))],
            2,
        );
        function.params = vec![1];
        let errors = verify_module("test.slp", &module_with(function));
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("leading locals")),
            "{errors:?}"
        );
    }

    #[test]
    fn accepts_arithmetic_and_rejects_it_on_a_non_numeric_type() {
        let good = module_with(probe(
            vec![BasicBlock::synthetic(
                vec![
                    Instruction::ConstInt { dst: 0, value: 1 },
                    Instruction::Binary {
                        dst: 1,
                        op: BinaryOp::Add,
                        lhs: 0,
                        rhs: 0,
                        ty: Type::I64,
                    },
                ],
                Terminator::Return(Some(1)),
            )],
            2,
        ));
        assert_eq!(verify_module("test.slp", &good), Vec::new());

        let bad = module_with(probe(
            vec![BasicBlock::synthetic(
                vec![
                    Instruction::StringNew {
                        dst: 0,
                        value: "x".into(),
                    },
                    Instruction::Binary {
                        dst: 1,
                        op: BinaryOp::Add,
                        lhs: 0,
                        rhs: 0,
                        ty: Type::String,
                    },
                ],
                Terminator::Return(Some(1)),
            )],
            2,
        ));
        assert!(verify_module("test.slp", &bad)
            .iter()
            .any(|error| error.message.contains("non-numeric")),);
    }
}
