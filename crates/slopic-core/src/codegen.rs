use crate::asm::{Assembly, Item, Section, Target};
use crate::ast::Type;
use crate::cfg::Cfg;
use crate::diagnostic::{codes, CompileResult, Diagnostic, SourceMap, Span};
use crate::lowering::{
    address_taken, call_symbol, call_words, clone_function, drop_function, enum_clone_size,
    enum_clone_symbol, enum_drop_symbol, enum_size, extern_declaration, function_symbol,
    is_pointer_like, struct_clone_symbol, struct_drop_symbol, struct_size, trap_usage, Argument,
    ExternClass, ExternWord, Step, Tail, TrapUsage,
};
use crate::mir::{BasicBlock, BinaryOp, Instruction, LocalId, MirFunction, MirModule, Terminator};
use crate::regalloc::{allocate, Allocation, Location};
use crate::x86_64_inst::{AluOp, Cond, Inst, Mem, Operand, Reg, ShiftOp, Size, SseOp};
use serde::Serialize;
use std::collections::HashMap;

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
/// What the argument marshalling ends with.
///
/// The two shapes differ in one instruction and nothing else — same registers,
/// same stack layout, same cleanup — which is why they share a body rather than
/// having the convention written twice.
enum Callee {
    Symbol(String),
    Register(Reg),
}

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

/// What a program can assume is under it.
///
/// It decides exactly three things (`D-066`, `D-081`): which runtime units are
/// materialized, whether the `main(argc, argv)` wrapper is emitted at all, and
/// which toolchain library a lone file gets by default. Anything a triple
/// decides — the calling convention, the object format — is not this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Environment {
    /// A C library and an operating system: the runtime brings its own
    /// allocator, `main` runs before the program does, and `std` is available.
    #[default]
    Hosted,
    /// Neither. The program supplies `sl_rt_alloc`, `sl_rt_free`,
    /// `sl_rt_abort` and `sl_rt_panic` (`D-080`), starts itself, and has
    /// `core` and nothing above it.
    Freestanding,
}

impl Environment {
    /// Whether a `main` the C runtime can call is emitted around the program's
    /// `main`. A freestanding program has no `argv` to record and no libc
    /// start-up to be called from, so the wrapper would only be an undefined
    /// reference to `sl_rt_args_init`.
    pub fn emits_entrypoint(self) -> bool {
        matches!(self, Self::Hosted)
    }

