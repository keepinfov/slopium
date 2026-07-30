use crate::ast::Type;
use crate::cfg::Cfg;
use crate::diagnostic::{codes, CompileResult, Diagnostic};
use crate::mir::{BasicBlock, BinaryOp, Instruction, LocalId, MirFunction, MirModule, Terminator};
use crate::regalloc::{allocate, Allocation, Location};
use std::collections::HashMap;
use std::fmt::Write;

pub const SUPPORTED_TARGET: &str = "x86_64-unknown-linux-gnu";

/// The registers one function may allocate locals to, in the order the
/// allocator should hand them out.
///
/// None of them is a scratch register of this generator — it uses `rax`, `rcx`,
/// `rdx` and the argument registers — so an allocated local is never disturbed
/// between two MIR statements.
struct RegisterFile {
    wide: &'static [&'static str],
    /// The 32-bit views of `wide`, index for index, for `i32` arithmetic.
    narrow: &'static [&'static str],
    /// How many leading entries of `wide` are caller-saved, and so need no
    /// prologue save. The allocator hands these out first.
    volatile: usize,
}

/// Registers for a function that calls something.
///
/// All five are callee-saved, which is the point: a local that survives a call
/// needs no save around the call, so allocation needs no notion of a clobber
/// set at all. The price is one save and one restore per register per function.
const CALLEE_SAVED: RegisterFile = RegisterFile {
    wide: &["rbx", "r12", "r13", "r14", "r15"],
    narrow: &["ebx", "r12d", "r13d", "r14d", "r15d"],
    volatile: 0,
};

/// Registers for a function that calls nothing.
///
/// `r10` and `r11` are caller-saved and this generator never touches them, so
/// in a function with no call they are free outright — no prologue, no
/// epilogue. They are also never argument registers, so storing a parameter
/// into one cannot overwrite a parameter that has not been stored yet.
///
/// A jump to a panic trampoline does reach a `call`, but that call never
/// returns, so what it clobbers cannot be observed.
const LEAF: RegisterFile = RegisterFile {
    wide: &["r10", "r11", "rbx", "r12", "r13", "r14", "r15"],
    narrow: &["r10d", "r11d", "ebx", "r12d", "r13d", "r14d", "r15d"],
    volatile: 2,
};

#[derive(Clone, Copy, Debug)]
pub struct TargetSpec {
    pub triple: &'static str,
    pub architecture: &'static str,
    pub abi: &'static str,
    pub object_format: &'static str,
    pub default_cc: &'static str,
}

pub const X86_64_LINUX_GNU: TargetSpec = TargetSpec {
    triple: SUPPORTED_TARGET,
    architecture: "x86_64",
    abi: "System V AMD64",
    object_format: "ELF",
    default_cc: "cc",
};

pub trait Backend {
    fn target(&self) -> &'static TargetSpec;
    fn emit(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<String>;
}

pub struct X86_64Backend;

impl Backend for X86_64Backend {
    fn target(&self) -> &'static TargetSpec {
        &X86_64_LINUX_GNU
    }

    fn emit(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<String> {
        Generator::new(file, module, options).generate()
    }
}

#[derive(Clone, Debug)]
pub struct CodegenOptions {
    pub target: String,
    pub test_harness: bool,
    pub emit_entrypoint: bool,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            target: SUPPORTED_TARGET.into(),
            test_harness: false,
            emit_entrypoint: true,
        }
    }
}

pub fn emit_assembly(
    file: &str,
    module: &MirModule,
    options: &CodegenOptions,
) -> CompileResult<String> {
    if options.target != SUPPORTED_TARGET {
        return Err(vec![Diagnostic::error(
            codes::UNSUPPORTED_TARGET,
            file,
            Default::default(),
            format!("unsupported target `{}`", options.target),
        )
        .with_help(format!(
            "the current backend supports `{SUPPORTED_TARGET}`"
        ))]);
    }
    X86_64Backend.emit(file, module, options)
}

struct Generator<'a> {
    file: &'a str,
    module: &'a MirModule,
    options: &'a CodegenOptions,
    output: String,
    strings: Vec<(String, String)>,
    string_ids: HashMap<String, String>,
    diagnostics: Vec<Diagnostic>,
    /// Where the locals of the function currently being emitted live, and the
    /// register set it draws on. The helper functions below read both, so both
    /// are replaced per function.
    alloc: Allocation,
    registers: &'static RegisterFile,
}

impl<'a> Generator<'a> {
    fn new(file: &'a str, module: &'a MirModule, options: &'a CodegenOptions) -> Self {
        Self {
            file,
            module,
            options,
            output: String::new(),
            strings: Vec::new(),
            string_ids: HashMap::new(),
            diagnostics: Vec::new(),
            alloc: Allocation::stack_only(0),
            registers: &CALLEE_SAVED,
        }
    }

