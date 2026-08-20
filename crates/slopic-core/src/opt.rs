//! Release-profile MIR optimization passes.
//!
//! Unlike the v0.3.0 foundation, these passes deliberately change generated
//! code. Correctness rests on three things: every pass preserves observable
//! behaviour, the MIR verifier runs after each one, and the fixture suite
//! compares program output byte for byte.
//!
//! Two rules constrain every pass here.
//!
//! **Drops are observable.** `Drop` and `Free` are how memory gets released;
//! they are never dead, and moving one across a branch changes when memory is
//! freed. Passes may delete a block only when it is unreachable, and may merge
//! blocks only when that preserves the exact statement order.
//!
//! **Arithmetic can trap.** `Binary` panics on overflow and on division by
//! zero, with a normalized message and exit status 101. That is observable
//! behaviour, so a `Binary` is never removed as dead and never folded unless
//! the fold is known not to trap — [`fold_binary`] returns `None` rather than
//! wrapping.

use crate::cfg::{defs, successors, terminator_uses, uses, Cfg};
use crate::diagnostic::CompileResult;
use crate::mir::{
    BasicBlock, BinaryOp, BlockId, Instruction, LocalId, MirFunction, MirModule, Statement,
    Terminator,
};
use crate::verify;

mod inline;

/// Upper bound on pipeline iterations.
///
/// Each pass is monotone in practice — it removes work — but a bound keeps a
/// mistake in one of them from hanging the compiler instead of failing a test.
const MAX_ROUNDS: usize = 8;

/// Runs the release optimization pipeline.
///
/// Returns the diagnostics from MIR verification if a pass produced invalid
/// MIR, naming the pass responsible.
pub fn optimize(file: &str, module: &mut MirModule) -> CompileResult<()> {
    // Inlining runs first and once: it enlarges function bodies, and the
    // per-function passes are what clean up the result.
    inline::run(module);
    check_after(file, module, "inlining")?;

    // The baseline is taken *after* inlining, deliberately: inlining a callee
    // at two call sites duplicates the volatile accesses in it, and both calls
    // really do reach the device (`D-114`).
    let mut volatile = volatile_counts(module);

    for round in 0..MAX_ROUNDS {
        let mut changed = false;

        for function in functions_mut(module) {
            changed |= propagate_constants(function);
        }
        check_after(file, module, "constant propagation")?;
        check_volatile(&volatile, module, "constant propagation", Bound::Exact);

        for function in functions_mut(module) {
            changed |= simplify_cfg(function);
        }
        check_after(file, module, "CFG simplification")?;
        // Only this pass may legitimately lose one: `remove_unreachable_blocks`
        // deletes a block that a folded branch made unreachable, and a volatile
        // access in it was never going to happen.
        check_volatile(&volatile, module, "CFG simplification", Bound::AtMost);
        volatile = volatile_counts(module);

        for function in functions_mut(module) {
            changed |= eliminate_dead_code(function);
        }
        check_after(file, module, "dead code elimination")?;
        check_volatile(&volatile, module, "dead code elimination", Bound::Exact);

        if !changed {
            break;
        }
        debug_assert!(
            round + 1 < MAX_ROUNDS,
            "optimization did not reach a fixpoint"
        );
    }
    Ok(())
}

fn check_after(file: &str, module: &MirModule, pass: &str) -> CompileResult<()> {
    verify::check(file, module, pass)
}

/// What a pass is allowed to do to a function's volatile count.
#[derive(Clone, Copy)]
enum Bound {
    /// It may not change it at all.
    Exact,
    /// It may lose one only by deleting the block it was in.
    AtMost,
}

/// Enforces that a pass neither invented nor lost a volatile access (`D-114`).
///
/// `D-067` asks for this in `verify.rs`, and it cannot go there: `verify.rs`
/// is handed one module, and "nothing was eliminated" is a statement about two.
/// So it lives beside the pass whose work it is checking, which is the only
/// place that has both numbers.
///
/// A `debug_assert!` rather than a diagnostic, matching how the rest of this
/// file states its invariants: a failure here is a compiler bug and not a
/// program's, and `verify::check` is already the channel for the shape errors a
/// release build still reports.
fn check_volatile(before: &[usize], module: &MirModule, pass: &str, bound: Bound) {
    if !cfg!(debug_assertions) {
        return;
    }
    let after = volatile_counts(module);
    debug_assert_eq!(
        before.len(),
        after.len(),
        "{pass} changed how many functions there are"
    );
    for (index, (before, after)) in before.iter().zip(&after).enumerate() {
        let ok = match bound {
            Bound::Exact => before == after,
            Bound::AtMost => after <= before,
        };
        debug_assert!(
            ok,
            "{pass} changed the volatile accesses of function {index} from {before} to {after}"
        );
    }
}

