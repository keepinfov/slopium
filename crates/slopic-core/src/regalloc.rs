//! Linear-scan register allocation over MIR live intervals.
//!
//! The allocator is target-independent: it is told how many registers exist and
//! which locals are ineligible, and it answers with a location for every local.
//! Naming those registers and honouring the answer is the backend's job.
//!
//! Allocation is whole-interval: a local either lives in one register for the
//! entire function or lives in its frame slot for the entire function. Nothing
//! is split, and no reload is inserted mid-interval. That is what keeps the
//! change to code generation a substitution of operands rather than a rewrite
//! of instruction selection, and it is why a local that loses the scan is
//! simply left in memory — the spill is the frame slot it would have had
//! anyway, so spilling can never fail or cost extra instructions.

use crate::cfg::{live_intervals, Cfg};
use crate::mir::{LocalId, MirFunction};

/// Where a local lives for the whole of its function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Location {
    /// Index into the backend's list of allocatable registers.
    Register(usize),
    /// Index of an 8-byte frame slot.
    Memory(usize),
}

pub struct Allocation {
    locations: Vec<Location>,
    used_registers: Vec<usize>,
    memory_slots: usize,
}

impl Allocation {
    /// Every local in its frame slot. The pre-v0.3.2 layout, and the fallback
    /// for code generated without a MIR function behind it.
    pub fn stack_only(locals: usize) -> Self {
        Self {
            locations: (0..locals).map(Location::Memory).collect(),
            used_registers: Vec::new(),
            memory_slots: locals,
        }
    }

    pub fn location(&self, local: LocalId) -> Location {
        self.locations
            .get(local)
            .copied()
            // Out-of-range locals are rejected by the MIR verifier. Answering
            // with a slot rather than panicking keeps a compiler bug to wrong
            // code in a debug build instead of a crash in a release one.
            .unwrap_or(Location::Memory(local))
    }

    /// Allocatable-register indices this function actually uses, ascending.
    /// The backend saves and restores exactly these.
    pub fn used_registers(&self) -> &[usize] {
        &self.used_registers
    }

    /// Number of 8-byte frame slots the locals need.
    pub fn memory_slots(&self) -> usize {
        self.memory_slots
    }
}

/// Assigns each local of `function` a register or a frame slot.
///
/// `registers` is how many allocatable registers the backend offers. `pinned`
/// marks locals the backend requires in memory — because it takes their
/// address, say — and is indexed by local id.
pub fn allocate(
    function: &MirFunction,
    cfg: &Cfg,
    registers: usize,
    pinned: &[bool],
) -> Allocation {
    let count = function.locals.len();
    let mut assigned: Vec<Option<usize>> = vec![None; count];

    let mut intervals = live_intervals(function, cfg);
    intervals.retain(|interval| !pinned.get(interval.local).copied().unwrap_or(false));
    // Ties are broken by local id so the assignment is a function of the MIR
    // alone; reproducible builds depend on it.
    intervals.sort_unstable_by_key(|interval| (interval.start, interval.end, interval.local));

    let mut busy = vec![false; registers];
    let mut active: Vec<crate::cfg::Interval> = Vec::new();

    for interval in intervals {
        // Endpoints are inclusive and an instruction reads before it writes, so
        // an interval ending exactly where the next begins could in fact share
        // its register. Expiring strictly is the conservative reading.
        active.retain(|other| {
            if other.end >= interval.start {
                return true;
            }
            if let Some(register) = assigned[other.local] {
                busy[register] = false;
            }
            false
        });

        if let Some(register) = (0..registers).find(|register| !busy[*register]) {
            busy[register] = true;
            assigned[interval.local] = Some(register);
            active.push(interval);
            continue;
        }

        // Every register is taken. The interval that ends last is the one whose
        // register is doing the least work per instruction it occupies, so it
        // yields — unless the newcomer ends even later, in which case the
        // newcomer is the cheaper thing to leave in memory.
        let victim = active
            .iter()
            .enumerate()
            .max_by_key(|(_, other)| (other.end, other.local));
        let Some((index, other)) = victim else {
            continue;
        };
        if other.end <= interval.end {
            continue;
        }
        let Some(register) = assigned[other.local] else {
            continue;
        };
        assigned[other.local] = None;
        assigned[interval.local] = Some(register);
        active.remove(index);
        active.push(interval);
    }

    let mut memory_slots = 0;
    let locations = (0..count)
        .map(|local| match assigned[local] {
            Some(register) => Location::Register(register),
            None => {
                let slot = memory_slots;
                memory_slots += 1;
                Location::Memory(slot)
            }
        })
        .collect();

    let mut used_registers: Vec<usize> = assigned.iter().flatten().copied().collect();
    used_registers.sort_unstable();
    used_registers.dedup();

    Allocation {
        locations,
        used_registers,
        memory_slots,
    }
}

#[cfg(test)]
mod tests {
    use super::{allocate, Location};
    use crate::ast::Type;
    use crate::cfg::{live_intervals, Cfg};
    use crate::mir::{BasicBlock, BinaryOp, Instruction, MirFunction, MirLocal, Terminator};
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

