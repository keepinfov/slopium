//! Control-flow and def/use analysis over MIR.
//!
//! MIR keeps numbered locals rather than SSA form (decision D-017), so the
//! analyses here derive the information an SSA form would have carried
//! implicitly: which blocks reach which, which local each instruction defines,
//! which locals it reads, and over what range of the function a local is live.
//!
//! The MIR verifier and the optimizer consume the control-flow half; register
//! allocation consumes [`liveness`] and [`live_intervals`].

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

/// Per-block live-in and live-out sets.
///
/// A local is live at a point when some path from there reads it before writing
/// it. Register allocation needs this rather than a first-mention to
/// last-mention hull, because a local written in a loop body and read back at
/// the loop header is live across the back edge — that is, at positions lying
/// between its two mentions in any linear block order.
pub struct Liveness {
    live_in: Vec<Vec<bool>>,
    live_out: Vec<Vec<bool>>,
}

impl Liveness {
    pub fn is_live_in(&self, block: BlockId, local: LocalId) -> bool {
        Self::lookup(&self.live_in, block, local)
    }

    pub fn is_live_out(&self, block: BlockId, local: LocalId) -> bool {
        Self::lookup(&self.live_out, block, local)
    }

    fn lookup(sets: &[Vec<bool>], block: BlockId, local: LocalId) -> bool {
        sets.get(block)
            .and_then(|set| set.get(local))
            .copied()
            .unwrap_or(false)
    }
}

/// Backward liveness dataflow, iterated to a fixpoint.
pub fn liveness(function: &MirFunction, cfg: &Cfg) -> Liveness {
    let blocks = function.blocks.len();
    let locals = function.locals.len();
    let mut live_in = vec![vec![false; locals]; blocks];
    let mut live_out = vec![vec![false; locals]; blocks];

    // Reverse postorder reversed carries a use back to its definition in a
    // single sweep for acyclic code; only back edges force further rounds.
    let mut order = cfg.reverse_postorder().to_vec();
    order.reverse();

    let mut buffer = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        for &block in &order {
            let mut out = vec![false; locals];
            for successor in successors(&function.blocks[block].terminator) {
                let Some(entry) = live_in.get(successor) else {
                    continue;
                };
                for (local, live) in entry.iter().enumerate() {
                    out[local] |= *live;
                }
            }

            let mut inside = out.clone();
            for local in terminator_uses(&function.blocks[block].terminator) {
                set_live(&mut inside, local, true);
            }
            for statement in function.blocks[block].statements.iter().rev() {
                // The write is cleared before the reads are added back, so
                // `x = x + 1` still counts as reading `x`.
                if let Some(local) = defs(&statement.instruction) {
                    set_live(&mut inside, local, false);
                }
                buffer.clear();
                uses(&statement.instruction, &mut buffer);
                for local in buffer.iter().copied() {
                    set_live(&mut inside, local, true);
                }
            }

            if inside != live_in[block] || out != live_out[block] {
                live_in[block] = inside;
                live_out[block] = out;
                changed = true;
            }
        }
    }

    Liveness { live_in, live_out }
}

fn set_live(set: &mut [bool], local: LocalId, live: bool) {
    if let Some(entry) = set.get_mut(local) {
        *entry = live;
    }
}

/// The range of a function over which one local is live.
///
/// Positions number statements consecutively across blocks in reverse
/// postorder, with one further position per block for its terminator, so an
/// interval is a closed range over that single linear order. This is the shape
/// a linear-scan allocator wants.
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

