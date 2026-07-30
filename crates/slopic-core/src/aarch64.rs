//! The AArch64 Linux backend.
//!
//! Same MIR, same allocator, same lowering decisions as the x86-64 backend —
//! see [`crate::lowering`] for everything the two share. What is here is the
//! machine: AAPCS64, a frame addressed upward from `sp`, and a load/store
//! architecture, which is the difference that shapes the code.
//!
//! x86-64 lets most instructions name a frame slot directly, so that backend
//! chooses per instruction whether an operand is a register or memory. AArch64
//! cannot: every operand of every arithmetic instruction is a register. So this
//! backend reads each operand into a scratch register when the allocator left
//! it in memory, and writes results back the same way. The allocator already
//! keeps most locals in registers (`D-020`), so the loads are the exception,
//! and the uniform shape is worth more here than the last instruction would be.

use crate::aarch64_inst::{Arith, Cond, FloatOp, Inst, Reg};
use crate::asm::{Assembly, Item, Section, Target};
use crate::ast::Type;
use crate::cfg::Cfg;
use crate::codegen::{align_to, Backend, CodegenOptions, TargetSpec, AARCH64_LINUX_GNU};
use crate::diagnostic::{codes, CompileResult, Diagnostic, Span};
use crate::lowering::{
    address_taken, clone_function, drop_function, enum_clone_size, enum_clone_symbol,
    enum_drop_symbol, enum_size, function_symbol, is_pointer_like, struct_clone_symbol,
    struct_drop_symbol, struct_size, Argument, Step, Tail,
};
use crate::mir::{BasicBlock, BinaryOp, Instruction, LocalId, MirFunction, MirModule, Terminator};
use crate::regalloc::{allocate, Allocation, Location};
use std::collections::HashMap;

/// The registers one function may allocate locals to, in the order the
/// allocator hands them out.
///
/// None is a scratch register of this generator and none is an argument
/// register, so an allocated local survives both an operand load and a call
/// setup untouched.
struct RegisterFile {
    wide: &'static [Reg],
    /// The 32-bit views of `wide`, index for index, for `i32` arithmetic.
    narrow: &'static [Reg],
    /// How many leading entries are caller-saved and so need no prologue save.
    volatile: usize,
}

/// Registers for a function that calls something: all callee-saved, so a local
/// that survives a call needs no save around the call site (`D-021`).
const CALLEE_SAVED: RegisterFile = RegisterFile {
    wide: &[
        Reg("x19"),
        Reg("x20"),
        Reg("x21"),
        Reg("x22"),
        Reg("x23"),
        Reg("x24"),
        Reg("x25"),
        Reg("x26"),
        Reg("x27"),
        Reg("x28"),
    ],
    narrow: &[
        Reg("w19"),
        Reg("w20"),
        Reg("w21"),
        Reg("w22"),
        Reg("w23"),
        Reg("w24"),
        Reg("w25"),
        Reg("w26"),
        Reg("w27"),
        Reg("w28"),
    ],
    volatile: 0,
};

/// Registers for a function that calls nothing.
///
/// `x9`–`x14` are caller-saved temporaries that this generator never touches,
/// so in a leaf they are free outright. They are also not argument registers,
/// so storing one parameter cannot overwrite another that has not arrived yet.
const LEAF: RegisterFile = RegisterFile {
    wide: &[
        Reg("x9"),
        Reg("x10"),
        Reg("x11"),
        Reg("x12"),
        Reg("x13"),
        Reg("x14"),
        Reg("x19"),
        Reg("x20"),
        Reg("x21"),
        Reg("x22"),
        Reg("x23"),
        Reg("x24"),
        Reg("x25"),
        Reg("x26"),
        Reg("x27"),
        Reg("x28"),
    ],
    narrow: &[
        Reg("w9"),
        Reg("w10"),
        Reg("w11"),
        Reg("w12"),
        Reg("w13"),
        Reg("w14"),
        Reg("w19"),
        Reg("w20"),
        Reg("w21"),
        Reg("w22"),
        Reg("w23"),
        Reg("w24"),
        Reg("w25"),
        Reg("w26"),
        Reg("w27"),
        Reg("w28"),
    ],
    volatile: 6,
};

/// The registers this generator uses to hold values between two instructions of
/// the same MIR statement. Never allocated, so an operand load cannot evict a
/// local.
///
/// `x16` and `x17` are the platform's own intra-procedure scratch pair, free
/// for exactly this. `x15` is taken out of the leaf pool to give the two
/// overflow checks that need three live values somewhere to put the third.
const SCRATCH: [(Reg, Reg); 3] = [
    (Reg("x15"), Reg("w15")),
    (Reg("x16"), Reg("w16")),
    (Reg("x17"), Reg("w17")),
];

/// Argument registers of AAPCS64.
const INTEGER_ARGUMENTS: [Reg; 8] = [
    Reg("x0"),
    Reg("x1"),
    Reg("x2"),
    Reg("x3"),
    Reg("x4"),
    Reg("x5"),
    Reg("x6"),
    Reg("x7"),
];
const NARROW_ARGUMENTS: [Reg; 8] = [
    Reg("w0"),
    Reg("w1"),
    Reg("w2"),
    Reg("w3"),
    Reg("w4"),
    Reg("w5"),
    Reg("w6"),
    Reg("w7"),
];
const FLOAT_ARGUMENTS: [Reg; 8] = [
    Reg("d0"),
    Reg("d1"),
    Reg("d2"),
    Reg("d3"),
    Reg("d4"),
    Reg("d5"),
    Reg("d6"),
    Reg("d7"),
];

/// The registers named directly by generated glue and by the fixed halves of
/// the calling convention.
const X0: Reg = Reg("x0");
const X15: Reg = Reg("x15");
const W15: Reg = Reg("w15");
const X16: Reg = Reg("x16");
const W16: Reg = Reg("w16");
const X17: Reg = Reg("x17");
const X29: Reg = Reg("x29");
const SP: Reg = Reg("sp");
const D0: Reg = Reg("d0");
const D1: Reg = Reg("d1");
const W1: Reg = Reg("w1");

/// Where an integer result is returned and where the builtin plan's "result"
/// lives.
const RESULT: Reg = X0;

pub struct Aarch64Backend;

impl Backend for Aarch64Backend {
    fn target(&self) -> &'static TargetSpec {
        &AARCH64_LINUX_GNU
    }

    fn emit(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<String> {
        Ok(Generator::new(file, module, options).generate()?.to_text())
    }

    fn object(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<Vec<u8>> {
        let assembly = Generator::new(file, module, options).generate()?;
        crate::codegen::write_object(file, &assembly, crate::elf::AARCH64)
    }
}

struct Generator<'a> {
    file: &'a str,
    module: &'a MirModule,
    options: &'a CodegenOptions,
    asm: Assembly<Inst>,
    strings: Vec<(String, String)>,
    string_ids: HashMap<String, String>,
    diagnostics: Vec<Diagnostic>,
    alloc: Allocation,
    registers: &'static RegisterFile,
    /// Byte offset from `sp` of frame slot zero. Everything a callee's stack
    /// arguments need sits below it, so writing those cannot reach a local.
    slot_base: usize,
    last_location: Option<(usize, usize, usize)>,
}

