//! Bounded inlining of small, non-recursive functions.
//!
//! # Why this only inlines within one module
//!
//! `slopium` emits one object per owner module and caches it on a key made of
//! that module's body plus every module's *interface*. A body-only change to
//! module A therefore does not rebuild module B's object.
//!
//! Inlining A's body into B would break that: B's object would contain a copy
//! of A's code, so a body-only edit to A would leave B stale, and the build
//! system would not know. Rather than widen every cache key, this pass refuses
//! to cross a module boundary — a caller and callee must share an owner module,
//! derived from the name prefix before the final `:`.
//!
//! Runtime builtins (`println`, `push`, …) have no `MirFunction` at all; they
//! are lowered directly by the backend and are skipped here by lookup failure.
//!
//! An `extern` is body-less for a stronger reason: its body is in another
//! language, in another object, and this compiler will never see it. Lookup
//! failure would skip it too, but that is an accident of where the bodies come
//! from, and `D-073` makes it a rule instead — a declared extern is refused by
//! name, so nothing a later change does to `collect_candidates` can start
//! inlining across the C boundary.

use crate::mir::{
    BasicBlock, BlockId, Instruction, LocalId, MirFunction, MirLocal, MirModule, Statement,
    Terminator,
};
use std::collections::{HashMap, HashSet};

/// Largest callee body worth inlining, in statements.
const MAX_CALLEE_STATEMENTS: usize = 12;

/// Largest callee body worth inlining, in blocks. Keeping this small avoids
/// duplicating loops.
const MAX_CALLEE_BLOCKS: usize = 3;

/// Ceiling on how much a single caller may grow, in statements.
const MAX_CALLER_GROWTH: usize = 96;

pub(crate) fn run(module: &mut MirModule) -> bool {
    let candidates = collect_candidates(module);
    if candidates.is_empty() {
        return false;
    }

    let opaque: HashSet<String> = module
        .externs
        .iter()
        .map(|declaration| declaration.name.clone())
        .collect();

    let mut changed = false;
    for index in 0..module.functions.len() {
        let mut function = std::mem::replace(&mut module.functions[index], placeholder());
        changed |= inline_into(&mut function, &candidates, &opaque);
        module.functions[index] = function;
    }
    for index in 0..module.tests.len() {
        let mut function = std::mem::replace(&mut module.tests[index].function, placeholder());
        changed |= inline_into(&mut function, &candidates, &opaque);
        module.tests[index].function = function;
    }
    changed
}

fn placeholder() -> MirFunction {
    MirFunction {
        name: String::new(),
        emit: false,
        params: Vec::new(),
        return_type: crate::ast::Type::Unit,
        locals: Vec::new(),
        blocks: vec![BasicBlock::synthetic(Vec::new(), Terminator::Unreachable)],
        entry: 0,
        span: Default::default(),
    }
}

/// The owner module of a symbol: everything before the final `:`.
fn owner(name: &str) -> &str {
    match name.rfind(':') {
        Some(index) => &name[..index],
        None => "",
    }
}

/// Functions small enough and safe enough to inline, keyed by name.
fn collect_candidates(module: &MirModule) -> HashMap<String, MirFunction> {
    let recursive = recursive_functions(module);
    module
        .functions
        .iter()
        .filter(|function| {
            !recursive.contains(&function.name)
                && function.blocks.len() <= MAX_CALLEE_BLOCKS
                && body_size(function) <= MAX_CALLEE_STATEMENTS
                // A callee whose body can fall off the end has no single
                // return value to bind; leave those alone.
                && function
                    .blocks
                    .iter()
                    .all(|block| !matches!(block.terminator, Terminator::Unreachable))
        })
        .map(|function| (function.name.clone(), function.clone()))
        .collect()
}

fn body_size(function: &MirFunction) -> usize {
    function
        .blocks
        .iter()
        .map(|block| block.statements.len())
        .sum()
}

