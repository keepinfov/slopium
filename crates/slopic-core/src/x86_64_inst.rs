//! The x86-64 instructions this compiler emits, and how they encode.
//!
//! Same contract as [`crate::aarch64_inst`]: each instruction prints exactly
//! the Intel-syntax text the backend used to write, and encodes to machine
//! code the object suite checks against `as` instruction by instruction.
//!
//! The check there is instruction-by-instruction rather than byte-for-byte,
//! because two encodings of one instruction are both correct and this encoder
//! does not always pick the assembler's. It always uses a 32-bit displacement
//! for a jump, where `as` shortens the ones that fit, and it uses the general
//! immediate forms where `as` has a shorter accumulator-specific one. Both
//! cost a few bytes and neither changes what runs.

use crate::asm::{Code, FixupKind, Instruction, Target};
use std::fmt;

/// A register, named the way the backend already names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reg(pub &'static str);

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// How wide a register is, which decides the operand size and the prefixes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Width {
    Byte,
    Word,
    Dword,
    Qword,
    Xmm,
}

const QWORD: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];
const DWORD: [&str; 16] = [
    "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d",
    "r13d", "r14d", "r15d",
];
/// Four names, and deliberately no more (`D-113`). Numbers 0 to 3 mean
/// `al`/`cl`/`dl`/`bl` whether or not a REX prefix is present; it is 4 to 7
/// that change meaning between `ah`–`bh` and `spl`–`dil`. Keeping the table at
/// four is what leaves that question closed, and the only byte store this
/// compiler emits goes through `cl`.
const BYTE: [&str; 4] = ["al", "cl", "dl", "bl"];
/// The 16-bit names, needed because `mov WORD PTR [rax], cx` cannot be spelled
/// without one and `object-check.sh` compares this text against the platform
/// assembler. Unlike the byte registers these carry no REX aliasing question at
/// all, so the table costs nothing but the lookup.
const WORD: [&str; 4] = ["ax", "cx", "dx", "bx"];

impl Reg {
    pub fn number(self) -> Result<u8, String> {
        self.parts().map(|(number, _)| number)
    }

    pub fn width(self) -> Result<Width, String> {
        self.parts().map(|(_, width)| width)
    }

    fn parts(self) -> Result<(u8, Width), String> {
        if let Some(index) = QWORD.iter().position(|name| *name == self.0) {
            return Ok((index as u8, Width::Qword));
        }
        if let Some(index) = DWORD.iter().position(|name| *name == self.0) {
            return Ok((index as u8, Width::Dword));
        }
        if let Some(index) = BYTE.iter().position(|name| *name == self.0) {
            return Ok((index as u8, Width::Byte));
        }
        if let Some(index) = WORD.iter().position(|name| *name == self.0) {
            return Ok((index as u8, Width::Word));
        }
        if let Some(digits) = self.0.strip_prefix("xmm") {
            if let Ok(number) = digits.parse::<u8>() {
                if number < 16 {
                    return Ok((number, Width::Xmm));
                }
            }
        }
        Err(format!("`{}` is not a register", self.0))
    }
}

/// The size prefix an operand in memory carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    Qword,
    Dword,
    /// The two narrow sizes exist for raw pointers and nothing else
    /// (`D-067`). Every other memory this compiler touches — a frame slot, a
    /// struct field, an enum payload — is a machine word, because a device
    /// register is the only thing whose width the program did not choose.
    Word,
    Byte,
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Size::Qword => "QWORD PTR",
            Size::Dword => "DWORD PTR",
            Size::Word => "WORD PTR",
            Size::Byte => "BYTE PTR",
        })
    }
}

/// A memory operand: one base register and a displacement.
///
/// That is the whole addressing this compiler generates. No index, no scale:
/// every aggregate access has a constant offset, and every frame slot has a
/// fixed one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mem {
    /// Absent for `lea`, whose operand is an address rather than something to
    /// read, and which therefore has no size to declare.
    pub size: Option<Size>,
    pub base: Reg,
    /// `None` prints `[rax]` and `Some(0)` prints `[rax+0]`. They encode the
    /// same; the difference is only what the backend already wrote.
    pub disp: Option<i64>,
}

impl fmt::Display for Mem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(size) = self.size {
            write!(f, "{size} ")?;
        }
        match self.disp {
            None => write!(f, "[{}]", self.base),
            Some(disp) if disp < 0 => write!(f, "[{}-{}]", self.base, -disp),
            Some(disp) => write!(f, "[{}+{disp}]", self.base),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operand {
    Reg(Reg),
    Imm(i64),
    /// A raw 64-bit pattern rather than a number — a double's encoding. It
    /// prints unsigned because that is what it is, and encodes exactly as the
    /// signed value with the same bits would.
    Bits(u64),
    Mem(Mem),
    /// `label[rip]` — an address the linker fills in.
    Rip(String),
}

impl Operand {
    /// A frame slot, the shape almost every memory operand here has.
    pub fn slot(size: Size, base: Reg, disp: i64) -> Self {
        Operand::Mem(Mem {
            size: Some(size),
            base,
            disp: Some(disp),
        })
    }

    fn width(&self) -> Result<Width, String> {
        match self {
            Operand::Reg(register) => register.width(),
            // Spelled out rather than defaulted: a size that fell through to
            // `Qword` here would encode a narrow access as a wide one, which
            // does not fault — it reads or writes the bytes next to the one
            // that was asked for. `None` is `lea`, whose operand is an address
            // and whose width is the register's.
            Operand::Mem(memory) => Ok(match memory.size {
                Some(Size::Byte) => Width::Byte,
                Some(Size::Word) => Width::Word,
                Some(Size::Dword) => Width::Dword,
                Some(Size::Qword) | None => Width::Qword,
            }),
            _ => Ok(Width::Qword),
        }
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Reg(register) => write!(f, "{register}"),
            Operand::Imm(value) => write!(f, "{value}"),
            Operand::Bits(value) => write!(f, "{value}"),
            Operand::Mem(memory) => write!(f, "{memory}"),
            Operand::Rip(label) => write!(f, "{label}[rip]"),
        }
    }
}