impl<'a> Generator<'a> {
    fn new(file: &'a str, module: &'a MirModule, options: &'a CodegenOptions) -> Self {
        Self {
            file,
            module,
            options,
            asm: Assembly::new(),
            strings: Vec::new(),
            string_ids: HashMap::new(),
            diagnostics: Vec::new(),
            alloc: Allocation::stack_only(0),
            registers: &CALLEE_SAVED,
            slot_base: 0,
            last_location: None,
        }
    }

    fn generate(mut self) -> CompileResult<Assembly<Inst>> {
        self.collect_strings();
        self.file_table();
        self.asm.push(Item::Section(Section::RoData));
        self.byte_string(".Lsl_panic_div_zero", b"division by zero");
        self.byte_string(".Lsl_panic_overflow", b"integer overflow");
        for (label, value) in self.strings.clone() {
            self.byte_string(&label, value.as_bytes());
        }
        self.asm.push(Item::Section(Section::Text));

        for function in self
            .module
            .functions
            .iter()
            .filter(|function| function.emit)
        {
            self.function(function, false);
        }
        for test in self.module.tests.iter().filter(|test| test.emit) {
            self.function(&test.function, true);
        }
        // Generated glue from here on. It writes no location and inherits the
        // last row, for the reason recorded in `D-023`.
        let structs = self.module.structs.clone();
        for structure in structs.iter().filter(|structure| structure.emit) {
            self.struct_clone_helper(&structure.name, &structure.fields);
            self.struct_drop_helper(&structure.name, &structure.fields);
        }
        let enums = self.module.enums.clone();
        for enumeration in enums.iter().filter(|enumeration| enumeration.emit) {
            self.enum_clone_helper(&enumeration.name, &enumeration.variants);
            self.enum_drop_helper(&enumeration.name, &enumeration.variants);
        }
        if self.options.test_harness {
            self.test_harness();
        } else if self.options.emit_entrypoint {
            self.program_entrypoint();
        }
        self.runtime_panic_trampolines();
        self.asm.push(Item::Section(Section::GnuStack));

        if self.diagnostics.is_empty() {
            self.asm.remove_redundant_copies();
            Ok(self.asm)
        } else {
            Err(self.diagnostics)
        }
    }

    // ----- data and debug directives -------------------------------------

    fn collect_strings(&mut self) {
        for function in self
            .module
            .functions
            .iter()
            .filter(|function| function.emit)
            .chain(
                self.module
                    .tests
                    .iter()
                    .filter(|test| test.emit)
                    .map(|test| &test.function),
            )
        {
            for instruction in function
                .blocks
                .iter()
                .flat_map(|block| block.instructions())
            {
                if let Instruction::StringNew { value, .. } = instruction {
                    self.intern(value);
                }
            }
        }
        for test in &self.module.tests {
            self.intern(&test.name);
        }
    }

    fn intern(&mut self, value: &str) -> String {
        if let Some(label) = self.string_ids.get(value) {
            return label.clone();
        }
        let label = format!(".Lsl_str_{}", self.strings.len());
        self.string_ids.insert(value.to_owned(), label.clone());
        self.strings.push((label.clone(), value.to_owned()));
        label
    }

    fn byte_string(&mut self, label: &str, bytes: &[u8]) {
        self.asm.push(Item::Label(label.to_owned()));
        let mut payload = bytes.to_vec();
        payload.push(0);
        self.asm.push(Item::Bytes(payload));
    }

    fn file_table(&mut self) {
        let Some(sources) = self.options.debug.as_ref() else {
            return;
        };
        for (index, path) in sources.paths().enumerate() {
            self.asm.push(Item::File {
                index: index + 1,
                path: path.to_owned(),
            });
        }
    }

    fn location(&mut self, span: Span) {
        let Some(sources) = self.options.debug.as_ref() else {
            return;
        };
        if span.line == 0 {
            return;
        }
        let Some(index) = sources.index_of(span) else {
            return;
        };
        let location = (index + 1, span.line, span.column);
        if self.last_location == Some(location) {
            return;
        }
        self.last_location = Some(location);
        self.asm.push(Item::Loc {
            file: location.0,
            line: location.1,
            column: location.2,
        });
    }

    fn label(&mut self, label: &str) {
        self.asm.push(Item::Label(label.to_owned()));
        self.last_location = None;
    }

    fn inst(&mut self, instruction: Inst) {
        self.asm.instruction(instruction);
    }

    // ----- operands -------------------------------------------------------

    /// Byte offset from `sp` of a local's frame slot.
    fn slot_offset(&self, local: LocalId) -> usize {
        match self.alloc.location(local) {
            Location::Memory(slot) => self.slot_base + slot * 8,
            Location::Register(_) => unreachable!("this local was allocated a register"),
        }
    }

    /// Puts a local's value in a register and names it.
    ///
    /// An allocated local is already in one and is named directly; anything
    /// else is loaded into `scratch`, which the caller must not be using for
    /// something it still needs.
    fn read(&mut self, local: LocalId, scratch: usize) -> Reg {
        match self.alloc.location(local) {
            Location::Register(register) => self.registers.wide[register],
            Location::Memory(_) => {
                let offset = self.slot_offset(local);
                let (wide, _) = SCRATCH[scratch];
                self.load(wide, offset);
                wide
            }
        }
    }

    /// The 32-bit view of whatever [`Generator::read`] would return.
    fn read_narrow(&mut self, local: LocalId, scratch: usize) -> Reg {
        match self.alloc.location(local) {
            Location::Register(register) => self.registers.narrow[register],
            Location::Memory(_) => {
                let offset = self.slot_offset(local);
                let (wide, narrow) = SCRATCH[scratch];
                self.load(wide, offset);
                narrow
            }
        }
    }

    /// Stores a register into a local, or moves it when the local has one.
    fn write(&mut self, local: LocalId, source: Reg) {
        match self.alloc.location(local) {
            Location::Register(register) => {
                let target = self.registers.wide[register];
                if target != source {
                    self.inst(Inst::Mov {
                        dst: target,
                        src: source,
                    });
                }
            }
            Location::Memory(_) => {
                let offset = self.slot_offset(local);
                self.store(source, offset);
            }
        }
    }

    /// `ldr` from the frame, spelling the offset a way the encoding accepts.
    ///
    /// The scaled immediate covers 32 KiB of frame, which every frame this
    /// compiler builds fits in; the computed form is there so that a frame that
    /// did not would still assemble.
    fn load(&mut self, target: Reg, offset: usize) {
        if offset <= 32760 {
            self.inst(Inst::Load {
                dst: target,
                base: SP,
                offset: Some(offset as u32),
            });
        } else {
            self.address_of_slot(target, offset);
            self.inst(Inst::Load {
                dst: target,
                base: target,
                offset: None,
            });
        }
    }