    fn generate(mut self) -> CompileResult<String> {
        self.collect_strings();
        writeln!(self.output, ".intel_syntax noprefix").unwrap();
        writeln!(self.output, ".section .rodata").unwrap();
        self.byte_string(".Lsl_panic_div_zero", b"division by zero");
        self.byte_string(".Lsl_panic_overflow", b"integer overflow");
        for (label, value) in self.strings.clone() {
            self.byte_string(&label, value.as_bytes());
        }
        writeln!(self.output, ".text").unwrap();

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
        writeln!(self.output, ".section .note.GNU-stack,\"\",@progbits").unwrap();

        if self.diagnostics.is_empty() {
            Ok(remove_redundant_copies(&self.output))
        } else {
            Err(self.diagnostics)
        }
    }

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
        writeln!(self.output, "{label}:").unwrap();
        write!(self.output, "  .byte ").unwrap();
        for (index, byte) in bytes.iter().chain(std::iter::once(&0)).enumerate() {
            if index != 0 {
                write!(self.output, ", ").unwrap();
            }
            write!(self.output, "{byte}").unwrap();
        }
        writeln!(self.output).unwrap();
    }

    fn function(&mut self, function: &MirFunction, is_test: bool) {
        let symbol = self.symbol(&function.name, is_test);
        let epilogue = format!(".L{}_epilogue", symbol);

        let cfg = Cfg::new(function);
        self.registers = if self.calls_something(function) {
            &CALLEE_SAVED
        } else {
            &LEAF
        };
        self.alloc = allocate(
            function,
            &cfg,
            self.registers.wide.len(),
            &address_taken(function),
        );

        // The saved registers sit above the locals, so a local's slot index is
        // independent of how many registers this function happens to use.
        let saved: Vec<usize> = self
            .alloc
            .used_registers()
            .iter()
            .copied()
            .filter(|register| *register >= self.registers.volatile)
            .collect();
        let save_base = self.alloc.memory_slots();
        let frame_size = align_to((save_base + saved.len()) * 8, 16);

        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        if frame_size != 0 {
            writeln!(self.output, "  sub rsp, {frame_size}").unwrap();
        }
        for (index, register) in saved.iter().enumerate() {
            writeln!(
                self.output,
                "  mov QWORD PTR {}, {}",
                frame_slot(save_base + index),
                self.registers.wide[*register]
            )
            .unwrap();
        }

        self.store_parameters(function);
        for (block_id, block) in function.blocks.iter().enumerate() {
            writeln!(self.output, ".L{}_bb{}:", symbol, block_id).unwrap();
            self.basic_block(function, block, &symbol, &epilogue);
        }

        writeln!(self.output, "{epilogue}:").unwrap();
        for (index, register) in saved.iter().enumerate() {
            writeln!(
                self.output,
                "  mov {}, QWORD PTR {}",
                self.registers.wide[*register],
                frame_slot(save_base + index)
            )
            .unwrap();
        }
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
    }

    /// Whether generated code for this function contains a `call` that
    /// returns, which decides whether caller-saved registers are usable.
    ///
    /// Deliberately conservative: it answers for the instruction, not for the
    /// exact operand types, so the only way to be wrong is to claim a call that
    /// is not emitted. That costs an allocation opportunity; the reverse would
    /// cost correctness.
    fn calls_something(&self, function: &MirFunction) -> bool {
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
                Instruction::Drop { ty, .. } => self.drop_function(ty).is_some(),
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

    fn store_parameters(&mut self, function: &MirFunction) {
        let integer_regs = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
        let float_regs = [
            "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
        ];
        let mut integers = 0;
        let mut floats = 0;
        let mut stack = 0;
        for local in &function.params {
            let ty = &function.locals[*local].ty;
            if *ty == Type::F64 {
                if floats >= float_regs.len() {
                    writeln!(self.output, "  mov rax, QWORD PTR [rbp+{}]", 16 + stack * 8).unwrap();
                    writeln!(
                        self.output,
                        "  mov {}, rax",
                        operand(&self.alloc, self.registers, *local)
                    )
                    .unwrap();
                    stack += 1;
                } else {
                    writeln!(
                        self.output,
                        "  movq {}, {}",
                        operand(&self.alloc, self.registers, *local),
                        float_regs[floats]
                    )
                    .unwrap();
                    floats += 1;
                }
            } else {
                if integers >= integer_regs.len() {
                    writeln!(self.output, "  mov rax, QWORD PTR [rbp+{}]", 16 + stack * 8).unwrap();
                    writeln!(
                        self.output,
                        "  mov {}, rax",
                        operand(&self.alloc, self.registers, *local)
                    )
                    .unwrap();
                    stack += 1;
                } else {
                    writeln!(
                        self.output,
                        "  mov {}, {}",
                        operand(&self.alloc, self.registers, *local),
                        integer_regs[integers]
                    )
                    .unwrap();
                    integers += 1;
                }
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
        for instruction in block.instructions() {
            self.instruction(function, instruction);
        }
        match &block.terminator {
            Terminator::Return(value) => {
                if let Some(local) = value {
                    if function.locals[*local].ty == Type::F64 {
                        writeln!(
                            self.output,
                            "  movq xmm0, {}",
                            operand(&self.alloc, self.registers, *local)
                        )
                        .unwrap();
                    } else {
                        writeln!(
                            self.output,
                            "  mov rax, {}",
                            operand(&self.alloc, self.registers, *local)
                        )
                        .unwrap();
                    }
                } else {
                    writeln!(self.output, "  xor eax, eax").unwrap();
                }
                writeln!(self.output, "  jmp {epilogue}").unwrap();
            }
            Terminator::Goto(target) => {
                writeln!(self.output, "  jmp .L{}_bb{}", symbol, target).unwrap();
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                writeln!(
                    self.output,
                    "  cmp {}, 0",
                    operand(&self.alloc, self.registers, *condition)
                )
                .unwrap();
                writeln!(self.output, "  jne .L{}_bb{}", symbol, then_block).unwrap();
                writeln!(self.output, "  jmp .L{}_bb{}", symbol, else_block).unwrap();
            }
            Terminator::Unreachable => writeln!(self.output, "  ud2").unwrap(),
        }
    }

    /// Materializes a 64-bit immediate into a local.
    ///
    /// x86-64 has no store of a 64-bit immediate to memory, so a memory
    /// destination still needs the trip through `rax`. A register destination
    /// takes the immediate directly.
    fn load_immediate(&mut self, dst: LocalId, value: &str) {
        if in_memory(&self.alloc, dst) {
            writeln!(self.output, "  mov rax, {value}").unwrap();
            writeln!(
                self.output,
                "  mov {}, rax",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
        } else {
            writeln!(
                self.output,
                "  mov {}, {value}",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
        }
    }

    /// Copies one local to another, going through `rax` only when both ends are
    /// in memory — `mov` allows at most one memory operand.
    fn copy(&mut self, dst: LocalId, src: LocalId) {
        if in_memory(&self.alloc, dst) && in_memory(&self.alloc, src) {
            writeln!(
                self.output,
                "  mov rax, {}",
                operand(&self.alloc, self.registers, src)
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov {}, rax",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
        } else {
            writeln!(
                self.output,
                "  mov {}, {}",
                operand(&self.alloc, self.registers, dst),
                operand(&self.alloc, self.registers, src)
            )
            .unwrap();
        }
    }

    fn instruction(&mut self, function: &MirFunction, instruction: &Instruction) {
        match instruction {
            Instruction::ConstInt { dst, value } => self.load_immediate(*dst, &value.to_string()),
            Instruction::ConstFloat { dst, bits } => self.load_immediate(*dst, &bits.to_string()),
            Instruction::ConstBool { dst, value } => {
                writeln!(
                    self.output,
                    "  mov {}, {}",
                    operand(&self.alloc, self.registers, *dst),
                    i32::from(*value)
                )
                .unwrap();
            }
            Instruction::StringNew { dst, value } => {
                let label = self.string_ids[value].clone();
                writeln!(self.output, "  lea rdi, {label}[rip]").unwrap();
                writeln!(self.output, "  mov rsi, {}", value.len()).unwrap();
                writeln!(self.output, "  call sl_rt_string_new").unwrap();
                writeln!(
                    self.output,
                    "  mov {}, rax",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::Assign { dst, src } => self.copy(*dst, *src),
            Instruction::AddressOf { dst, src } => {
                // Borrowing a pointer-shaped value copies the pointer; anything
                // else needs the address of its slot, which is why
                // `address_taken` pins those locals to memory.
                if function
                    .locals
                    .get(*src)
                    .is_some_and(|local| is_pointer_like(&local.ty))
                {
                    self.copy(*dst, *src);
                } else {
                    writeln!(self.output, "  lea rax, {}", address(&self.alloc, *src)).unwrap();
                    writeln!(
                        self.output,
                        "  mov {}, rax",
                        operand(&self.alloc, self.registers, *dst)
                    )
                    .unwrap();
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
            } => {
                self.call(*dst, callee, args, arg_types, result);
            }
            Instruction::Drop { local, ty } => {
                writeln!(
                    self.output,
                    "  mov rdi, {}",
                    operand(&self.alloc, self.registers, *local)
                )
                .unwrap();
                match ty {
                    Type::String => writeln!(self.output, "  call sl_rt_string_drop").unwrap(),
                    Type::List(_) | Type::Array { .. } => {
                        writeln!(self.output, "  call sl_rt_list_drop").unwrap()
                    }
                    Type::Slice(_) => writeln!(self.output, "  call sl_rt_slice_drop").unwrap(),
                    Type::Named(name)
                        if self.module.structs.iter().any(|item| &item.name == name) =>
                    {
                        writeln!(self.output, "  call {}", struct_drop_symbol(name)).unwrap()
                    }
                    Type::Named(name)
                        if self.module.enums.iter().any(|item| &item.name == name) =>
                    {
                        writeln!(self.output, "  call {}", enum_drop_symbol(name)).unwrap()
                    }
                    _ => {}
                }
                writeln!(
                    self.output,
                    "  mov {}, 0",
                    operand(&self.alloc, self.registers, *local)
                )
                .unwrap();
            }
            Instruction::StructNew { dst, name, fields } => {
                let size = self
                    .module
                    .structs
                    .iter()
                    .find(|item| &item.name == name)
                    .map(|item| item.fields.len() * 8)
                    .unwrap_or(0)
                    .max(8);
                writeln!(self.output, "  mov rdi, {size}").unwrap();
                writeln!(self.output, "  call sl_rt_alloc").unwrap();
                for (index, field) in fields.iter().enumerate() {
                    writeln!(
                        self.output,
                        "  mov rcx, {}",
                        operand(&self.alloc, self.registers, *field)
                    )
                    .unwrap();
                    writeln!(self.output, "  mov QWORD PTR [rax+{}], rcx", index * 8).unwrap();
                }
                writeln!(
                    self.output,
                    "  mov {}, rax",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::FieldLoad { dst, base, index } => {
                writeln!(
                    self.output,
                    "  mov rax, {}",
                    operand(&self.alloc, self.registers, *base)
                )
                .unwrap();
                writeln!(self.output, "  mov rcx, QWORD PTR [rax+{}]", index * 8).unwrap();
                writeln!(
                    self.output,
                    "  mov {}, rcx",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::EnumNew {
                dst, tag, fields, ..
            } => {
                let size = ((fields.len() + 1) * 8).max(8);
                writeln!(self.output, "  mov rdi, {size}").unwrap();
                writeln!(self.output, "  call sl_rt_alloc").unwrap();
                writeln!(self.output, "  mov QWORD PTR [rax], {tag}").unwrap();
                for (index, field) in fields.iter().enumerate() {
                    writeln!(
                        self.output,
                        "  mov rcx, {}",
                        operand(&self.alloc, self.registers, *field)
                    )
                    .unwrap();
                    writeln!(
                        self.output,
                        "  mov QWORD PTR [rax+{}], rcx",
                        (index + 1) * 8
                    )
                    .unwrap();
                }
                writeln!(
                    self.output,
                    "  mov {}, rax",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::EnumTag { dst, base } => {
                writeln!(
                    self.output,
                    "  mov rax, {}",
                    operand(&self.alloc, self.registers, *base)
                )
                .unwrap();
                writeln!(self.output, "  mov rcx, QWORD PTR [rax]").unwrap();
                writeln!(
                    self.output,
                    "  mov {}, rcx",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::EnumFieldLoad { dst, base, index } => {
                writeln!(
                    self.output,
                    "  mov rax, {}",
                    operand(&self.alloc, self.registers, *base)
                )
                .unwrap();
                writeln!(
                    self.output,
                    "  mov rcx, QWORD PTR [rax+{}]",
                    (index + 1) * 8
                )
                .unwrap();
                writeln!(
                    self.output,
                    "  mov {}, rcx",
                    operand(&self.alloc, self.registers, *dst)
                )
                .unwrap();
            }
            Instruction::Free { local } => {
                writeln!(
                    self.output,
                    "  mov rdi, {}",
                    operand(&self.alloc, self.registers, *local)
                )
                .unwrap();
                writeln!(self.output, "  call sl_rt_free").unwrap();
                writeln!(
                    self.output,
                    "  mov {}, 0",
                    operand(&self.alloc, self.registers, *local)
                )
                .unwrap();
            }
        }
    }

    fn integer_binary(
        &mut self,
        dst: LocalId,
        op: BinaryOp,
        lhs: LocalId,
        rhs: LocalId,
        ty: &Type,
    ) {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                if self.accumulate_in_place(dst, op, lhs, rhs, ty) => {}
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal
                if self.compare_in_place(dst, op, lhs, rhs) => {}
            _ => self.integer_binary_through_rax(dst, op, lhs, rhs, ty),
        }
    }

    /// Computes an addition, subtraction or multiplication straight into the
    /// destination register, so the result never travels through `rax`.
    ///
    /// Returns false when the shape does not allow it: a destination in memory
    /// (`add [slot], [slot]` has two memory operands), or a destination that is
    /// also the right-hand operand (the initial copy would overwrite it).
    fn accumulate_in_place(
        &mut self,
        dst: LocalId,
        op: BinaryOp,
        lhs: LocalId,
        rhs: LocalId,
        ty: &Type,
    ) -> bool {
        let Location::Register(register) = self.alloc.location(dst) else {
            return false;
        };
        if self.alloc.location(dst) == self.alloc.location(rhs) {
            return false;
        }
        if self.alloc.location(dst) != self.alloc.location(lhs) {
            self.copy(dst, lhs);
        }
        let narrow = *ty == Type::I32;
        let (target, source) = if narrow {
            (
                self.registers.narrow[register].to_owned(),
                narrow_operand(&self.alloc, self.registers, rhs),
            )
        } else {
            (
                self.registers.wide[register].to_owned(),
                operand(&self.alloc, self.registers, rhs),
            )
        };
        let mnemonic = match op {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "imul",
            _ => unreachable!("only the accumulating operators reach here"),
        };
        writeln!(self.output, "  {mnemonic} {target}, {source}").unwrap();
        writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
        if narrow {
            // The 32-bit form zero-extends into the full register; the local's
            // value is a sign-extended i32.
            writeln!(
                self.output,
                "  movsxd {}, {target}",
                self.registers.wide[register]
            )
            .unwrap();
        }
        true
    }

    /// Compares without loading either side into `rax` first, and widens the
    /// flag byte straight into the destination.
    ///
    /// Returns false when both operands sit in memory, which `cmp` cannot
    /// encode.
    fn compare_in_place(&mut self, dst: LocalId, op: BinaryOp, lhs: LocalId, rhs: LocalId) -> bool {
        if in_memory(&self.alloc, lhs) && in_memory(&self.alloc, rhs) {
            return false;
        }
        writeln!(
            self.output,
            "  cmp {}, {}",
            operand(&self.alloc, self.registers, lhs),
            operand(&self.alloc, self.registers, rhs)
        )
        .unwrap();
        writeln!(self.output, "  {} al", set_condition(op)).unwrap();
        match self.alloc.location(dst) {
            Location::Register(register) => {
                writeln!(self.output, "  movzx {}, al", self.registers.wide[register]).unwrap()
            }
            Location::Memory(_) => {
                writeln!(self.output, "  movzx rax, al").unwrap();
                writeln!(
                    self.output,
                    "  mov {}, rax",
                    operand(&self.alloc, self.registers, dst)
                )
                .unwrap();
            }
        }
        true
    }

    fn integer_binary_through_rax(
        &mut self,
        dst: LocalId,
        op: BinaryOp,
        lhs: LocalId,
        rhs: LocalId,
        ty: &Type,
    ) {
        writeln!(
            self.output,
            "  mov rax, {}",
            operand(&self.alloc, self.registers, lhs)
        )
        .unwrap();
        writeln!(
            self.output,
            "  mov rcx, {}",
            operand(&self.alloc, self.registers, rhs)
        )
        .unwrap();
        let width = if *ty == Type::I32 { "e" } else { "r" };
        let accumulator = if width == "e" { "eax" } else { "rax" };
        let argument = if width == "e" { "ecx" } else { "rcx" };
        match op {
            BinaryOp::Add => {
                writeln!(self.output, "  add {accumulator}, {argument}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Sub => {
                writeln!(self.output, "  sub {accumulator}, {argument}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Mul => {
                writeln!(self.output, "  imul {accumulator}, {argument}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Div => {
                writeln!(self.output, "  test {argument}, {argument}").unwrap();
                writeln!(self.output, "  je .Lsl_panic_div_zero_trampoline").unwrap();
                if *ty == Type::I32 {
                    writeln!(self.output, "  cmp eax, -2147483648").unwrap();
                    writeln!(self.output, "  jne 1f").unwrap();
                    writeln!(self.output, "  cmp ecx, -1").unwrap();
                    writeln!(self.output, "  je .Lsl_panic_overflow_trampoline").unwrap();
                    writeln!(self.output, "1:").unwrap();
                    writeln!(self.output, "  cdq").unwrap();
                    writeln!(self.output, "  idiv ecx").unwrap();
                } else {
                    writeln!(self.output, "  mov rdx, -9223372036854775808").unwrap();
                    writeln!(self.output, "  cmp rax, rdx").unwrap();
                    writeln!(self.output, "  jne 1f").unwrap();
                    writeln!(self.output, "  cmp rcx, -1").unwrap();
                    writeln!(self.output, "  je .Lsl_panic_overflow_trampoline").unwrap();
                    writeln!(self.output, "1:").unwrap();
                    writeln!(self.output, "  cqo").unwrap();
                    writeln!(self.output, "  idiv rcx").unwrap();
                }
            }
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal => {
                writeln!(self.output, "  cmp rax, rcx").unwrap();
                writeln!(self.output, "  {} al", set_condition(op)).unwrap();
                writeln!(self.output, "  movzx rax, al").unwrap();
            }
        }
        if *ty == Type::I32 && !matches!(op, BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal) {
            writeln!(self.output, "  movsxd rax, eax").unwrap();
        }
        writeln!(
            self.output,
            "  mov {}, rax",
            operand(&self.alloc, self.registers, dst)
        )
        .unwrap();
    }

    fn float_binary(&mut self, dst: LocalId, op: BinaryOp, lhs: LocalId, rhs: LocalId) {
        writeln!(
            self.output,
            "  movq xmm0, {}",
            operand(&self.alloc, self.registers, lhs)
        )
        .unwrap();
        writeln!(
            self.output,
            "  movq xmm1, {}",
            operand(&self.alloc, self.registers, rhs)
        )
        .unwrap();
        match op {
            BinaryOp::Add => writeln!(self.output, "  addsd xmm0, xmm1").unwrap(),
            BinaryOp::Sub => writeln!(self.output, "  subsd xmm0, xmm1").unwrap(),
            BinaryOp::Mul => writeln!(self.output, "  mulsd xmm0, xmm1").unwrap(),
            BinaryOp::Div => writeln!(self.output, "  divsd xmm0, xmm1").unwrap(),
            BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal => {
                writeln!(self.output, "  ucomisd xmm0, xmm1").unwrap();
                let condition = match op {
                    BinaryOp::Less => "setb",
                    BinaryOp::Greater => "seta",
                    BinaryOp::Equal => "sete",
                    _ => unreachable!(),
                };
                writeln!(self.output, "  {condition} al").unwrap();
                writeln!(self.output, "  movzx rax, al").unwrap();
                writeln!(self.output, "  movq xmm0, rax").unwrap();
            }
        }
        writeln!(
            self.output,
            "  movq {}, xmm0",
            operand(&self.alloc, self.registers, dst)
        )
        .unwrap();
    }

    fn call(
        &mut self,
        dst: LocalId,
        callee: &str,
        args: &[LocalId],
        arg_types: &[Type],
        result: &Type,
    ) {
        if callee == "clone" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            if let Some(clone_function) = self.clone_function(&arg_types[0]) {
                writeln!(self.output, "  call {clone_function}").unwrap();
            } else {
                writeln!(self.output, "  mov rax, rdi").unwrap();
            }
        } else if matches!(callee, "list" | "array") {
            writeln!(self.output, "  mov rdi, 8").unwrap();
            let element = match result {
                Type::List(element) => element.as_ref(),
                Type::Array { element, .. } => element.as_ref(),
                _ => unreachable!("collection constructor must return List or Array"),
            };
            if let Some(drop_function) = self.drop_function(element) {
                writeln!(self.output, "  lea rsi, {drop_function}[rip]").unwrap();
            } else {
                writeln!(self.output, "  xor esi, esi").unwrap();
            }
            if let Some(clone_function) = self.clone_function(element) {
                writeln!(self.output, "  lea rdx, {clone_function}[rip]").unwrap();
            } else {
                writeln!(self.output, "  xor edx, edx").unwrap();
            }
            writeln!(self.output, "  call sl_rt_list_new").unwrap();
            writeln!(
                self.output,
                "  mov {}, rax",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
            for arg in args {
                writeln!(
                    self.output,
                    "  mov rdi, {}",
                    operand(&self.alloc, self.registers, dst)
                )
                .unwrap();
                writeln!(self.output, "  lea rsi, {}", address(&self.alloc, *arg)).unwrap();
                writeln!(self.output, "  call sl_rt_list_push").unwrap();
            }
            writeln!(
                self.output,
                "  mov rax, {}",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
        } else if callee == "slice" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov rsi, {}",
                operand(&self.alloc, self.registers, args[1])
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov rdx, {}",
                operand(&self.alloc, self.registers, args[2])
            )
            .unwrap();
            writeln!(self.output, "  call sl_rt_slice_new").unwrap();
        } else if callee == "len" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            if reference_is_slice(&arg_types[0]) {
                writeln!(self.output, "  call sl_rt_slice_len").unwrap();
            } else {
                writeln!(self.output, "  call sl_rt_list_len").unwrap();
            }
        } else if callee == "push" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(self.output, "  lea rsi, {}", address(&self.alloc, args[1])).unwrap();
            writeln!(self.output, "  call sl_rt_list_push").unwrap();
        } else if callee == "get" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov rsi, {}",
                operand(&self.alloc, self.registers, args[1])
            )
            .unwrap();
            if reference_is_slice(&arg_types[0]) {
                writeln!(self.output, "  call sl_rt_slice_get").unwrap();
            } else {
                writeln!(self.output, "  call sl_rt_list_get").unwrap();
            }
            writeln!(self.output, "  mov rax, QWORD PTR [rax]").unwrap();
        } else if callee == "get-ref" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov rsi, {}",
                operand(&self.alloc, self.registers, args[1])
            )
            .unwrap();
            if reference_is_slice(&arg_types[0]) {
                writeln!(self.output, "  call sl_rt_slice_get").unwrap();
            } else {
                writeln!(self.output, "  call sl_rt_list_get").unwrap();
            }
            if matches!(
                result,
                Type::Ref { inner, .. }
                    if matches!(
                        inner.as_ref(),
                        Type::String
                            | Type::List(_)
                            | Type::Array { .. }
                            | Type::Slice(_)
                            | Type::Named(_)
                    )
            ) {
                writeln!(self.output, "  mov rax, QWORD PTR [rax]").unwrap();
            }
        } else if callee == "pop" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(self.output, "  lea rsi, {}", address(&self.alloc, dst)).unwrap();
            writeln!(self.output, "  call sl_rt_list_try_pop").unwrap();
            let Type::Named(option_name) = result else {
                unreachable!("pop must return Option<T>");
            };
            let option = self
                .module
                .enums
                .iter()
                .find(|item| &item.name == option_name)
                .expect("Option layout must be present");
            let none_tag = option
                .variants
                .iter()
                .find(|variant| variant.name == "None")
                .map(|variant| variant.tag)
                .expect("Option must define None");
            let some_tag = option
                .variants
                .iter()
                .find(|variant| variant.name == "Some")
                .map(|variant| variant.tag)
                .expect("Option must define Some");
            writeln!(self.output, "  test rax, rax").unwrap();
            writeln!(self.output, "  jz 1f").unwrap();
            writeln!(self.output, "  mov rdi, 16").unwrap();
            writeln!(self.output, "  call sl_rt_alloc").unwrap();
            writeln!(self.output, "  mov QWORD PTR [rax], {some_tag}").unwrap();
            writeln!(
                self.output,
                "  mov rcx, {}",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap();
            writeln!(self.output, "  mov QWORD PTR [rax+8], rcx").unwrap();
            writeln!(self.output, "  jmp 2f").unwrap();
            writeln!(self.output, "1:").unwrap();
            writeln!(self.output, "  mov rdi, 8").unwrap();
            writeln!(self.output, "  call sl_rt_alloc").unwrap();
            writeln!(self.output, "  mov QWORD PTR [rax], {none_tag}").unwrap();
            writeln!(self.output, "2:").unwrap();
        } else if callee == "remove" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(
                self.output,
                "  mov rsi, {}",
                operand(&self.alloc, self.registers, args[1])
            )
            .unwrap();
            writeln!(self.output, "  call sl_rt_list_remove").unwrap();
        } else if callee == "read-i64" {
            writeln!(self.output, "  call sl_rt_read_i64").unwrap();
        } else if callee == "read-line" {
            writeln!(self.output, "  call sl_rt_read_line").unwrap();
        } else if callee == "parse-i64" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(self.output, "  call sl_rt_parse_i64").unwrap();
        } else if callee == "env" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(self.output, "  call sl_rt_env").unwrap();
        } else if callee == "args-len" {
            writeln!(self.output, "  call sl_rt_args_len").unwrap();
        } else if callee == "arg" {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            writeln!(self.output, "  call sl_rt_arg").unwrap();
        } else if matches!(callee, "print" | "println") {
            writeln!(
                self.output,
                "  mov rdi, {}",
                operand(&self.alloc, self.registers, args[0])
            )
            .unwrap();
            let string = match &arg_types[0] {
                Type::String => true,
                Type::Ref { inner, .. } => inner.as_ref() == &Type::String,
                _ => false,
            };
            let suffix = if callee == "println" {
                "println"
            } else {
                "print"
            };
            let runtime = if string {
                format!("sl_rt_{suffix}_string")
            } else {
                format!("sl_rt_{suffix}_i64")
            };
            writeln!(self.output, "  call {runtime}").unwrap();
        } else {
            let integer_regs = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
            let float_regs = [
                "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
            ];
            let mut integers = 0;
            let mut floats = 0;
            let mut stack_args = Vec::new();
            for (arg, ty) in args.iter().zip(arg_types) {
                if *ty == Type::F64 {
                    if floats >= float_regs.len() {
                        stack_args.push((*arg, ty));
                    } else {
                        writeln!(
                            self.output,
                            "  movq {}, {}",
                            float_regs[floats],
                            operand(&self.alloc, self.registers, *arg)
                        )
                        .unwrap();
                        floats += 1;
                    }
                } else {
                    if integers >= integer_regs.len() {
                        stack_args.push((*arg, ty));
                    } else {
                        writeln!(
                            self.output,
                            "  mov {}, {}",
                            integer_regs[integers],
                            operand(&self.alloc, self.registers, *arg)
                        )
                        .unwrap();
                        integers += 1;
                    }
                }
            }
            let padding = usize::from(stack_args.len() % 2 != 0);
            if padding != 0 {
                writeln!(self.output, "  sub rsp, 8").unwrap();
            }
            for (arg, _) in stack_args.iter().rev() {
                writeln!(
                    self.output,
                    "  push {}",
                    operand(&self.alloc, self.registers, *arg)
                )
                .unwrap();
            }
            writeln!(self.output, "  call {}", self.symbol(callee, false)).unwrap();
            let cleanup = (stack_args.len() + padding) * 8;
            if cleanup != 0 {
                writeln!(self.output, "  add rsp, {cleanup}").unwrap();
            }
        }
        match result {
            Type::Unit => {}
            Type::F64 => writeln!(
                self.output,
                "  movq {}, xmm0",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap(),
            _ => writeln!(
                self.output,
                "  mov {}, rax",
                operand(&self.alloc, self.registers, dst)
            )
            .unwrap(),
        }
    }

    fn test_harness(&mut self) {
        writeln!(self.output, ".globl main").unwrap();
        writeln!(self.output, ".type main, @function").unwrap();
        writeln!(self.output, "main:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-16], rsi").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        writeln!(self.output, "  call sl_rt_args_init").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], 0").unwrap();
        for (index, test) in self.module.tests.iter().enumerate() {
            let name = self.string_ids[&test.name].clone();
            writeln!(
                self.output,
                "  call {}",
                self.symbol(&test.function.name, true)
            )
            .unwrap();
            writeln!(self.output, "  mov esi, eax").unwrap();
            writeln!(self.output, "  lea rdi, {name}[rip]").unwrap();
            writeln!(self.output, "  call sl_rt_test_result").unwrap();
            writeln!(self.output, "  add QWORD PTR [rbp-8], rax").unwrap();
            let _ = index;
        }
        writeln!(self.output, "  mov rax, QWORD PTR [rbp-8]").unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size main, .-main").unwrap();
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
        writeln!(self.output, ".globl main").unwrap();
        writeln!(self.output, ".type main, @function").unwrap();
        writeln!(self.output, "main:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-16], rsi").unwrap();
        writeln!(self.output, "  call sl_rt_args_init").unwrap();
        writeln!(self.output, "  call {}", self.symbol(&main.name, false)).unwrap();
        if main.return_type == Type::Unit {
            writeln!(self.output, "  xor eax, eax").unwrap();
        }
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size main, .-main").unwrap();
    }

    fn runtime_panic_trampolines(&mut self) {
        writeln!(self.output, ".Lsl_panic_div_zero_trampoline:").unwrap();
        writeln!(self.output, "  lea rdi, .Lsl_panic_div_zero[rip]").unwrap();
        writeln!(self.output, "  call sl_rt_panic").unwrap();
        writeln!(self.output, "  ud2").unwrap();
        writeln!(self.output, ".Lsl_panic_overflow_trampoline:").unwrap();
        writeln!(self.output, "  lea rdi, .Lsl_panic_overflow[rip]").unwrap();
        writeln!(self.output, "  call sl_rt_panic").unwrap();
        writeln!(self.output, "  ud2").unwrap();
    }

    fn struct_clone_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_clone_symbol(name);
        let size = (fields.len() * 8).max(8);
        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        writeln!(self.output, "  mov rdi, {size}").unwrap();
        writeln!(self.output, "  call sl_rt_alloc").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-16], rax").unwrap();
        for (index, (_, ty)) in fields.iter().enumerate() {
            writeln!(self.output, "  mov rax, QWORD PTR [rbp-8]").unwrap();
            writeln!(self.output, "  mov rdi, QWORD PTR [rax+{}]", index * 8).unwrap();
            if let Some(clone_function) = self.clone_function(ty) {
                writeln!(self.output, "  call {clone_function}").unwrap();
            } else {
                writeln!(self.output, "  mov rax, rdi").unwrap();
            }
            writeln!(self.output, "  mov rcx, QWORD PTR [rbp-16]").unwrap();
            writeln!(self.output, "  mov QWORD PTR [rcx+{}], rax", index * 8).unwrap();
        }
        writeln!(self.output, "  mov rax, QWORD PTR [rbp-16]").unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
    }

    fn enum_clone_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_clone_symbol(name);
        let size = variants
            .iter()
            .map(|variant| (variant.fields.len() + 1) * 8)
            .max()
            .unwrap_or(8)
            .max(8);
        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        writeln!(self.output, "  mov rdi, {size}").unwrap();
        writeln!(self.output, "  call sl_rt_alloc").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-16], rax").unwrap();
        writeln!(self.output, "  mov rcx, QWORD PTR [rbp-8]").unwrap();
        writeln!(self.output, "  mov rcx, QWORD PTR [rcx]").unwrap();
        writeln!(self.output, "  mov QWORD PTR [rax], rcx").unwrap();
        for variant in variants {
            writeln!(self.output, "  cmp rcx, {}", variant.tag).unwrap();
            writeln!(
                self.output,
                "  je .L{}_clone_variant_{}",
                symbol, variant.tag
            )
            .unwrap();
        }
        writeln!(self.output, "  jmp .L{}_clone_return", symbol).unwrap();
        for variant in variants {
            writeln!(self.output, ".L{}_clone_variant_{}:", symbol, variant.tag).unwrap();
            for (index, (_, ty)) in variant.fields.iter().enumerate() {
                writeln!(self.output, "  mov rax, QWORD PTR [rbp-8]").unwrap();
                writeln!(
                    self.output,
                    "  mov rdi, QWORD PTR [rax+{}]",
                    (index + 1) * 8
                )
                .unwrap();
                if let Some(clone_function) = self.clone_function(ty) {
                    writeln!(self.output, "  call {clone_function}").unwrap();
                } else {
                    writeln!(self.output, "  mov rax, rdi").unwrap();
                }
                writeln!(self.output, "  mov rcx, QWORD PTR [rbp-16]").unwrap();
                writeln!(
                    self.output,
                    "  mov QWORD PTR [rcx+{}], rax",
                    (index + 1) * 8
                )
                .unwrap();
            }
            writeln!(self.output, "  jmp .L{}_clone_return", symbol).unwrap();
        }
        writeln!(self.output, ".L{}_clone_return:", symbol).unwrap();
        writeln!(self.output, "  mov rax, QWORD PTR [rbp-16]").unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
    }

    fn struct_drop_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_drop_symbol(name);
        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        // Match sl_rt_string_drop/sl_rt_list_drop: a null pointer is a no-op
        // rather than a wild load, so a dropped-and-zeroed slot stays benign.
        writeln!(self.output, "  test rdi, rdi").unwrap();
        writeln!(self.output, "  je .L{}_return", symbol).unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        for (index, (_, ty)) in fields.iter().enumerate().rev() {
            let drop_function = self.drop_function(ty);
            if let Some(drop_function) = drop_function {
                writeln!(self.output, "  mov rax, QWORD PTR [rbp-8]").unwrap();
                writeln!(self.output, "  mov rdi, QWORD PTR [rax+{}]", index * 8).unwrap();
                writeln!(self.output, "  call {drop_function}").unwrap();
            }
        }
        writeln!(self.output, "  mov rdi, QWORD PTR [rbp-8]").unwrap();
        writeln!(self.output, "  call sl_rt_free").unwrap();
        writeln!(self.output, ".L{}_return:", symbol).unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
    }

    fn enum_drop_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_drop_symbol(name);
        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        writeln!(self.output, "  sub rsp, 16").unwrap();
        // As in the struct helper: tolerate a null pointer instead of loading
        // the tag from address zero.
        writeln!(self.output, "  test rdi, rdi").unwrap();
        writeln!(self.output, "  je .L{}_return", symbol).unwrap();
        writeln!(self.output, "  mov QWORD PTR [rbp-8], rdi").unwrap();
        writeln!(self.output, "  mov rax, QWORD PTR [rdi]").unwrap();
        for variant in variants {
            writeln!(self.output, "  cmp rax, {}", variant.tag).unwrap();
            writeln!(self.output, "  je .L{}_variant_{}", symbol, variant.tag).unwrap();
        }
        writeln!(self.output, "  jmp .L{}_free", symbol).unwrap();
        for variant in variants {
            writeln!(self.output, ".L{}_variant_{}:", symbol, variant.tag).unwrap();
            for (index, (_, ty)) in variant.fields.iter().enumerate().rev() {
                if let Some(drop_function) = self.drop_function(ty) {
                    writeln!(self.output, "  mov rax, QWORD PTR [rbp-8]").unwrap();
                    writeln!(
                        self.output,
                        "  mov rdi, QWORD PTR [rax+{}]",
                        (index + 1) * 8
                    )
                    .unwrap();
                    writeln!(self.output, "  call {drop_function}").unwrap();
                }
            }
            writeln!(self.output, "  jmp .L{}_free", symbol).unwrap();
        }
        writeln!(self.output, ".L{}_free:", symbol).unwrap();
        writeln!(self.output, "  mov rdi, QWORD PTR [rbp-8]").unwrap();
        writeln!(self.output, "  call sl_rt_free").unwrap();
        writeln!(self.output, ".L{}_return:", symbol).unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
    }

    fn drop_function(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::String => Some("sl_rt_string_drop".to_owned()),
            Type::List(_) | Type::Array { .. } => Some("sl_rt_list_drop".to_owned()),
            Type::Slice(_) => Some("sl_rt_slice_drop".to_owned()),
            Type::Named(inner) if self.module.structs.iter().any(|item| &item.name == inner) => {
                Some(struct_drop_symbol(inner))
            }
            Type::Named(inner) if self.module.enums.iter().any(|item| &item.name == inner) => {
                Some(enum_drop_symbol(inner))
            }
            _ => None,
        }
    }

    fn clone_function(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::String => Some("sl_rt_string_clone".to_owned()),
            Type::List(_) | Type::Array { .. } => Some("sl_rt_list_clone".to_owned()),
            Type::Slice(_) => Some("sl_rt_slice_clone".to_owned()),
            Type::Named(inner) if self.module.structs.iter().any(|item| &item.name == inner) => {
                Some(struct_clone_symbol(inner))
            }
            Type::Named(inner) if self.module.enums.iter().any(|item| &item.name == inner) => {
                Some(enum_clone_symbol(inner))
            }
            _ => None,
        }
    }

    fn symbol(&self, name: &str, is_test: bool) -> String {
        let prefix = if is_test { "sl_test" } else { "sl_fn" };
        let encoded = name
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("{prefix}_{encoded}")
    }
}

