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
        (extern "sl_rt_show" (show (text (& String))) -> unit)
        (fn probe () -> i64
          (let unused 7)
          (let text "kept")
          (show (& text))
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
            Instruction::Call { callee, .. } if callee.contains("show")
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
        (extern "sl_rt_note" (note (value i64)) -> unit)
        (fn shout () -> i64 (note 1) 0)
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

/// A body over the ordinary ceiling, so that the annotation is the only thing
/// that can decide it (`D-122`).
const BIG_CALLEE: &str = r#"
        (fn blend ((a i64) (b i64)) -> i64
          (let one (+ a b))
          (let two (* one 3))
          (let three (- two a))
          (let four (+ three b))
          (let five (* four 2))
          (let six (- five 1))
          (let seven (+ six a))
          (let eight (* seven 2))
          (- eight b))
        (fn probe ((n i64)) -> i64 (blend n 3))
        (fn main () -> i32 0)
    "#;

#[test]
fn a_body_over_the_ceiling_is_left_alone() {
    let module = release(BIG_CALLEE);
    assert!(
        calls(function(&module, "probe"), "blend"),
        "the ceiling is what the annotation moves, so it has to hold without one"
    );
}

#[test]
fn an_inline_annotation_moves_the_ceiling() {
    let module = release(&BIG_CALLEE.replace("(fn blend", "(fn (inline) blend"));
    let probe = function(&module, "probe");
    assert!(
        !calls(probe, "blend"),
        "the hint should have inlined the call: {:#?}",
        instructions(probe)
    );
}

#[test]
fn an_inline_annotation_does_not_inline_a_recursive_function() {
    // The hint moves one number. Everything that makes inlining sound — and
    // termination is the loudest of them — is still the pass's to decide.
    let source = r#"
        (fn (inline) countdown ((n i64)) -> i64
          (if (< n 1) 0 (countdown (- n 1))))
        (fn probe ((n i64)) -> i64 (countdown n))
        (fn main () -> i32 0)
    "#;
    let module = release(source);
    assert!(calls(function(&module, "probe"), "countdown"));
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
        let mut options = CompileOptions {
            optimize: true,
            ..CompileOptions::default()
        };
        for (item, path) in crate::language_items_of(crate::STD_PACKAGE) {
            let slot = match item.as_str() {
                "option" => &mut options.language_items.option,
                "result" => &mut options.language_items.result,
                "result-ok" => &mut options.language_items.result_ok,
                _ => &mut options.language_items.result_err,
            };
            *slot = Some(path);
        }
        // A fixture prints, so it needs the library; one that also needs a path
        // dependency does not lower here at all, and is skipped.
        let mut files = crate::std_package_sources(crate::STD_PACKAGE);
        files.push(crate::package::PackageSource {
            path: name.clone(),
            namespace: None,
            module: "main".into(),
            source,
        });
        let input = crate::package::PackageInput {
            name: "fixture".into(),
            entry_module: "main".into(),
            files,
        };
        let Ok(module) = crate::compile_package_to_mir(&input, &options) else {
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

// ------------------------------------------------------ volatile (`D-067`)

/// Every volatile test writes through a pointer the runtime allocator gave it,
/// because a raw pointer has to come from somewhere and `sl_rt_alloc` is
/// already linked into every program.
const POINTER_PRELUDE: &str = r#"
    (extern "sl_rt_alloc" (rt-alloc (size u64)) -> (Ptr u8))
"#;

fn volatile_accesses(function: &crate::mir::MirFunction) -> usize {
    instructions(function)
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                Instruction::VolatileLoad { .. } | Instruction::VolatileStore { .. }
            )
        })
        .count()
}

/// The `is_pure` test. A read whose result nothing uses is how a device
/// register is cleared, so it is not dead however unused it looks.
#[test]
fn a_volatile_load_survives_with_its_result_unused() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let port (rt-alloc 8))
            (let ignored (volatile-read port)))
          5)
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    assert_eq!(
        volatile_accesses(function(&module, "probe")),
        1,
        "an unused volatile read is still an access: {:#?}",
        instructions(function(&module, "probe"))
    );
}

#[test]
fn a_volatile_store_survives() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let port (rt-alloc 8))
            (volatile-write port 1))
          5)
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    assert_eq!(volatile_accesses(function(&module, "probe")), 1);
}

/// A tripwire for a future common-subexpression pass. Nothing merges these
/// today; the point is that the day something does, this fails rather than a
/// driver silently reading one word where it asked for two.
#[test]
fn two_volatile_loads_of_one_address_are_both_kept() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let port (rt-alloc 8))
            (let first (volatile-read port))
            (let second (volatile-read port))
            (as i64 (+ first second))))
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    assert_eq!(volatile_accesses(function(&module, "probe")), 2);
}

/// The `cfg::defs` test, and the one most worth having: a device decides what
/// a volatile read answers, so a constant written through the pointer a moment
/// earlier must not be propagated into the read of it.
#[test]
fn a_volatile_load_is_never_folded_to_what_was_written() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let port (as (Ptr i64) (rt-alloc 8)))
            (volatile-write port 7)
            (+ (volatile-read port) 1)))
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    let probe = function(&module, "probe");
    assert_eq!(volatile_accesses(probe), 2);
    assert!(
        instructions(probe)
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Binary { .. })),
        "the addition must survive rather than folding to 8: {:#?}",
        instructions(probe)
    );
}