    fn store(&mut self, source: Reg, offset: usize) {
        if offset <= 32760 {
            self.inst(Inst::Store {
                src: source,
                base: SP,
                offset: Some(offset as u32),
            });
        } else {
            // `x17` is scratch and no store's source, so borrowing it here
            // cannot clobber the value being stored.
            self.address_of_slot(X17, offset);
            self.inst(Inst::Store {
                src: source,
                base: X17,
                offset: None,
            });
        }
    }

    /// Puts the address of a frame offset in a register.
    fn address_of_slot(&mut self, target: Reg, offset: usize) {
        if offset <= 4095 {
            self.inst(Inst::ArithImm {
                op: Arith::Add,
                dst: target,
                src: SP,
                imm: offset as u32,
            });
        } else {
            self.materialize(target, offset as u64);
            self.inst(Inst::Arith {
                op: Arith::Add,
                dst: target,
                lhs: SP,
                rhs: target,
            });
        }
    }

    /// Builds a 64-bit constant, one 16-bit field at a time.
    ///
    /// AArch64 has no instruction that takes a 64-bit immediate, so every
    /// constant wider than what a single `movz` covers is assembled from its
    /// halfwords. Zero halfwords are skipped, which is why small constants —
    /// the overwhelming majority — still cost one instruction.
    fn materialize(&mut self, target: Reg, bits: u64) {
        let mut written = false;
        for index in 0..4 {
            let half = (bits >> (16 * index)) & 0xffff;
            if half == 0 {
                continue;
            }
            self.inst(Inst::Half {
                keep: written,
                dst: target,
                half: half as u16,
                shift: 16 * index,
            });
            written = true;
        }
        if !written {
            self.inst(Inst::Half {
                keep: false,
                dst: target,
                half: 0,
                shift: 0,
            });
        }
    }

    /// Puts the address of a rodata label in a register: page, then offset.
    fn address_of_label(&mut self, target: Reg, label: &str) {
        self.inst(Inst::Adrp {
            dst: target,
            label: label.to_owned(),
        });
        self.inst(Inst::AddLow {
            dst: target,
            src: target,
            label: label.to_owned(),
        });
    }

    // ----- functions ------------------------------------------------------