fn functions_mut(module: &mut MirModule) -> impl Iterator<Item = &mut MirFunction> {
    module
        .functions
        .iter_mut()
        .chain(module.tests.iter_mut().map(|test| &mut test.function))
}

// ---------------------------------------------------------------- constants

/// A compile-time value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Constant {
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

/// Lattice element for a local at a program point.
///
/// `Unknown` is the top element: no information yet, and the identity for the
/// meet. `Varying` is the bottom: the local holds different values on different
/// paths. Meeting two different constants yields `Varying`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    Unknown,
    Known(Constant),
    Varying,
}

impl Value {
    fn meet(self, other: Value) -> Value {
        match (self, other) {
            (Value::Unknown, value) | (value, Value::Unknown) => value,
            (Value::Known(a), Value::Known(b)) if a == b => Value::Known(a),
            (Value::Varying, _) | (_, Value::Varying) => Value::Varying,
            _ => Value::Varying,
        }
    }
}

type State = Vec<Value>;

fn meet_states(into: &mut State, from: &State) -> bool {
    let mut changed = false;
    for (slot, incoming) in into.iter_mut().zip(from.iter()) {
        let merged = slot.meet(*incoming);
        if merged != *slot {
            *slot = merged;
            changed = true;
        }
    }
    changed
}

/// Cross-block constant propagation.
///
/// A forward dataflow over the CFG. Constants flow through assignments and
/// foldable binary operations, and meet at joins: a local is constant on entry
/// to a block only if every predecessor agrees on the same value.
///
/// This replaces the intra-block folder, which reset its state at every block
/// boundary and so could not see through a branch or a loop preheader.
fn propagate_constants(function: &mut MirFunction) -> bool {
    let cfg = Cfg::new(function);
    let locals = function.locals.len();
    let blocks = function.blocks.len();

    // Parameters arrive with unknown values; everything else starts at the top
    // and is lowered by the analysis.
    let mut entry: Vec<Option<State>> = vec![None; blocks];
    let mut initial = vec![Value::Unknown; locals];
    for param in &function.params {
        if *param < locals {
            initial[*param] = Value::Varying;
        }
    }
    entry[function.entry] = Some(initial);

    let order = cfg.reverse_postorder().to_vec();
    let mut rounds = 0;
    loop {
        let mut changed = false;
        for &block in &order {
            let Some(state) = entry[block].clone() else {
                continue;
            };
            let state = transfer(function, block, state);
            for target in successors(&function.blocks[block].terminator) {
                if target >= blocks {
                    continue;
                }
                match &mut entry[target] {
                    Some(existing) => changed |= meet_states(existing, &state),
                    slot @ None => {
                        *slot = Some(state.clone());
                        changed = true;
                    }
                }
            }
        }
        rounds += 1;
        if !changed || rounds >= MAX_ROUNDS * 4 {
            break;
        }
    }

    rewrite_with_constants(function, &entry)
}

/// Runs a block's statements over `state`, returning the state at its exit.
fn transfer(function: &MirFunction, block: BlockId, mut state: State) -> State {
    for statement in &function.blocks[block].statements {
        apply(&statement.instruction, &mut state);
    }
    state
}

