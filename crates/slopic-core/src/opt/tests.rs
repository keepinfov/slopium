use super::{optimize, stats};
use crate::mir::{Instruction, MirModule, Terminator};
use crate::{compile_to_mir, CompileOptions};

fn release(source: &str) -> MirModule {
    compile_to_mir(
        "test.slp",
        source,
        &CompileOptions {
            optimize: true,
            ..CompileOptions::default()
        },
    )
    .unwrap()
}

fn debug(source: &str) -> MirModule {
    compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap()
}

fn function<'a>(module: &'a MirModule, name: &str) -> &'a crate::mir::MirFunction {
    module
        .functions
        .iter()
        .find(|function| function.name == name || function.name.ends_with(&format!(":{name}")))
        .unwrap_or_else(|| panic!("no function {name} in {:?}", names(module)))
}

fn names(module: &MirModule) -> Vec<&str> {
    module
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect()
}

/// Whether a function still calls `name`, matching both the bare name used in
/// single-file compilation and the `module:name` form used inside a package.
fn calls(function: &crate::mir::MirFunction, name: &str) -> bool {
    instructions(function).iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::Call { callee, .. }
                if callee == name || callee.ends_with(&format!(":{name}"))
        )
    })
}

fn instructions(function: &crate::mir::MirFunction) -> Vec<&Instruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
        .collect()
}

#[test]
fn folds_across_a_branch() {
    // The constant reaches the merge from both arms, so the addition after the
    // branch is foldable only by a cross-block analysis.
    let source = r#"
        (fn probe ((flag bool)) -> i64
          (let base (if flag 10 10))
          (+ base 5))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    assert!(
        instructions(probe)
            .iter()
            .any(|inst| matches!(inst, Instruction::ConstInt { value: 15, .. })),
        "expected a folded 15, got {:#?}",
        instructions(probe)
    );
}

#[test]
fn does_not_fold_an_overflowing_operation() {
    // Overflow panics at runtime with a normalized message and status 101.
    // Folding it away would delete that observable failure.
    let source = r#"
        (fn probe () -> i64
          (let big 9223372036854775807)
          (+ big 1))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    assert!(
        instructions(probe)
            .iter()
            .any(|inst| matches!(inst, Instruction::Binary { .. })),
        "the overflowing addition must survive to code generation"
    );
}

#[test]
fn does_not_fold_division_by_zero() {
    let source = r#"
        (fn probe () -> i64
          (let zero 0)
          (/ 10 zero))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    assert!(
        instructions(probe)
            .iter()
            .any(|inst| matches!(inst, Instruction::Binary { .. })),
        "the trapping division must survive to code generation"
    );
}

#[test]
fn resolves_a_constant_branch_and_drops_the_dead_arm() {
    let source = r#"
        (fn probe () -> i64 (if true 1 2))
        (fn main () -> i32 0)
    "#;
    let optimized = release(source);
    let unoptimized = debug(source);

    let before = function(&unoptimized, "probe").blocks.len();
    let after = function(&optimized, "probe").blocks.len();
    assert!(
        after < before,
        "a constant branch should shrink the CFG: {before} -> {after}"
    );
    assert!(
        !instructions(function(&optimized, "probe"))
            .iter()
            .any(|inst| matches!(inst, Instruction::ConstInt { value: 2, .. })),
        "the untaken arm should be gone"
    );
}

#[test]
fn every_block_stays_terminated_and_reachable_after_simplification() {
    let source = r#"
        (fn probe ((n i64)) -> i64
          (let mut total 0)
          (while (< total n) (set total (+ total 1)))
          (if (> total 5) total 0))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    let cfg = crate::cfg::Cfg::new(probe);
    for block in 0..probe.blocks.len() {
        assert!(
            cfg.is_reachable(block),
            "simplification left bb{block} unreachable"
        );
    }
    assert_eq!(probe.entry, 0, "code generation requires entry at block 0");
}

#[test]
fn dead_pure_instructions_are_removed_but_drops_are_kept() {
    let source = r#"
        (fn probe () -> i64
          (let unused 7)
          (let text "kept")
          (println (& text))
          42)
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    let body = instructions(probe);

    assert!(
        body.iter()
            .any(|inst| matches!(inst, Instruction::Drop { .. })),
        "the string must still be dropped"
    );
    assert!(
        body.iter().any(|inst| matches!(
            inst,
            Instruction::Call { callee, .. } if callee.contains("print")
        )),
        "the call must survive; it has side effects"
    );
    assert!(
        !body
            .iter()
            .any(|inst| matches!(inst, Instruction::ConstInt { value: 7, .. })),
        "the unused constant should be gone"
    );
}