    fn function(&mut self, function: &MirFunction, is_test: bool) {
        let symbol = function_symbol(&function.name, is_test);
        let epilogue = format!(".L{symbol}_epilogue");

        let cfg = Cfg::new(function);
        self.registers = if calls_something(self.module, function) {
            &CALLEE_SAVED
        } else {
            &LEAF
        };
        self.alloc = allocate(
            function,
            &cfg,
            self.registers.wide.len(),
            &address_taken(self.module, function),
        );

        // The frame is addressed upward from `sp`, so the outgoing-argument
        // area has to come first: a callee reads its stack arguments at `sp`
        // and would otherwise be reading this function's locals.
        let outgoing = align_to(outgoing_bytes(function), 16);
        self.slot_base = outgoing;
        let saved: Vec<usize> = self
            .alloc
            .used_registers()
            .iter()
            .copied()
            .filter(|register| *register >= self.registers.volatile)
            .collect();
        let save_base = self.alloc.memory_slots();
        let frame_size = align_to(outgoing + (save_base + saved.len()) * 8, 16);

        self.asm.push(Item::Global(symbol.clone()));
        self.asm.push(Item::Function(symbol.clone()));
        self.label(&symbol);
        self.location(function.span);
        self.inst(Inst::PushFrame);
        self.inst(Inst::Mov { dst: X29, src: SP });
        if frame_size != 0 {
            if frame_size <= 4095 {
                self.inst(Inst::ArithImm {
                    op: Arith::Sub,
                    dst: SP,
                    src: SP,
                    imm: frame_size as u32,
                });
            } else {
                self.materialize(X16, frame_size as u64);
                self.inst(Inst::Arith {
                    op: Arith::Sub,
                    dst: SP,
                    lhs: SP,
                    rhs: X16,
                });
            }
        }
        for (index, register) in saved.iter().enumerate() {
            let name = self.registers.wide[*register];
            self.store(name, outgoing + (save_base + index) * 8);
        }

        self.store_parameters(function);
        for (block_id, block) in function.blocks.iter().enumerate() {
            self.label(&format!(".L{symbol}_bb{block_id}"));
            self.basic_block(function, block, &symbol, &epilogue);
        }

        self.label(epilogue.as_str());
        for (index, register) in saved.iter().enumerate() {
            let name = self.registers.wide[*register];
            self.load(name, outgoing + (save_base + index) * 8);
        }
        self.inst(Inst::Mov { dst: SP, src: X29 });
        self.inst(Inst::PopFrame);
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.clone()));
    }

    fn store_parameters(&mut self, function: &MirFunction) {
        let mut integers = 0;
        let mut floats = 0;
        let mut stack = 0;
        for local in &function.params {
            let ty = function.locals[*local].ty.clone();
            if ty == Type::F64 {
                if floats >= FLOAT_ARGUMENTS.len() {
                    // Incoming stack arguments sit above the saved frame
                    // record, which `x29` points at.
                    self.inst(Inst::Load {
                        dst: X16,
                        base: X29,
                        offset: Some((16 + stack * 8) as u32),
                    });
                    self.write(*local, X16);
                    stack += 1;
                } else {
                    self.inst(Inst::Fmov {
                        dst: X16,
                        src: FLOAT_ARGUMENTS[floats],
                    });
                    self.write(*local, X16);
                    floats += 1;
                }
            } else if integers >= INTEGER_ARGUMENTS.len() {
                self.inst(Inst::Load {
                    dst: X16,
                    base: X29,
                    offset: Some((16 + stack * 8) as u32),
                });
                self.write(*local, X16);
                stack += 1;
            } else {
                self.write(*local, INTEGER_ARGUMENTS[integers]);
                integers += 1;
            }
        }
    }

    fn basic_block(
        &mut self,
        function: &MirFunction,
        block: &BasicBlock,
        symbol: &str,
        epilogue: &str,
    ) {
        for statement in &block.statements {
            self.location(statement.span);
            self.instruction(function, &statement.instruction);
        }
        self.location(block.terminator_span);
        match &block.terminator {
            Terminator::Return(value) => {
                match value {
                    Some(local) if function.locals[*local].ty == Type::F64 => {
                        let source = self.read(*local, 1);
                        self.inst(Inst::Fmov {
                            dst: D0,
                            src: source,
                        });
                    }
                    Some(local) => {
                        let source = self.read(*local, 1);
                        if source != RESULT {
                            self.inst(Inst::Mov {
                                dst: RESULT,
                                src: source,
                            });
                        }
                    }
                    None => self.materialize(RESULT, 0),
                }
                self.inst(Inst::B(Target::Named(epilogue.to_owned())));
            }
            Terminator::Goto(target) => {
                self.inst(Inst::B(Target::Named(format!(".L{symbol}_bb{target}"))))
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.read(*condition, 1);
                self.inst(Inst::Cbnz(
                    condition,
                    Target::Named(format!(".L{symbol}_bb{then_block}")),
                ));
                self.inst(Inst::B(Target::Named(format!(".L{symbol}_bb{else_block}"))));
            }
            Terminator::Unreachable => self.inst(Inst::Brk(1)),
        }
    }

    fn instruction(&mut self, function: &MirFunction, instruction: &Instruction) {
        match instruction {
            Instruction::ConstInt { dst, value } => self.constant(*dst, *value as u64),
            Instruction::ConstFloat { dst, bits } => self.constant(*dst, *bits),
            Instruction::ConstBool { dst, value } => self.constant(*dst, u64::from(*value)),
            Instruction::StringNew { dst, value } => {
                let label = self.string_ids[value].clone();
                let length = value.len() as u64;
                self.address_of_label(X0, &label);
                self.materialize(Reg("x1"), length);
                self.inst(Inst::Bl("sl_rt_string_new".into()));
                self.write(*dst, RESULT);
            }
            Instruction::Assign { dst, src } => {
                let source = self.read(*src, 1);
                self.write(*dst, source);
            }
            Instruction::AddressOf { dst, src } => {
                // Borrowing a pointer-shaped value copies the pointer; anything
                // else needs the address of its slot, which is why
                // `address_taken` pins those locals to memory.
                if function
                    .locals
                    .get(*src)
                    .is_some_and(|local| is_pointer_like(&local.ty))
                {
                    let source = self.read(*src, 1);
                    self.write(*dst, source);
                } else {
                    let offset = self.slot_offset(*src);
                    self.address_of_slot(X16, offset);
                    self.write(*dst, X16);
                }
            }
            Instruction::Binary {
                dst,
                op,
                lhs,
                rhs,
                ty,
            } => {
                if *ty == Type::F64 {
                    self.float_binary(*dst, *op, *lhs, *rhs);
                } else {
                    self.integer_binary(*dst, *op, *lhs, *rhs, ty);
                }
            }
            Instruction::Call {
                dst,
                callee,
                args,
                arg_types,
                result,
            } => self.call(*dst, callee, args, arg_types, result),
            Instruction::Drop { local, ty } => {
                let source = self.read(*local, 1);
                if source != X0 {
                    self.inst(Inst::Mov {
                        dst: X0,
                        src: source,
                    });
                }
                if let Some(symbol) = drop_function(self.module, ty) {
                    self.inst(Inst::Bl(symbol));
                }
                self.materialize(X16, 0);
                self.write(*local, X16);
            }
            Instruction::StructNew { dst, name, fields } => {
                let size = struct_size(self.module, name) as u64;
                self.materialize(X0, size);
                self.inst(Inst::Bl("sl_rt_alloc".into()));
                for (index, field) in fields.iter().enumerate() {
                    let source = self.read(*field, 1);
                    self.inst(Inst::Store {
                        src: source,
                        base: X0,
                        offset: Some((index * 8) as u32),
                    });
                }
                self.write(*dst, RESULT);
            }
            Instruction::FieldLoad { dst, base, index } => {
                let base = self.read(*base, 1);
                self.inst(Inst::Load {
                    dst: X16,
                    base,
                    offset: Some((index * 8) as u32),
                });
                self.write(*dst, X16);
            }
            Instruction::EnumNew {
                dst, tag, fields, ..
            } => {
                let size = enum_size(fields.len()) as u64;
                self.materialize(X0, size);
                self.inst(Inst::Bl("sl_rt_alloc".into()));
                self.materialize(X16, *tag as u64);
                self.inst(Inst::Store {
                    src: X16,
                    base: X0,
                    offset: None,
                });
                for (index, field) in fields.iter().enumerate() {
                    let source = self.read(*field, 1);
                    self.inst(Inst::Store {
                        src: source,
                        base: X0,
                        offset: Some(((index + 1) * 8) as u32),
                    });
                }
                self.write(*dst, RESULT);
            }
            Instruction::EnumTag { dst, base } => {
                let base = self.read(*base, 1);
                self.inst(Inst::Load {
                    dst: X16,
                    base,
                    offset: None,
                });
                self.write(*dst, X16);
            }
            Instruction::EnumFieldLoad { dst, base, index } => {
                let base = self.read(*base, 1);
                self.inst(Inst::Load {
                    dst: X16,
                    base,
                    offset: Some(((index + 1) * 8) as u32),
                });
                self.write(*dst, X16);
            }
            Instruction::Free { local } => {
                let source = self.read(*local, 1);
                if source != X0 {
                    self.inst(Inst::Mov {
                        dst: X0,
                        src: source,
                    });
                }
                self.inst(Inst::Bl("sl_rt_free".into()));
                self.materialize(X16, 0);
                self.write(*local, X16);
            }
        }
    }

    /// Materializes a constant into a local, in place when it has a register.
    fn constant(&mut self, dst: LocalId, bits: u64) {
        match self.alloc.location(dst) {
            Location::Register(register) => {
                let target = self.registers.wide[register];
                self.materialize(target, bits);
            }
            Location::Memory(_) => {
                self.materialize(X16, bits);
                self.write(dst, X16);
            }
        }
    }

    // ----- arithmetic ------------------------------------------------------

    fn integer_binary(
        &mut self,
        dst: LocalId,
        op: BinaryOp,
        lhs: LocalId,
        rhs: LocalId,
        ty: &Type,
    ) {
        let narrow = *ty == Type::I32;
        match op {
            BinaryOp::Add | BinaryOp::Sub => {
                let op = if matches!(op, BinaryOp::Add) {
                    Arith::Adds
                } else {
                    Arith::Subs
                };
                if narrow {
                    let left = self.read_narrow(lhs, 1);
                    let right = self.read_narrow(rhs, 2);
                    self.inst(Inst::Arith {
                        op,
                        dst: W16,
                        lhs: left,
                        rhs: right,
                    });
                    self.trap_on_overflow();
                    // The 32-bit form leaves the upper half zero; the local
                    // holds a sign-extended i32.
                    self.inst(Inst::Sxtw { dst: X16, src: W16 });
                } else {
                    let left = self.read(lhs, 1);
                    let right = self.read(rhs, 2);
                    self.inst(Inst::Arith {
                        op,
                        dst: X16,
                        lhs: left,
                        rhs: right,
                    });
                    self.trap_on_overflow();
                }
                self.write(dst, X16);
            }
            BinaryOp::Mul if narrow => {
                let left = self.read_narrow(lhs, 1);
                let right = self.read_narrow(rhs, 2);
                // The full 64-bit product; it fits in an i32 exactly when it
                // equals the sign extension of its own low half.
                self.inst(Inst::Arith {
                    op: Arith::Smull,
                    dst: X16,
                    lhs: left,
                    rhs: right,
                });
                self.inst(Inst::CmpExtended { lhs: X16, rhs: W16 });
                self.inst(Inst::Bcond(Cond::Ne, overflow_trampoline()));
                self.inst(Inst::Sxtw { dst: X16, src: W16 });
                self.write(dst, X16);
            }
            BinaryOp::Mul => {
                let left = self.read(lhs, 1);
                let right = self.read(rhs, 2);
                // `mul` does not set flags, so the check is the high half of
                // the product against the sign of the low half.
                self.inst(Inst::Arith {
                    op: Arith::Smulh,
                    dst: X15,
                    lhs: left,
                    rhs: right,
                });
                self.inst(Inst::Arith {
                    op: Arith::Mul,
                    dst: X16,
                    lhs: left,
                    rhs: right,
                });
                self.inst(Inst::CmpShifted {
                    lhs: X15,
                    rhs: X16,
                    amount: 63,
                });
                self.inst(Inst::Bcond(Cond::Ne, overflow_trampoline()));
                self.write(dst, X16);
            }
            BinaryOp::Div => self.divide(dst, lhs, rhs, narrow),
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal => {
                let left = self.read(lhs, 1);
                let right = self.read(rhs, 2);
                self.inst(Inst::Cmp {
                    lhs: left,
                    rhs: right,
                });
                self.inst(Inst::Cset {
                    dst: X16,
                    cond: integer_condition(op),
                });
                self.write(dst, X16);
            }
        }
    }

    /// Signed division, with the two inputs that have no quotient rejected
    /// first: a zero divisor, and the most negative value over `-1`.
    ///
    /// `sdiv` traps on neither — it returns zero and saturates respectively —
    /// so unlike x86 the checks are the only thing standing between those
    /// inputs and a wrong answer, rather than a nicer message for a fault that
    /// would happen anyway.
    fn divide(&mut self, dst: LocalId, lhs: LocalId, rhs: LocalId, narrow: bool) {
        let (left, right) = if narrow {
            (self.read_narrow(lhs, 1), self.read_narrow(rhs, 2))
        } else {
            (self.read(lhs, 1), self.read(rhs, 2))
        };
        self.inst(Inst::Cbz(
            right,
            Target::Named(".Lsl_panic_div_zero_trampoline".into()),
        ));
        let most_negative = if narrow {
            i64::from(i32::MIN) as u64
        } else {
            i64::MIN as u64
        };
        self.materialize(X15, most_negative);
        let extreme = if narrow { W15 } else { X15 };
        self.inst(Inst::Cmp {
            lhs: left,
            rhs: extreme,
        });
        self.inst(Inst::Bcond(Cond::Ne, Target::Forward(1)));
        // `cmn r, #1` is `cmp r, #-1` without needing a negative immediate.
        self.inst(Inst::CmnImm { lhs: right, imm: 1 });
        self.inst(Inst::Bcond(Cond::Eq, overflow_trampoline()));
        self.asm.push(Item::Numeric(1));
        if narrow {
            self.inst(Inst::Arith {
                op: Arith::Sdiv,
                dst: W16,
                lhs: left,
                rhs: right,
            });
            self.inst(Inst::Sxtw { dst: X16, src: W16 });
        } else {
            self.inst(Inst::Arith {
                op: Arith::Sdiv,
                dst: X16,
                lhs: left,
                rhs: right,
            });
        }
        self.write(dst, X16);
    }

    /// Branches to the overflow trampoline when the last flag-setting
    /// instruction overflowed.
    ///
    /// A conditional branch reaches ±1 MiB, so a `.text` section larger than
    /// that between an arithmetic instruction and the trampoline at its end
    /// fails to assemble. That is a loud build-time error, not a miscompile,
    /// and no program this compiler can express comes close.
    fn trap_on_overflow(&mut self) {
        self.inst(Inst::Bcond(Cond::Vs, overflow_trampoline()));
    }

    fn float_binary(&mut self, dst: LocalId, op: BinaryOp, lhs: LocalId, rhs: LocalId) {
        let left = self.read(lhs, 1);
        self.inst(Inst::Fmov { dst: D0, src: left });
        let right = self.read(rhs, 2);
        self.inst(Inst::Fmov {
            dst: D1,
            src: right,
        });
        let arithmetic = |op| Inst::Float {
            op,
            dst: D0,
            lhs: D0,
            rhs: D1,
        };
        match op {
            BinaryOp::Add => self.inst(arithmetic(FloatOp::Add)),
            BinaryOp::Sub => self.inst(arithmetic(FloatOp::Sub)),
            BinaryOp::Mul => self.inst(arithmetic(FloatOp::Mul)),
            BinaryOp::Div => self.inst(arithmetic(FloatOp::Div)),
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal => {
                self.inst(Inst::Fcmp { lhs: D0, rhs: D1 });
                self.inst(Inst::Cset {
                    dst: X16,
                    cond: float_condition(op),
                });
                self.inst(Inst::Fmov { dst: D0, src: X16 });
            }
        }
        self.inst(Inst::Fmov { dst: X16, src: D0 });
        self.write(dst, X16);
    }

    // ----- calls -----------------------------------------------------------

    fn call(
        &mut self,
        dst: LocalId,
        callee: &str,
        args: &[LocalId],
        arg_types: &[Type],
        result: &Type,
    ) {
        match crate::lowering::builtin(self.module, dst, callee, args, arg_types, result) {
            Some(steps) => self.builtin(dst, &steps),
            None => self.ordinary_call(callee, args, arg_types),
        }
        match result {
            Type::Unit => {}
            Type::F64 => {
                self.inst(Inst::Fmov { dst: X16, src: D0 });
                self.write(dst, X16);
            }
            _ => self.write(dst, RESULT),
        }
    }

    /// Carries out a builtin's lowering plan. The plan is shared with the other
    /// backend; the argument registers and the branch spelling are not.
    fn builtin(&mut self, dst: LocalId, steps: &[Step]) {
        for step in steps {
            match step {
                Step::Invoke { arguments, tail } => {
                    for (index, argument) in arguments.iter().enumerate() {
                        let register = INTEGER_ARGUMENTS[index];
                        match argument {
                            Argument::Value(local) => {
                                let source = self.read(*local, 1);
                                if source != register {
                                    self.inst(Inst::Mov {
                                        dst: register,
                                        src: source,
                                    });
                                }
                            }
                            Argument::Address(local) => {
                                let offset = self.slot_offset(*local);
                                self.address_of_slot(register, offset);
                            }
                            Argument::Immediate(value) => self.materialize(register, *value as u64),
                            Argument::Function(Some(symbol)) => {
                                let symbol = symbol.clone();
                                self.address_of_label(register, &symbol);
                            }
                            Argument::Function(None) => self.inst(Inst::Half {
                                keep: false,
                                dst: NARROW_ARGUMENTS[index],
                                half: 0,
                                shift: 0,
                            }),
                        }
                    }
                    match tail {
                        Tail::Call(symbol) => self.inst(Inst::Bl(symbol.clone())),
                        // The first argument already is the result: on this ABI
                        // argument zero and the result are the same register,
                        // so there is nothing left to do.
                        Tail::FirstArgument => {
                            debug_assert_eq!(INTEGER_ARGUMENTS[0], RESULT);
                        }
                    }
                }
                Step::Save => self.write(dst, RESULT),
                Step::Restore => {
                    let source = self.read(dst, 1);
                    if source != RESULT {
                        self.inst(Inst::Mov {
                            dst: RESULT,
                            src: source,
                        });
                    }
                }
                Step::Load => self.inst(Inst::Load {
                    dst: RESULT,
                    base: RESULT,
                    offset: None,
                }),
                Step::WrapOption { some_tag, none_tag } => {
                    self.inst(Inst::Cbz(RESULT, Target::Forward(1)));
                    self.materialize(X0, enum_size(1) as u64);
                    self.inst(Inst::Bl("sl_rt_alloc".into()));
                    self.materialize(X16, *some_tag as u64);
                    self.inst(Inst::Store {
                        src: X16,
                        base: X0,
                        offset: None,
                    });
                    let payload = self.read(dst, 1);
                    self.inst(Inst::Store {
                        src: payload,
                        base: X0,
                        offset: Some(8),
                    });
                    self.inst(Inst::B(Target::Forward(2)));
                    self.asm.push(Item::Numeric(1));
                    self.materialize(X0, enum_size(0) as u64);
                    self.inst(Inst::Bl("sl_rt_alloc".into()));
                    self.materialize(X16, *none_tag as u64);
                    self.inst(Inst::Store {
                        src: X16,
                        base: X0,
                        offset: None,
                    });
                    self.asm.push(Item::Numeric(2));
                }
            }
        }
    }

    /// A call to a Slopium function, by AAPCS64.
    ///
    /// Arguments past the registers go into the outgoing area reserved at the
    /// bottom of this function's own frame, rather than being pushed: `sp` has
    /// to stay 16-byte aligned and pointing at the frame the locals are
    /// addressed from.
    fn ordinary_call(&mut self, callee: &str, args: &[LocalId], arg_types: &[Type]) {
        let mut integers = 0;
        let mut floats = 0;
        let mut stack = 0;
        for (arg, ty) in args.iter().zip(arg_types) {
            if *ty == Type::F64 {
                if floats >= FLOAT_ARGUMENTS.len() {
                    let source = self.read(*arg, 1);
                    self.inst(Inst::Store {
                        src: source,
                        base: SP,
                        offset: Some((stack * 8) as u32),
                    });
                    stack += 1;
                } else {
                    let source = self.read(*arg, 1);
                    self.inst(Inst::Fmov {
                        dst: FLOAT_ARGUMENTS[floats],
                        src: source,
                    });
                    floats += 1;
                }
            } else if integers >= INTEGER_ARGUMENTS.len() {
                let source = self.read(*arg, 1);
                self.inst(Inst::Store {
                    src: source,
                    base: SP,
                    offset: Some((stack * 8) as u32),
                });
                stack += 1;
            } else {
                let source = self.read(*arg, 1);
                let register = INTEGER_ARGUMENTS[integers];
                if source != register {
                    self.inst(Inst::Mov {
                        dst: register,
                        src: source,
                    });
                }
                integers += 1;
            }
        }
        self.inst(Inst::Bl(function_symbol(callee, false)));
    }

    // ----- generated glue --------------------------------------------------

    /// Opens a helper: a frame record plus `bytes` of locals at `sp`.
    fn open_helper(&mut self, symbol: &str, bytes: usize) {
        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.asm.push(Item::Label(symbol.to_owned()));
        self.inst(Inst::PushFrame);
        self.inst(Inst::Mov { dst: X29, src: SP });
        self.inst(Inst::ArithImm {
            op: Arith::Sub,
            dst: SP,
            src: SP,
            imm: align_to(bytes, 16) as u32,
        });
    }

    fn close_helper(&mut self, symbol: &str) {
        self.inst(Inst::Mov { dst: SP, src: X29 });
        self.inst(Inst::PopFrame);
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
    }

    fn test_harness(&mut self) {
        self.open_helper("main", 16);
        self.inst(Inst::Bl("sl_rt_args_init".into()));
        self.materialize(X16, 0);
        self.store(X16, 0);
        for test in &self.module.tests {
            let name = self.string_ids[&test.name].clone();
            let symbol = function_symbol(&test.function.name, true);
            self.inst(Inst::Bl(symbol));
            self.inst(Inst::Mov {
                dst: W1,
                src: Reg("w0"),
            });
            self.address_of_label(X0, &name);
            self.inst(Inst::Bl("sl_rt_test_result".into()));
            self.load(X16, 0);
            self.inst(Inst::Arith {
                op: Arith::Add,
                dst: X16,
                lhs: X16,
                rhs: X0,
            });
            self.store(X16, 0);
        }
        self.load(X0, 0);
        self.close_helper("main");
    }

    fn program_entrypoint(&mut self) {
        let Some(main) = self
            .module
            .functions
            .iter()
            .find(|function| function.name == "main")
        else {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::UNSUPPORTED_ABI,
                    self.file,
                    Default::default(),
                    "executable does not define `main`",
                )
                .with_note("test-only programs must be compiled with `--test`"),
            );
            return;
        };
        let symbol = function_symbol(&main.name, false);
        let returns_unit = main.return_type == Type::Unit;
        self.open_helper("main", 16);
        // `argc` and `argv` arrive in exactly the registers the runtime wants.
        self.inst(Inst::Bl("sl_rt_args_init".into()));
        self.inst(Inst::Bl(symbol));
        if returns_unit {
            self.materialize(X0, 0);
        }
        self.close_helper("main");
    }

    fn runtime_panic_trampolines(&mut self) {
        for (label, message) in [
            (".Lsl_panic_div_zero", "div_zero"),
            (".Lsl_panic_overflow", "overflow"),
        ] {
            self.asm
                .push(Item::Label(format!(".Lsl_panic_{message}_trampoline")));
            self.address_of_label(X0, label);
            self.inst(Inst::Bl("sl_rt_panic".into()));
            self.inst(Inst::Brk(1));
        }
    }

    fn struct_clone_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_clone_symbol(name);
        let size = struct_size(self.module, name) as u64;
        self.open_helper(&symbol, 16);
        self.store(X0, 0);
        self.materialize(X0, size);
        self.inst(Inst::Bl("sl_rt_alloc".into()));
        self.store(X0, 8);
        for (index, (_, ty)) in fields.iter().enumerate() {
            self.load(X16, 0);
            self.inst(Inst::Load {
                dst: X0,
                base: X16,
                offset: Some((index * 8) as u32),
            });
            if let Some(clone) = clone_function(self.module, ty) {
                self.inst(Inst::Bl(clone));
            }
            self.load(X16, 8);
            self.inst(Inst::Store {
                src: X0,
                base: X16,
                offset: Some((index * 8) as u32),
            });
        }
        self.load(X0, 8);
        self.close_helper(&symbol);
    }

    fn enum_clone_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_clone_symbol(name);
        let size = enum_clone_size(self.module, name) as u64;
        self.open_helper(&symbol, 16);
        self.store(X0, 0);
        self.materialize(X0, size);
        self.inst(Inst::Bl("sl_rt_alloc".into()));
        self.store(X0, 8);
        self.load(X16, 0);
        self.inst(Inst::Load {
            dst: X17,
            base: X16,
            offset: None,
        });
        self.inst(Inst::Store {
            src: X17,
            base: X0,
            offset: None,
        });
        for variant in variants {
            self.materialize(X16, variant.tag as u64);
            self.inst(Inst::Cmp { lhs: X17, rhs: X16 });
            self.inst(Inst::Bcond(
                Cond::Eq,
                Target::Named(format!(".L{symbol}_clone_variant_{}", variant.tag)),
            ));
        }
        self.inst(Inst::B(Target::Named(format!(".L{symbol}_clone_return"))));
        for variant in variants {
            self.asm.push(Item::Label(format!(
                ".L{symbol}_clone_variant_{}",
                variant.tag
            )));
            for (index, (_, ty)) in variant.fields.iter().enumerate() {
                self.load(X16, 0);
                self.inst(Inst::Load {
                    dst: X0,
                    base: X16,
                    offset: Some(((index + 1) * 8) as u32),
                });
                if let Some(clone) = clone_function(self.module, ty) {
                    self.inst(Inst::Bl(clone));
                }
                self.load(X16, 8);
                self.inst(Inst::Store {
                    src: X0,
                    base: X16,
                    offset: Some(((index + 1) * 8) as u32),
                });
            }
            self.inst(Inst::B(Target::Named(format!(".L{symbol}_clone_return"))));
        }
        self.asm
            .push(Item::Label(format!(".L{symbol}_clone_return")));
        self.load(X0, 8);
        self.close_helper(&symbol);
    }

    fn struct_drop_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_drop_symbol(name);
        self.open_helper(&symbol, 16);
        // Match the runtime's own drops: a null pointer is a no-op rather than
        // a wild load, so a dropped-and-zeroed slot stays benign.
        self.inst(Inst::Cbz(X0, Target::Named(format!(".L{symbol}_return"))));
        self.store(X0, 0);
        for (index, (_, ty)) in fields.iter().enumerate().rev() {
            if let Some(drop) = drop_function(self.module, ty) {
                self.load(X16, 0);
                self.inst(Inst::Load {
                    dst: X0,
                    base: X16,
                    offset: Some((index * 8) as u32),
                });
                self.inst(Inst::Bl(drop));
            }
        }
        self.load(X0, 0);
        self.inst(Inst::Bl("sl_rt_free".into()));
        self.asm.push(Item::Label(format!(".L{symbol}_return")));
        self.close_helper(&symbol);
    }

    fn enum_drop_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_drop_symbol(name);
        self.open_helper(&symbol, 16);
        self.inst(Inst::Cbz(X0, Target::Named(format!(".L{symbol}_return"))));
        self.store(X0, 0);
        self.inst(Inst::Load {
            dst: X17,
            base: X0,
            offset: None,
        });
        for variant in variants {
            self.materialize(X16, variant.tag as u64);
            self.inst(Inst::Cmp { lhs: X17, rhs: X16 });
            self.inst(Inst::Bcond(
                Cond::Eq,
                Target::Named(format!(".L{symbol}_variant_{}", variant.tag)),
            ));
        }
        self.inst(Inst::B(Target::Named(format!(".L{symbol}_free"))));
        for variant in variants {
            self.asm
                .push(Item::Label(format!(".L{symbol}_variant_{}", variant.tag)));
            for (index, (_, ty)) in variant.fields.iter().enumerate().rev() {
                if let Some(drop) = drop_function(self.module, ty) {
                    self.load(X16, 0);
                    self.inst(Inst::Load {
                        dst: X0,
                        base: X16,
                        offset: Some(((index + 1) * 8) as u32),
                    });
                    self.inst(Inst::Bl(drop));
                }
            }
            self.inst(Inst::B(Target::Named(format!(".L{symbol}_free"))));
        }
        self.asm.push(Item::Label(format!(".L{symbol}_free")));
        self.load(X0, 0);
        self.inst(Inst::Bl("sl_rt_free".into()));
        self.asm.push(Item::Label(format!(".L{symbol}_return")));
        self.close_helper(&symbol);
    }
}