/// The flag-reading conditions the backend names.
///
/// `E` and `Z` are the same four bits and two different spellings; both are
/// written, so both are here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cond {
    E,
    Ne,
    O,
    Z,
    G,
    L,
    Ge,
    Le,
    A,
    /// Above or equal — the *unsigned* one. It reads a shift count against the
    /// operand width, where a negative count is an enormous unsigned number and
    /// so trips the same branch, and it reads `ucomisd`'s carry flag, which is
    /// set both when the left side is smaller and when either side is a NaN.
    Ae,
    /// Below — the unsigned counterpart of `L`, and the same encoding the
    /// assembler spells `jc`. A `u64` addition that carried and a `u64`
    /// subtraction that borrowed are both read through it (`D-107`).
    B,
    Be,
    P,
    Np,
}

impl Cond {
    pub fn code(self) -> u8 {
        match self {
            Cond::O => 0x0,
            Cond::B => 0x2,
            Cond::Ae => 0x3,
            Cond::E | Cond::Z => 0x4,
            Cond::Ne => 0x5,
            Cond::Be => 0x6,
            Cond::A => 0x7,
            Cond::P => 0xa,
            Cond::Np => 0xb,
            Cond::L => 0xc,
            Cond::Ge => 0xd,
            Cond::Le => 0xe,
            Cond::G => 0xf,
        }
    }
}

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Cond::E => "e",
            Cond::Ne => "ne",
            Cond::O => "o",
            Cond::Z => "z",
            Cond::G => "g",
            Cond::L => "l",
            Cond::Ge => "ge",
            Cond::Le => "le",
            Cond::A => "a",
            Cond::Ae => "ae",
            Cond::B => "b",
            Cond::Be => "be",
            Cond::P => "p",
            Cond::Np => "np",
        })
    }
}

/// The arithmetic and logical operations that share one encoding shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AluOp {
    Add,
    Or,
    Sub,
    And,
    Xor,
    Cmp,
}

impl AluOp {
    /// The digit in `/n`, which is also the position of this operation in the
    /// family's opcode block.
    fn digit(self) -> u8 {
        match self {
            AluOp::Add => 0,
            AluOp::Or => 1,
            AluOp::And => 4,
            AluOp::Sub => 5,
            AluOp::Xor => 6,
            AluOp::Cmp => 7,
        }
    }

    /// The `r/m, r` opcode. The other three in the group follow from it.
    fn base(self) -> u8 {
        match self {
            AluOp::Add => 0x00,
            AluOp::Or => 0x08,
            AluOp::And => 0x20,
            AluOp::Sub => 0x28,
            AluOp::Xor => 0x30,
            AluOp::Cmp => 0x38,
        }
    }
}

impl fmt::Display for AluOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AluOp::Add => "add",
            AluOp::Or => "or",
            AluOp::Sub => "sub",
            AluOp::And => "and",
            AluOp::Xor => "xor",
            AluOp::Cmp => "cmp",
        })
    }
}

/// The three shifts the backend selects, which share the `D3 /n` and `C1 /n ib`
/// encodings and differ only in the digit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShiftOp {
    /// `shl`, which the assembler also spells `sal`.
    Shl,
    /// `sar` — arithmetic, so the sign is carried down.
    Sar,
    /// `shr` — logical, which is what an unsigned right shift means (`D-107`).
    Shr,
}

impl ShiftOp {
    fn digit(self) -> u8 {
        match self {
            ShiftOp::Shl => 4,
            ShiftOp::Shr => 5,
            ShiftOp::Sar => 7,
        }
    }
}

impl fmt::Display for ShiftOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ShiftOp::Shl => "shl",
            ShiftOp::Sar => "sar",
            ShiftOp::Shr => "shr",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseOp {
    Add,
    Sub,
    Mul,
    Div,
    Ucomi,
}

impl SseOp {
    fn opcode(self) -> (Option<u8>, u8) {
        match self {
            SseOp::Add => (Some(0xf2), 0x58),
            SseOp::Sub => (Some(0xf2), 0x5c),
            SseOp::Mul => (Some(0xf2), 0x59),
            SseOp::Div => (Some(0xf2), 0x5e),
            SseOp::Ucomi => (Some(0x66), 0x2e),
        }
    }
}

impl fmt::Display for SseOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SseOp::Add => "addsd",
            SseOp::Sub => "subsd",
            SseOp::Mul => "mulsd",
            SseOp::Div => "divsd",
            SseOp::Ucomi => "ucomisd",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inst {
    Mov(Operand, Operand),
    Lea(Reg, Operand),
    Alu(AluOp, Operand, Operand),
    /// `imul dst, src`, which has no memory-destination form and so is its own
    /// instruction rather than a member of the family above.
    Imul(Reg, Operand),
    Test(Operand, Operand),
    /// `movzx r64, r8`.
    Movzx(Reg, Reg),
    /// `movzx r64, BYTE PTR [m]` and `movzx r64, WORD PTR [m]` — the two
    /// narrow zero-extending loads (`D-067`).
    ///
    /// Its own variant rather than a widened `Movzx`, because the register
    /// form's source is always a byte register and this one's width comes from
    /// the memory operand. The four- and eight-byte loads need nothing here: a
    /// `mov` into a 32-bit register already zeroes the upper half.
    MovzxMem(Reg, Mem),
    /// `movsxd r64, r32`.
    Movsxd(Reg, Reg),
    /// `movq` between a general register and an SSE register, either way.
    Movq(Reg, Reg),
    Push(Operand),
    Pop(Reg),
    Call(String),
    /// A call through the address in a register (`D-092`).
    ///
    /// Written `call r` here, because this backend emits Intel syntax; a
    /// disassembler shows it as `call *%r`.
    CallReg(Reg),
    Jmp(Target),
    Jcc(Cond, Target),
    Setcc(Cond, Reg),
    /// `shl`/`sar`/`shr r, cl`. The count register is always `cl` — the machine
    /// has no other variable-count form — so it is not an operand here.
    Shift(ShiftOp, Reg),
    /// The same shift by a constant count, `C1 /n ib`.
    ///
    /// A shift the *program* writes always arrives in a register, because sema
    /// refuses an out-of-range literal and the folder turns the rest into a
    /// value. This form is the backend's own: canonicalising an `i8` or an
    /// `i16` into its machine word is `shl r, 56` and `sar r, 56`, which is
    /// register-agnostic where `movsx r64, r8` would need a byte register
    /// (`D-107`).
    ShiftImm(ShiftOp, Reg, u8),
    Idiv(Reg),
    /// `div r` — the unsigned divide, which reads `rdx:rax` and needs `rdx`
    /// cleared rather than sign-extended into.
    Div(Reg),
    /// `mul r` — the unsigned multiply. Its low half agrees with `imul`; what
    /// differs is that it sets the overflow flag from the *unsigned* high half,
    /// which is the only way to ask whether a `u64` product fit.
    Mul(Reg),
    /// Sign-extend `rax` into `rdx`, and its 32-bit counterpart.
    Cqo,
    Cdq,
    Ret,
    Ud2,
    Sse(SseOp, Reg, Reg),
}