fn reference_is_slice(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Ref { inner, .. } if matches!(inner.as_ref(), Type::Slice(_))
    )
}

/// A value that is represented by a pointer, so borrowing it copies the pointer
/// rather than taking the address of the slot holding it.
fn is_pointer_like(ty: &Type) -> bool {
    matches!(
        ty,
        Type::String | Type::List(_) | Type::Array { .. } | Type::Slice(_) | Type::Named(_)
    )
}

/// Locals whose frame address this backend hands to something else, and which
/// therefore cannot live in a register.
///
/// This must list exactly the operands passed to [`address`]: borrowing a
/// scalar, the elements a collection constructor copies in, the value pushed
/// onto a list, and the destination the runtime pops into.
fn address_taken(function: &MirFunction) -> Vec<bool> {
    let mut pinned = vec![false; function.locals.len()];
    let mut pin = |local: LocalId| {
        if let Some(entry) = pinned.get_mut(local) {
            *entry = true;
        }
    };
    for instruction in function
        .blocks
        .iter()
        .flat_map(|block| block.instructions())
    {
        match instruction {
            Instruction::AddressOf { src, .. } => {
                let scalar = function
                    .locals
                    .get(*src)
                    .is_some_and(|local| !is_pointer_like(&local.ty));
                if scalar {
                    pin(*src);
                }
            }
            Instruction::Call {
                dst, callee, args, ..
            } => match callee.as_str() {
                "list" | "array" => args.iter().copied().for_each(&mut pin),
                "push" => {
                    if let Some(value) = args.get(1) {
                        pin(*value);
                    }
                }
                "pop" => pin(*dst),
                _ => {}
            },
            _ => {}
        }
    }
    pinned
}