fn apply(instruction: &Instruction, state: &mut State) {
    let set = |state: &mut State, local: LocalId, value: Value| {
        if local < state.len() {
            state[local] = value;
        }
    };
    let get = |state: &State, local: LocalId| state.get(local).copied().unwrap_or(Value::Varying);

    match instruction {
        Instruction::ConstInt { dst, value } => {
            set(state, *dst, Value::Known(Constant::Int(*value)))
        }
        Instruction::ConstBool { dst, value } => {
            set(state, *dst, Value::Known(Constant::Bool(*value)))
        }
        Instruction::ConstFloat { dst, bits } => {
            set(state, *dst, Value::Known(Constant::Float(*bits)))
        }
        Instruction::Assign { dst, src } => {
            let value = get(state, *src);
            set(state, *dst, value)
        }
        Instruction::Binary {
            dst,
            op,
            lhs,
            rhs,
            ty,
        } => {
            let folded = match (get(state, *lhs), get(state, *rhs)) {
                (Value::Known(lhs), Value::Known(rhs)) => {
                    fold_binary(*op, ty, lhs, rhs).map_or(Value::Varying, Value::Known)
                }
                _ => Value::Varying,
            };
            set(state, *dst, folded)
        }
        // Spelled out rather than left to a catch-all, so that the next
        // instruction added to the language has to be classified here instead
        // of inheriting an answer. The one that made this worth doing is
        // `VolatileLoad`: its result is `Varying` because a device decides it,
        // and a catch-all gave the right answer only for as long as `cfg::defs`
        // reported the destination — a silent coupling between two files, whose
        // failure mode is a device register folded to a constant (`D-067`).
        Instruction::VolatileLoad { dst, .. } => set(state, *dst, Value::Varying),
        // Taking a local's address ends what this pass can say about the local
        // itself, and not only about the word the address lands in. The slot is
        // reachable from somewhere the pass does not model — a C function with
        // a `(&mut T)` out-parameter writes through it (`D-124`) — so a value
        // that was known before the borrow is not known after it. Until an
        // out-parameter existed nothing could write through a scalar borrow and
        // this was unreachable; the cross-backend suite caught it on the first
        // release build that had one.
        Instruction::AddressOf { dst, src } => {
            set(state, *src, Value::Varying);
            set(state, *dst, Value::Varying)
        }
        Instruction::StringNew { dst, .. }
        | Instruction::Call { dst, .. }
        | Instruction::FnAddr { dst, .. }
        | Instruction::CallValue { dst, .. }
        | Instruction::StructNew { dst, .. }
        | Instruction::FieldLoad { dst, .. }
        | Instruction::Load { dst, .. }
        | Instruction::FieldAddr { dst, .. }
        | Instruction::EnumNew { dst, .. }
        | Instruction::EnumTag { dst, .. }
        | Instruction::EnumFieldLoad { dst, .. }
        | Instruction::EnumFieldAddr { dst, .. } => set(state, *dst, Value::Varying),
        // Define nothing, so there is nothing to say about the state. A field
        // write defines no local either: what it changes is memory, which this
        // pass models nowhere and therefore never folds (`D-120`).
        Instruction::Drop { .. }
        | Instruction::Free { .. }
        | Instruction::FieldStore { .. }
        | Instruction::EnumFieldStore { .. }
        | Instruction::VolatileStore { .. } => {}
    }
}

/// Rewrites each block using the fixpoint entry states.
fn rewrite_with_constants(function: &mut MirFunction, entry: &[Option<State>]) -> bool {
    let mut changed = false;
    for (block, incoming) in entry.iter().enumerate() {
        let Some(mut state) = incoming.clone() else {
            continue;
        };
        let mut statements = Vec::with_capacity(function.blocks[block].statements.len());
        for statement in std::mem::take(&mut function.blocks[block].statements) {
            let span = statement.span;
            let instruction = statement.instruction;

            // A binary operation whose operands are known and whose fold does
            // not trap becomes a constant.
            let rewritten = match &instruction {
                Instruction::Binary {
                    dst,
                    op,
                    lhs,
                    rhs,
                    ty,
                } => match (value_of(&state, *lhs), value_of(&state, *rhs)) {
                    (Some(lhs), Some(rhs)) => {
                        fold_binary(*op, ty, lhs, rhs).map(|folded| folded.instruction(*dst))
                    }
                    _ => None,
                },
                _ => None,
            };
            if rewritten.is_some() {
                changed = true;
            }
            let instruction = rewritten.unwrap_or(instruction);

            apply(&instruction, &mut state);

            // `_x = _x` carries no information and no side effect.
            if let Instruction::Assign { dst, src } = &instruction {
                if dst == src {
                    changed = true;
                    continue;
                }
            }
            statements.push(Statement { instruction, span });
        }
        function.blocks[block].statements = statements;

        // A branch on a known condition is a jump.
        if let Terminator::Branch {
            condition,
            then_block,
            else_block,
        } = function.blocks[block].terminator
        {
            if let Some(Constant::Bool(taken)) = value_of(&state, condition) {
                function.blocks[block].terminator =
                    Terminator::Goto(if taken { then_block } else { else_block });
                changed = true;
            }
        }
    }
    changed
}