/// Whether generated code for this function contains a call that returns, which
/// decides whether caller-saved registers are usable (`D-021`).
///
/// Deliberately conservative in the same way the other backend is: claiming a
/// call that is not emitted costs an allocation opportunity, claiming a leaf
/// that is not one would corrupt `x9` at the first call.
fn calls_something(module: &MirModule, function: &MirFunction) -> bool {
    function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
        .any(|instruction| match instruction {
            Instruction::Call { .. }
            | Instruction::StringNew { .. }
            | Instruction::StructNew { .. }
            | Instruction::EnumNew { .. }
            | Instruction::Free { .. } => true,
            Instruction::Drop { ty, .. } => drop_function(module, ty).is_some(),
            Instruction::ConstInt { .. }
            | Instruction::ConstFloat { .. }
            | Instruction::ConstBool { .. }
            | Instruction::Assign { .. }
            | Instruction::AddressOf { .. }
            | Instruction::Binary { .. }
            | Instruction::FieldLoad { .. }
            | Instruction::EnumTag { .. }
            | Instruction::EnumFieldLoad { .. } => false,
        })
}

/// Bytes of outgoing stack arguments the widest call in this function needs.
///
/// Reserved once in the prologue rather than adjusted per call, because `sp`
/// also anchors every local: moving it between two statements would move the
/// frame out from under them.
fn outgoing_bytes(function: &MirFunction) -> usize {
    function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
        .map(|instruction| match instruction {
            // Builtins never take more arguments than there are registers.
            Instruction::Call { arg_types, .. } => {
                let floats = arg_types.iter().filter(|ty| **ty == Type::F64).count();
                let integers = arg_types.len() - floats;
                let stacked = integers.saturating_sub(INTEGER_ARGUMENTS.len())
                    + floats.saturating_sub(FLOAT_ARGUMENTS.len());
                stacked * 8
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// The trampoline every arithmetic check branches to.
fn overflow_trampoline() -> Target {
    Target::Named(".Lsl_panic_overflow_trampoline".into())
}

fn integer_condition(op: BinaryOp) -> Cond {
    match op {
        BinaryOp::Less => Cond::Lt,
        BinaryOp::Greater => Cond::Gt,
        BinaryOp::Equal => Cond::Eq,
        _ => unreachable!("only the comparison operators produce a condition"),
    }
}

/// Conditions for a comparison of two doubles.
///
/// `fcmp` reports "unordered" — either side a NaN — as a fourth outcome, and
/// these three conditions are all false for it. That is what IEEE 754 asks for:
/// a NaN is neither less than, greater than, nor equal to anything, itself
/// included.
fn float_condition(op: BinaryOp) -> Cond {
    match op {
        BinaryOp::Less => Cond::Mi,
        BinaryOp::Greater => Cond::Gt,
        BinaryOp::Equal => Cond::Eq,
        _ => unreachable!("only the comparison operators produce a condition"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{is_location, DEFAULT_TARGET};
    use crate::{compile_to_assembly, CompileOptions};

    fn options() -> CompileOptions {
        CompileOptions {
            target: AARCH64_LINUX_GNU.triple.into(),
            ..Default::default()
        }
    }

    fn assemble(source: &str) -> String {
        compile_to_assembly("test.slp", source, &options()).unwrap()
    }

    fn body_of<'a>(assembly: &'a str, name: &str) -> &'a str {
        let symbol = function_symbol(name, false);
        let start = assembly
            .find(&format!("\n{symbol}:\n"))
            .unwrap_or_else(|| panic!("{symbol} must be emitted"));
        let end = assembly[start..]
            .find(".size")
            .expect("a function ends with its size directive");
        &assembly[start..start + end]
    }

    /// The x86-64 backend must be unaffected by the presence of a second one.
    #[test]
    fn the_two_backends_emit_different_code_for_the_same_source() {
        let source = "(fn main () -> i32 (+ 20 22))";
        let x86 = compile_to_assembly(
            "test.slp",
            source,
            &CompileOptions {
                target: DEFAULT_TARGET.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let arm = assemble(source);
        assert!(x86.contains(".intel_syntax noprefix"));
        assert!(!arm.contains(".intel_syntax noprefix"));
        assert!(arm.contains("stp x29, x30, [sp, #-16]!"));
    }

    #[test]
    fn an_unknown_target_is_a_diagnostic_rather_than_a_panic() {
        let error = compile_to_assembly(
            "test.slp",
            "(fn main () -> i32 0)",
            &CompileOptions {
                target: "riscv64-unknown-linux-gnu".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error[0].code, codes::UNSUPPORTED_TARGET);
        assert!(error[0]
            .help
            .as_ref()
            .is_some_and(|help| help.contains("aarch64-unknown-linux-gnu")));
    }

    /// Every arithmetic operator that can overflow reaches the trampoline, and
    /// division reaches both of them. This is `D-019` on the second backend.
    #[test]
    fn every_trapping_operator_checks_before_it_commits() {
        for (source, checks) in [
            ("(fn main () -> i64 (+ 2 3))", vec!["b.vs"]),
            ("(fn main () -> i64 (- 2 3))", vec!["b.vs"]),
            ("(fn main () -> i64 (* 2 3))", vec!["smulh", "b.ne"]),
            (
                "(fn main () -> i64 (/ 6 3))",
                vec!["cbz", ".Lsl_panic_div_zero_trampoline", "b.eq"],
            ),
        ] {
            // Constant folding is off in the default profile, so the operator
            // survives to code generation.
            let body = assemble(source).to_owned();
            let body = body_of(&body, "main").to_owned();
            for check in checks {
                assert!(
                    body.contains(check),
                    "{source} must contain {check}:\n{body}"
                );
            }
        }
    }

    /// A leaf reaches for the caller-saved registers and pays no prologue for
    /// them; a function that calls something takes callee-saved ones and saves
    /// exactly those (`D-021`).
    #[test]
    fn a_leaf_costs_no_register_saves_and_a_caller_does() {
        let leaf = assemble("(fn twice ((n i64)) -> i64 (+ n n))\n(fn main () -> i32 0)");
        let leaf = body_of(&leaf, "twice");
        assert!(!leaf.contains("str x19"), "a leaf saves nothing:\n{leaf}");

        let caller = assemble(
            "(fn helper ((n i64)) -> i64 n)\n\
             (fn work ((n i64)) -> i64 (+ (helper n) (helper n)))\n\
             (fn main () -> i32 0)",
        );
        let caller = body_of(&caller, "work");
        assert!(
            caller.contains("str x19"),
            "a calling function saves what it allocates:\n{caller}"
        );
        assert!(caller.contains("ldr x19"), "and restores it:\n{caller}");
    }

    /// The frame is addressed upward from `sp`, so a local's slot must never
    /// overlap the area a callee reads its stack arguments from.
    #[test]
    fn stack_arguments_do_not_overlap_the_locals() {
        let source = "(fn many ((a i64) (b i64) (c i64) (d i64) (e i64) (f i64) (g i64) (h i64) \
                      (i i64) (j i64)) -> i64 (+ i j))\n\
                      (fn main () -> i32 (do (many 1 2 3 4 5 6 7 8 9 10) 0))";
        let assembly = assemble(source);
        let caller = body_of(&assembly, "main");
        // Two arguments past the eight registers, written at the bottom of the
        // frame, and a frame at least that big reserved for them.
        assert!(caller.contains("str x16, [sp, #0]") || caller.contains(", [sp, #0]"));
        assert!(caller.contains("[sp, #8]"));
        assert!(
            caller.contains("sub sp, sp, #"),
            "the caller must reserve the outgoing area:\n{caller}"
        );
    }

    /// Constants wider than a halfword are assembled from their halves, and
    /// small ones still cost a single instruction.
    #[test]
    fn a_wide_constant_is_built_from_its_halfwords() {
        let small = assemble("(fn main () -> i64 7)");
        assert!(
            small
                .lines()
                .any(|line| line.trim().ends_with(", #7") && line.trim().starts_with("movz")),
            "a small constant is one movz:\n{small}"
        );
        assert!(!small.contains("movk"), "and needs no movk:\n{small}");

        // 0x100_0000_0006: halfword 0 and halfword 2 set, halfword 1 clear, so
        // the skipped halfword is exercised too.
        let wide = assemble("(fn main () -> i64 1099511627782)");
        assert!(wide.contains("movz"), "a wide constant opens with movz");
        assert!(
            wide.contains("movk") && wide.contains("lsl #32"),
            "and reaches its high halfword:\n{wide}"
        );
    }

    /// `D-024` on this backend: the only difference debug information makes is
    /// the directives themselves.
    #[test]
    fn debug_information_adds_directives_and_changes_no_instruction() {
        let source = "(fn add ((a i64) (b i64)) -> i64 (+ a b))\n\
                      (fn main () -> i32 (do (add 1 2) 0))";
        let plain = assemble(source);
        let debug = compile_to_assembly(
            "test.slp",
            source,
            &CompileOptions {
                debug: true,
                ..options()
            },
        )
        .unwrap();
        assert!(debug.contains(".loc 1 "), "debug output must carry rows");
        let stripped: String = debug
            .lines()
            .filter(|line| !is_location(line) && !line.starts_with(".file "))
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(stripped, plain);
    }
}
