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

    for round in 0..MAX_ROUNDS {
        let mut changed = false;

        for function in functions_mut(module) {
            changed |= propagate_constants(function);
        }
        check_after(file, module, "constant propagation")?;

        for function in functions_mut(module) {
            changed |= simplify_cfg(function);
        }
        check_after(file, module, "CFG simplification")?;

        for function in functions_mut(module) {
            changed |= eliminate_dead_code(function);
        }
        check_after(file, module, "dead code elimination")?;

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
        other => {
            if let Some(dst) = defs(other) {
                set(state, dst, Value::Varying)
            }
        }
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
            BinaryOp::Equal => Constant::Bool(lhs == rhs),
        });
    }

    let integer = |value: Constant| match value {
        Constant::Int(value) => Some(value),
        Constant::Bool(value) => Some(i64::from(value)),
        Constant::Float(_) => None,
    };
    let lhs = integer(lhs)?;
    let rhs = integer(rhs)?;
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
            return i32::try_from(value)
                .ok()
                .map(|value| Constant::Int(i64::from(value)));
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
        | Instruction::EnumTag { .. }
        | Instruction::EnumFieldLoad { .. }
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
        | Instruction::Free { .. } => false,
    }
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