fn value_of(state: &State, local: LocalId) -> Option<Constant> {
    match state.get(local) {
        Some(Value::Known(constant)) => Some(*constant),
        _ => None,
    }
}

/// Folds a binary operation, or returns `None` when the result is not a
/// compile-time constant.
///
/// Returns `None` for anything that would trap at runtime — integer overflow
/// and division by zero — so the operation survives to code generation and
/// panics exactly as it would have. Folding a trap into a constant would
/// silently delete an observable failure.
pub(crate) fn fold_binary(
    op: BinaryOp,
    ty: &crate::ast::Type,
    lhs: Constant,
    rhs: Constant,
) -> Option<Constant> {
    use crate::ast::Type;

    if *ty == Type::F64 {
        let (Constant::Float(lhs), Constant::Float(rhs)) = (lhs, rhs) else {
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
            BinaryOp::LessEqual => Constant::Bool(lhs <= rhs),
            BinaryOp::GreaterEqual => Constant::Bool(lhs >= rhs),
            BinaryOp::Equal => Constant::Bool(lhs == rhs),
            // Not `!(lhs == rhs)` by accident: Rust's `!=` on `f64` is already
            // the unordered-is-not-equal answer that IEEE 754 and both
            // backends give, and writing the negation would have been a
            // different function at a NaN.
            BinaryOp::NotEqual => Constant::Bool(lhs != rhs),
            // `sema` refuses every one of these on an `f64` and `verify` says
            // so again, so reaching here means a lowering bug. Declining to
            // fold is how a bug stays visible instead of becoming a constant.
            BinaryOp::Rem
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr => return None,
        });
    }

    let integer = |value: Constant| match value {
        Constant::Int(value) => Some(value),
        Constant::Bool(value) => Some(i64::from(value)),
        Constant::Float(_) => None,
    };
    let lhs = integer(lhs)?;
    let rhs = integer(rhs)?;

    // Every regime the backends have, in the same three shapes (`D-107`): a
    // signed word, a `u64`, and a narrow type computed at 64 bits and then put
    // back into its own width.
    let kind = crate::codegen::regime(ty);

    // Shifts leave before the generic tail below, because a shift truncates
    // rather than overflowing (`D-112`) — `(shl 1 31)` on an `i32` is
    // `i32::MIN`, a value the backend produces and a range check would decline.
    // Declining is safe but wrong: it would leave the one fold a mask-writing
    // program most wants undone.
    if op.shifts() {
        let amount = u32::try_from(rhs)
            .ok()
            .filter(|amount| *amount < u32::from(kind.bits))?;
        let shifted = match (op, kind.signed) {
            (BinaryOp::Shl, _) => lhs.wrapping_shl(amount),
            (_, true) => lhs >> amount,
            (_, false) => ((lhs as u64) >> amount) as i64,
        };
        return Some(Constant::Int(kind.canonicalize(shifted)));
    }

    // A `u64` reads the same bits as a different number, so its arithmetic and
    // its comparisons are the unsigned ones. Everything narrower is held
    // zero-extended, which makes it a small non-negative word that the signed
    // operations below already answer correctly.
    if !kind.signed && kind.is_full_width() {
        let (left, right) = (lhs as u64, rhs as u64);
        let word = |value: u64| Constant::Int(value as i64);
        return match op {
            BinaryOp::Add => left.checked_add(right).map(word),
            BinaryOp::Sub => left.checked_sub(right).map(word),
            BinaryOp::Mul => left.checked_mul(right).map(word),
            BinaryOp::Div => left.checked_div(right).map(word),
            BinaryOp::Rem => left.checked_rem(right).map(word),
            BinaryOp::Less => Some(Constant::Bool(left < right)),
            BinaryOp::Greater => Some(Constant::Bool(left > right)),
            BinaryOp::LessEqual => Some(Constant::Bool(left <= right)),
            BinaryOp::GreaterEqual => Some(Constant::Bool(left >= right)),
            BinaryOp::Equal => Some(Constant::Bool(left == right)),
            BinaryOp::NotEqual => Some(Constant::Bool(left != right)),
            BinaryOp::BitAnd => Some(word(left & right)),
            BinaryOp::BitOr => Some(word(left | right)),
            BinaryOp::BitXor => Some(word(left ^ right)),
            BinaryOp::Shl | BinaryOp::Shr => unreachable!("shifts returned above"),
        };
    }

    let value = match op {
        BinaryOp::Add => lhs.checked_add(rhs).map(Constant::Int),
        BinaryOp::Sub => lhs.checked_sub(rhs).map(Constant::Int),
        BinaryOp::Mul => lhs.checked_mul(rhs).map(Constant::Int),
        BinaryOp::Div => lhs.checked_div(rhs).map(Constant::Int),
        // `checked_rem` declines on both inputs that trap — a zero divisor and
        // the most negative value over `-1` — which is the same reason
        // `checked_div` is here rather than `%`.
        BinaryOp::Rem => lhs.checked_rem(rhs).map(Constant::Int),
        BinaryOp::Less => Some(Constant::Bool(lhs < rhs)),
        BinaryOp::Greater => Some(Constant::Bool(lhs > rhs)),
        BinaryOp::LessEqual => Some(Constant::Bool(lhs <= rhs)),
        BinaryOp::GreaterEqual => Some(Constant::Bool(lhs >= rhs)),
        BinaryOp::Equal => Some(Constant::Bool(lhs == rhs)),
        BinaryOp::NotEqual => Some(Constant::Bool(lhs != rhs)),
        // Bitwise operations cannot trap and cannot leave the width they were
        // given, so they fold unconditionally.
        BinaryOp::BitAnd => Some(Constant::Int(lhs & rhs)),
        BinaryOp::BitOr => Some(Constant::Int(lhs | rhs)),
        BinaryOp::BitXor => Some(Constant::Int(lhs ^ rhs)),
        BinaryOp::Shl | BinaryOp::Shr => unreachable!("shifts returned above"),
    }?;
    // A narrow operation overflows exactly when canonicalising its result
    // changes it, which is the rule the backends emit as a compare-and-trap. A
    // fold that survives it is the value that would have been computed; one
    // that does not must not be folded, because the program is meant to trap.
    if let Constant::Int(value) = value {
        if kind.canonicalize(value) != value {
            return None;
        }
    }
    Some(value)
}