/// Names involved in any call cycle, found by walking the call graph.
///
/// Conservative: a function that reaches itself through any chain of calls is
/// excluded, so mutual recursion is covered as well as direct.
fn recursive_functions(module: &MirModule) -> HashSet<String> {
    let mut callees: HashMap<&str, Vec<&str>> = HashMap::new();
    for function in &module.functions {
        let mut targets = Vec::new();
        for block in &function.blocks {
            for instruction in block.instructions() {
                if let Instruction::Call { callee, .. } = instruction {
                    targets.push(callee.as_str());
                }
            }
        }
        callees.insert(function.name.as_str(), targets);
    }

    let mut recursive = HashSet::new();
    for start in callees.keys().copied() {
        // Iterative reachability; a deep call graph must not overflow the
        // stack any more than deep source nesting may.
        let mut seen = HashSet::new();
        let mut stack = callees.get(start).cloned().unwrap_or_default();
        while let Some(next) = stack.pop() {
            if next == start {
                recursive.insert(start.to_owned());
                break;
            }
            if !seen.insert(next) {
                continue;
            }
            if let Some(more) = callees.get(next) {
                stack.extend(more.iter().copied());
            }
        }
    }
    recursive
}

fn inline_into(
    caller: &mut MirFunction,
    candidates: &HashMap<String, MirFunction>,
    opaque: &HashSet<String>,
) -> bool {
    let mut changed = false;
    let mut growth = 0usize;

    // One call site per sweep: splicing renumbers nothing but does append
    // blocks, so re-scanning keeps the logic simple and obviously correct.
    loop {
        let Some((block, position, callee_name)) = find_call_site(caller, candidates, opaque)
        else {
            return changed;
        };
        let callee = &candidates[&callee_name];
        if growth + body_size(callee) > MAX_CALLER_GROWTH {
            return changed;
        }
        growth += body_size(callee);
        splice(caller, block, position, callee);
        changed = true;
    }
}

fn find_call_site(
    caller: &MirFunction,
    candidates: &HashMap<String, MirFunction>,
    opaque: &HashSet<String>,
) -> Option<(BlockId, usize, String)> {
    for (block, basic) in caller.blocks.iter().enumerate() {
        for (position, statement) in basic.statements.iter().enumerate() {
            let Instruction::Call { callee, args, .. } = &statement.instruction else {
                continue;
            };
            if callee == &caller.name {
                continue;
            }
            // The body is in C. There is nothing here to splice, and saying so
            // by name does not depend on `candidates` staying body-only.
            if opaque.contains(callee) {
                continue;
            }
            if owner(callee) != owner(&caller.name) {
                continue;
            }
            let Some(target) = candidates.get(callee) else {
                continue;
            };
            if target.params.len() != args.len() {
                continue;
            }
            return Some((block, position, callee.clone()));
        }
    }
    None
}

/// Replaces one call with the callee's body.
///
/// The caller block is split at the call: statements before it stay, the
/// callee's blocks are appended with their locals and blocks renumbered, and
/// the statements after the call move to a fresh continuation block that every
/// callee `return` jumps to.
fn splice(caller: &mut MirFunction, block: BlockId, position: usize, callee: &MirFunction) {
    let Instruction::Call { dst, args, .. } = caller.blocks[block].statements[position]
        .instruction
        .clone()
    else {
        unreachable!("the call site was located by matching on Call");
    };
    let span = caller.blocks[block].statements[position].span;

    // Locals: append a fresh copy of every callee local.
    let local_base = caller.locals.len();
    for local in &callee.locals {
        caller.locals.push(MirLocal {
            ty: local.ty.clone(),
            name: None,
            // Callee parameters become ordinary locals of the caller.
            is_param: false,
        });
    }
    let map_local = |local: LocalId| local + local_base;

    // Blocks: the callee's entry lands here, and everything after the call
    // goes to the continuation.
    let block_base = caller.blocks.len();
    let continuation = block_base + callee.blocks.len();
    let map_block = |target: BlockId| target + block_base;

    for basic in &callee.blocks {
        let statements = basic
            .statements
            .iter()
            .map(|statement| Statement {
                instruction: remap_instruction(&statement.instruction, &map_local),
                span: statement.span,
            })
            .collect::<Vec<_>>();
        let terminator = match &basic.terminator {
            // A callee return binds the result and continues in the caller.
            Terminator::Return(value) => {
                let mut statements = statements.clone();
                if let Some(value) = value {
                    statements.push(Statement {
                        instruction: Instruction::Assign {
                            dst,
                            src: map_local(*value),
                        },
                        span,
                    });
                }
                caller.blocks.push(BasicBlock {
                    statements,
                    terminator: Terminator::Goto(continuation),
                    terminator_span: basic.terminator_span,
                });
                continue;
            }
            Terminator::Goto(target) => Terminator::Goto(map_block(*target)),
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => Terminator::Branch {
                condition: map_local(*condition),
                then_block: map_block(*then_block),
                else_block: map_block(*else_block),
            },
            Terminator::Unreachable => Terminator::Unreachable,
        };
        caller.blocks.push(BasicBlock {
            statements,
            terminator,
            terminator_span: basic.terminator_span,
        });
    }

    // The continuation carries everything that followed the call.
    let tail = caller.blocks[block].statements.split_off(position + 1);
    let tail_terminator = caller.blocks[block].terminator.clone();
    let tail_span = caller.blocks[block].terminator_span;
    caller.blocks.push(BasicBlock {
        statements: tail,
        terminator: tail_terminator,
        terminator_span: tail_span,
    });
    debug_assert_eq!(caller.blocks.len(), continuation + 1);

    // Replace the call itself with argument binding, then jump into the body.
    caller.blocks[block].statements.pop();
    for (param, argument) in callee.params.iter().zip(args.iter()) {
        caller.blocks[block].statements.push(Statement {
            instruction: Instruction::Assign {
                dst: map_local(*param),
                src: *argument,
            },
            span,
        });
    }
    caller.blocks[block].terminator = Terminator::Goto(block_base + callee.entry);
    caller.blocks[block].terminator_span = span;
}