    /// The toolchain library a lone file gets when nothing names one.
    pub fn default_library(self) -> &'static str {
        match self {
            Self::Hosted => slopium_std::STD_PACKAGE,
            Self::Freestanding => slopium_std::CORE_PACKAGE,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TargetSpec {
    pub triple: &'static str,
    pub architecture: &'static str,
    pub abi: &'static str,
    pub object_format: &'static str,
    pub default_cc: &'static str,
    /// The environment this target implies when the command line does not
    /// override it. Both targets are hosted today; a `-none` triple at v0.7 is
    /// then a row in this table and not a new mechanism (`D-081`).
    pub environment: Environment,
}

pub const X86_64_LINUX_GNU: TargetSpec = TargetSpec {
    triple: "x86_64-unknown-linux-gnu",
    architecture: "x86_64",
    abi: "System V AMD64",
    object_format: "ELF",
    default_cc: "cc",
    environment: Environment::Hosted,
};

pub const AARCH64_LINUX_GNU: TargetSpec = TargetSpec {
    triple: "aarch64-unknown-linux-gnu",
    architecture: "aarch64",
    abi: "AAPCS64",
    object_format: "ELF",
    // A cross toolchain names its driver after the target it builds for, and
    // the host `cc` would silently produce host objects, so there is no
    // sensible bare fallback here the way there is for the host target.
    default_cc: "aarch64-unknown-linux-gnu-cc",
    environment: Environment::Hosted,
};

/// Every target this compiler emits for.
pub const TARGETS: &[TargetSpec] = &[X86_64_LINUX_GNU, AARCH64_LINUX_GNU];

/// The target chosen when nothing asks for one. It is the host, so building
/// without a `--target` needs no cross toolchain.
pub const DEFAULT_TARGET: &str = X86_64_LINUX_GNU.triple;

/// The triples of [`TARGETS`], in the same order.
///
/// A separate list because `CompilerInfo` is serialized and a `const fn` cannot
/// build one from the table; `every_target_is_listed_once` keeps the two in
/// step.
pub const TARGET_TRIPLES: &[&str] = &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

/// Argument registers of the System V AMD64 calling convention, and their
/// 32-bit views for the one place a zero is cheaper to write narrow.
const INTEGER_ARGUMENTS: [&str; 6] = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
const NARROW_ARGUMENTS: [&str; 6] = ["edi", "esi", "edx", "ecx", "r8d", "r9d"];
const FLOAT_ARGUMENTS: [&str; 8] = [
    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
];

/// The trampolines the arithmetic checks branch to.
fn overflow_trampoline() -> Target {
    Target::Named(".Lsl_panic_overflow_trampoline".into())
}

fn div_zero_trampoline() -> Target {
    Target::Named(".Lsl_panic_div_zero_trampoline".into())
}

fn shift_trampoline() -> Target {
    Target::Named(".Lsl_panic_shift_trampoline".into())
}

/// A register as an operand, which is what most of them are.
fn reg(name: &'static str) -> Operand {
    Operand::Reg(Reg(name))
}

pub trait Backend {
    fn target(&self) -> &'static TargetSpec;
    fn emit(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<String>;

    /// The same program, as a relocatable ELF object.
    ///
    /// Assembling is no longer the assembler's: a backend encodes what it
    /// selected and [`crate::elf`] writes the file. Linking still is, for the
    /// reason `D-001` gives.
    fn object(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<Vec<u8>>;
}

/// Lays out an assembled program and writes it as an object file.
///
/// A failure here is a compiler bug, not a program error: instruction
/// selection produced something it has no encoding for, or a label nothing
/// defines. `SL0700` says so rather than pretending the source was at fault.
pub fn write_object<I: crate::asm::Instruction>(
    file: &str,
    assembly: &crate::asm::Assembly<I>,
    machine: crate::elf::Machine,
) -> CompileResult<Vec<u8>> {
    let internal = |error: String| {
        vec![Diagnostic::error(
            codes::INTERNAL,
            file,
            Default::default(),
            format!("cannot write an object for this program: {error}"),
        )
        .with_help("compile with `--emit asm` and assemble it, and report this")]
    };
    let object = assembly.finish().map_err(internal)?;
    crate::elf::write(&object, machine).map_err(internal)
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
        Ok(Generator::new(file, module, options).generate()?.to_text())
    }

    fn object(
        &self,
        file: &str,
        module: &MirModule,
        options: &CodegenOptions,
    ) -> CompileResult<Vec<u8>> {
        let assembly = Generator::new(file, module, options).generate()?;
        write_object(file, &assembly, crate::elf::X86_64)
    }
}

/// The backend that emits for `triple`, or `None` when no backend claims it.
pub fn backend_for(triple: &str) -> Option<Box<dyn Backend>> {
    if triple == X86_64_LINUX_GNU.triple {
        Some(Box::new(X86_64Backend))
    } else if triple == AARCH64_LINUX_GNU.triple {
        Some(Box::new(crate::aarch64::Aarch64Backend))
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct CodegenOptions {
    pub target: String,
    pub test_harness: bool,
    pub emit_entrypoint: bool,
    /// The files spans refer to, when debug line tables are wanted. `None`
    /// emits no `.file` or `.loc` at all, so assembly is exactly what it was
    /// before debug information existed.
    pub debug: Option<SourceMap>,
    /// Whether a trap aborts without a message. The trampolines then call
    /// `sl_rt_abort` and carry no string, so nothing names the panic messages.
    pub panic_abort: bool,
}

impl Default for CodegenOptions {
    fn default() -> Self {
        Self {
            target: DEFAULT_TARGET.into(),
            test_harness: false,
            emit_entrypoint: true,
            panic_abort: false,
            debug: None,
        }
    }
}

pub fn emit_assembly(
    file: &str,
    module: &MirModule,
    options: &CodegenOptions,
) -> CompileResult<String> {
    let Some(backend) = backend_for(&options.target) else {
        return Err(vec![Diagnostic::error(
            codes::UNSUPPORTED_TARGET,
            file,
            Default::default(),
            format!("unsupported target `{}`", options.target),
        )
        .with_help(format!(
            "available targets: {}",
            TARGET_TRIPLES.join(", ")
        ))]);
    };
    backend.emit(file, module, options)
}

/// The program as a relocatable object, for the targets whose backend writes
/// one.
pub fn emit_object(
    file: &str,
    module: &MirModule,
    options: &CodegenOptions,
) -> CompileResult<Vec<u8>> {
    let Some(backend) = backend_for(&options.target) else {
        return Err(vec![Diagnostic::error(
            codes::UNSUPPORTED_TARGET,
            file,
            Default::default(),
            format!("unsupported target `{}`", options.target),
        )]);
    };
    backend.object(file, module, options)
}

/// Whether an assembly line only attributes the instructions around it.
///
/// Debug information adds directives and changes no instruction (`D-024`), and
/// this is what the tests that assert it filter with.
#[cfg(test)]
pub(crate) fn is_location(line: &str) -> bool {
    line.trim_start().starts_with(".loc ")
}

/// Whether some backend claims `triple` and can therefore write its objects.
pub fn writes_objects(triple: &str) -> bool {
    backend_for(triple).is_some()
}

struct Generator<'a> {
    file: &'a str,
    module: &'a MirModule,
    options: &'a CodegenOptions,
    asm: Assembly<Inst>,
    strings: Vec<(String, Vec<u8>)>,
    string_ids: HashMap<Vec<u8>, String>,
    diagnostics: Vec<Diagnostic>,
    /// Where the locals of the function currently being emitted live, and the
    /// register set it draws on. The helper functions below read both, so both
    /// are replaced per function.
    alloc: Allocation,
    registers: &'static RegisterFile,
    /// The last `.loc` written, so a run of statements lowered from the same
    /// expression produces one line-table row instead of one per instruction.
    /// Reset at every label, because a jump can arrive there from a row that
    /// says something else.
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
            last_location: None,
        }
    }

    fn generate(mut self) -> CompileResult<Assembly<Inst>> {
        self.collect_strings();
        self.asm.push(Item::Syntax(".intel_syntax noprefix"));
        self.file_table();
        self.asm.push(Item::Section(Section::RoData));
        let traps = self.trap_usage();
        // Only the messages a check can actually reach: a program with no
        // division carries no "division by zero". `panic = "abort"` reaches
        // none of them, because the trampolines then carry no message.
        if !self.options.panic_abort {
            if traps.div_zero {
                self.byte_string(".Lsl_panic_div_zero", b"division by zero");
            }
            if traps.overflow {
                self.byte_string(".Lsl_panic_overflow", b"integer overflow");
            }
            if traps.shift {
                self.byte_string(".Lsl_panic_shift", b"shift amount out of range");
            }
        }
        for (label, value) in self.strings.clone() {
            self.byte_string(&label, &value);
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
        // A test is code only the harness calls, so a build without one has no
        // reason to carry it. Emitting the bodies anyway left every `sl_test_*`
        // function — and, through it, `sl_rt_test_result` — sitting dead in an
        // ordinary release binary.
        if self.options.test_harness {
            for test in self.module.tests.iter().filter(|test| test.emit) {
                self.function(&test.function, true);
            }
        }
        // Everything past this point is generated glue — clone/drop helpers,
        // the entry wrapper, the panic trampolines — and emits no location of
        // its own, so it inherits the last row written. DWARF spells "not in
        // the source" as line 0, but GAS discards a `.loc` naming it, and
        // ending the line sequence early would mean giving the glue its own
        // section. Neither is worth it for code nobody wrote.
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
        self.runtime_panic_trampolines(traps);
        self.asm.push(Item::Section(Section::GnuStack));

        if self.diagnostics.is_empty() {
            self.asm.remove_redundant_copies();
            Ok(self.asm)
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
                    .filter(|test| test.emit && self.options.test_harness)
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
        // The test name is what the harness prints; without a harness there is
        // nothing to print it, so the string need not exist either.
        if self.options.test_harness {
            for test in &self.module.tests {
                self.intern(test.name.as_bytes());
            }
        }
    }

    fn intern(&mut self, value: &[u8]) -> String {
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

    /// Declares every file the line table may name, numbered from 1 in the
    /// order [`SourceMap::index_of`] uses.
    ///
    /// Every object of a package declares the whole list, including files it
    /// emits no code from, so a file number means the same thing in all of
    /// them. Emitting only the referenced files would need a pre-scan and a
    /// remap of the map's indices, to save a few dozen bytes of unreferenced
    /// path per object.
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

    /// Attributes the instructions that follow to `span`.
    fn location(&mut self, span: Span) {
        let Some(sources) = self.options.debug.as_ref() else {
            return;
        };
        // A statement the builder synthesized — a drop spliced in at a CFG
        // merge, say — carries no span, and there is nothing to say instead:
        // DWARF spells "not in the source" as line 0 and GAS discards a `.loc`
        // naming it. Such a statement stays under the row before it.
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

    /// Writes a label and forgets the last `.loc`.
    ///
    /// Forgetting it makes each block open a row of its own rather than
    /// continue the previous block's, so the address a breakpoint resolves to
    /// is the start of the block that begins the line.
    fn inst(&mut self, instruction: Inst) {
        self.asm.instruction(instruction);
    }

    fn label(&mut self, label: &str) {
        self.asm.push(Item::Label(label.to_owned()));
        self.last_location = None;
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
            &address_taken(self.module, function),
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

        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.label(&symbol);
        // The prologue is attributed to the declaration, so a breakpoint on the
        // function stops before its body rather than inside its first statement.
        self.location(function.span);
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        if frame_size != 0 {
            self.inst(Inst::Alu(
                AluOp::Sub,
                reg("rsp"),
                Operand::Imm(frame_size as i64),
            ));
        }
        for (index, register) in saved.iter().enumerate() {
            self.inst(Inst::Mov(
                Operand::Mem(frame_slot(Some(Size::Qword), save_base + index)),
                Operand::Reg(Reg(self.registers.wide[*register])),
            ));
        }

        self.store_parameters(function);
        for (block_id, block) in function.blocks.iter().enumerate() {
            self.label(&format!(".L{}_bb{}", symbol, block_id));
            self.basic_block(function, block, &symbol, &epilogue);
        }

        self.label(epilogue.as_str());
        for (index, register) in saved.iter().enumerate() {
            self.inst(Inst::Mov(
                Operand::Reg(Reg(self.registers.wide[*register])),
                Operand::Mem(frame_slot(Some(Size::Qword), save_base + index)),
            ));
        }
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
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
                | Instruction::CallValue { .. }
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
                | Instruction::FnAddr { .. }
                | Instruction::FieldLoad { .. }
                | Instruction::FieldAddr { .. }
                | Instruction::Load { .. }
                | Instruction::EnumTag { .. }
                | Instruction::EnumFieldLoad { .. }
                | Instruction::EnumFieldAddr { .. } => false,
            })
    }

    fn store_parameters(&mut self, function: &MirFunction) {
        let integer_regs = INTEGER_ARGUMENTS;
        let float_regs = FLOAT_ARGUMENTS;
        let mut integers = 0;
        let mut floats = 0;
        let mut stack = 0;
        for local in &function.params {
            let ty = &function.locals[*local].ty;
            if *ty == Type::F64 {
                if floats >= float_regs.len() {
                    self.inst(Inst::Mov(
                        reg("rax"),
                        Operand::slot(Size::Qword, Reg("rbp"), (16 + stack * 8) as i64),
                    ));
                    self.inst(Inst::Mov(
                        operand(&self.alloc, self.registers, *local),
                        reg("rax"),
                    ));
                    stack += 1;
                } else {
                    let destination = operand(&self.alloc, self.registers, *local);
                    self.store_double(destination, float_regs[floats]);
                    floats += 1;
                }
            } else {
                if integers >= integer_regs.len() {
                    self.inst(Inst::Mov(
                        reg("rax"),
                        Operand::slot(Size::Qword, Reg("rbp"), (16 + stack * 8) as i64),
                    ));
                    self.inst(Inst::Mov(
                        operand(&self.alloc, self.registers, *local),
                        reg("rax"),
                    ));
                    stack += 1;
                } else {
                    let destination = operand(&self.alloc, self.registers, *local);
                    self.inst(Inst::Mov(
                        destination,
                        Operand::Reg(Reg(integer_regs[integers])),
                    ));
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
        for statement in &block.statements {
            self.location(statement.span);
            self.instruction(function, &statement.instruction);
        }
        self.location(block.terminator_span);
        match &block.terminator {
            Terminator::Return(value) => {
                if let Some(local) = value {
                    if function.locals[*local].ty == Type::F64 {
                        let source = operand(&self.alloc, self.registers, *local);
                        self.load_double(source);
                    } else {
                        self.inst(Inst::Mov(
                            reg("rax"),
                            operand(&self.alloc, self.registers, *local),
                        ));
                    }
                } else {
                    self.inst(Inst::Alu(AluOp::Xor, reg("eax"), reg("eax")));
                }
                self.inst(Inst::Jmp(Target::Named(epilogue.to_owned())));
            }
            Terminator::Goto(target) => {
                self.inst(Inst::Jmp(Target::Named(format!(
                    ".L{}_bb{}",
                    symbol, target
                ))));
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                self.inst(Inst::Alu(
                    AluOp::Cmp,
                    operand(&self.alloc, self.registers, *condition),
                    Operand::Imm(0),
                ));
                self.inst(Inst::Jcc(
                    Cond::Ne,
                    Target::Named(format!(".L{}_bb{}", symbol, then_block)),
                ));
                self.inst(Inst::Jmp(Target::Named(format!(
                    ".L{}_bb{}",
                    symbol, else_block
                ))));
            }
            Terminator::Unreachable => self.inst(Inst::Ud2),
        }
    }

    /// Materializes a 64-bit immediate into a local.
    ///
    /// x86-64 has no store of a 64-bit immediate to memory, so a memory
    /// destination still needs the trip through `rax`. A register destination
    /// takes the immediate directly.
    fn load_immediate(&mut self, dst: LocalId, value: Operand) {
        let destination = operand(&self.alloc, self.registers, dst);
        if in_memory(&self.alloc, dst) {
            self.inst(Inst::Mov(reg("rax"), value));
            self.inst(Inst::Mov(destination, reg("rax")));
        } else {
            self.inst(Inst::Mov(destination, value));
        }
    }

    /// Moves a double out of the SSE register it arrived in.
    ///
    /// A local holding a double is an ordinary 64-bit value everywhere except
    /// at the two boundaries where the ABI puts it in `xmm0`.
    fn store_double(&mut self, destination: Operand, source: &'static str) {
        match destination {
            Operand::Reg(register) => self.inst(Inst::Movq(register, Reg(source))),
            destination => {
                self.inst(Inst::Movq(Reg("rax"), Reg(source)));
                self.inst(Inst::Mov(destination, reg("rax")));
            }
        }
    }

    /// And back into it, for a return or an argument.
    fn load_double(&mut self, source: Operand) {
        self.load_double_into(Reg("xmm0"), source);
    }

    fn load_double_into(&mut self, destination: Reg, source: Operand) {
        match source {
            Operand::Reg(register) => self.inst(Inst::Movq(destination, register)),
            source => {
                self.inst(Inst::Mov(reg("rax"), source));
                self.inst(Inst::Movq(destination, Reg("rax")));
            }
        }
    }

    /// `dst = base + offset`, the address of a field inside a pointer-shaped
    /// aggregate.
    ///
    /// At offset zero the field's address *is* the base, so the arithmetic is a
    /// copy — which also spares the assembler a `lea` with a zero displacement,
    /// a form it and this encoder spell differently.
    fn field_address(&mut self, dst: LocalId, base: LocalId, offset: usize) {
        if offset == 0 {
            self.copy(dst, base);
            return;
        }
        self.inst(Inst::Mov(
            reg("rax"),
            operand(&self.alloc, self.registers, base),
        ));
        self.inst(Inst::Lea(
            Reg("rcx"),
            Operand::Mem(Mem {
                size: None,
                base: Reg("rax"),
                disp: Some(offset as i64),
            }),
        ));
        self.inst(Inst::Mov(
            operand(&self.alloc, self.registers, dst),
            reg("rcx"),
        ));
    }

    /// Copies one local to another, going through `rax` only when both ends are
    /// in memory — `mov` allows at most one memory operand.
    fn copy(&mut self, dst: LocalId, src: LocalId) {
        if in_memory(&self.alloc, dst) && in_memory(&self.alloc, src) {
            self.inst(Inst::Mov(
                reg("rax"),
                operand(&self.alloc, self.registers, src),
            ));
            self.inst(Inst::Mov(
                operand(&self.alloc, self.registers, dst),
                reg("rax"),
            ));
        } else {
            self.inst(Inst::Mov(
                operand(&self.alloc, self.registers, dst),
                operand(&self.alloc, self.registers, src),
            ));
        }
    }

    fn instruction(&mut self, function: &MirFunction, instruction: &Instruction) {
        match instruction {
            Instruction::ConstInt { dst, value } => self.load_immediate(*dst, Operand::Imm(*value)),
            Instruction::ConstFloat { dst, bits } => {
                self.load_immediate(*dst, Operand::Bits(*bits))
            }
            Instruction::ConstBool { dst, value } => {
                let destination = operand(&self.alloc, self.registers, *dst);
                self.inst(Inst::Mov(destination, Operand::Imm(i64::from(*value))));
            }
            Instruction::StringNew { dst, value } => {
                let label = self.string_ids[value].clone();
                self.inst(Inst::Lea(Reg("rdi"), Operand::Rip(label)));
                self.inst(Inst::Mov(reg("rsi"), Operand::Imm(value.len() as i64)));
                self.inst(Inst::Call("sl_rt_string_new".into()));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rax"),
                ));
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
                    self.inst(Inst::Lea(Reg("rax"), address(&self.alloc, *src)));
                    self.inst(Inst::Mov(
                        operand(&self.alloc, self.registers, *dst),
                        reg("rax"),
                    ));
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
            Instruction::FnAddr { dst, symbol } => {
                // `rax` is this generator's scratch and never an allocated
                // local, so the address lands there and is stored, whether the
                // destination is a register or a stack slot.
                self.inst(Inst::Lea(Reg("rax"), Operand::Rip(symbol.clone())));
                let destination = operand(&self.alloc, self.registers, *dst);
                self.inst(Inst::Mov(destination, reg("rax")));
            }
            Instruction::CallValue {
                dst,
                callee,
                args,
                arg_types,
                result,
            } => {
                self.indirect_call(*callee, args, arg_types);
                match result {
                    Type::Unit => {}
                    Type::F64 => {
                        let destination = operand(&self.alloc, self.registers, *dst);
                        self.store_double(destination, "xmm0");
                    }
                    // No narrow-return extension: the callee is a Slopium
                    // function, which already returns an extended value
                    // (`D-074`). Only the C boundary needs that, and a function
                    // value never crosses it.
                    _ => {
                        let destination = operand(&self.alloc, self.registers, *dst);
                        self.inst(Inst::Mov(destination, reg("rax")));
                    }
                }
            }
            Instruction::Drop { local, ty } => {
                self.inst(Inst::Mov(
                    reg("rdi"),
                    operand(&self.alloc, self.registers, *local),
                ));
                // Through `lowering`, like the AArch64 backend already does:
                // this arm answered the question a second time and had to be
                // taught `Fn` separately at v0.7.4, which is the kind of
                // divergence `D-025` puts the answer in one place to avoid.
                if let Some(symbol) = self.drop_function(ty) {
                    self.inst(Inst::Call(symbol));
                }
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *local),
                    Operand::Imm(0),
                ));
            }
            Instruction::StructNew { dst, name, fields } => {
                let size = struct_size(self.module, name);
                self.inst(Inst::Mov(reg("rdi"), Operand::Imm(size as i64)));
                self.inst(Inst::Call("sl_rt_alloc".into()));
                for (index, field) in fields.iter().enumerate() {
                    self.inst(Inst::Mov(
                        reg("rcx"),
                        operand(&self.alloc, self.registers, *field),
                    ));
                    self.inst(Inst::Mov(
                        Operand::slot(Size::Qword, Reg("rax"), (index * 8) as i64),
                        reg("rcx"),
                    ));
                }
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rax"),
                ));
            }
            Instruction::FieldLoad { dst, base, index } => {
                self.inst(Inst::Mov(
                    reg("rax"),
                    operand(&self.alloc, self.registers, *base),
                ));
                self.inst(Inst::Mov(
                    reg("rcx"),
                    Operand::slot(Size::Qword, Reg("rax"), (index * 8) as i64),
                ));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rcx"),
                ));
            }
            // The address of a field, rather than the word in it (`D-099`). The
            // zero-offset case is the base itself, and saying so keeps the
            // assembler from being asked for `lea rcx, [rax+0]`.
            Instruction::FieldAddr { dst, base, index } => {
                self.field_address(*dst, *base, index * 8);
            }
            Instruction::EnumFieldAddr { dst, base, index } => {
                self.field_address(*dst, *base, (index + 1) * 8);
            }
            // The dereference (`D-100`), and the same two moves `EnumTag` makes
            // — a tag is the word at offset zero, which is what this is.
            Instruction::Load { dst, src } => {
                self.inst(Inst::Mov(
                    reg("rax"),
                    operand(&self.alloc, self.registers, *src),
                ));
                self.inst(Inst::Mov(
                    reg("rcx"),
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: Reg("rax"),
                        disp: None,
                    }),
                ));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rcx"),
                ));
            }
            Instruction::EnumNew {
                dst, tag, fields, ..
            } => {
                let size = enum_size(fields.len());
                self.inst(Inst::Mov(reg("rdi"), Operand::Imm(size as i64)));
                self.inst(Inst::Call("sl_rt_alloc".into()));
                self.inst(Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: Reg("rax"),
                        disp: None,
                    }),
                    Operand::Imm(*tag as i64),
                ));
                for (index, field) in fields.iter().enumerate() {
                    self.inst(Inst::Mov(
                        reg("rcx"),
                        operand(&self.alloc, self.registers, *field),
                    ));
                    self.inst(Inst::Mov(
                        Operand::slot(Size::Qword, Reg("rax"), ((index + 1) * 8) as i64),
                        reg("rcx"),
                    ));
                }
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rax"),
                ));
            }
            Instruction::EnumTag { dst, base } => {
                self.inst(Inst::Mov(
                    reg("rax"),
                    operand(&self.alloc, self.registers, *base),
                ));
                self.inst(Inst::Mov(
                    reg("rcx"),
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: Reg("rax"),
                        disp: None,
                    }),
                ));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rcx"),
                ));
            }
            Instruction::EnumFieldLoad { dst, base, index } => {
                self.inst(Inst::Mov(
                    reg("rax"),
                    operand(&self.alloc, self.registers, *base),
                ));
                self.inst(Inst::Mov(
                    reg("rcx"),
                    Operand::slot(Size::Qword, Reg("rax"), ((index + 1) * 8) as i64),
                ));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *dst),
                    reg("rcx"),
                ));
            }
            Instruction::Free { local } => {
                self.inst(Inst::Mov(
                    reg("rdi"),
                    operand(&self.alloc, self.registers, *local),
                ));
                self.inst(Inst::Call("sl_rt_free".into()));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, *local),
                    Operand::Imm(0),
                ));
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
            _ if op.bitwise() && self.accumulate_in_place(dst, op, lhs, rhs, ty) => {}
            _ if op.compares() && self.compare_in_place(dst, op, lhs, rhs) => {}
            _ => self.integer_binary_through_rax(dst, op, lhs, rhs, ty),
        }
    }

    /// Computes an addition, subtraction, multiplication or bit operation
    /// straight into the destination register, so the result never travels
    /// through `rax`.
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
                Reg(self.registers.narrow[register]),
                narrow_operand(&self.alloc, self.registers, rhs),
            )
        } else {
            (
                Reg(self.registers.wide[register]),
                operand(&self.alloc, self.registers, rhs),
            )
        };
        match op {
            BinaryOp::Add => self.inst(Inst::Alu(AluOp::Add, Operand::Reg(target), source)),
            BinaryOp::Sub => self.inst(Inst::Alu(AluOp::Sub, Operand::Reg(target), source)),
            BinaryOp::Mul => self.inst(Inst::Imul(target, source)),
            BinaryOp::BitAnd => self.inst(Inst::Alu(AluOp::And, Operand::Reg(target), source)),
            BinaryOp::BitOr => self.inst(Inst::Alu(AluOp::Or, Operand::Reg(target), source)),
            BinaryOp::BitXor => self.inst(Inst::Alu(AluOp::Xor, Operand::Reg(target), source)),
            _ => unreachable!("only the accumulating operators reach here"),
        }
        // A bit operation cannot overflow — it produces a pattern, not a
        // magnitude — and the flag `and`/`or`/`xor` leave is always clear, so
        // the branch would be dead weight rather than merely harmless.
        if !op.bitwise() {
            self.inst(Inst::Jcc(Cond::O, overflow_trampoline()));
        }
        if narrow {
            // The 32-bit form zero-extends into the full register; the local's
            // value is a sign-extended i32.
            self.inst(Inst::Movsxd(Reg(self.registers.wide[register]), target));
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
        self.inst(Inst::Alu(
            AluOp::Cmp,
            operand(&self.alloc, self.registers, lhs),
            operand(&self.alloc, self.registers, rhs),
        ));
        self.inst(Inst::Setcc(set_condition(op), Reg("al")));
        match self.alloc.location(dst) {
            Location::Register(register) => {
                self.inst(Inst::Movzx(Reg(self.registers.wide[register]), Reg("al")))
            }
            Location::Memory(_) => {
                self.inst(Inst::Movzx(Reg("rax"), Reg("al")));
                self.inst(Inst::Mov(
                    operand(&self.alloc, self.registers, dst),
                    reg("rax"),
                ));
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
        self.inst(Inst::Mov(
            reg("rax"),
            operand(&self.alloc, self.registers, lhs),
        ));
        self.inst(Inst::Mov(
            reg("rcx"),
            operand(&self.alloc, self.registers, rhs),
        ));
        let width = if *ty == Type::I32 { "e" } else { "r" };
        let accumulator = reg(if width == "e" { "eax" } else { "rax" });
        let argument = reg(if width == "e" { "ecx" } else { "rcx" });
        match op {
            BinaryOp::Add => {
                self.inst(Inst::Alu(AluOp::Add, accumulator.clone(), argument.clone()));
                self.inst(Inst::Jcc(Cond::O, overflow_trampoline()));
            }
            BinaryOp::Sub => {
                self.inst(Inst::Alu(AluOp::Sub, accumulator.clone(), argument.clone()));
                self.inst(Inst::Jcc(Cond::O, overflow_trampoline()));
            }
            BinaryOp::Mul => {
                self.inst(Inst::Imul(
                    Reg(if width == "e" { "eax" } else { "rax" }),
                    argument.clone(),
                ));
                self.inst(Inst::Jcc(Cond::O, overflow_trampoline()));
            }
            // One sequence for both, because the machine computes both at once:
            // `idiv` leaves the quotient in the accumulator and the remainder
            // in `rdx`, so `%` differs from `/` only in which register is read
            // afterwards. The two checks in front are `D-031`'s and are not
            // optional — `#DE` for a zero divisor is a fault with no message,
            // and the most negative value over `-1` has no quotient at all.
            BinaryOp::Div | BinaryOp::Rem => {
                self.inst(Inst::Test(argument.clone(), argument.clone()));
                self.inst(Inst::Jcc(Cond::E, div_zero_trampoline()));
                if *ty == Type::I32 {
                    self.inst(Inst::Alu(AluOp::Cmp, reg("eax"), Operand::Imm(-2147483648)));
                    self.inst(Inst::Jcc(Cond::Ne, Target::Forward(1)));
                    self.inst(Inst::Alu(AluOp::Cmp, reg("ecx"), Operand::Imm(-1)));
                    self.inst(Inst::Jcc(Cond::E, overflow_trampoline()));
                    self.asm.push(Item::Numeric(1));
                    self.inst(Inst::Cdq);
                    self.inst(Inst::Idiv(Reg("ecx")));
                } else {
                    self.inst(Inst::Mov(reg("rdx"), Operand::Imm(i64::MIN)));
                    self.inst(Inst::Alu(AluOp::Cmp, reg("rax"), reg("rdx")));
                    self.inst(Inst::Jcc(Cond::Ne, Target::Forward(1)));
                    self.inst(Inst::Alu(AluOp::Cmp, reg("rcx"), Operand::Imm(-1)));
                    self.inst(Inst::Jcc(Cond::E, overflow_trampoline()));
                    self.asm.push(Item::Numeric(1));
                    self.inst(Inst::Cqo);
                    self.inst(Inst::Idiv(Reg("rcx")));
                }
                if op == BinaryOp::Rem {
                    let remainder = reg(if width == "e" { "edx" } else { "rdx" });
                    self.inst(Inst::Mov(accumulator.clone(), remainder));
                }
            }
            BinaryOp::BitAnd => self.inst(Inst::Alu(AluOp::And, accumulator.clone(), argument)),
            BinaryOp::BitOr => self.inst(Inst::Alu(AluOp::Or, accumulator.clone(), argument)),
            BinaryOp::BitXor => self.inst(Inst::Alu(AluOp::Xor, accumulator.clone(), argument)),
            // The count is checked against the width before the shift, and the
            // comparison is *unsigned*: a negative count is an enormous
            // unsigned number and takes the same branch, so one compare covers
            // both halves of `D-106`'s rule. Without it the two backends would
            // not even fault — x86-64 masks the count to five or six bits and
            // AArch64 reduces it modulo the width, so a shift by the width
            // would quietly answer two different things.
            BinaryOp::Shl | BinaryOp::Shr => {
                let bits = if *ty == Type::I32 { 32 } else { 64 };
                self.inst(Inst::Alu(AluOp::Cmp, argument, Operand::Imm(bits)));
                self.inst(Inst::Jcc(Cond::Ae, shift_trampoline()));
                let shift = if op == BinaryOp::Shl {
                    ShiftOp::Shl
                } else {
                    ShiftOp::Sar
                };
                self.inst(Inst::Shift(
                    shift,
                    Reg(if width == "e" { "eax" } else { "rax" }),
                ));
            }
            BinaryOp::Less
            | BinaryOp::Greater
            | BinaryOp::LessEqual
            | BinaryOp::GreaterEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual => {
                self.inst(Inst::Alu(AluOp::Cmp, reg("rax"), reg("rcx")));
                self.inst(Inst::Setcc(set_condition(op), Reg("al")));
                self.inst(Inst::Movzx(Reg("rax"), Reg("al")));
            }
        }
        if *ty == Type::I32 && !op.compares() {
            self.inst(Inst::Movsxd(Reg("rax"), Reg("eax")));
        }
        self.inst(Inst::Mov(
            operand(&self.alloc, self.registers, dst),
            reg("rax"),
        ));
    }

    fn float_binary(&mut self, dst: LocalId, op: BinaryOp, lhs: LocalId, rhs: LocalId) {
        let left = operand(&self.alloc, self.registers, lhs);
        self.load_double(left);
        let right = operand(&self.alloc, self.registers, rhs);
        self.load_double_into(Reg("xmm1"), right);
        match op {
            BinaryOp::Add => self.inst(Inst::Sse(SseOp::Add, Reg("xmm0"), Reg("xmm1"))),
            BinaryOp::Sub => self.inst(Inst::Sse(SseOp::Sub, Reg("xmm0"), Reg("xmm1"))),
            BinaryOp::Mul => self.inst(Inst::Sse(SseOp::Mul, Reg("xmm0"), Reg("xmm1"))),
            BinaryOp::Div => self.inst(Inst::Sse(SseOp::Div, Reg("xmm0"), Reg("xmm1"))),
            // `ucomisd` reports "unordered" — either operand a NaN — with the
            // same flags it uses for "below" and for "equal", so `setb` and a
            // bare `sete` both answer true for a NaN. IEEE 754 says a NaN is
            // neither less than, greater than, nor equal to anything, which is
            // also what the constant folder computes and what the other backend
            // emits; these three sequences are what make the machine agree.
            BinaryOp::Less => {
                // Asking "is the right side above the left" rather than "is the
                // left below the right": `seta` is false when unordered.
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm1"), Reg("xmm0")));
                self.inst(Inst::Setcc(Cond::A, Reg("al")));
                self.widen_flag_into_xmm0();
            }
            BinaryOp::Greater => {
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm0"), Reg("xmm1")));
                self.inst(Inst::Setcc(Cond::A, Reg("al")));
                self.widen_flag_into_xmm0();
            }
            BinaryOp::LessEqual => {
                // The mirror of `Less`: ask whether the right side is above or
                // equal, because `setae` reads the carry alone and `ucomisd`
                // sets the carry when the comparison was unordered.
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm1"), Reg("xmm0")));
                self.inst(Inst::Setcc(Cond::Ae, Reg("al")));
                self.widen_flag_into_xmm0();
            }
            BinaryOp::GreaterEqual => {
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm0"), Reg("xmm1")));
                self.inst(Inst::Setcc(Cond::Ae, Reg("al")));
                self.widen_flag_into_xmm0();
            }
            BinaryOp::Equal => {
                // Equality has no single condition that excludes unordered, so
                // the parity flag — set only when unordered — is anded in.
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm0"), Reg("xmm1")));
                self.inst(Inst::Setcc(Cond::E, Reg("al")));
                self.inst(Inst::Setcc(Cond::Np, Reg("cl")));
                self.inst(Inst::Alu(AluOp::And, reg("al"), reg("cl")));
                self.widen_flag_into_xmm0();
            }
            BinaryOp::NotEqual => {
                // And its exact opposite: a NaN is *not equal* to everything,
                // including itself, so parity is ored in rather than anded.
                // `(not (= a b))` would have been a different function here,
                // which is why `!=` is an operator and not a rewrite.
                self.inst(Inst::Sse(SseOp::Ucomi, Reg("xmm0"), Reg("xmm1")));
                self.inst(Inst::Setcc(Cond::Ne, Reg("al")));
                self.inst(Inst::Setcc(Cond::P, Reg("cl")));
                self.inst(Inst::Alu(AluOp::Or, reg("al"), reg("cl")));
                self.widen_flag_into_xmm0();
            }
            // `sema` refuses each of these on an `f64` and `verify` refuses it
            // again, so arriving here is a lowering bug rather than a program.
            BinaryOp::Rem
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr => {
                unreachable!("`{op:?}` is refused on `f64` before it reaches code generation")
            }
        }
        let destination = operand(&self.alloc, self.registers, dst);
        self.store_double(destination, "xmm0");
    }

    /// Moves the flag byte a float comparison just produced into `xmm0`,
    /// where the rest of `float_binary` expects its result.
    fn widen_flag_into_xmm0(&mut self) {
        self.inst(Inst::Movzx(Reg("rax"), Reg("al")));
        self.inst(Inst::Movq(Reg("xmm0"), Reg("rax")));
    }

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
                let destination = operand(&self.alloc, self.registers, dst);
                self.store_double(destination, "xmm0");
            }
            _ => {
                // C only defines the low half of the result register for a
                // narrow return, and a Slopium `i32` is sign-extended
                // everywhere else, so the extension happens here (`D-074`).
                // Slopium callees already return an extended value, so this
                // costs one instruction on an FFI call and nothing elsewhere.
                if extern_declaration(self.module, callee).is_some() {
                    match result {
                        Type::I32 => self.inst(Inst::Movsxd(Reg("rax"), Reg("eax"))),
                        Type::Bool => self.inst(Inst::Movzx(Reg("rax"), Reg("al"))),
                        _ => {}
                    }
                }
                let destination = operand(&self.alloc, self.registers, dst);
                self.inst(Inst::Mov(destination, reg("rax")))
            }
        }
    }

    /// Carries out a builtin's lowering plan.
    ///
    /// Every step is a statement about values, not about x86: what this adds is
    /// the argument registers, the addressing mode, and the branch spelling.
    fn builtin(&mut self, dst: LocalId, steps: &[Step]) {
        for step in steps {
            match step {
                Step::Invoke { arguments, tail } => {
                    for (index, argument) in arguments.iter().enumerate() {
                        let register = INTEGER_ARGUMENTS[index];
                        match argument {
                            Argument::Value(local) => {
                                let source = operand(&self.alloc, self.registers, *local);
                                self.inst(Inst::Mov(Operand::Reg(Reg(register)), source))
                            }
                            Argument::Address(local) => {
                                let source = address(&self.alloc, *local);
                                self.inst(Inst::Lea(Reg(register), source))
                            }
                            Argument::Immediate(value) => self
                                .inst(Inst::Mov(Operand::Reg(Reg(register)), Operand::Imm(*value))),
                            Argument::Function(Some(symbol)) => {
                                self.inst(Inst::Lea(Reg(register), Operand::Rip(symbol.clone())))
                            }
                            // Writing the 32-bit view clears the whole
                            // register and encodes one byte shorter.
                            Argument::Function(None) => {
                                let narrow = NARROW_ARGUMENTS[index];
                                self.inst(Inst::Alu(
                                    AluOp::Xor,
                                    Operand::Reg(Reg(narrow)),
                                    Operand::Reg(Reg(narrow)),
                                ))
                            }
                        }
                    }
                    match tail {
                        Tail::Call(symbol) => self.inst(Inst::Call(symbol.clone())),
                        Tail::FirstArgument => self.inst(Inst::Mov(
                            reg("rax"),
                            Operand::Reg(Reg(INTEGER_ARGUMENTS[0])),
                        )),
                    }
                }
                Step::Save => {
                    let destination = operand(&self.alloc, self.registers, dst);
                    self.inst(Inst::Mov(destination, reg("rax")))
                }
                Step::Restore => {
                    let source = operand(&self.alloc, self.registers, dst);
                    self.inst(Inst::Mov(reg("rax"), source))
                }
                Step::Load => self.inst(Inst::Mov(
                    reg("rax"),
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: Reg("rax"),
                        disp: None,
                    }),
                )),
                Step::WrapOption { some_tag, none_tag } => {
                    self.inst(Inst::Test(reg("rax"), reg("rax")));
                    self.inst(Inst::Jcc(Cond::Z, Target::Forward(1)));
                    self.inst(Inst::Mov(reg("rdi"), Operand::Imm(enum_size(1) as i64)));
                    self.inst(Inst::Call("sl_rt_alloc".into()));
                    self.inst(Inst::Mov(
                        Operand::Mem(Mem {
                            size: Some(Size::Qword),
                            base: Reg("rax"),
                            disp: None,
                        }),
                        Operand::Imm(*some_tag as i64),
                    ));
                    self.inst(Inst::Mov(
                        reg("rcx"),
                        operand(&self.alloc, self.registers, dst),
                    ));
                    self.inst(Inst::Mov(
                        Operand::slot(Size::Qword, Reg("rax"), 8),
                        reg("rcx"),
                    ));
                    self.inst(Inst::Jmp(Target::Forward(2)));
                    self.asm.push(Item::Numeric(1));
                    self.inst(Inst::Mov(reg("rdi"), Operand::Imm(enum_size(0) as i64)));
                    self.inst(Inst::Call("sl_rt_alloc".into()));
                    self.inst(Inst::Mov(
                        Operand::Mem(Mem {
                            size: Some(Size::Qword),
                            base: Reg("rax"),
                            disp: None,
                        }),
                        Operand::Imm(*none_tag as i64),
                    ));
                    self.asm.push(Item::Numeric(2));
                }
            }
        }
    }

    /// A call to a Slopium function, by the platform calling convention.
    fn ordinary_call(&mut self, callee: &str, args: &[LocalId], arg_types: &[Type]) {
        let words = call_words(self.module, callee, args, arg_types);
        let symbol = self.symbol(callee, false);
        self.marshalled_call(&words, &Callee::Symbol(symbol));
    }

    /// A call through a function value.
    ///
    /// The callee is read into `r11` *before* the argument registers are
    /// loaded: a function that calls something allocates only from
    /// `CALLEE_SAVED`, so `r11` is free here, and it is never an argument
    /// register — reading it after marshalling would be the bug that reading it
    /// first cannot be.
    fn indirect_call(&mut self, callee: LocalId, args: &[LocalId], arg_types: &[Type]) {
        let words = crate::lowering::value_words(args, arg_types);
        let source = operand(&self.alloc, self.registers, callee);
        self.inst(Inst::Mov(reg("r11"), source));
        self.marshalled_call(&words, &Callee::Register(Reg("r11")));
    }

    /// The calling convention, shared by both call shapes.
    fn marshalled_call(&mut self, words: &[(ExternWord, ExternClass)], callee: &Callee) {
        let mut integers = 0;
        let mut floats = 0;
        let mut stack_words = Vec::new();
        for (word, class) in words {
            match class {
                ExternClass::Float if floats < FLOAT_ARGUMENTS.len() => {
                    let source = self.word_operand(*word);
                    self.load_double_into(Reg(FLOAT_ARGUMENTS[floats]), source);
                    floats += 1;
                }
                ExternClass::Integer if integers < INTEGER_ARGUMENTS.len() => {
                    let target = Reg(INTEGER_ARGUMENTS[integers]);
                    match *word {
                        ExternWord::Value(local) => {
                            let source = operand(&self.alloc, self.registers, local);
                            self.inst(Inst::Mov(Operand::Reg(target), source));
                        }
                        // The pointer first, then the word it points at — the
                        // argument register is its own scratch here.
                        ExternWord::Indirect { base, offset } => {
                            let source = operand(&self.alloc, self.registers, base);
                            self.inst(Inst::Mov(Operand::Reg(target), source));
                            self.inst(Inst::Mov(
                                Operand::Reg(target),
                                Operand::slot(Size::Qword, target, offset),
                            ));
                        }
                    }
                    integers += 1;
                }
                _ => stack_words.push(*word),
            }
        }
        let padding = usize::from(stack_words.len() % 2 != 0);
        if padding != 0 {
            self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(8)));
        }
        for word in stack_words.iter().rev() {
            let source = self.word_operand(*word);
            self.inst(Inst::Push(source));
        }
        match callee {
            Callee::Symbol(symbol) => self.inst(Inst::Call(symbol.clone())),
            Callee::Register(register) => self.inst(Inst::CallReg(*register)),
        }
        let cleanup = (stack_words.len() + padding) * 8;
        if cleanup != 0 {
            self.inst(Inst::Alu(
                AluOp::Add,
                reg("rsp"),
                Operand::Imm(cleanup as i64),
            ));
        }
    }

    /// The operand holding one argument word, materializing an indirect one
    /// into `rax` — a scratch register of this generator, never an allocated
    /// local and never an argument register.
    fn word_operand(&mut self, word: ExternWord) -> Operand {
        match word {
            ExternWord::Value(local) => operand(&self.alloc, self.registers, local),
            ExternWord::Indirect { base, offset } => {
                let source = operand(&self.alloc, self.registers, base);
                self.inst(Inst::Mov(reg("rax"), source));
                self.inst(Inst::Mov(
                    reg("rax"),
                    Operand::slot(Size::Qword, Reg("rax"), offset),
                ));
                reg("rax")
            }
        }
    }

    fn test_harness(&mut self) {
        self.asm.push(Item::Global("main".into()));
        self.asm.push(Item::Function("main".into()));
        self.asm.push(Item::Label("main".into()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -16),
            reg("rsi"),
        ));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        self.inst(Inst::Call("sl_rt_args_init".into()));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            Operand::Imm(0),
        ));
        for (index, test) in self.module.tests.iter().enumerate() {
            let name = self.string_ids[test.name.as_bytes()].clone();
            self.inst(Inst::Call(self.symbol(&test.function.name, true)));
            self.inst(Inst::Mov(reg("esi"), reg("eax")));
            self.inst(Inst::Lea(Reg("rdi"), Operand::Rip(name.to_owned())));
            self.inst(Inst::Call("sl_rt_test_result".into()));
            self.inst(Inst::Alu(
                AluOp::Add,
                Operand::slot(Size::Qword, Reg("rbp"), -8),
                reg("rax"),
            ));
            let _ = index;
        }
        self.inst(Inst::Mov(
            reg("rax"),
            Operand::slot(Size::Qword, Reg("rbp"), -8),
        ));
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size("main".into()));
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
        self.asm.push(Item::Global("main".into()));
        self.asm.push(Item::Function("main".into()));
        self.asm.push(Item::Label("main".into()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -16),
            reg("rsi"),
        ));
        self.inst(Inst::Call("sl_rt_args_init".into()));
        self.inst(Inst::Call(self.symbol(&main.name, false)));
        if main.return_type == Type::Unit {
            self.inst(Inst::Alu(AluOp::Xor, reg("eax"), reg("eax")));
        }
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size("main".into()));
    }

    fn runtime_panic_trampolines(&mut self, traps: TrapUsage) {
        for (used, trampoline, message) in [
            (
                traps.div_zero,
                ".Lsl_panic_div_zero_trampoline",
                ".Lsl_panic_div_zero",
            ),
            (
                traps.overflow,
                ".Lsl_panic_overflow_trampoline",
                ".Lsl_panic_overflow",
            ),
            (
                traps.shift,
                ".Lsl_panic_shift_trampoline",
                ".Lsl_panic_shift",
            ),
        ] {
            if !used {
                continue;
            }
            self.asm.push(Item::Label(trampoline.into()));
            if self.options.panic_abort {
                // No message to load, and a distinct entry that just exits, so
                // the message strings can be absent from the binary entirely.
                self.inst(Inst::Call("sl_rt_abort".into()));
            } else {
                self.inst(Inst::Lea(Reg("rdi"), Operand::Rip(message.into())));
                self.inst(Inst::Call("sl_rt_panic".into()));
            }
            self.inst(Inst::Ud2);
        }
    }

    /// The panic trampolines this program's arithmetic can reach.
    fn trap_usage(&self) -> TrapUsage {
        trap_usage(
            self.module
                .functions
                .iter()
                .filter(|function| function.emit)
                .chain(
                    self.module
                        .tests
                        .iter()
                        .filter(|test| test.emit && self.options.test_harness)
                        .map(|test| &test.function),
                ),
        )
    }

    fn struct_clone_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_clone_symbol(name);
        let size = struct_size(self.module, name);
        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.asm.push(Item::Label(symbol.to_owned()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        self.inst(Inst::Mov(reg("rdi"), Operand::Imm(size as i64)));
        self.inst(Inst::Call("sl_rt_alloc".into()));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -16),
            reg("rax"),
        ));
        for (index, (_, ty)) in fields.iter().enumerate() {
            self.inst(Inst::Mov(
                reg("rax"),
                Operand::slot(Size::Qword, Reg("rbp"), -8),
            ));
            self.inst(Inst::Mov(
                reg("rdi"),
                Operand::slot(Size::Qword, Reg("rax"), (index * 8) as i64),
            ));
            if let Some(clone_function) = self.clone_function(ty) {
                self.inst(Inst::Call(clone_function));
            } else {
                self.inst(Inst::Mov(reg("rax"), reg("rdi")));
            }
            self.inst(Inst::Mov(
                reg("rcx"),
                Operand::slot(Size::Qword, Reg("rbp"), -16),
            ));
            self.inst(Inst::Mov(
                Operand::slot(Size::Qword, Reg("rcx"), (index * 8) as i64),
                reg("rax"),
            ));
        }
        self.inst(Inst::Mov(
            reg("rax"),
            Operand::slot(Size::Qword, Reg("rbp"), -16),
        ));
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
    }

    fn enum_clone_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_clone_symbol(name);
        let size = enum_clone_size(self.module, name);
        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.asm.push(Item::Label(symbol.to_owned()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        self.inst(Inst::Mov(reg("rdi"), Operand::Imm(size as i64)));
        self.inst(Inst::Call("sl_rt_alloc".into()));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -16),
            reg("rax"),
        ));
        self.inst(Inst::Mov(
            reg("rcx"),
            Operand::slot(Size::Qword, Reg("rbp"), -8),
        ));
        self.inst(Inst::Mov(
            reg("rcx"),
            Operand::Mem(Mem {
                size: Some(Size::Qword),
                base: Reg("rcx"),
                disp: None,
            }),
        ));
        self.inst(Inst::Mov(
            Operand::Mem(Mem {
                size: Some(Size::Qword),
                base: Reg("rax"),
                disp: None,
            }),
            reg("rcx"),
        ));
        for variant in variants {
            self.inst(Inst::Alu(
                AluOp::Cmp,
                reg("rcx"),
                Operand::Imm(variant.tag as i64),
            ));
            self.inst(Inst::Jcc(
                Cond::E,
                Target::Named(format!(".L{}_clone_variant_{}", symbol, variant.tag)),
            ));
        }
        self.inst(Inst::Jmp(Target::Named(format!(
            ".L{}_clone_return",
            symbol
        ))));
        for variant in variants {
            self.asm.push(Item::Label(format!(
                ".L{}_clone_variant_{}",
                symbol, variant.tag
            )));
            for (index, (_, ty)) in variant.fields.iter().enumerate() {
                self.inst(Inst::Mov(
                    reg("rax"),
                    Operand::slot(Size::Qword, Reg("rbp"), -8),
                ));
                self.inst(Inst::Mov(
                    reg("rdi"),
                    Operand::slot(Size::Qword, Reg("rax"), ((index + 1) * 8) as i64),
                ));
                if let Some(clone_function) = self.clone_function(ty) {
                    self.inst(Inst::Call(clone_function));
                } else {
                    self.inst(Inst::Mov(reg("rax"), reg("rdi")));
                }
                self.inst(Inst::Mov(
                    reg("rcx"),
                    Operand::slot(Size::Qword, Reg("rbp"), -16),
                ));
                self.inst(Inst::Mov(
                    Operand::slot(Size::Qword, Reg("rcx"), ((index + 1) * 8) as i64),
                    reg("rax"),
                ));
            }
            self.inst(Inst::Jmp(Target::Named(format!(
                ".L{}_clone_return",
                symbol
            ))));
        }
        self.asm
            .push(Item::Label(format!(".L{}_clone_return", symbol)));
        self.inst(Inst::Mov(
            reg("rax"),
            Operand::slot(Size::Qword, Reg("rbp"), -16),
        ));
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
    }

    fn struct_drop_helper(&mut self, name: &str, fields: &[(String, Type)]) {
        let symbol = struct_drop_symbol(name);
        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.asm.push(Item::Label(symbol.to_owned()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        // Match sl_rt_string_drop/sl_rt_list_drop: a null pointer is a no-op
        // rather than a wild load, so a dropped-and-zeroed slot stays benign.
        self.inst(Inst::Test(reg("rdi"), reg("rdi")));
        self.inst(Inst::Jcc(
            Cond::E,
            Target::Named(format!(".L{}_return", symbol)),
        ));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        for (index, (_, ty)) in fields.iter().enumerate().rev() {
            let drop_function = self.drop_function(ty);
            if let Some(drop_function) = drop_function {
                self.inst(Inst::Mov(
                    reg("rax"),
                    Operand::slot(Size::Qword, Reg("rbp"), -8),
                ));
                self.inst(Inst::Mov(
                    reg("rdi"),
                    Operand::slot(Size::Qword, Reg("rax"), (index * 8) as i64),
                ));
                self.inst(Inst::Call(drop_function));
            }
        }
        self.inst(Inst::Mov(
            reg("rdi"),
            Operand::slot(Size::Qword, Reg("rbp"), -8),
        ));
        self.inst(Inst::Call("sl_rt_free".into()));
        self.asm.push(Item::Label(format!(".L{}_return", symbol)));
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
    }

    fn enum_drop_helper(&mut self, name: &str, variants: &[crate::mir::MirVariant]) {
        let symbol = enum_drop_symbol(name);
        self.asm.push(Item::Global(symbol.to_owned()));
        self.asm.push(Item::Function(symbol.to_owned()));
        self.asm.push(Item::Label(symbol.to_owned()));
        self.inst(Inst::Push(reg("rbp")));
        self.inst(Inst::Mov(reg("rbp"), reg("rsp")));
        self.inst(Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(16)));
        // As in the struct helper: tolerate a null pointer instead of loading
        // the tag from address zero.
        self.inst(Inst::Test(reg("rdi"), reg("rdi")));
        self.inst(Inst::Jcc(
            Cond::E,
            Target::Named(format!(".L{}_return", symbol)),
        ));
        self.inst(Inst::Mov(
            Operand::slot(Size::Qword, Reg("rbp"), -8),
            reg("rdi"),
        ));
        self.inst(Inst::Mov(
            reg("rax"),
            Operand::Mem(Mem {
                size: Some(Size::Qword),
                base: Reg("rdi"),
                disp: None,
            }),
        ));
        for variant in variants {
            self.inst(Inst::Alu(
                AluOp::Cmp,
                reg("rax"),
                Operand::Imm(variant.tag as i64),
            ));
            self.inst(Inst::Jcc(
                Cond::E,
                Target::Named(format!(".L{}_variant_{}", symbol, variant.tag)),
            ));
        }
        self.inst(Inst::Jmp(Target::Named(format!(".L{}_free", symbol))));
        for variant in variants {
            self.asm
                .push(Item::Label(format!(".L{}_variant_{}", symbol, variant.tag)));
            for (index, (_, ty)) in variant.fields.iter().enumerate().rev() {
                if let Some(drop_function) = self.drop_function(ty) {
                    self.inst(Inst::Mov(
                        reg("rax"),
                        Operand::slot(Size::Qword, Reg("rbp"), -8),
                    ));
                    self.inst(Inst::Mov(
                        reg("rdi"),
                        Operand::slot(Size::Qword, Reg("rax"), ((index + 1) * 8) as i64),
                    ));
                    self.inst(Inst::Call(drop_function));
                }
            }
            self.inst(Inst::Jmp(Target::Named(format!(".L{}_free", symbol))));
        }
        self.asm.push(Item::Label(format!(".L{}_free", symbol)));
        self.inst(Inst::Mov(
            reg("rdi"),
            Operand::slot(Size::Qword, Reg("rbp"), -8),
        ));
        self.inst(Inst::Call("sl_rt_free".into()));
        self.asm.push(Item::Label(format!(".L{}_return", symbol)));
        self.inst(Inst::Mov(reg("rsp"), reg("rbp")));
        self.inst(Inst::Pop(Reg("rbp")));
        self.inst(Inst::Ret);
        self.asm.push(Item::Size(symbol.to_owned()));
    }

    fn drop_function(&self, ty: &Type) -> Option<String> {
        drop_function(self.module, ty)
    }

    fn clone_function(&self, ty: &Type) -> Option<String> {
        clone_function(self.module, ty)
    }

    /// A callee's object-file name. Tests are always Slopium's own; anything
    /// else may be an `extern`, whose symbol is C's rather than ours.
    fn symbol(&self, name: &str, is_test: bool) -> String {
        if is_test {
            function_symbol(name, true)
        } else {
            call_symbol(self.module, name)
        }
    }
}