/// The assembly operand naming a local, ready to substitute into any
/// instruction that accepts a register or a 64-bit memory operand.
fn operand(allocation: &Allocation, file: &RegisterFile, local: LocalId) -> String {
    match allocation.location(local) {
        Location::Register(register) => file.wide[register].to_owned(),
        Location::Memory(slot) => format!("QWORD PTR {}", frame_slot(slot)),
    }
}

/// The address of a local, for the instructions that need one.
///
/// `lea` has no register form, so every local reaching here must have been
/// pinned to memory by [`address_taken`]. The two are kept in step by a test.
fn address(allocation: &Allocation, local: LocalId) -> String {
    match allocation.location(local) {
        Location::Memory(slot) => frame_slot(slot),
        Location::Register(_) => {
            unreachable!("local {local} has its address taken but was given a register")
        }
    }
}

fn frame_slot(slot: usize) -> String {
    format!("[rbp-{}]", (slot + 1) * 8)
}

fn in_memory(allocation: &Allocation, local: LocalId) -> bool {
    matches!(allocation.location(local), Location::Memory(_))
}

/// The 32-bit view of a local, for `i32` arithmetic.
fn narrow_operand(allocation: &Allocation, file: &RegisterFile, local: LocalId) -> String {
    match allocation.location(local) {
        Location::Register(register) => file.narrow[register].to_owned(),
        Location::Memory(slot) => format!("DWORD PTR {}", frame_slot(slot)),
    }
}