impl fmt::Display for Inst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inst::Mov(dst, src) => write!(f, "mov {dst}, {src}"),
            Inst::Lea(dst, src) => write!(f, "lea {dst}, {src}"),
            Inst::Alu(op, dst, src) => write!(f, "{op} {dst}, {src}"),
            Inst::Imul(dst, src) => write!(f, "imul {dst}, {src}"),
            Inst::Test(lhs, rhs) => write!(f, "test {lhs}, {rhs}"),
            Inst::Movzx(dst, src) => write!(f, "movzx {dst}, {src}"),
            Inst::MovzxMem(dst, src) => write!(f, "movzx {dst}, {src}"),
            Inst::Movsxd(dst, src) => write!(f, "movsxd {dst}, {src}"),
            Inst::Movq(dst, src) => write!(f, "movq {dst}, {src}"),
            Inst::Push(operand) => write!(f, "push {operand}"),
            Inst::Pop(register) => write!(f, "pop {register}"),
            Inst::Call(symbol) => write!(f, "call {symbol}"),
            // No `*`: this text is `.intel_syntax noprefix`, where the sigil is
            // an AT&T spelling and `as` rejects it. `scripts/object-check.sh`
            // is what says so, because the object writer never sees this.
            Inst::CallReg(register) => write!(f, "call {register}"),
            Inst::Jmp(target) => write!(f, "jmp {target}"),
            Inst::Jcc(cond, target) => write!(f, "j{cond} {target}"),
            Inst::Setcc(cond, register) => write!(f, "set{cond} {register}"),
            Inst::Shift(op, register) => write!(f, "{op} {register}, cl"),
            Inst::ShiftImm(op, register, count) => write!(f, "{op} {register}, {count}"),
            Inst::Idiv(register) => write!(f, "idiv {register}"),
            Inst::Div(register) => write!(f, "div {register}"),
            Inst::Mul(register) => write!(f, "mul {register}"),
            Inst::Cqo => f.write_str("cqo"),
            Inst::Cdq => f.write_str("cdq"),
            Inst::Ret => f.write_str("ret"),
            Inst::Ud2 => f.write_str("ud2"),
            Inst::Sse(op, dst, src) => write!(f, "{op} {dst}, {src}"),
        }
    }
}

/// The width a register-and-memory `mov` acts at, refusing a disagreement.
///
/// The register and the `PTR` size are written independently, so they can
/// disagree, and a disagreement is the one mistake this feature can make that
/// does not fault: the wrong width writes over the register next to the one
/// that was asked for. It fails to encode instead, which is the same doctrine
/// `Reg::number` follows for a name that is not a register.
///
/// `None` is `lea` and the frame slots written before sizes were tracked; both
/// mean the register decides.
fn access_width(register: Width, memory: &Mem) -> Result<Width, String> {
    let declared = match memory.size {
        Some(Size::Byte) => Width::Byte,
        Some(Size::Word) => Width::Word,
        Some(Size::Dword) => Width::Dword,
        Some(Size::Qword) => Width::Qword,
        None => return Ok(register),
    };
    if declared != register {
        return Err(format!(
            "a `{declared:?}` memory operand does not match a `{register:?}` register"
        ));
    }
    Ok(declared)
}

/// The state of one instruction's encoding.
///
/// x86-64 puts the prefix that widens an instruction *before* the opcode but
/// derives it from operands that are only known once they are examined, so an
/// encoder either looks ahead or buffers. This buffers.
struct Encoding {
    rex: u8,
    wants_rex: bool,
    opcode: Vec<u8>,
    modrm: Option<u8>,
    displacement: Vec<u8>,
    immediate: Vec<u8>,
    prefix: Option<u8>,
    /// A relocation this instruction needs, at an offset from the end of the
    /// displacement it is part of.
    fixup: Option<(FixupKind, Target, i64)>,
}

impl Encoding {
    fn new() -> Self {
        Self {
            rex: 0x40,
            wants_rex: false,
            opcode: Vec::new(),
            modrm: None,
            displacement: Vec::new(),
            immediate: Vec::new(),
            prefix: None,
            fixup: None,
        }
    }

    fn wide(&mut self) {
        self.rex |= 0x08;
        self.wants_rex = true;
    }

    fn extend(&mut self, bit: u8, number: u8) {
        if number >= 8 {
            self.rex |= bit;
            self.wants_rex = true;
        }
    }

    /// `reg` is the register in the ModRM `reg` field; `rm` the one in `r/m`.
    fn registers(&mut self, reg: u8, rm: u8) {
        self.extend(0x04, reg);
        self.extend(0x01, rm);
        self.modrm = Some(0xc0 | ((reg & 7) << 3) | (rm & 7));
    }

    /// A memory operand in `r/m`, with `reg` in the other field.
    fn memory(&mut self, reg: u8, memory: &Mem) -> Result<(), String> {
        let base = memory.base.number()?;
        if base & 7 == 4 {
            return Err(format!(
                "`{}` as a base needs an index byte this encoder does not write",
                memory.base
            ));
        }
        self.extend(0x04, reg);
        self.extend(0x01, base);
        let disp = memory.disp.unwrap_or(0);
        // A zero displacement is free unless the base is `rbp` or `r13`, whose
        // "no displacement" encoding means something else entirely.
        let mode = if disp == 0 && base & 7 != 5 {
            0x00
        } else if (-128..128).contains(&disp) {
            self.displacement.push(disp as u8);
            0x40
        } else {
            let narrow = i32::try_from(disp)
                .map_err(|_| format!("a displacement of {disp} does not fit"))?;
            self.displacement.extend_from_slice(&narrow.to_le_bytes());
            0x80
        };
        self.modrm = Some(mode | ((reg & 7) << 3) | (base & 7));
        Ok(())
    }

    /// A `label[rip]` operand, whose displacement the linker supplies.
    fn rip(&mut self, reg: u8, label: &str) {
        self.extend(0x04, reg);
        self.modrm = Some(((reg & 7) << 3) | 0x05);
        self.displacement.extend_from_slice(&[0; 4]);
        self.fixup = Some((
            FixupKind::Pc32,
            Target::Named(label.to_owned()),
            // Measured from the end of the instruction, which is four bytes on
            // from the field itself.
            -4,
        ));
    }