/// The assembly operand naming a local, ready to substitute into any
/// instruction that accepts a register or a 64-bit memory operand.
fn operand(allocation: &Allocation, file: &RegisterFile, local: LocalId) -> Operand {
    match allocation.location(local) {
        Location::Register(register) => Operand::Reg(Reg(file.wide[register])),
        Location::Memory(slot) => Operand::Mem(frame_slot(Some(Size::Qword), slot)),
    }
}

/// The address of a local, for the instructions that need one.
///
/// `lea` has no register form, so every local reaching here must have been
/// pinned to memory by [`address_taken`]. The two are kept in step by a test.
fn address(allocation: &Allocation, local: LocalId) -> Operand {
    match allocation.location(local) {
        Location::Memory(slot) => Operand::Mem(frame_slot(None, slot)),
        Location::Register(_) => {
            unreachable!("local {local} has its address taken but was given a register")
        }
    }
}

/// The `n`th frame slot, with the size prefix an instruction reading it needs
/// and `None` for `lea`, which takes the address rather than the value.
fn frame_slot(size: Option<Size>, slot: usize) -> Mem {
    Mem {
        size,
        base: Reg("rbp"),
        disp: Some(-(((slot + 1) * 8) as i64)),
    }
}

fn in_memory(allocation: &Allocation, local: LocalId) -> bool {
    matches!(allocation.location(local), Location::Memory(_))
}