fn set_condition(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Less => "setl",
        BinaryOp::Greater => "setg",
        BinaryOp::Equal => "sete",
        _ => unreachable!("only the comparison operators produce a flag byte"),
    }
}

/// Deletes a `mov` that copies a value straight back where it just came from.
///
/// Instruction selection is per-MIR-statement and routes results through `rax`,
/// so a result written to a register and immediately read again — the shape of
/// `let x = ...` followed by `return x` — leaves a copy that undoes itself.
/// Only adjacent lines are considered, so a label or any other instruction in
/// between blocks the rewrite, and `mov` sets no flags, so removing one cannot
/// change what a following branch sees.
fn remove_redundant_copies(assembly: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for line in assembly.lines() {
        let undoes_previous = kept
            .last()
            .and_then(|previous| move_operands(previous))
            .zip(move_operands(line))
            .is_some_and(|((dst, src), (next_dst, next_src))| dst == next_src && src == next_dst);
        if undoes_previous {
            continue;
        }
        kept.push(line);
    }
    let mut output = kept.join("\n");
    output.push('\n');
    output
}

/// The destination and source of a plain `mov`, or `None` for anything else.
fn move_operands(line: &str) -> Option<(&str, &str)> {
    let operands = line.strip_prefix("  mov ")?;
    let (dst, src) = operands.split_once(", ")?;
    // A `mov` whose source is an immediate or a `[...]` expression that is not
    // a plain slot reference cannot be part of a mirrored pair anyway, but
    // rejecting nothing here is still correct: equality of the two operand
    // strings is what makes the pair redundant.
    Some((dst.trim(), src.trim()))
}