/// The `cfg::uses` test: the arithmetic that computed the address is not dead.
#[test]
fn the_address_of_a_volatile_access_is_not_dead() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let base (rt-alloc 64))
            (volatile-write (ptr-offset base 3) 9))
          5)
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    let probe = function(&module, "probe");
    assert_eq!(volatile_accesses(probe), 1);
    assert!(
        instructions(probe)
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Binary { .. })),
        "the offset arithmetic must survive: {:#?}",
        instructions(probe)
    );
}

/// Optimizing must not change how many times the device is touched, which is
/// the property `opt::check_volatile` asserts pass by pass (`D-114`).
#[test]
fn optimization_does_not_change_the_volatile_count() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn probe () -> i64
          (unsafe
            (let port (rt-alloc 64))
            (volatile-write port 1)
            (volatile-write (ptr-offset port 1) 2)
            (as i64 (volatile-read port))))
        (fn main () -> i32 0)"
    );
    assert_eq!(
        volatile_accesses(function(&debug(&source), "probe")),
        volatile_accesses(function(&release(&source), "probe")),
    );
}

/// Two calls to a function holding one access make two accesses, and that is
/// correct rather than a bug: both calls really do reach the device. It is
/// also why `D-114` counts accesses instead of giving each one an identity —
/// an identity would have to be duplicated here, and then it would identify
/// nothing.
#[test]
fn inlining_a_volatile_access_duplicates_it_once_per_call() {
    let source = format!(
        "{POINTER_PRELUDE}
        (fn poke ((port (Ptr u8))) -> unit
          (unsafe (volatile-write port 1)))
        (fn probe () -> i64
          (unsafe
            (let port (rt-alloc 8))
            (poke port)
            (poke port))
          5)
        (fn main () -> i32 0)"
    );
    let module = release(&source);
    let probe = function(&module, "probe");
    let inlined = volatile_accesses(probe);
    let calls_poke = calls(probe, "poke");
    assert!(
        (inlined == 2 && !calls_poke) || (inlined == 0 && calls_poke),
        "either both copies were inlined or neither was: {inlined} accesses, \
         calls_poke = {calls_poke}"
    );
}

#[test]
fn a_lattice_that_has_not_settled_folds_nothing_and_says_so() {
    // The states of a constant-propagation dataflow are too optimistic until
    // the loop settles: a local still reads `Known` where a predecessor not yet
    // visited would have made it `Varying`. Folding on one of those rewrites a
    // `Branch` into a `Goto` that the program never asked for, so reaching the
    // bound has to end the compilation rather than produce a result.
    let source = r#"
        (fn probe ((limit i64)) -> i64
          (let mut total 0)
          (while (< total limit)
            (set total (+ total 1)))
          total)
        (fn main () -> i32 0)
    "#;
    let mut module = debug(source);
    let before = module.clone();
    let probe = module
        .functions
        .iter_mut()
        .find(|function| function.name.ends_with("probe"))
        .expect("probe is in the module");

    let error = super::propagate_constants_within("test.slp", probe, 1)
        .expect_err("one round cannot settle a loop");
    assert_eq!(error[0].code, "SL0700");
    assert!(
        error[0].message.contains("did not settle in"),
        "unexpected message: {}",
        error[0].message
    );
    assert_eq!(
        format!("{:?}", instructions(function(&module, "probe"))),
        format!("{:?}", instructions(function(&before, "probe"))),
        "a run that did not settle must leave the function alone"
    );
}

#[test]
fn the_same_dataflow_settles_within_its_real_bound() {
    // The other half of the claim above: the bound is generous, and reaching it
    // means the bound is wrong rather than that the program is unusual.
    let source = r#"
        (fn probe ((limit i64)) -> i64
          (let mut total 0)
          (while (< total limit)
            (set total (+ total 1)))
          total)
        (fn main () -> i32 0)
    "#;
    let mut module = debug(source);
    let probe = module
        .functions
        .iter_mut()
        .find(|function| function.name.ends_with("probe"))
        .expect("probe is in the module");
    super::propagate_constants("test.slp", probe).expect("the dataflow settles");
}

#[test]
fn a_terminator_that_leaves_the_function_is_reported_rather_than_panicked_on() {
    // `simplify_cfg` indexes `function.blocks` by a terminator's target and
    // renumbers blocks through a table indexed the same way, so a target past
    // the end is an out-of-bounds index rather than a bad program. The verifier
    // catches it, and the verifier does not run in the profile this pipeline
    // does, which is why the block-target check is unconditional.
    let mut module = debug("(fn main () -> i32 0)");
    let last = module.functions[0].blocks.len();
    module.functions[0].blocks[0].terminator = Terminator::Goto(last + 7);

    let error = optimize("test.slp", &mut module).expect_err("the target is out of range");
    assert_eq!(error[0].code, "SL0700");
    assert!(
        error[0].message.contains("out of range"),
        "unexpected message: {}",
        error[0].message
    );
}

#[test]
fn a_pipeline_that_runs_out_of_rounds_still_answers() {
    // A program can simply be large enough to keep paying off after the last
    // round. Every pass preserves observable behaviour on its own, so what the
    // bound costs is optimization; this used to be a `debug_assert!` that
    // aborted the build instead.
    let source = r#"
        (fn probe ((flag bool)) -> i64
          (let base (if flag 10 10))
          (let doubled (+ base base))
          (+ doubled 5))
        (fn main () -> i32 0)
    "#;
    let mut module = debug(source);
    super::optimize_within("test.slp", &mut module, 1).expect("one round is a valid answer");
    crate::verify::check("test.slp", &module, "a bounded pipeline")
        .expect("the MIR is still valid");
}