/// Live intervals for every local live somewhere in the reachable CFG.
///
/// Each interval is the hull, in linear order, of every position where its
/// local is live. Locals never live in a reachable block are omitted.
///
/// Two locals whose intervals do not overlap are never simultaneously live and
/// may share a register. The converse does not hold: a hull also spans blocks
/// that merely sit between the local's live blocks in linear order. That costs
/// allocation quality, never correctness.
pub fn live_intervals(function: &MirFunction, cfg: &Cfg) -> Vec<Interval> {
    let live = liveness(function, cfg);
    let mut range = Ranges {
        first: vec![usize::MAX; function.locals.len()],
        last: vec![0usize; function.locals.len()],
        seen: vec![false; function.locals.len()],
    };

    let mut buffer = Vec::new();
    let mut at = 0usize;
    for &block in cfg.reverse_postorder() {
        let start = at;
        let terminator = start + function.blocks[block].statements.len();
        for local in 0..function.locals.len() {
            if live.is_live_in(block, local) {
                range.mark(local, start);
            }
            if live.is_live_out(block, local) {
                range.mark(local, terminator);
            }
        }
        for statement in &function.blocks[block].statements {
            buffer.clear();
            uses(&statement.instruction, &mut buffer);
            for local in buffer.iter().copied() {
                range.mark(local, at);
            }
            if let Some(local) = defs(&statement.instruction) {
                range.mark(local, at);
            }
            at += 1;
        }
        for local in terminator_uses(&function.blocks[block].terminator) {
            range.mark(local, terminator);
        }
        at = terminator + 1;
    }

    let Ranges { first, last, seen } = range;
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
    use super::{defs, live_intervals, liveness, successors, terminator_uses, uses, Cfg};
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

    fn add(dst: usize, lhs: usize, rhs: usize) -> Instruction {
        Instruction::Binary {
            dst,
            op: crate::mir::BinaryOp::Add,
            lhs,
            rhs,
            ty: Type::I64,
        }
    }

    /// A loop whose carried value is read in the header before the latch
    /// rewrites it, so the two mentions bracket only part of the loop.
    ///
    /// ```text
    /// bb0: c = 1                      bb1: y = c + c; t = x + y; branch t
    /// bb2: x = 7; z = 1; w = z + z    bb3: return t
    /// ```
    ///
    /// `x` must stay live through the whole latch. A first-mention to
    /// last-mention hull ends it at its definition in `bb2`, which would let a
    /// linear-scan allocator hand `x`'s register to `z` while the next
    /// iteration still needs it.
    fn loop_carried_value() -> MirFunction {
        let (c, y, t, x, z, w) = (0, 1, 2, 3, 4, 5);
        function(
            vec![
                block(
                    vec![Instruction::ConstInt { dst: c, value: 1 }],
                    Terminator::Goto(1),
                ),
                block(
                    vec![add(y, c, c), add(t, x, y)],
                    Terminator::Branch {
                        condition: t,
                        then_block: 2,
                        else_block: 3,
                    },
                ),
                block(
                    vec![
                        Instruction::ConstInt { dst: x, value: 7 },
                        Instruction::ConstInt { dst: z, value: 1 },
                        add(w, z, z),
                    ],
                    Terminator::Goto(1),
                ),
                block(Vec::new(), Terminator::Return(Some(t))),
            ],
            6,
        )
    }

    #[test]
    fn a_value_carried_across_a_back_edge_is_live_through_the_latch() {
        let mir = loop_carried_value();
        let cfg = Cfg::new(&mir);
        let live = liveness(&mir, &cfg);

        assert!(live.is_live_in(1, 3), "the header reads `x`");
        assert!(
            live.is_live_out(2, 3),
            "the latch hands `x` back to the header"
        );
        assert!(!live.is_live_out(2, 4), "`z` dies inside the latch");
    }

    #[test]
    fn a_loop_carried_interval_overlaps_the_latch_temporaries() {
        let mir = loop_carried_value();
        let cfg = Cfg::new(&mir);
        let intervals = live_intervals(&mir, &cfg);
        let range = |local| {
            *intervals
                .iter()
                .find(|interval| interval.local == local)
                .unwrap_or_else(|| panic!("local {local} is live"))
        };

        let carried = range(3);
        let temporary = range(4);
        assert!(
            carried.start <= temporary.start && carried.end >= temporary.end,
            "`x` must span the latch temporary, got {carried:?} against {temporary:?}"
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