// --------------------------------------------------------------------- DCE

/// Removes instructions whose result is never read.
///
/// An instruction is removable only when it has no effect beyond defining its
/// destination. Deliberately **not** removable:
///
/// - `Drop` and `Free` — releasing memory is the effect.
/// - `Call` — arbitrary effects, including printing and process exit.
/// - `Binary` — traps on overflow and division by zero, and that trap is
///   observable. A dead `Binary` whose operands were constant has already been
///   folded to a constant by [`propagate_constants`], and *that* constant is
///   removable, so the common case still shrinks.
///
/// `Drop` counts as a use of its operand, so an owned value that is dropped is
/// never dead and its allocation is never removed out from under its release.
fn eliminate_dead_code(function: &mut MirFunction) -> bool {
    let mut changed = false;
    loop {
        let mut used = vec![false; function.locals.len()];
        let mut buffer = Vec::new();
        for block in &function.blocks {
            for instruction in block.instructions() {
                buffer.clear();
                uses(instruction, &mut buffer);
                for local in buffer.iter().copied() {
                    if local < used.len() {
                        used[local] = true;
                    }
                }
            }
            for local in terminator_uses(&block.terminator) {
                if local < used.len() {
                    used[local] = true;
                }
            }
        }
        // Parameters are defined by the ABI, not by an instruction; never
        // consider their slots removable.
        for param in &function.params {
            if *param < used.len() {
                used[*param] = true;
            }
        }

        let mut removed = false;
        for block in &mut function.blocks {
            let before = block.statements.len();
            block.statements.retain(|statement| {
                if !is_pure(&statement.instruction) {
                    return true;
                }
                match defs(&statement.instruction) {
                    Some(dst) => dst >= used.len() || used[dst],
                    None => true,
                }
            });
            removed |= block.statements.len() != before;
        }
        if !removed {
            return changed;
        }
        changed = true;
    }
}