    fn write(self, code: &mut Code) -> Result<(), String> {
        if let Some(prefix) = self.prefix {
            code.byte(prefix);
        }
        if self.wants_rex {
            code.byte(self.rex);
        }
        code.extend(&self.opcode);
        if let Some(modrm) = self.modrm {
            code.byte(modrm);
        }
        let at = code.here();
        code.extend(&self.displacement);
        if let Some((kind, target, addend)) = self.fixup {
            code.relocate(at, kind, target, addend);
        }
        code.extend(&self.immediate);
        Ok(())
    }
}

/// The immediate encoding for one of the `add`/`cmp` family.
///
/// The one-byte form is used whenever the value fits, which is what makes a
/// frame adjustment three bytes instead of six.
fn alu_immediate(encoding: &mut Encoding, op: AluOp, value: i64) -> Result<u8, String> {
    if (-128..128).contains(&value) {
        encoding.immediate.push(value as u8);
        Ok(0x83)
    } else {
        let narrow = i32::try_from(value)
            .map_err(|_| format!("{value} does not fit in a 32-bit immediate"))?;
        encoding.immediate.extend_from_slice(&narrow.to_le_bytes());
        let _ = op;
        Ok(0x81)
    }
}

impl Instruction for Inst {
    fn undo(&self) -> Option<Self> {
        match self {
            Inst::Mov(dst, src) => Some(Inst::Mov(src.clone(), dst.clone())),
            _ => None,
        }
    }