fn remap_instruction(instruction: &Instruction, map: &impl Fn(LocalId) -> LocalId) -> Instruction {
    let m = map;
    match instruction {
        Instruction::ConstInt { dst, value } => Instruction::ConstInt {
            dst: m(*dst),
            value: *value,
        },
        Instruction::ConstFloat { dst, bits } => Instruction::ConstFloat {
            dst: m(*dst),
            bits: *bits,
        },
        Instruction::ConstBool { dst, value } => Instruction::ConstBool {
            dst: m(*dst),
            value: *value,
        },
        Instruction::StringNew { dst, value } => Instruction::StringNew {
            dst: m(*dst),
            value: value.clone(),
        },
        Instruction::Assign { dst, src } => Instruction::Assign {
            dst: m(*dst),
            src: m(*src),
        },
        Instruction::AddressOf { dst, src } => Instruction::AddressOf {
            dst: m(*dst),
            src: m(*src),
        },
        Instruction::Binary {
            dst,
            op,
            lhs,
            rhs,
            ty,
        } => Instruction::Binary {
            dst: m(*dst),
            op: *op,
            lhs: m(*lhs),
            rhs: m(*rhs),
            ty: ty.clone(),
        },
        Instruction::Call {
            dst,
            callee,
            args,
            arg_types,
            result,
        } => Instruction::Call {
            dst: m(*dst),
            callee: callee.clone(),
            args: args.iter().map(|arg| m(*arg)).collect(),
            arg_types: arg_types.clone(),
            result: result.clone(),
        },
        Instruction::FnAddr { dst, symbol } => Instruction::FnAddr {
            dst: m(*dst),
            symbol: symbol.clone(),
        },
        Instruction::CallValue {
            dst,
            callee,
            args,
            arg_types,
            result,
        } => Instruction::CallValue {
            dst: m(*dst),
            callee: m(*callee),
            args: args.iter().map(|arg| m(*arg)).collect(),
            arg_types: arg_types.clone(),
            result: result.clone(),
        },
        Instruction::Drop { local, ty } => Instruction::Drop {
            local: m(*local),
            ty: ty.clone(),
        },
        Instruction::StructNew { dst, name, fields } => Instruction::StructNew {
            dst: m(*dst),
            name: name.clone(),
            fields: fields.iter().map(|field| m(*field)).collect(),
        },
        Instruction::FieldLoad { dst, base, index } => Instruction::FieldLoad {
            dst: m(*dst),
            base: m(*base),
            index: *index,
        },
        Instruction::EnumNew {
            dst,
            enum_name,
            tag,
            fields,
        } => Instruction::EnumNew {
            dst: m(*dst),
            enum_name: enum_name.clone(),
            tag: *tag,
            fields: fields.iter().map(|field| m(*field)).collect(),
        },
        Instruction::EnumTag { dst, base } => Instruction::EnumTag {
            dst: m(*dst),
            base: m(*base),
        },
        Instruction::EnumFieldLoad { dst, base, index } => Instruction::EnumFieldLoad {
            dst: m(*dst),
            base: m(*base),
            index: *index,
        },
        Instruction::Free { local } => Instruction::Free { local: m(*local) },
    }
}