fn align_to(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn struct_drop_symbol(name: &str) -> String {
    let encoded = name
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sl_drop_struct_{encoded}")
}

fn struct_clone_symbol(name: &str) -> String {
    let encoded = name
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sl_clone_struct_{encoded}")
}

fn enum_drop_symbol(name: &str) -> String {
    let encoded = name
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sl_drop_enum_{encoded}")
}

fn enum_clone_symbol(name: &str) -> String {
    let encoded = name
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sl_clone_enum_{encoded}")
}

#[cfg(test)]
mod tests {
    use super::{address_taken, remove_redundant_copies, CALLEE_SAVED, LEAF};
    use crate::ast::Type;
    use crate::cfg::Cfg;
    use crate::mir::{BasicBlock, Instruction, MirFunction, MirLocal, Terminator};
    use crate::regalloc::{allocate, Location};
    use crate::{compile_to_assembly, CompileOptions};

    fn assemble(source: &str) -> String {
        compile_to_assembly("test.slp", source, &CompileOptions::default()).unwrap()
    }

    #[test]
    fn emits_native_function_and_checked_add() {
        let assembly = assemble("(fn main () -> i32 (+ 20 22))");
        assert!(assembly.contains(".globl main"));
        assert!(assembly.contains("jo .Lsl_panic_overflow_trampoline"));
        assert!(
            LEAF.narrow
                .iter()
                .any(|register| assembly.contains(&format!("add {register}, "))),
            "an i32 addition accumulates in a 32-bit register:\n{assembly}"
        );
    }

    /// The body of a named function, between its entry label and its epilogue.
    fn body_of<'a>(assembly: &'a str, name: &str) -> &'a str {
        let symbol: String = name.bytes().map(|byte| format!("{byte:02x}")).collect();
        assembly
            .split(&format!(".Lsl_fn_{symbol}_bb0:"))
            .nth(1)
            .and_then(|rest| rest.split(&format!(".Lsl_fn_{symbol}_epilogue:")).next())
            .expect("the body is delimited by the entry and epilogue labels")
    }

    #[test]
    fn scalar_locals_leave_the_frame() {
        let assembly = assemble(
            "(fn probe ((a i64) (b i64)) -> i64 (let c (+ a b)) (* c c))
             (fn main () -> i32 0)",
        );
        let body = body_of(&assembly, "probe");
        assert!(
            !body.contains("[rbp"),
            "no local should touch the frame:\n{body}"
        );
    }

    /// A function with no call may use the caller-saved registers outright.
    #[test]
    fn a_leaf_function_takes_volatile_registers_for_free() {
        let assembly = assemble(
            "(fn probe ((a i64)) -> i64 (* a a))
             (fn main () -> i32 0)",
        );
        let function = assembly
            .split(".globl sl_fn_70726f6265\n")
            .nth(1)
            .and_then(|rest| rest.split(".size").next())
            .expect("`probe` is emitted");
        assert!(
            LEAF.wide[..LEAF.volatile]
                .iter()
                .any(|register| function.contains(&format!("mov {register}, "))),
            "a leaf should reach for a volatile register first:\n{function}"
        );
        assert!(
            !function.contains("sub rsp,"),
            "a leaf using only volatile registers needs no frame:\n{function}"
        );
    }

    /// A function that calls must save whatever callee-saved registers it took.
    #[test]
    fn a_calling_function_saves_the_registers_it_allocates() {
        let assembly = assemble(
            "(fn helper ((a i64)) -> i64 (* a 2))
             (fn probe ((a i64) (b i64)) -> i64 (+ (helper a) (helper b)))
             (fn main () -> i32 0)",
        );
        let body = body_of(&assembly, "probe");
        let mut checked = 0;
        for register in CALLEE_SAVED.wide {
            if !body.contains(&format!("mov {register}, ")) {
                continue;
            }
            checked += 1;
            assert!(
                assembly.contains(&format!("], {register}\n")),
                "{register} is used but never saved:\n{assembly}"
            );
            assert!(
                assembly.contains(&format!("  mov {register}, QWORD PTR [rbp-")),
                "{register} is used but never restored:\n{assembly}"
            );
        }
        assert!(checked > 0, "`probe` allocated nothing:\n{assembly}");
        for register in LEAF.wide[..LEAF.volatile].iter() {
            assert!(
                !body.contains(&format!("mov {register}, ")),
                "a call would clobber {register}:\n{body}"
            );
        }
    }

    /// A local whose address is taken must stay in memory, because `lea` has no
    /// register form. The surface language cannot express a borrow of a scalar
    /// that survives to be used, so this builds the MIR directly.
    #[test]
    fn borrowing_a_scalar_pins_it_to_memory() {
        let mut function = MirFunction {
            name: "probe".into(),
            emit: true,
            params: Vec::new(),
            return_type: Type::I64,
            locals: vec![
                MirLocal {
                    ty: Type::I64,
                    name: None,
                    is_param: false,
                },
                MirLocal {
                    ty: Type::Ref {
                        inner: Box::new(Type::I64),
                        mutable: false,
                    },
                    name: None,
                    is_param: false,
                },
            ],
            blocks: Vec::new(),
            entry: 0,
            span: Default::default(),
        };
        function.blocks = vec![BasicBlock::synthetic(
            vec![
                Instruction::ConstInt { dst: 0, value: 1 },
                Instruction::AddressOf { dst: 1, src: 0 },
            ],
            Terminator::Return(Some(1)),
        )];

        let pinned = address_taken(&function);
        assert!(pinned[0], "the borrowed scalar must be pinned");
        assert!(!pinned[1], "the reference itself is an ordinary value");

        let cfg = Cfg::new(&function);
        let allocation = allocate(&function, &cfg, LEAF.wide.len(), &pinned);
        assert!(matches!(allocation.location(0), Location::Memory(_)));
    }

    /// Whatever the allocator decides, `lea` must never be handed a register.
    /// Checking the emitted text covers every construct the fixtures use,
    /// including the collection builtins that pass a slot address.
    #[test]
    fn no_lea_in_the_shipped_corpus_addresses_a_register() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/projects");
        let Ok(categories) = std::fs::read_dir(&root) else {
            // Absent from packaged source snapshots; nothing to check there.
            return;
        };
        let mut checked = 0;
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
                    // Fixtures needing dependencies, or meant to fail, do not
                    // reach code generation here; skip them.
                    let Ok(assembly) = compile_to_assembly(&name, &source, &options) else {
                        continue;
                    };
                    checked += 1;
                    for line in assembly.lines() {
                        let Some(operands) = line.strip_prefix("  lea ") else {
                            continue;
                        };
                        let Some((_, source)) = operands.split_once(", ") else {
                            continue;
                        };
                        assert!(
                            source.starts_with('[') || source.contains("[rip]"),
                            "`{line}` in {name} takes the address of a register"
                        );
                    }
                }
            }
        }
        assert!(checked > 0, "no fixture compiled; the corpus path is wrong");
    }

    #[test]
    fn a_copy_that_undoes_the_previous_one_is_deleted() {
        let assembly = remove_redundant_copies("  mov r13, rax\n  mov rax, r13\n  ret\n");
        assert_eq!(assembly, "  mov r13, rax\n  ret\n");
    }

    #[test]
    fn a_label_between_two_copies_stops_the_rewrite() {
        let source = "  mov r13, rax\n.Lsomewhere:\n  mov rax, r13\n";
        assert_eq!(remove_redundant_copies(source), source);
    }

    #[test]
    fn an_unrelated_copy_survives() {
        let source = "  mov r13, rax\n  mov rcx, r12\n";
        assert_eq!(remove_redundant_copies(source), source);
    }
}