/// The 32-bit view of a local, for `i32` arithmetic.
fn narrow_operand(allocation: &Allocation, file: &RegisterFile, local: LocalId) -> Operand {
    match allocation.location(local) {
        Location::Register(register) => Operand::Reg(Reg(file.narrow[register])),
        Location::Memory(slot) => Operand::Mem(frame_slot(Some(Size::Dword), slot)),
    }
}

fn set_condition(op: BinaryOp) -> Cond {
    match op {
        BinaryOp::Less => Cond::L,
        BinaryOp::Greater => Cond::G,
        BinaryOp::LessEqual => Cond::Le,
        BinaryOp::GreaterEqual => Cond::Ge,
        BinaryOp::Equal => Cond::E,
        BinaryOp::NotEqual => Cond::Ne,
        _ => unreachable!("only the comparison operators produce a flag byte"),
    }
}

pub(crate) fn align_to(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::{
        address_taken, backend_for, is_location, reg, AluOp, Inst, Operand, Reg, Size,
        CALLEE_SAVED, DEFAULT_TARGET, LEAF, TARGETS, TARGET_TRIPLES,
    };
    use crate::ast::Type;
    use crate::cfg::Cfg;
    use crate::mir::{BasicBlock, Instruction, MirFunction, MirLocal, MirModule, Terminator};
    use crate::regalloc::{allocate, Location};
    use crate::{compile_to_assembly, CompileOptions};

    fn assemble(source: &str) -> String {
        compile_to_assembly("test.slp", source, &CompileOptions::default()).unwrap()
    }

    /// The serialized triple list and the target table are two spellings of
    /// one thing, and a backend added to only one of them would either be
    /// unreachable or advertised without existing.
    #[test]
    fn every_target_is_listed_once() {
        let from_table: Vec<&str> = TARGETS.iter().map(|spec| spec.triple).collect();
        assert_eq!(from_table, TARGET_TRIPLES);
        for spec in TARGETS {
            assert!(
                backend_for(spec.triple).is_some(),
                "{} is listed but has no backend",
                spec.triple
            );
        }
        assert!(backend_for(DEFAULT_TARGET).is_some());
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

        let module = MirModule {
            functions: Vec::new(),
            externs: Vec::new(),
            tests: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
        };
        let pinned = address_taken(&module, &function);
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

    /// The peephole itself is [`crate::asm::Assembly::remove_redundant_copies`]
    /// and is tested there. What is architecture-specific, and tested here, is
    /// which instructions it is allowed to see as a mirrored pair — that is
    /// `Inst::undo`, and getting it wrong would delete something that is not a
    /// copy.
    #[test]
    fn only_a_copy_undoes_a_copy() {
        let pairs: Vec<(Inst, Option<Inst>)> = vec![
            (
                Inst::Mov(reg("r13"), reg("rax")),
                Some(Inst::Mov(reg("rax"), reg("r13"))),
            ),
            (
                Inst::Mov(Operand::slot(Size::Qword, Reg("rbp"), -8), reg("rax")),
                Some(Inst::Mov(
                    reg("rax"),
                    Operand::slot(Size::Qword, Reg("rbp"), -8),
                )),
            ),
            (Inst::Alu(AluOp::Add, reg("rax"), reg("rcx")), None),
            (Inst::Lea(Reg("rax"), Operand::Rip(".Lstr".into())), None),
            (Inst::Push(reg("rbp")), None),
            (Inst::Call("sl_rt_alloc".into()), None),
        ];
        for (instruction, undo) in pairs {
            assert_eq!(
                crate::asm::Instruction::undo(&instruction),
                undo,
                "for `{instruction}`"
            );
        }
    }

    #[test]
    fn a_copy_that_undoes_the_previous_one_is_deleted() {
        let source = "(fn identity ((n i32)) -> i32 n)\n(fn main () -> i32 (identity 1))";
        let assembly = compile_to_assembly("copy.slp", source, &CompileOptions::default()).unwrap();
        for pair in assembly.lines().collect::<Vec<_>>().windows(2) {
            let (first, second) = (pair[0].trim(), pair[1].trim());
            let operands = |line: &str| {
                line.strip_prefix("mov ")
                    .and_then(|rest| rest.split_once(", "))
                    .map(|(dst, src)| (dst.to_owned(), src.to_owned()))
            };
            if let (Some((dst, src)), Some((next_dst, next_src))) =
                (operands(first), operands(second))
            {
                assert!(
                    !(dst == next_src && src == next_dst),
                    "`{first}` and `{second}` undo each other"
                );
            }
        }
    }

    const DEBUGGED: &str = "\
(fn square ((n i64)) -> i64
  (* n n))

(fn main () -> i32
  (let a 6)
  (let b (square a))
  (note b)
  0)

(extern \"sl_rt_note\" (note (value i64)) -> unit)
";

    fn assemble_with_debug(file: &str, source: &str) -> String {
        compile_to_assembly(
            file,
            source,
            &CompileOptions {
                debug: true,
                ..CompileOptions::default()
            },
        )
        .unwrap()
    }

    /// A NaN compares false three ways, on the machine as well as in the
    /// folder. `ucomisd` alone answers otherwise, so each of the three has a
    /// sequence that excludes the unordered case.
    #[test]
    fn a_float_comparison_excludes_the_unordered_case() {
        let less = assemble("(fn a () -> f64 1.0)\n(fn main () -> i32 (if (< (a) (a)) 1 0))");
        assert!(
            less.contains("ucomisd xmm1, xmm0") && less.contains("seta al"),
            "less-than reverses the operands so `seta` answers it:\n{less}"
        );
        assert!(!less.contains("setb"), "`setb` is true for a NaN:\n{less}");

        let greater = assemble("(fn a () -> f64 1.0)\n(fn main () -> i32 (if (> (a) (a)) 1 0))");
        assert!(greater.contains("ucomisd xmm0, xmm1") && greater.contains("seta al"));

        let equal = assemble("(fn a () -> f64 1.0)\n(fn main () -> i32 (if (= (a) (a)) 1 0))");
        assert!(
            equal.contains("sete al") && equal.contains("setnp cl") && equal.contains("and al, cl"),
            "equality has to and in the parity flag:\n{equal}"
        );
    }

    #[test]
    fn no_line_directives_without_debug() {
        let assembly = assemble(DEBUGGED);
        assert!(!assembly.contains(".loc "), "{assembly}");
        assert!(!assembly.contains(".file "), "{assembly}");
    }

    /// The property that makes `--debug` safe to turn on: it adds directives
    /// and changes nothing else. Anything that made a `.loc` alter register
    /// allocation, instruction selection or the peephole would show up here.
    #[test]
    fn debug_information_adds_directives_and_changes_no_instruction() {
        let plain = assemble(DEBUGGED);
        let debugged = assemble_with_debug("test.slp", DEBUGGED);
        let stripped: String = debugged
            .lines()
            .filter(|line| !is_location(line) && !line.starts_with(".file "))
            .map(|line| format!("{line}\n"))
            .collect();

        assert_eq!(stripped, plain);
    }

    #[test]
    fn every_line_directive_names_a_line_of_its_file() {
        let assembly = assemble_with_debug("test.slp", DEBUGGED);
        assert_eq!(assembly.matches(".file ").count(), 1);
        assert!(assembly.contains(".file 1 \"test.slp\""));

        let lines = DEBUGGED.lines().count();
        let mut seen = Vec::new();
        for directive in assembly.lines().filter(|line| is_location(line)) {
            let fields: Vec<&str> = directive.split_whitespace().collect();
            let [_, file, line, column] = fields[..] else {
                panic!("`{directive}` is not `.loc FILE LINE COLUMN`");
            };
            assert_eq!(file, "1", "only one file is in play");
            let line: usize = line.parse().expect("a line number");
            let column: usize = column.parse().expect("a column number");
            assert!(
                (1..=lines).contains(&line),
                "`{directive}` names a line outside a {lines}-line file"
            );
            assert!(column >= 1, "DWARF column 0 means `unknown`");
            seen.push(line);
        }

        // Both function bodies and the statements between them are covered.
        for line in [1, 2, 4, 5, 6, 7, 8] {
            assert!(seen.contains(&line), "line {line} has no row: {seen:?}");
        }
    }

    /// A statement is attributed to the module it was written in, not to
    /// whichever module the object happens to be emitted for.
    #[test]
    fn a_multi_module_package_numbers_every_file_it_names() {
        use crate::package::{PackageInput, PackageSource};

        let source = |module: &str, text: &str| PackageSource {
            path: format!("src/{module}.slp"),
            namespace: None,
            module: module.to_owned(),
            source: text.to_owned(),
        };
        let input = PackageInput {
            name: "demo".into(),
            entry_module: "main".into(),
            files: vec![
                source(
                    "helper",
                    "(export twice)\n(fn twice ((n i64)) -> i64\n  (* n 2))\n",
                ),
                source(
                    "main",
                    "(take helper twice)\n(fn main () -> i32\n  (let doubled (twice 21))\n  0)\n",
                ),
            ],
        };
        let assembly = crate::compile_package_to_assembly(
            &input,
            &CompileOptions {
                debug: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();

        assert!(
            assembly.contains(".file 1 \"src/helper.slp\""),
            "{assembly}"
        );
        assert!(assembly.contains(".file 2 \"src/main.slp\""), "{assembly}");
        assert!(
            body_of(&assembly, "helper:twice").contains(".loc 1 3 "),
            "`twice` is attributed to helper.slp line 3:\n{assembly}"
        );
        assert!(
            assembly.contains(".loc 2 3 "),
            "`main` is attributed to main.slp line 3:\n{assembly}"
        );
    }

    /// A test body is code only the harness reaches, so a build without one
    /// must not carry it — nor the runtime entry it calls, which the linker
    /// then drops because nothing else refers to it.
    #[test]
    fn a_test_reaches_the_binary_only_through_the_harness() {
        let source = "\
(fn double ((n i64)) -> i64 (* n 2))

(test \"doubling\"
  (= (double 3) 6))

(fn main () -> i32 0)";

        let plain = compile_to_assembly("t.slp", source, &CompileOptions::default()).unwrap();
        assert!(
            !plain.contains("sl_test_"),
            "an ordinary build emitted a test body:\n{plain}"
        );
        assert!(
            !plain.contains("sl_rt_test_result"),
            "an ordinary build kept the test-reporting entry:\n{plain}"
        );

        let harness = compile_to_assembly(
            "t.slp",
            source,
            &CompileOptions {
                test_harness: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        assert!(
            harness.contains("sl_test_") && harness.contains("sl_rt_test_result"),
            "a --test build lost its test:\n{harness}"
        );
    }

    /// A program carries a trap message only when a check can reach it.
    #[test]
    fn a_program_carries_only_the_trap_messages_it_can_reach() {
        let assembly = |source: &str| {
            compile_to_assembly("t.slp", source, &CompileOptions::default()).unwrap()
        };

        // No arithmetic at all: neither trap. The parameters keep the operands
        // out of the constant folder, which would otherwise answer at compile
        // time and emit no check.
        let none = assembly("(fn main () -> i32 0)");
        assert!(!none.contains(".Lsl_panic_overflow:"), "{none}");
        assert!(!none.contains(".Lsl_panic_div_zero:"), "{none}");

        // Addition overflows but never divides: overflow only.
        let adds = assembly("(fn add ((a i64) (b i64)) -> i64 (+ a b))\n(fn main () -> i32 0)");
        assert!(adds.contains(".Lsl_panic_overflow:"), "{adds}");
        assert!(adds.contains(".Lsl_panic_overflow_trampoline:"), "{adds}");
        assert!(!adds.contains(".Lsl_panic_div_zero:"), "{adds}");
        assert!(!adds.contains(".Lsl_panic_div_zero_trampoline:"), "{adds}");

        // Division reaches both — the zero divisor and the INT_MIN/-1 overflow.
        let divides = assembly("(fn div ((a i64) (b i64)) -> i64 (/ a b))\n(fn main () -> i32 0)");
        assert!(divides.contains(".Lsl_panic_div_zero:"), "{divides}");
        assert!(divides.contains(".Lsl_panic_overflow:"), "{divides}");
    }

    /// `panic = "abort"` emits no message and calls the message-less entry.
    #[test]
    fn an_aborting_build_carries_no_trap_message() {
        let source = "(fn div ((a i64) (b i64)) -> i64 (/ a b))\n(fn main () -> i32 0)";
        let aborting = compile_to_assembly(
            "t.slp",
            source,
            &CompileOptions {
                panic_abort: true,
                ..CompileOptions::default()
            },
        )
        .unwrap();
        assert!(!aborting.contains(".Lsl_panic_div_zero:"), "{aborting}");
        assert!(!aborting.contains(".Lsl_panic_overflow:"), "{aborting}");
        assert!(aborting.contains("call sl_rt_abort"), "{aborting}");
        assert!(!aborting.contains("call sl_rt_panic"), "{aborting}");
        // The trampolines still exist and still trap — only the message is gone.
        assert!(
            aborting.contains(".Lsl_panic_div_zero_trampoline:"),
            "{aborting}"
        );
    }
}
