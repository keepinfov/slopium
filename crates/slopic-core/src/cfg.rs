//! Control-flow and def/use analysis over MIR.
//!
//! MIR keeps numbered locals rather than SSA form (decision D-017), so the
//! analyses here derive the information an SSA form would have carried
//! implicitly: which blocks reach which, which local each instruction defines,
//! which locals it reads, and over what range of the function a local is live.
//!
//! The MIR verifier consumes all of it today. Register allocation will consume
//! [`live_intervals`] later; nothing in code generation depends on this module
//! yet.

use crate::mir::{BlockId, Instruction, LocalId, MirFunction, Terminator};

/// Blocks that control flow can transfer to from `terminator`.
pub fn successors(terminator: &Terminator) -> Vec<BlockId> {
    match terminator {
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
        Terminator::Goto(target) => vec![*target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
    }
}

/// The local an instruction writes, if any.
///
/// `Drop` and `Free` read their operand and define nothing.
pub fn defs(instruction: &Instruction) -> Option<LocalId> {
    match instruction {
        Instruction::ConstInt { dst, .. }
        | Instruction::ConstFloat { dst, .. }
        | Instruction::ConstBool { dst, .. }
        | Instruction::StringNew { dst, .. }
        | Instruction::Assign { dst, .. }
        | Instruction::AddressOf { dst, .. }
        | Instruction::Binary { dst, .. }
        | Instruction::Call { dst, .. }
        | Instruction::StructNew { dst, .. }
        | Instruction::FieldLoad { dst, .. }
        | Instruction::EnumNew { dst, .. }
        | Instruction::EnumTag { dst, .. }
        | Instruction::EnumFieldLoad { dst, .. } => Some(*dst),
        Instruction::Drop { .. } | Instruction::Free { .. } => None,
    }
}

/// Appends every local an instruction reads to `out`.
///
/// `AddressOf` counts as a read of its source: the borrow requires the value to
/// already exist.
pub fn uses(instruction: &Instruction, out: &mut Vec<LocalId>) {
    match instruction {
        Instruction::ConstInt { .. }
        | Instruction::ConstFloat { .. }
        | Instruction::ConstBool { .. }
        | Instruction::StringNew { .. } => {}
        Instruction::Assign { src, .. } | Instruction::AddressOf { src, .. } => out.push(*src),
        Instruction::Binary { lhs, rhs, .. } => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Instruction::Call { args, .. } => out.extend(args.iter().copied()),
        Instruction::StructNew { fields, .. } | Instruction::EnumNew { fields, .. } => {
            out.extend(fields.iter().copied())
        }
        Instruction::FieldLoad { base, .. }
        | Instruction::EnumTag { base, .. }
        | Instruction::EnumFieldLoad { base, .. } => out.push(*base),
        Instruction::Drop { local, .. } | Instruction::Free { local } => out.push(*local),
    }
}

/// Locals a terminator reads.
pub fn terminator_uses(terminator: &Terminator) -> Vec<LocalId> {
    match terminator {
        Terminator::Return(value) => value.iter().copied().collect(),
        Terminator::Branch { condition, .. } => vec![*condition],
        Terminator::Goto(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Precomputed control-flow information for one function.
pub struct Cfg {
    predecessors: Vec<Vec<BlockId>>,
    reverse_postorder: Vec<BlockId>,
    reachable: Vec<bool>,
}

impl Cfg {
    pub fn new(function: &MirFunction) -> Self {
        let count = function.blocks.len();
        let mut predecessors = vec![Vec::new(); count];
        for (index, block) in function.blocks.iter().enumerate() {
            for target in successors(&block.terminator) {
                if target < count {
                    predecessors[target].push(index);
                }
            }
        }

        let mut reachable = vec![false; count];
        let mut postorder = Vec::with_capacity(count);
        if function.entry < count {
            // Iterative depth-first search; recursion would risk blowing the
            // stack on deeply nested user code.
            let mut stack = vec![(function.entry, 0usize)];
            reachable[function.entry] = true;
            while let Some((block, next)) = stack.pop() {
                let targets = successors(&function.blocks[block].terminator);
                match targets.get(next) {
                    Some(&target) => {
                        stack.push((block, next + 1));
                        if target < count && !reachable[target] {
                            reachable[target] = true;
                            stack.push((target, 0));
                        }
                    }
                    None => postorder.push(block),
                }
            }
        }
        postorder.reverse();

        Self {
            predecessors,
            reverse_postorder: postorder,
            reachable,
        }
    }

    pub fn predecessors(&self, block: BlockId) -> &[BlockId] {
        self.predecessors.get(block).map_or(&[], Vec::as_slice)
    }

    /// Reachable blocks, each listed after at least one predecessor except
    /// across back edges. Forward dataflow should visit blocks in this order.
    pub fn reverse_postorder(&self) -> &[BlockId] {
        &self.reverse_postorder
    }

    pub fn is_reachable(&self, block: BlockId) -> bool {
        self.reachable.get(block).copied().unwrap_or(false)
    }
}

/// The range of a function over which one local is live.
///
/// Positions number instructions consecutively across blocks in reverse
/// postorder, so an interval is a half-open range over that single linear
/// order. This is the shape a linear-scan allocator wants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interval {
    pub local: LocalId,
    pub start: usize,
    pub end: usize,
}

/// First and last position at which each local is mentioned.
struct Ranges {
    first: Vec<usize>,
    last: Vec<usize>,
    seen: Vec<bool>,
}

impl Ranges {
    fn mark(&mut self, local: LocalId, at: usize) {
        if local >= self.seen.len() {
            return;
        }
        self.seen[local] = true;
        self.first[local] = self.first[local].min(at);
        self.last[local] = self.last[local].max(at);
    }
}

/// Live intervals for every local that appears in the reachable CFG.
///
/// Locals never mentioned by a reachable block are omitted. A local read inside
/// a loop is extended to the end of the loop body, because the linear order
/// alone cannot express a back edge.
pub fn live_intervals(function: &MirFunction, cfg: &Cfg) -> Vec<Interval> {
    let order = cfg.reverse_postorder();
    let mut position_of_block = vec![None; function.blocks.len()];
    let mut block_end = vec![0usize; function.blocks.len()];
    let mut position = 0usize;
    for &block in order {
        position_of_block[block] = Some(position);
        position += function.blocks[block].statements.len() + 1;
        block_end[block] = position;
    }

    let mut range = Ranges {
        first: vec![usize::MAX; function.locals.len()],
        last: vec![0usize; function.locals.len()],
        seen: vec![false; function.locals.len()],
    };

    let mut buffer = Vec::new();
    for &block in order {
        let mut at = position_of_block[block].expect("block is in the order");
        for instruction in function.blocks[block].instructions() {
            buffer.clear();
            uses(instruction, &mut buffer);
            for local in buffer.iter().copied() {
                range.mark(local, at);
            }
            if let Some(local) = defs(instruction) {
                range.mark(local, at);
            }
            at += 1;
        }
        for local in terminator_uses(&function.blocks[block].terminator) {
            range.mark(local, at);
        }
    }
    let Ranges {
        first,
        mut last,
        seen,
    } = range;

    // A back edge means everything live at the loop header stays live to the
    // end of the latch block. Without this a linear-scan allocator would reuse
    // a register that the next iteration still needs.
    for &block in order {
        let Some(header_start) = position_of_block[block] else {
            continue;
        };
        for &predecessor in cfg.predecessors(block) {
            let Some(latch_start) = position_of_block[predecessor] else {
                continue;
            };
            if latch_start < header_start {
                continue;
            }
            let latch_end = block_end[predecessor];
            for local in 0..first.len() {
                if seen[local] && first[local] <= header_start && last[local] >= header_start {
                    last[local] = last[local].max(latch_end);
                }
            }
        }
    }

    (0..function.locals.len())
        .filter(|local| seen[*local])
        .map(|local| Interval {
            local,
            start: first[local],
            end: last[local],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{defs, live_intervals, successors, terminator_uses, uses, Cfg};
    use crate::ast::Type;
    use crate::mir::{BasicBlock, Instruction, MirFunction, MirLocal, Terminator};
    use crate::{compile_to_mir, CompileOptions};

    fn function(blocks: Vec<BasicBlock>, locals: usize) -> MirFunction {
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

    fn block(instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock::synthetic(instructions, terminator)
    }

    #[test]
    fn successors_cover_every_terminator_kind() {
        assert_eq!(successors(&Terminator::Return(None)), Vec::<usize>::new());
        assert_eq!(successors(&Terminator::Unreachable), Vec::<usize>::new());
        assert_eq!(successors(&Terminator::Goto(3)), vec![3]);
        assert_eq!(
            successors(&Terminator::Branch {
                condition: 0,
                then_block: 1,
                else_block: 2,
            }),
            vec![1, 2]
        );
    }

    #[test]
    fn defs_and_uses_split_reads_from_writes() {
        let binary = Instruction::Binary {
            dst: 2,
            op: crate::mir::BinaryOp::Add,
            lhs: 0,
            rhs: 1,
            ty: Type::I64,
        };
        let mut read = Vec::new();
        uses(&binary, &mut read);
        assert_eq!(defs(&binary), Some(2));
        assert_eq!(read, vec![0, 1]);

        let drop = Instruction::Drop {
            local: 4,
            ty: Type::String,
        };
        read.clear();
        uses(&drop, &mut read);
        assert_eq!(defs(&drop), None, "a drop defines nothing");
        assert_eq!(read, vec![4]);

        assert_eq!(terminator_uses(&Terminator::Return(Some(7))), vec![7]);
    }

    #[test]
    fn predecessors_and_order_follow_an_if_else_diamond() {
        let cfg = Cfg::new(&function(
            vec![
                block(
                    Vec::new(),
                    Terminator::Branch {
                        condition: 0,
                        then_block: 1,
                        else_block: 2,
                    },
                ),
                block(Vec::new(), Terminator::Goto(3)),
                block(Vec::new(), Terminator::Goto(3)),
                block(Vec::new(), Terminator::Return(None)),
            ],
            1,
        ));

        assert_eq!(cfg.predecessors(0), &[] as &[usize]);
        assert_eq!(cfg.predecessors(3), &[1, 2]);

        let order = cfg.reverse_postorder();
        assert_eq!(order[0], 0, "the entry comes first");
        assert_eq!(order[order.len() - 1], 3, "the merge comes last");
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn unreachable_blocks_are_excluded_from_the_order() {
        let cfg = Cfg::new(&function(
            vec![
                block(Vec::new(), Terminator::Goto(2)),
                block(Vec::new(), Terminator::Return(None)),
                block(Vec::new(), Terminator::Return(None)),
            ],
            1,
        ));

        assert!(cfg.is_reachable(0) && cfg.is_reachable(2));
        assert!(!cfg.is_reachable(1), "block 1 has no predecessor");
        assert_eq!(cfg.reverse_postorder(), &[0, 2]);
    }

    #[test]
    fn a_back_edge_extends_liveness_past_the_latch() {
        // bb0 -> bb1 (header) -> bb2 (latch) -> bb1
        let counter = Instruction::ConstInt { dst: 0, value: 0 };
        let bump = Instruction::Binary {
            dst: 0,
            op: crate::mir::BinaryOp::Add,
            lhs: 0,
            rhs: 0,
            ty: Type::I64,
        };
        let mir = function(
            vec![
                block(vec![counter], Terminator::Goto(1)),
                block(
                    Vec::new(),
                    Terminator::Branch {
                        condition: 0,
                        then_block: 2,
                        else_block: 3,
                    },
                ),
                block(vec![bump], Terminator::Goto(1)),
                block(Vec::new(), Terminator::Return(Some(0))),
            ],
            1,
        );
        let cfg = Cfg::new(&mir);
        let intervals = live_intervals(&mir, &cfg);

        let counter = intervals
            .iter()
            .find(|interval| interval.local == 0)
            .expect("the counter is live");
        let latch_position = cfg
            .reverse_postorder()
            .iter()
            .position(|block| *block == 2)
            .expect("the latch is reachable");
        assert!(
            counter.end > latch_position,
            "liveness must survive the back edge, got {counter:?}"
        );
    }

    #[test]
    fn every_lowered_local_gets_an_interval() {
        let source = r#"
            (fn probe ((a i64)) -> i64
              (let b (+ a 2))
              (if (< a b) b a))
            (fn main () -> i32 0)
        "#;
        let mir = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let probe = mir
            .functions
            .iter()
            .find(|function| function.name.contains("probe"))
            .expect("probe was lowered");
        let cfg = Cfg::new(probe);
        let intervals = live_intervals(probe, &cfg);

        assert!(!intervals.is_empty());
        for interval in &intervals {
            assert!(
                interval.start <= interval.end,
                "interval is inverted: {interval:?}"
            );
        }
        assert!(
            intervals
                .iter()
                .any(|interval| interval.local == probe.params[0]),
            "the parameter must be live somewhere"
        );
    }
}