/// Whether an instruction's only effect is to define its destination.
fn is_pure(instruction: &Instruction) -> bool {
    match instruction {
        Instruction::ConstInt { .. }
        | Instruction::ConstFloat { .. }
        | Instruction::ConstBool { .. }
        | Instruction::Assign { .. }
        | Instruction::AddressOf { .. }
        | Instruction::FieldLoad { .. }
        | Instruction::FieldAddr { .. }
        | Instruction::EnumTag { .. }
        | Instruction::EnumFieldLoad { .. }
        | Instruction::EnumFieldAddr { .. }
        // Reading through a borrow observes memory and changes none of it. The
        // borrow checker is what says the memory is live, so an unused load is
        // as dead as an unused field read.
        | Instruction::Load { .. }
        // Taking a function's address reads nothing and writes nothing; an
        // unused one is as dead as an unused constant.
        | Instruction::FnAddr { .. } => true,
        // Allocating instructions are pure in the sense that matters here: if
        // nothing reads the result and nothing drops it, the allocation was
        // leaked, and removing it is a fix rather than a behaviour change.
        Instruction::StringNew { .. }
        | Instruction::StructNew { .. }
        | Instruction::EnumNew { .. } => true,
        Instruction::Binary { .. }
        | Instruction::Call { .. }
        | Instruction::CallValue { .. }
        | Instruction::Drop { .. }
        // A volatile access is the one memory operation whose *happening* is
        // the point. A read with an unused result is how a device register is
        // cleared, and a write is observable by definition, so neither is ever
        // dead (`D-067`). This one answer is what stops elimination; nothing
        // else in this file has to know about them.
        | Instruction::VolatileLoad { .. }
        | Instruction::VolatileStore { .. }
        // A field write's whole effect is on memory, which nothing here models,
        // so an eliminator that judged it by its destination would judge it by
        // something it does not have (`D-120`).
        | Instruction::FieldStore { .. }
        | Instruction::EnumFieldStore { .. }
        | Instruction::Free { .. } => false,
    }
}

/// How many volatile accesses a function performs.
///
/// The measure `optimize` compares across each pass to enforce the half of
/// `D-067` that is a property of a *pair* of modules rather than of one, and
/// so cannot live in `verify.rs` at all (`D-114`).
fn volatile_count(function: &MirFunction) -> usize {
    function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
        .filter(|instruction| {
            matches!(
                instruction,
                Instruction::VolatileLoad { .. } | Instruction::VolatileStore { .. }
            )
        })
        .count()
}

/// The volatile counts of every function, in iteration order.
///
/// Empty in a release build, where the comparison that consumes it is compiled
/// out — the same shape `verify::check` already has, and for the same reason:
/// this is the compiler checking itself, not the program.
fn volatile_counts(module: &MirModule) -> Vec<usize> {
    if !cfg!(debug_assertions) {
        return Vec::new();
    }
    functions(module).map(volatile_count).collect()
}

fn functions(module: &MirModule) -> impl Iterator<Item = &MirFunction> {
    module
        .functions
        .iter()
        .chain(module.tests.iter().map(|test| &test.function))
}

// -------------------------------------------------------------------- CFG

/// Simplifies control flow without moving any statement across a branch.
///
/// Three transformations, in order:
///
/// 1. thread a `Goto` that targets a statement-free block straight to that
///    block's own target;
/// 2. merge a block into its unique predecessor when that predecessor's only
///    successor is this block, concatenating statements in order;
/// 3. drop unreachable blocks and renumber what remains.
fn simplify_cfg(function: &mut MirFunction) -> bool {
    let mut changed = thread_gotos(function);
    changed |= merge_linear_chains(function);
    changed |= remove_unreachable_blocks(function);
    changed
}