    fn encode(&self, code: &mut Code) -> Result<(), String> {
        let mut encoding = Encoding::new();
        match self {
            Inst::Mov(dst, src) => match (dst, src) {
                (Operand::Reg(dst), Operand::Reg(src)) => {
                    let (d, width) = (dst.number()?, dst.width()?);
                    let s = src.number()?;
                    if width == Width::Qword {
                        encoding.wide();
                    }
                    encoding.opcode.push(0x89);
                    encoding.registers(s, d);
                }
                (Operand::Reg(dst), Operand::Mem(memory)) => {
                    let width = access_width(dst.width()?, memory)?;
                    match width {
                        Width::Qword => encoding.wide(),
                        Width::Word => encoding.prefix = Some(0x66),
                        _ => {}
                    }
                    // `8a` is the byte form; every other width reads with `8b`.
                    // A 32-bit read zeroes the upper half of the destination,
                    // which is what makes `mov r32, DWORD PTR [r]` the
                    // zero-extending four-byte load with no extra instruction.
                    encoding
                        .opcode
                        .push(if width == Width::Byte { 0x8a } else { 0x8b });
                    encoding.memory(dst.number()?, memory)?;
                }
                (Operand::Mem(memory), Operand::Reg(src)) => {
                    let width = access_width(src.width()?, memory)?;
                    match width {
                        Width::Qword => encoding.wide(),
                        Width::Word => encoding.prefix = Some(0x66),
                        _ => {}
                    }
                    encoding
                        .opcode
                        .push(if width == Width::Byte { 0x88 } else { 0x89 });
                    encoding.memory(src.number()?, memory)?;
                }
                (Operand::Reg(dst), Operand::Bits(bits)) => {
                    let value = &(*bits as i64);
                    let (d, width) = (dst.number()?, dst.width()?);
                    if width == Width::Qword {
                        encoding.wide();
                    }
                    match i32::try_from(*value) {
                        Ok(narrow) => {
                            encoding.opcode.push(0xc7);
                            encoding.registers(0, d);
                            encoding.immediate.extend_from_slice(&narrow.to_le_bytes());
                        }
                        Err(_) => {
                            encoding.extend(0x01, d);
                            encoding.opcode.push(0xb8 + (d & 7));
                            encoding.immediate.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
                (Operand::Reg(dst), Operand::Imm(value)) => {
                    let (d, width) = (dst.number()?, dst.width()?);
                    if width == Width::Qword {
                        encoding.wide();
                    }
                    match i32::try_from(*value) {
                        Ok(narrow) => {
                            encoding.opcode.push(0xc7);
                            encoding.registers(0, d);
                            encoding.immediate.extend_from_slice(&narrow.to_le_bytes());
                        }
                        // No sign-extended form reaches it, so the whole
                        // constant has to be in the instruction.
                        Err(_) => {
                            encoding.extend(0x01, d);
                            encoding.opcode.push(0xb8 + (d & 7));
                            encoding.immediate.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
                (Operand::Mem(memory), Operand::Imm(value)) => {
                    // Spelled out because it used to be `!= Some(Dword)`, which
                    // was correct only while `Dword` and `Qword` were the two
                    // sizes there were: the moment `Byte` existed, a byte store
                    // took REX.W and wrote eight bytes. Nothing emitted one yet,
                    // so this was a trap rather than a bug — and the narrow
                    // immediate forms are refused rather than guessed, because
                    // nothing needs them and `0xc7` is not their opcode.
                    match memory.size {
                        Some(Size::Qword) | None => encoding.wide(),
                        Some(Size::Dword) => {}
                        Some(size) => {
                            return Err(format!("no `mov {size}, imm` is emitted"));
                        }
                    }
                    let narrow = i32::try_from(*value)
                        .map_err(|_| format!("{value} does not fit in a stored immediate"))?;
                    encoding.opcode.push(0xc7);
                    encoding.memory(0, memory)?;
                    encoding.immediate.extend_from_slice(&narrow.to_le_bytes());
                }
                _ => return Err("this pair of operands has no `mov`".into()),
            },
            Inst::Lea(dst, src) => {
                encoding.wide();
                encoding.opcode.push(0x8d);
                match src {
                    Operand::Mem(memory) => encoding.memory(dst.number()?, memory)?,
                    Operand::Rip(label) => encoding.rip(dst.number()?, label),
                    _ => return Err("`lea` takes an address".into()),
                }
            }
            Inst::Alu(op, dst, src) => {
                let width = dst.width()?;
                if width == Width::Qword {
                    encoding.wide();
                }
                let byte = width == Width::Byte;
                match (dst, src) {
                    (Operand::Reg(dst), Operand::Reg(src)) => {
                        encoding.opcode.push(op.base() + if byte { 0 } else { 1 });
                        encoding.registers(src.number()?, dst.number()?);
                    }
                    (Operand::Mem(memory), Operand::Reg(src)) => {
                        encoding.opcode.push(op.base() + 1);
                        encoding.memory(src.number()?, memory)?;
                    }
                    (Operand::Reg(dst), Operand::Mem(memory)) => {
                        encoding.opcode.push(op.base() + 3);
                        encoding.memory(dst.number()?, memory)?;
                    }
                    (Operand::Reg(dst), Operand::Imm(value)) => {
                        let opcode = alu_immediate(&mut encoding, *op, *value)?;
                        encoding.opcode.push(opcode);
                        encoding.registers(op.digit(), dst.number()?);
                    }
                    (Operand::Mem(memory), Operand::Imm(value)) => {
                        let opcode = alu_immediate(&mut encoding, *op, *value)?;
                        encoding.opcode.push(opcode);
                        encoding.memory(op.digit(), memory)?;
                    }
                    _ => return Err(format!("this pair of operands has no `{op}`")),
                }
            }
            Inst::Imul(dst, src) => {
                if dst.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.extend_from_slice(&[0x0f, 0xaf]);
                match src {
                    Operand::Reg(src) => encoding.registers(dst.number()?, src.number()?),
                    Operand::Mem(memory) => encoding.memory(dst.number()?, memory)?,
                    _ => return Err("`imul` takes a register or a location".into()),
                }
            }
            Inst::Test(lhs, rhs) => {
                if lhs.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0x85);
                match (lhs, rhs) {
                    (Operand::Reg(lhs), Operand::Reg(rhs)) => {
                        encoding.registers(rhs.number()?, lhs.number()?)
                    }
                    (Operand::Mem(memory), Operand::Reg(rhs)) => {
                        encoding.memory(rhs.number()?, memory)?
                    }
                    _ => return Err("this pair of operands has no `test`".into()),
                }
            }
            Inst::Movzx(dst, src) => {
                if dst.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.extend_from_slice(&[0x0f, 0xb6]);
                encoding.registers(dst.number()?, src.number()?);
            }
            Inst::MovzxMem(dst, src) => {
                if dst.width()? == Width::Qword {
                    encoding.wide();
                }
                let opcode = match src.size {
                    Some(Size::Byte) => 0xb6,
                    Some(Size::Word) => 0xb7,
                    // Refused rather than guessed: `movzx` from four or eight
                    // bytes is not an instruction, and the caller that wanted
                    // one wanted a plain `mov`.
                    other => return Err(format!("`movzx` cannot read {other:?}")),
                };
                encoding.opcode.extend_from_slice(&[0x0f, opcode]);
                encoding.memory(dst.number()?, src)?;
            }
            Inst::Movsxd(dst, src) => {
                encoding.wide();
                encoding.opcode.push(0x63);
                encoding.registers(dst.number()?, src.number()?);
            }
            Inst::Movq(dst, src) => {
                encoding.prefix = Some(0x66);
                encoding.wide();
                // The direction is read off the register classes: one side is
                // always an SSE register and the other always a general one.
                match (dst.width()?, src.width()?) {
                    (Width::Xmm, Width::Qword) => {
                        encoding.opcode.extend_from_slice(&[0x0f, 0x6e]);
                        encoding.registers(dst.number()?, src.number()?);
                    }
                    (Width::Qword, Width::Xmm) => {
                        encoding.opcode.extend_from_slice(&[0x0f, 0x7e]);
                        encoding.registers(src.number()?, dst.number()?);
                    }
                    _ => return Err("`movq` moves between the two register files".into()),
                }
            }
            Inst::Push(operand) => match operand {
                Operand::Reg(register) => {
                    let number = register.number()?;
                    encoding.extend(0x01, number);
                    encoding.opcode.push(0x50 + (number & 7));
                }
                Operand::Mem(memory) => {
                    encoding.opcode.push(0xff);
                    encoding.memory(6, memory)?;
                }
                _ => return Err("`push` takes a register or a location".into()),
            },
            Inst::Pop(register) => {
                let number = register.number()?;
                encoding.extend(0x01, number);
                encoding.opcode.push(0x58 + (number & 7));
            }
            Inst::Call(symbol) => {
                encoding.opcode.push(0xe8);
                encoding.displacement.extend_from_slice(&[0; 4]);
                encoding.fixup = Some((FixupKind::Plt32, Target::Named(symbol.clone()), -4));
            }
            // `FF /2` already defaults to a 64-bit operand in long mode, so it
            // takes no `REX.W`; `registers` still emits `REX.B` for r8–r15.
            Inst::CallReg(register) => {
                encoding.opcode.push(0xff);
                encoding.registers(2, register.number()?);
            }
            Inst::Jmp(target) => {
                encoding.opcode.push(0xe9);
                encoding.displacement.extend_from_slice(&[0; 4]);
                encoding.fixup = Some((FixupKind::Pc32, target.clone(), -4));
            }
            Inst::Jcc(cond, target) => {
                encoding
                    .opcode
                    .extend_from_slice(&[0x0f, 0x80 + cond.code()]);
                encoding.displacement.extend_from_slice(&[0; 4]);
                encoding.fixup = Some((FixupKind::Pc32, target.clone(), -4));
            }
            Inst::Setcc(cond, register) => {
                encoding
                    .opcode
                    .extend_from_slice(&[0x0f, 0x90 + cond.code()]);
                encoding.registers(0, register.number()?);
            }
            Inst::Shift(op, register) => {
                if register.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0xd3);
                encoding.registers(op.digit(), register.number()?);
            }
            Inst::ShiftImm(op, register, count) => {
                if register.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0xc1);
                encoding.registers(op.digit(), register.number()?);
                encoding.immediate.push(*count);
            }
            Inst::Idiv(register) => {
                if register.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0xf7);
                encoding.registers(7, register.number()?);
            }
            Inst::Div(register) => {
                if register.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0xf7);
                encoding.registers(6, register.number()?);
            }
            Inst::Mul(register) => {
                if register.width()? == Width::Qword {
                    encoding.wide();
                }
                encoding.opcode.push(0xf7);
                encoding.registers(4, register.number()?);
            }
            Inst::Cqo => {
                encoding.wide();
                encoding.opcode.push(0x99);
            }
            Inst::Cdq => encoding.opcode.push(0x99),
            Inst::Ret => encoding.opcode.push(0xc3),
            Inst::Ud2 => encoding.opcode.extend_from_slice(&[0x0f, 0x0b]),
            Inst::Sse(op, dst, src) => {
                let (prefix, opcode) = op.opcode();
                encoding.prefix = prefix;
                encoding.opcode.extend_from_slice(&[0x0f, opcode]);
                encoding.registers(dst.number()?, src.number()?);
            }
        }
        encoding.write(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{Assembly, Item, Section};

    fn bytes(instruction: Inst) -> Vec<u8> {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(instruction));
        assembly.finish().unwrap().text
    }

    fn r(name: &'static str) -> Reg {
        Reg(name)
    }

    fn reg(name: &'static str) -> Operand {
        Operand::Reg(Reg(name))
    }

    fn slot(disp: i64) -> Operand {
        Operand::slot(Size::Qword, Reg("rbp"), disp)
    }

    /// Every expected sequence here came from `as` assembling the text on the
    /// left, so a change to the encoder has to disagree with the assembler out
    /// loud rather than quietly.
    #[test]
    fn every_form_encodes_the_way_the_assembler_encodes_it() {
        let cases: Vec<(Inst, &[u8])> = vec![
            (Inst::Push(reg("rbp")), &[0x55]),
            (Inst::Push(reg("rbx")), &[0x53]),
            (Inst::Push(slot(-8)), &[0xff, 0x75, 0xf8]),
            (Inst::Pop(r("rbp")), &[0x5d]),
            (Inst::Mov(reg("rbp"), reg("rsp")), &[0x48, 0x89, 0xe5]),
            (Inst::Mov(reg("rax"), reg("rdi")), &[0x48, 0x89, 0xf8]),
            (Inst::Mov(reg("rbx"), reg("r12")), &[0x4c, 0x89, 0xe3]),
            (Inst::Mov(reg("r12"), reg("rbx")), &[0x49, 0x89, 0xdc]),
            (Inst::Mov(slot(-8), reg("rdi")), &[0x48, 0x89, 0x7d, 0xf8]),
            (
                Inst::Mov(slot(-8), Operand::Imm(0)),
                &[0x48, 0xc7, 0x45, 0xf8, 0x00, 0x00, 0x00, 0x00],
            ),
            (
                Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: r("rax"),
                        disp: None,
                    }),
                    reg("rcx"),
                ),
                &[0x48, 0x89, 0x08],
            ),
            (
                Inst::Mov(Operand::slot(Size::Qword, r("rax"), 8), reg("rcx")),
                &[0x48, 0x89, 0x48, 0x08],
            ),
            (
                Inst::Mov(Operand::slot(Size::Qword, r("rax"), 24), reg("rcx")),
                &[0x48, 0x89, 0x48, 0x18],
            ),
            (Inst::Mov(reg("rax"), slot(-8)), &[0x48, 0x8b, 0x45, 0xf8]),
            (Inst::Mov(reg("rax"), slot(16)), &[0x48, 0x8b, 0x45, 0x10]),
            (
                Inst::Mov(reg("rax"), Operand::Imm(20)),
                &[0x48, 0xc7, 0xc0, 0x14, 0x00, 0x00, 0x00],
            ),
            (
                Inst::Mov(reg("rax"), Operand::Imm(-1)),
                &[0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff],
            ),
            (
                Inst::Mov(reg("r13"), Operand::Imm(4294967296)),
                &[0x49, 0xbd, 0, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                Inst::Mov(reg("rdx"), Operand::Imm(i64::MIN)),
                &[0x48, 0xba, 0, 0, 0, 0, 0, 0, 0, 0x80],
            ),
            (Inst::Mov(reg("esi"), reg("eax")), &[0x89, 0xc6]),
            (Inst::Alu(AluOp::Xor, reg("eax"), reg("eax")), &[0x31, 0xc0]),
            (
                Inst::Alu(AluOp::Xor, reg("r8d"), reg("r8d")),
                &[0x45, 0x31, 0xc0],
            ),
            (
                Inst::Lea(
                    r("rax"),
                    Operand::Mem(Mem {
                        size: None,
                        base: r("rbp"),
                        disp: Some(-8),
                    }),
                ),
                &[0x48, 0x8d, 0x45, 0xf8],
            ),
            (
                Inst::Alu(AluOp::Add, reg("rax"), reg("rcx")),
                &[0x48, 0x01, 0xc8],
            ),
            (
                Inst::Alu(AluOp::Add, reg("r12"), reg("r13")),
                &[0x4d, 0x01, 0xec],
            ),
            (
                Inst::Alu(AluOp::Add, slot(-8), reg("rax")),
                &[0x48, 0x01, 0x45, 0xf8],
            ),
            (
                Inst::Alu(AluOp::Add, reg("rsp"), Operand::Imm(16)),
                &[0x48, 0x83, 0xc4, 0x10],
            ),
            (
                Inst::Alu(AluOp::Add, reg("rax"), slot(-8)),
                &[0x48, 0x03, 0x45, 0xf8],
            ),
            (
                Inst::Alu(AluOp::Sub, reg("rax"), reg("rcx")),
                &[0x48, 0x29, 0xc8],
            ),
            (
                Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(8)),
                &[0x48, 0x83, 0xec, 0x08],
            ),
            (
                Inst::Alu(AluOp::Sub, reg("rsp"), Operand::Imm(32)),
                &[0x48, 0x83, 0xec, 0x20],
            ),
            (Inst::Alu(AluOp::And, reg("al"), reg("cl")), &[0x20, 0xc8]),
            (
                Inst::Alu(AluOp::Cmp, reg("rax"), reg("rcx")),
                &[0x48, 0x39, 0xc8],
            ),
            (
                Inst::Alu(AluOp::Cmp, reg("rax"), slot(-8)),
                &[0x48, 0x3b, 0x45, 0xf8],
            ),
            (
                Inst::Alu(AluOp::Cmp, reg("rcx"), Operand::Imm(-1)),
                &[0x48, 0x83, 0xf9, 0xff],
            ),
            (
                Inst::Alu(AluOp::Cmp, reg("ecx"), Operand::Imm(-1)),
                &[0x83, 0xf9, 0xff],
            ),
            (Inst::Test(reg("rax"), reg("rax")), &[0x48, 0x85, 0xc0]),
            (Inst::Test(reg("rdi"), reg("rdi")), &[0x48, 0x85, 0xff]),
            (Inst::Imul(r("rax"), reg("rcx")), &[0x48, 0x0f, 0xaf, 0xc1]),
            (Inst::Imul(r("r12"), reg("r13")), &[0x4d, 0x0f, 0xaf, 0xe5]),
            (Inst::Movzx(r("rax"), r("al")), &[0x48, 0x0f, 0xb6, 0xc0]),
            (Inst::Movzx(r("r12"), r("al")), &[0x4c, 0x0f, 0xb6, 0xe0]),
            // The narrow memory a raw pointer reaches through (`D-067`). The
            // byte store takes no REX at all, which is the whole reason the
            // byte-register table can stay at four names: number 1 is `cl`
            // with the prefix and without it.
            (
                Inst::MovzxMem(
                    r("rax"),
                    Mem {
                        size: Some(Size::Byte),
                        base: r("rcx"),
                        disp: None,
                    },
                ),
                &[0x48, 0x0f, 0xb6, 0x01],
            ),
            (
                Inst::MovzxMem(
                    r("rax"),
                    Mem {
                        size: Some(Size::Word),
                        base: r("rcx"),
                        disp: None,
                    },
                ),
                &[0x48, 0x0f, 0xb7, 0x01],
            ),
            (
                Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Byte),
                        base: r("rax"),
                        disp: None,
                    }),
                    reg("cl"),
                ),
                &[0x88, 0x08],
            ),
            // A REX.B for the base leaves the register field alone, so this is
            // still `cl` and never `spl`.
            (
                Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Byte),
                        base: r("r10"),
                        disp: None,
                    }),
                    reg("cl"),
                ),
                &[0x41, 0x88, 0x0a],
            ),
            (
                Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Word),
                        base: r("rax"),
                        disp: None,
                    }),
                    reg("cx"),
                ),
                &[0x66, 0x89, 0x08],
            ),
            (
                Inst::Mov(
                    Operand::Mem(Mem {
                        size: Some(Size::Dword),
                        base: r("rax"),
                        disp: None,
                    }),
                    reg("ecx"),
                ),
                &[0x89, 0x08],
            ),
            // The four-byte load needs no `movzx`: writing a 32-bit register
            // already clears the upper half.
            (
                Inst::Mov(
                    reg("eax"),
                    Operand::Mem(Mem {
                        size: Some(Size::Dword),
                        base: r("rcx"),
                        disp: None,
                    }),
                ),
                &[0x8b, 0x01],
            ),
            (Inst::Movsxd(r("rax"), r("eax")), &[0x48, 0x63, 0xc0]),
            (
                Inst::Movq(r("xmm0"), r("rax")),
                &[0x66, 0x48, 0x0f, 0x6e, 0xc0],
            ),
            (
                Inst::Movq(r("rax"), r("xmm0")),
                &[0x66, 0x48, 0x0f, 0x7e, 0xc0],
            ),
            (Inst::Idiv(r("rcx")), &[0x48, 0xf7, 0xf9]),
            (Inst::Idiv(r("ecx")), &[0xf7, 0xf9]),
            // The unsigned pair, `F7 /6` and `F7 /4` (`D-107`).
            (Inst::Div(r("rcx")), &[0x48, 0xf7, 0xf1]),
            (Inst::Div(r("r9")), &[0x49, 0xf7, 0xf1]),
            (Inst::Mul(r("rcx")), &[0x48, 0xf7, 0xe1]),
            // `D3 /n` — the count is `cl` and is not encoded.
            (Inst::Shift(ShiftOp::Shl, r("rax")), &[0x48, 0xd3, 0xe0]),
            (Inst::Shift(ShiftOp::Sar, r("rax")), &[0x48, 0xd3, 0xf8]),
            (Inst::Shift(ShiftOp::Shr, r("rax")), &[0x48, 0xd3, 0xe8]),
            (Inst::Shift(ShiftOp::Shl, r("eax")), &[0xd3, 0xe0]),
            (Inst::Shift(ShiftOp::Sar, r("eax")), &[0xd3, 0xf8]),
            (Inst::Shift(ShiftOp::Shr, r("eax")), &[0xd3, 0xe8]),
            // `C1 /n ib` — the canonicalising pair for an `i8` and an `i16`.
            (
                Inst::ShiftImm(ShiftOp::Shl, r("rax"), 56),
                &[0x48, 0xc1, 0xe0, 0x38],
            ),
            (
                Inst::ShiftImm(ShiftOp::Sar, r("rax"), 56),
                &[0x48, 0xc1, 0xf8, 0x38],
            ),
            (
                Inst::ShiftImm(ShiftOp::Shl, r("rcx"), 48),
                &[0x48, 0xc1, 0xe1, 0x30],
            ),
            // `FF /2` is 64-bit by default in long mode, so `rax` takes no REX
            // at all and `r11` takes only REX.B.
            (Inst::CallReg(r("rax")), &[0xff, 0xd0]),
            (Inst::CallReg(r("rsp")), &[0xff, 0xd4]),
            (Inst::CallReg(r("r11")), &[0x41, 0xff, 0xd3]),
            (Inst::Cqo, &[0x48, 0x99]),
            (Inst::Cdq, &[0x99]),
            (Inst::Setcc(Cond::E, r("al")), &[0x0f, 0x94, 0xc0]),
            (Inst::Setcc(Cond::G, r("al")), &[0x0f, 0x9f, 0xc0]),
            (Inst::Setcc(Cond::L, r("al")), &[0x0f, 0x9c, 0xc0]),
            (Inst::Setcc(Cond::A, r("al")), &[0x0f, 0x97, 0xc0]),
            (Inst::Setcc(Cond::Np, r("cl")), &[0x0f, 0x9b, 0xc1]),
            (Inst::Setcc(Cond::Le, r("al")), &[0x0f, 0x9e, 0xc0]),
            (Inst::Setcc(Cond::Ge, r("al")), &[0x0f, 0x9d, 0xc0]),
            (Inst::Setcc(Cond::Ae, r("al")), &[0x0f, 0x93, 0xc0]),
            (Inst::Setcc(Cond::P, r("cl")), &[0x0f, 0x9a, 0xc1]),
            (
                Inst::Alu(AluOp::Or, reg("rax"), reg("rcx")),
                &[0x48, 0x09, 0xc8],
            ),
            (Inst::Alu(AluOp::Or, reg("eax"), reg("ecx")), &[0x09, 0xc8]),
            (
                Inst::Alu(AluOp::Or, slot(-8), reg("rax")),
                &[0x48, 0x09, 0x45, 0xf8],
            ),
            (
                Inst::Sse(SseOp::Add, r("xmm0"), r("xmm1")),
                &[0xf2, 0x0f, 0x58, 0xc1],
            ),
            (
                Inst::Sse(SseOp::Sub, r("xmm0"), r("xmm1")),
                &[0xf2, 0x0f, 0x5c, 0xc1],
            ),
            (
                Inst::Sse(SseOp::Mul, r("xmm0"), r("xmm1")),
                &[0xf2, 0x0f, 0x59, 0xc1],
            ),
            (
                Inst::Sse(SseOp::Div, r("xmm0"), r("xmm1")),
                &[0xf2, 0x0f, 0x5e, 0xc1],
            ),
            (
                Inst::Sse(SseOp::Ucomi, r("xmm0"), r("xmm1")),
                &[0x66, 0x0f, 0x2e, 0xc1],
            ),
            (
                Inst::Sse(SseOp::Ucomi, r("xmm1"), r("xmm0")),
                &[0x66, 0x0f, 0x2e, 0xc8],
            ),
            (Inst::Ud2, &[0x0f, 0x0b]),
            (Inst::Ret, &[0xc3]),
        ];
        for (instruction, expected) in cases {
            let actual = bytes(instruction.clone());
            assert_eq!(
                actual, expected,
                "`{instruction}` encoded as {actual:02x?}, not {expected:02x?}"
            );
        }
    }

    #[test]
    fn a_jump_inside_the_section_is_resolved_without_the_linker() {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(Inst::Jmp(Target::Named(".Lend".into()))));
        assembly.push(Item::Instruction(Inst::Ret));
        assembly.push(Item::Label(".Lend".into()));
        let object = assembly.finish().unwrap();
        assert!(object.relocations.is_empty());
        // `e9` and a displacement measured from the end of the instruction,
        // which is one byte past the `ret`.
        assert_eq!(object.text, vec![0xe9, 0x01, 0, 0, 0, 0xc3]);
    }

    #[test]
    fn a_call_and_an_address_are_the_linkers() {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::RoData));
        assembly.push(Item::Label(".Lstr".into()));
        assembly.push(Item::Bytes(vec![104, 105, 0]));
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(Inst::Lea(
            r("rdi"),
            Operand::Rip(".Lstr".into()),
        )));
        assembly.push(Item::Instruction(Inst::Call("sl_rt_alloc".into())));
        let object = assembly.finish().unwrap();
        assert_eq!(object.text[0..3], [0x48, 0x8d, 0x3d]);
        assert_eq!(object.text[7], 0xe8);
        let kinds: Vec<_> = object
            .relocations
            .iter()
            .map(|relocation| (relocation.kind, relocation.offset, relocation.addend))
            .collect();
        assert_eq!(
            kinds,
            vec![(FixupKind::Pc32, 3, -4), (FixupKind::Plt32, 8, -4)],
            "both measured from the end of their instruction"
        );
    }

    #[test]
    fn text_matches_what_the_backend_used_to_write() {
        let lines: Vec<(Inst, &str)> = vec![
            (Inst::Mov(reg("rax"), reg("rdi")), "mov rax, rdi"),
            (
                Inst::Mov(slot(-8), reg("rdi")),
                "mov QWORD PTR [rbp-8], rdi",
            ),
            (
                Inst::Mov(Operand::slot(Size::Qword, r("rax"), 0), reg("rcx")),
                "mov QWORD PTR [rax+0], rcx",
            ),
            (
                Inst::Mov(
                    reg("rcx"),
                    Operand::Mem(Mem {
                        size: Some(Size::Qword),
                        base: r("rax"),
                        disp: None,
                    }),
                ),
                "mov rcx, QWORD PTR [rax]",
            ),
            (
                Inst::Alu(
                    AluOp::Cmp,
                    Operand::slot(Size::Dword, r("rbp"), -8),
                    Operand::Imm(-1),
                ),
                "cmp DWORD PTR [rbp-8], -1",
            ),
            (
                Inst::Lea(
                    r("rax"),
                    Operand::Mem(Mem {
                        size: None,
                        base: r("rbp"),
                        disp: Some(-8),
                    }),
                ),
                "lea rax, [rbp-8]",
            ),
            (
                Inst::Lea(r("rdi"), Operand::Rip(".Lsl_str_0".into())),
                "lea rdi, .Lsl_str_0[rip]",
            ),
            (Inst::Jcc(Cond::Z, Target::Forward(1)), "jz 1f"),
            (
                Inst::Jcc(Cond::Ne, Target::Named(".Lbb1".into())),
                "jne .Lbb1",
            ),
            (Inst::Setcc(Cond::Np, r("cl")), "setnp cl"),
            (Inst::Setcc(Cond::Le, r("al")), "setle al"),
            (Inst::Setcc(Cond::Ge, r("al")), "setge al"),
            (Inst::Setcc(Cond::Ae, r("al")), "setae al"),
            (Inst::Setcc(Cond::P, r("cl")), "setp cl"),
            (
                Inst::Jcc(
                    Cond::Ae,
                    Target::Named(".Lsl_panic_shift_trampoline".into()),
                ),
                "jae .Lsl_panic_shift_trampoline",
            ),
            (Inst::Alu(AluOp::Or, reg("rax"), reg("rcx")), "or rax, rcx"),
            (Inst::Shift(ShiftOp::Shl, r("rax")), "shl rax, cl"),
            (Inst::Shift(ShiftOp::Sar, r("eax")), "sar eax, cl"),
            (Inst::Shift(ShiftOp::Shr, r("rax")), "shr rax, cl"),
            (Inst::ShiftImm(ShiftOp::Shl, r("rax"), 56), "shl rax, 56"),
            (Inst::ShiftImm(ShiftOp::Sar, r("rax"), 56), "sar rax, 56"),
            (Inst::Div(r("rcx")), "div rcx"),
            (Inst::Mul(r("rcx")), "mul rcx"),
            (Inst::Jcc(Cond::B, Target::Forward(1)), "jb 1f"),
            (Inst::Setcc(Cond::Be, r("al")), "setbe al"),
            (Inst::Imul(r("rax"), reg("rcx")), "imul rax, rcx"),
            (
                Inst::Sse(SseOp::Ucomi, r("xmm0"), r("xmm1")),
                "ucomisd xmm0, xmm1",
            ),
        ];
        for (instruction, text) in lines {
            assert_eq!(instruction.to_string(), text);
        }
    }

    #[test]
    fn a_name_that_is_not_a_register_fails_to_encode() {
        for name in ["x0", "rip", "", "xmm16", "ah"] {
            assert!(Reg(name).number().is_err(), "`{name}` was accepted");
        }
        for name in ["rax", "r15", "eax", "r8d", "al", "xmm15"] {
            assert!(Reg(name).number().is_ok(), "`{name}` was refused");
        }
    }
}