    /// `count` independent const/use pairs, each dead before the next begins.
    fn sequential_pairs(count: usize) -> MirFunction {
        let mut statements = Vec::new();
        for local in 0..count {
            statements.push(Instruction::ConstInt {
                dst: local,
                value: local as i64,
            });
            statements.push(Instruction::Binary {
                dst: count,
                op: BinaryOp::Add,
                lhs: local,
                rhs: local,
                ty: Type::I64,
            });
        }
        function(
            vec![BasicBlock::synthetic(
                statements,
                Terminator::Return(Some(count)),
            )],
            count + 1,
        )
    }

    #[test]
    fn disjoint_intervals_reuse_one_register() {
        let mir = sequential_pairs(8);
        let cfg = Cfg::new(&mir);
        let allocation = allocate(&mir, &cfg, 4, &vec![false; mir.locals.len()]);

        assert!(
            allocation.memory_slots() == 0,
            "eight short-lived locals fit in four registers, got {} slots",
            allocation.memory_slots()
        );
        assert!(allocation.used_registers().len() <= 4);
    }

    #[test]
    fn overlapping_intervals_never_share_a_register() {
        let source = r#"
            (fn probe ((a i64) (b i64) (c i64) (d i64) (e i64) (f i64)) -> i64
              (let g (+ a b))
              (let h (+ c d))
              (let i (+ e f))
              (+ (+ g h) (+ i (+ a b))))
            (fn main () -> i32 0)
        "#;
        let module = compile_to_mir("test.slp", source, &CompileOptions::default()).unwrap();
        let probe = module
            .functions
            .iter()
            .find(|function| function.name.contains("probe"))
            .expect("probe was lowered");
        let cfg = Cfg::new(probe);
        let allocation = allocate(probe, &cfg, 5, &vec![false; probe.locals.len()]);

        for left in live_intervals(probe, &cfg) {
            for right in live_intervals(probe, &cfg) {
                if left.local >= right.local {
                    continue;
                }
                if left.end < right.start || right.end < left.start {
                    continue;
                }
                let (Location::Register(one), Location::Register(other)) = (
                    allocation.location(left.local),
                    allocation.location(right.local),
                ) else {
                    continue;
                };
                assert_ne!(
                    one, other,
                    "overlapping {left:?} and {right:?} share a register"
                );
            }
        }
    }

    #[test]
    fn more_live_values_than_registers_spill_to_memory() {
        // Every local is live from its definition to the final sum, so no two
        // can share; with one register the rest must land in slots.
        let mut statements = Vec::new();
        for local in 0..6 {
            statements.push(Instruction::ConstInt {
                dst: local,
                value: 1,
            });
        }
        for local in 1..6 {
            statements.push(Instruction::Binary {
                dst: 6,
                op: BinaryOp::Add,
                lhs: 0,
                rhs: local,
                ty: Type::I64,
            });
        }
        let mir = function(
            vec![BasicBlock::synthetic(
                statements,
                Terminator::Return(Some(6)),
            )],
            7,
        );
        let cfg = Cfg::new(&mir);
        let allocation = allocate(&mir, &cfg, 1, &vec![false; mir.locals.len()]);

        assert_eq!(allocation.used_registers(), &[0]);
        assert!(
            allocation.memory_slots() >= 5,
            "only one value can hold the register, got {} slots",
            allocation.memory_slots()
        );
    }

    #[test]
    fn a_pinned_local_stays_in_memory() {
        let mir = sequential_pairs(3);
        let cfg = Cfg::new(&mir);
        let mut pinned = vec![false; mir.locals.len()];
        pinned[1] = true;
        let allocation = allocate(&mir, &cfg, 4, &pinned);

        assert!(matches!(allocation.location(1), Location::Memory(_)));
        assert!(matches!(allocation.location(0), Location::Register(_)));
    }

    #[test]
    fn frame_slots_are_numbered_without_gaps() {
        let mir = sequential_pairs(6);
        let cfg = Cfg::new(&mir);
        let allocation = allocate(&mir, &cfg, 2, &vec![false; mir.locals.len()]);

        let mut slots: Vec<usize> = (0..mir.locals.len())
            .filter_map(|local| match allocation.location(local) {
                Location::Memory(slot) => Some(slot),
                Location::Register(_) => None,
            })
            .collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..allocation.memory_slots()).collect::<Vec<_>>());
    }

    #[test]
    fn allocation_is_a_function_of_the_mir_alone() {
        let mir = sequential_pairs(9);
        let cfg = Cfg::new(&mir);
        let pinned = vec![false; mir.locals.len()];
        let first = allocate(&mir, &cfg, 3, &pinned);
        for _ in 0..8 {
            let again = allocate(&mir, &cfg, 3, &pinned);
            for local in 0..mir.locals.len() {
                assert_eq!(first.location(local), again.location(local));
            }
        }
    }

    #[test]
    fn no_registers_available_leaves_everything_in_memory() {
        let mir = sequential_pairs(4);
        let cfg = Cfg::new(&mir);
        let allocation = allocate(&mir, &cfg, 0, &vec![false; mir.locals.len()]);

        assert_eq!(allocation.memory_slots(), mir.locals.len());
        assert!(allocation.used_registers().is_empty());
    }
}