/// Follows `Goto` edges through blocks that contain no statements.
fn thread_gotos(function: &mut MirFunction) -> bool {
    let resolve = |mut target: BlockId| {
        // Bounded by the block count so a `goto` cycle cannot spin forever.
        for _ in 0..function.blocks.len() {
            let block = &function.blocks[target];
            match block.terminator {
                Terminator::Goto(next) if block.statements.is_empty() && next != target => {
                    target = next
                }
                _ => break,
            }
        }
        target
    };

    let mut rewritten = Vec::with_capacity(function.blocks.len());
    let mut changed = false;
    for block in &function.blocks {
        let terminator = match &block.terminator {
            Terminator::Goto(target) => {
                let resolved = resolve(*target);
                if resolved != *target {
                    changed = true;
                }
                Terminator::Goto(resolved)
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let then_resolved = resolve(*then_block);
                let else_resolved = resolve(*else_block);
                if then_resolved != *then_block || else_resolved != *else_block {
                    changed = true;
                }
                Terminator::Branch {
                    condition: *condition,
                    then_block: then_resolved,
                    else_block: else_resolved,
                }
            }
            other => other.clone(),
        };
        rewritten.push(terminator);
    }
    for (block, terminator) in function.blocks.iter_mut().zip(rewritten) {
        block.terminator = terminator;
    }
    changed
}

/// Merges `a -> b` when `a` ends in `Goto(b)` and `b` has no other predecessor.
///
/// Statements are concatenated in order, so drop placement is preserved
/// exactly; only the block boundary disappears.
fn merge_linear_chains(function: &mut MirFunction) -> bool {
    let mut changed = false;
    loop {
        let cfg = Cfg::new(function);
        let mut merge: Option<(BlockId, BlockId)> = None;
        for block in 0..function.blocks.len() {
            if !cfg.is_reachable(block) {
                continue;
            }
            let Terminator::Goto(target) = function.blocks[block].terminator else {
                continue;
            };
            if target == block || target == function.entry {
                continue;
            }
            if cfg.predecessors(target) == [block] {
                merge = Some((block, target));
                break;
            }
        }
        let Some((block, target)) = merge else {
            return changed;
        };

        let moved = std::mem::take(&mut function.blocks[target].statements);
        let terminator = function.blocks[target].terminator.clone();
        let terminator_span = function.blocks[target].terminator_span;
        function.blocks[block].statements.extend(moved);
        function.blocks[block].terminator = terminator;
        function.blocks[block].terminator_span = terminator_span;
        // The emptied block becomes unreachable; the next pass removes it.
        function.blocks[target].terminator = Terminator::Unreachable;
        changed = true;
    }
}

/// Deletes blocks not reachable from the entry and renumbers the rest.
fn remove_unreachable_blocks(function: &mut MirFunction) -> bool {
    let cfg = Cfg::new(function);
    if (0..function.blocks.len()).all(|block| cfg.is_reachable(block)) {
        return false;
    }

    let mut mapping = vec![None; function.blocks.len()];
    let mut next = 0;
    for (block, slot) in mapping.iter_mut().enumerate() {
        if cfg.is_reachable(block) {
            *slot = Some(next);
            next += 1;
        }
    }

    let mut kept = Vec::with_capacity(next);
    for (block, slot) in mapping.iter().enumerate() {
        if slot.is_none() {
            continue;
        }
        let mut basic = std::mem::replace(
            &mut function.blocks[block],
            BasicBlock::synthetic(Vec::new(), Terminator::Unreachable),
        );
        basic.terminator = remap(&basic.terminator, &mapping);
        kept.push(basic);
    }
    function.entry = mapping[function.entry].expect("the entry is reachable from itself");
    function.blocks = kept;
    true
}

fn remap(terminator: &Terminator, mapping: &[Option<BlockId>]) -> Terminator {
    let to = |block: BlockId| mapping[block].expect("a reachable block targets reachable blocks");
    match terminator {
        Terminator::Goto(target) => Terminator::Goto(to(*target)),
        Terminator::Branch {
            condition,
            then_block,
            else_block,
        } => Terminator::Branch {
            condition: *condition,
            then_block: to(*then_block),
            else_block: to(*else_block),
        },
        other => other.clone(),
    }
}

/// Counts of what a module contains, for measuring a pipeline run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub functions: usize,
    pub blocks: usize,
    pub statements: usize,
}

pub fn stats(module: &MirModule) -> Stats {
    let mut stats = Stats::default();
    for function in module
        .functions
        .iter()
        .chain(module.tests.iter().map(|test| &test.function))
    {
        stats.functions += 1;
        stats.blocks += function.blocks.len();
        stats.statements += function
            .blocks
            .iter()
            .map(|block| block.statements.len())
            .sum::<usize>();
    }
    stats
}

#[cfg(test)]
mod tests;
