use crate::ast::Type;
use crate::diagnostic::{codes, CompileResult, Diagnostic};
use crate::mir::{BasicBlock, BinaryOp, Instruction, LocalId, MirFunction, MirModule, Terminator};
use std::collections::HashMap;
use std::fmt::Write;

pub const SUPPORTED_TARGET: &str = "x86_64-unknown-linux-gnu";

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
            Ok(self.output)
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
        let frame_size = align_to(function.locals.len() * 8, 16);

        writeln!(self.output, ".globl {symbol}").unwrap();
        writeln!(self.output, ".type {symbol}, @function").unwrap();
        writeln!(self.output, "{symbol}:").unwrap();
        writeln!(self.output, "  push rbp").unwrap();
        writeln!(self.output, "  mov rbp, rsp").unwrap();
        if frame_size != 0 {
            writeln!(self.output, "  sub rsp, {frame_size}").unwrap();
        }

        self.store_parameters(function);
        for (block_id, block) in function.blocks.iter().enumerate() {
            writeln!(self.output, ".L{}_bb{}:", symbol, block_id).unwrap();
            self.basic_block(function, block, &symbol, &epilogue);
        }

        writeln!(self.output, "{epilogue}:").unwrap();
        writeln!(self.output, "  mov rsp, rbp").unwrap();
        writeln!(self.output, "  pop rbp").unwrap();
        writeln!(self.output, "  ret").unwrap();
        writeln!(self.output, ".size {symbol}, .-{symbol}").unwrap();
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
                    writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*local)).unwrap();
                    stack += 1;
                } else {
                    writeln!(
                        self.output,
                        "  movq QWORD PTR {}, {}",
                        slot(*local),
                        float_regs[floats]
                    )
                    .unwrap();
                    floats += 1;
                }
            } else {
                if integers >= integer_regs.len() {
                    writeln!(self.output, "  mov rax, QWORD PTR [rbp+{}]", 16 + stack * 8).unwrap();
                    writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*local)).unwrap();
                    stack += 1;
                } else {
                    writeln!(
                        self.output,
                        "  mov QWORD PTR {}, {}",
                        slot(*local),
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
                        writeln!(self.output, "  movq xmm0, QWORD PTR {}", slot(*local)).unwrap();
                    } else {
                        writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*local)).unwrap();
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
                writeln!(self.output, "  cmp QWORD PTR {}, 0", slot(*condition)).unwrap();
                writeln!(self.output, "  jne .L{}_bb{}", symbol, then_block).unwrap();
                writeln!(self.output, "  jmp .L{}_bb{}", symbol, else_block).unwrap();
            }
            Terminator::Unreachable => writeln!(self.output, "  ud2").unwrap(),
        }
    }

    fn instruction(&mut self, function: &MirFunction, instruction: &Instruction) {
        match instruction {
            Instruction::ConstInt { dst, value } => {
                writeln!(self.output, "  mov rax, {value}").unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::ConstFloat { dst, bits } => {
                writeln!(self.output, "  mov rax, {bits}").unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::ConstBool { dst, value } => {
                writeln!(
                    self.output,
                    "  mov QWORD PTR {}, {}",
                    slot(*dst),
                    i32::from(*value)
                )
                .unwrap();
            }
            Instruction::StringNew { dst, value } => {
                let label = self.string_ids[value].clone();
                writeln!(self.output, "  lea rdi, {label}[rip]").unwrap();
                writeln!(self.output, "  mov rsi, {}", value.len()).unwrap();
                writeln!(self.output, "  call sl_rt_string_new").unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::Assign { dst, src } => {
                writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*src)).unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::AddressOf { dst, src } => {
                if matches!(
                    function.locals[*src].ty,
                    Type::String
                        | Type::List(_)
                        | Type::Array { .. }
                        | Type::Slice(_)
                        | Type::Named(_)
                ) {
                    writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*src)).unwrap();
                } else {
                    writeln!(self.output, "  lea rax, {}", slot(*src)).unwrap();
                }
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
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
                writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(*local)).unwrap();
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
                writeln!(self.output, "  mov QWORD PTR {}, 0", slot(*local)).unwrap();
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
                    writeln!(self.output, "  mov rcx, QWORD PTR {}", slot(*field)).unwrap();
                    writeln!(self.output, "  mov QWORD PTR [rax+{}], rcx", index * 8).unwrap();
                }
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::FieldLoad { dst, base, index } => {
                writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*base)).unwrap();
                writeln!(self.output, "  mov rcx, QWORD PTR [rax+{}]", index * 8).unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rcx", slot(*dst)).unwrap();
            }
            Instruction::EnumNew {
                dst, tag, fields, ..
            } => {
                let size = ((fields.len() + 1) * 8).max(8);
                writeln!(self.output, "  mov rdi, {size}").unwrap();
                writeln!(self.output, "  call sl_rt_alloc").unwrap();
                writeln!(self.output, "  mov QWORD PTR [rax], {tag}").unwrap();
                for (index, field) in fields.iter().enumerate() {
                    writeln!(self.output, "  mov rcx, QWORD PTR {}", slot(*field)).unwrap();
                    writeln!(
                        self.output,
                        "  mov QWORD PTR [rax+{}], rcx",
                        (index + 1) * 8
                    )
                    .unwrap();
                }
                writeln!(self.output, "  mov QWORD PTR {}, rax", slot(*dst)).unwrap();
            }
            Instruction::EnumTag { dst, base } => {
                writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*base)).unwrap();
                writeln!(self.output, "  mov rcx, QWORD PTR [rax]").unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rcx", slot(*dst)).unwrap();
            }
            Instruction::EnumFieldLoad { dst, base, index } => {
                writeln!(self.output, "  mov rax, QWORD PTR {}", slot(*base)).unwrap();
                writeln!(
                    self.output,
                    "  mov rcx, QWORD PTR [rax+{}]",
                    (index + 1) * 8
                )
                .unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, rcx", slot(*dst)).unwrap();
            }
            Instruction::Free { local } => {
                writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(*local)).unwrap();
                writeln!(self.output, "  call sl_rt_free").unwrap();
                writeln!(self.output, "  mov QWORD PTR {}, 0", slot(*local)).unwrap();
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
        writeln!(self.output, "  mov rax, QWORD PTR {}", slot(lhs)).unwrap();
        writeln!(self.output, "  mov rcx, QWORD PTR {}", slot(rhs)).unwrap();
        let width = if *ty == Type::I32 { "e" } else { "r" };
        let accumulator = if width == "e" { "eax" } else { "rax" };
        let operand = if width == "e" { "ecx" } else { "rcx" };
        match op {
            BinaryOp::Add => {
                writeln!(self.output, "  add {accumulator}, {operand}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Sub => {
                writeln!(self.output, "  sub {accumulator}, {operand}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Mul => {
                writeln!(self.output, "  imul {accumulator}, {operand}").unwrap();
                writeln!(self.output, "  jo .Lsl_panic_overflow_trampoline").unwrap();
            }
            BinaryOp::Div => {
                writeln!(self.output, "  test {operand}, {operand}").unwrap();
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
                let condition = match op {
                    BinaryOp::Less => "setl",
                    BinaryOp::Greater => "setg",
                    BinaryOp::Equal => "sete",
                    _ => unreachable!(),
                };
                writeln!(self.output, "  {condition} al").unwrap();
                writeln!(self.output, "  movzx rax, al").unwrap();
            }
        }
        if *ty == Type::I32 && !matches!(op, BinaryOp::Less | BinaryOp::Greater | BinaryOp::Equal) {
            writeln!(self.output, "  movsxd rax, eax").unwrap();
        }
        writeln!(self.output, "  mov QWORD PTR {}, rax", slot(dst)).unwrap();
    }

    fn float_binary(&mut self, dst: LocalId, op: BinaryOp, lhs: LocalId, rhs: LocalId) {
        writeln!(self.output, "  movq xmm0, QWORD PTR {}", slot(lhs)).unwrap();
        writeln!(self.output, "  movq xmm1, QWORD PTR {}", slot(rhs)).unwrap();
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
        writeln!(self.output, "  movq QWORD PTR {}, xmm0", slot(dst)).unwrap();
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
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
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
            writeln!(self.output, "  mov QWORD PTR {}, rax", slot(dst)).unwrap();
            for arg in args {
                writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(dst)).unwrap();
                writeln!(self.output, "  lea rsi, {}", slot(*arg)).unwrap();
                writeln!(self.output, "  call sl_rt_list_push").unwrap();
            }
            writeln!(self.output, "  mov rax, QWORD PTR {}", slot(dst)).unwrap();
        } else if callee == "slice" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  mov rsi, QWORD PTR {}", slot(args[1])).unwrap();
            writeln!(self.output, "  mov rdx, QWORD PTR {}", slot(args[2])).unwrap();
            writeln!(self.output, "  call sl_rt_slice_new").unwrap();
        } else if callee == "len" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            if reference_is_slice(&arg_types[0]) {
                writeln!(self.output, "  call sl_rt_slice_len").unwrap();
            } else {
                writeln!(self.output, "  call sl_rt_list_len").unwrap();
            }
        } else if callee == "push" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  lea rsi, {}", slot(args[1])).unwrap();
            writeln!(self.output, "  call sl_rt_list_push").unwrap();
        } else if callee == "get" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  mov rsi, QWORD PTR {}", slot(args[1])).unwrap();
            if reference_is_slice(&arg_types[0]) {
                writeln!(self.output, "  call sl_rt_slice_get").unwrap();
            } else {
                writeln!(self.output, "  call sl_rt_list_get").unwrap();
            }
            writeln!(self.output, "  mov rax, QWORD PTR [rax]").unwrap();
        } else if callee == "get-ref" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  mov rsi, QWORD PTR {}", slot(args[1])).unwrap();
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
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  lea rsi, {}", slot(dst)).unwrap();
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
            writeln!(self.output, "  mov rcx, QWORD PTR {}", slot(dst)).unwrap();
            writeln!(self.output, "  mov QWORD PTR [rax+8], rcx").unwrap();
            writeln!(self.output, "  jmp 2f").unwrap();
            writeln!(self.output, "1:").unwrap();
            writeln!(self.output, "  mov rdi, 8").unwrap();
            writeln!(self.output, "  call sl_rt_alloc").unwrap();
            writeln!(self.output, "  mov QWORD PTR [rax], {none_tag}").unwrap();
            writeln!(self.output, "2:").unwrap();
        } else if callee == "remove" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  mov rsi, QWORD PTR {}", slot(args[1])).unwrap();
            writeln!(self.output, "  call sl_rt_list_remove").unwrap();
        } else if callee == "read-i64" {
            writeln!(self.output, "  call sl_rt_read_i64").unwrap();
        } else if callee == "read-line" {
            writeln!(self.output, "  call sl_rt_read_line").unwrap();
        } else if callee == "parse-i64" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  call sl_rt_parse_i64").unwrap();
        } else if callee == "env" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  call sl_rt_env").unwrap();
        } else if callee == "args-len" {
            writeln!(self.output, "  call sl_rt_args_len").unwrap();
        } else if callee == "arg" {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
            writeln!(self.output, "  call sl_rt_arg").unwrap();
        } else if matches!(callee, "print" | "println") {
            writeln!(self.output, "  mov rdi, QWORD PTR {}", slot(args[0])).unwrap();
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
                            "  movq {}, QWORD PTR {}",
                            float_regs[floats],
                            slot(*arg)
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
                            "  mov {}, QWORD PTR {}",
                            integer_regs[integers],
                            slot(*arg)
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
                writeln!(self.output, "  push QWORD PTR {}", slot(*arg)).unwrap();
            }
            writeln!(self.output, "  call {}", self.symbol(callee, false)).unwrap();
            let cleanup = (stack_args.len() + padding) * 8;
            if cleanup != 0 {
                writeln!(self.output, "  add rsp, {cleanup}").unwrap();
            }
        }
        match result {
            Type::Unit => {}
            Type::F64 => writeln!(self.output, "  movq QWORD PTR {}, xmm0", slot(dst)).unwrap(),
            _ => writeln!(self.output, "  mov QWORD PTR {}, rax", slot(dst)).unwrap(),
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

fn slot(local: LocalId) -> String {
    format!("[rbp-{}]", (local + 1) * 8)
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
    use crate::{compile_to_assembly, CompileOptions};

    #[test]
    fn emits_native_function_and_checked_add() {
        let source = "(fn main () -> i32 (+ 20 22))";
        let assembly = compile_to_assembly("test.slp", source, &CompileOptions::default()).unwrap();
        assert!(assembly.contains(".globl main"));
        assert!(assembly.contains("add eax, ecx"));
        assert!(assembly.contains("jo .Lsl_panic_overflow_trampoline"));
    }
}