#[test]
fn a_call_is_never_removed_even_when_its_result_is_unused() {
    let source = r#"
        (fn shout () -> i64 (println 1) 0)
        (fn probe () -> i64 (let ignored (shout)) 5)
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    let has_effect = instructions(probe)
        .iter()
        .any(|inst| matches!(inst, Instruction::Call { .. }))
        || instructions(probe).iter().any(
            |inst| matches!(inst, Instruction::Call { callee, .. } if callee.contains("print")),
        );
    assert!(
        has_effect,
        "the callee's side effect must survive, inlined or not: {:#?}",
        instructions(probe)
    );
}

#[test]
fn inlines_a_small_leaf_function() {
    let source = r#"
        (fn double ((n i64)) -> i64 (* n 2))
        (fn probe ((n i64)) -> i64 (double n))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    assert!(
        !calls(probe, "double"),
        "the call should have been inlined: {:#?}",
        instructions(probe)
    );
}

#[test]
fn does_not_inline_an_extern_even_when_a_body_appears_under_its_name() {
    // Opacity today rests on an extern having no `MirFunction`. Planting one
    // under its name is the future this guards against: the body a later pass
    // might find is not the body the linker will call (`D-073`).
    let source = r#"
        (extern "hal_double" (hal-double (n i64)) -> i64)
        (fn probe ((n i64)) -> i64 (hal-double n))
        (fn main () -> i32 0)
    "#;
    let mut module = release(source);
    let name = module.externs[0].name.clone();
    let mut impostor = function(&module, "probe").clone();
    impostor.name = name.clone();
    module.functions.push(impostor);

    optimize("test.slp", &mut module).unwrap();
    let probe = function(&module, "probe");
    assert!(
        calls(probe, &name),
        "an extern call must survive inlining: {:#?}",
        instructions(probe)
    );
}

#[test]
fn does_not_inline_a_recursive_function() {
    let source = r#"
        (fn countdown ((n i64)) -> i64
          (if (< n 1) 0 (countdown (- n 1))))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let countdown = function(&module, "countdown");
    assert!(
        calls(countdown, "countdown"),
        "a recursive call must not be inlined: {:#?}",
        instructions(countdown)
    );
}

#[test]
fn optimization_reaches_a_fixpoint_and_shrinks_the_module() {
    let source = r#"
        (fn double ((n i64)) -> i64 (* n 2))
        (fn probe () -> i64
          (let a 3)
          (let b (double a))
          (if true (+ b 1) 0))
        (fn main () -> i32 0)
    "#;
    let before = stats(&debug(source));
    let after = stats(&release(source));
    assert!(
        after.statements < before.statements,
        "optimization should reduce statement count: {before:?} -> {after:?}"
    );
}

#[test]
fn optimizing_twice_is_idempotent() {
    let source = r#"
        (fn probe ((n i64)) -> i64
          (let a (+ 1 2))
          (if (< n a) a n))
        (fn main () -> i32 0)
    "#;
    let mut once = release(source);
    let first = stats(&once);
    optimize("test.slp", &mut once).expect("re-optimizing stays valid");
    assert_eq!(
        first,
        stats(&once),
        "a second pipeline run should find nothing left to do"
    );
}

#[test]
fn unreachable_blocks_are_renumbered_consistently() {
    let source = r#"
        (fn probe ((n i64)) -> i64
          (loop (break))
          n)
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    let probe = function(&module, "probe");
    for block in &probe.blocks {
        for target in crate::cfg::successors(&block.terminator) {
            assert!(
                target < probe.blocks.len(),
                "a terminator points past the end after renumbering"
            );
        }
        assert!(
            !matches!(block.terminator, Terminator::Unreachable) || block.statements.is_empty(),
            "an unreachable block should not carry statements"
        );
    }
}

#[test]
fn the_shipped_fixture_corpus_still_optimizes_cleanly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/projects/pass");
    let Ok(projects) = std::fs::read_dir(&root) else {
        return;
    };
    let mut checked = 0;
    for project in projects.flatten() {
        let entry = project.path().join("src/main.slp");
        let Ok(source) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let name = entry.display().to_string();
        let options = CompileOptions {
            optimize: true,
            ..CompileOptions::default()
        };
        // Fixtures needing dependencies do not lower standalone; skip them.
        let Ok(module) = compile_to_mir(&name, &source, &options) else {
            continue;
        };
        checked += 1;
        for function in &module.functions {
            assert_eq!(function.entry, 0, "{name}: entry moved off block 0");
            let cfg = crate::cfg::Cfg::new(function);
            for block in 0..function.blocks.len() {
                assert!(
                    cfg.is_reachable(block),
                    "{name}: {} left bb{block} unreachable",
                    function.name
                );
            }
        }
    }
    assert!(checked > 0, "no fixture lowered; the corpus path is wrong");
}
