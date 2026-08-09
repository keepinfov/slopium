//! The AArch64 instructions this compiler emits, and how they encode.
//!
//! Every instruction here prints exactly the text the backend used to write,
//! and encodes to exactly what the assembler produced for that text — the
//! object suite checks the second claim against `as` over the whole corpus,
//! byte for byte, which fixed-width instructions make possible.
//!
//! The set is deliberately closed. It holds what the backend selects and
//! nothing else, so an encoding this file does not implement is a form the
//! backend cannot ask for.

use crate::asm::{Code, FixupKind, Instruction, Target};
use std::fmt;

/// A register, named the way the backend already names it.
///
/// The backend has always chosen registers by name — the register files are
/// lists of names — so this is what it has to hand. Turning a name into a
/// number is one table, checked by [`Reg::number`], and a name that is not a
/// register fails to encode rather than assembling into something else.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reg(pub &'static str);

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Reg {
    /// The register number, 0–31.
    ///
    /// `sp` and the zero register share number 31; which one a 31 means is
    /// decided by the instruction, not by the register.
    pub fn number(self) -> Result<u32, String> {
        let name = self.0;
        match name {
            "sp" | "xzr" | "wzr" => return Ok(31),
            _ => {}
        }
        if name.is_empty() {
            return Err("the empty name is not a register".into());
        }
        let (prefix, digits) = name.split_at(1);
        if !matches!(prefix, "x" | "w" | "d") {
            return Err(format!("`{name}` is not a register"));
        }
        let number: u32 = digits
            .parse()
            .map_err(|_| format!("`{name}` is not a register"))?;
        let limit = if prefix == "d" { 31 } else { 30 };
        if number > limit {
            return Err(format!("`{name}` is not a register"));
        }
        Ok(number)
    }

    /// Whether this names a 64-bit general register, `sp` included.
    pub fn is_wide(self) -> bool {
        self.0.starts_with('x') || self.0 == "sp" || self.0 == "xzr"
    }

    pub fn is_float(self) -> bool {
        self.0.starts_with('d')
    }

    /// Whether this is the stack pointer, which several encodings refuse to
    /// treat like any other register.
    pub fn is_stack_pointer(self) -> bool {
        self.0 == "sp"
    }
}

/// A condition, as both a name and the four bits that spell it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cond {
    Eq,
    Ne,
    Vs,
    Mi,
    Lt,
    Gt,
}

impl Cond {
    pub fn code(self) -> u32 {
        match self {
            Cond::Eq => 0,
            Cond::Ne => 1,
            Cond::Mi => 4,
            Cond::Vs => 6,
            Cond::Lt => 11,
            Cond::Gt => 12,
        }
    }

    /// The condition true exactly when this one is false.
    ///
    /// Conditions come in pairs that differ in their lowest bit, which is what
    /// makes `cset` — "set if the *opposite* did not hold" — one instruction.
    pub fn inverted(self) -> u32 {
        self.code() ^ 1
    }
}

impl fmt::Display for Cond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Cond::Eq => "eq",
            Cond::Ne => "ne",
            Cond::Vs => "vs",
            Cond::Mi => "mi",
            Cond::Lt => "lt",
            Cond::Gt => "gt",
        })
    }
}

/// A three-register arithmetic operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arith {
    Add,
    Adds,
    Sub,
    Subs,
    Mul,
    Smulh,
    Smull,
    Sdiv,
}

impl fmt::Display for Arith {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arith::Add => "add",
            Arith::Adds => "adds",
            Arith::Sub => "sub",
            Arith::Subs => "subs",
            Arith::Mul => "mul",
            Arith::Smulh => "smulh",
            Arith::Smull => "smull",
            Arith::Sdiv => "sdiv",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl fmt::Display for FloatOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FloatOp::Add => "fadd",
            FloatOp::Sub => "fsub",
            FloatOp::Mul => "fmul",
            FloatOp::Div => "fdiv",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inst {
    /// `mov Xd, Xn`, including the two forms that name `sp`.
    Mov {
        dst: Reg,
        src: Reg,
    },
    /// `movz`/`movk Xd, #half` with an optional `, lsl #shift`.
    Half {
        keep: bool,
        dst: Reg,
        half: u16,
        shift: u32,
    },
    /// `add Xd, Xn, Xm` and its seven relatives.
    Arith {
        op: Arith,
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    /// `add Xd, Xn, #imm` — and `sub`, which is the only other one used.
    ArithImm {
        op: Arith,
        dst: Reg,
        src: Reg,
        imm: u32,
    },
    /// `add Xd, Xn, :lo12:label`, the second half of an address.
    AddLow {
        dst: Reg,
        src: Reg,
        label: String,
    },
    /// `adrp Xd, label`, the first half.
    Adrp {
        dst: Reg,
        label: String,
    },
    /// `cmp Xn, Xm`.
    Cmp {
        lhs: Reg,
        rhs: Reg,
    },
    /// `cmp Xn, Xm, asr #amount`, for the high half of a product.
    CmpShifted {
        lhs: Reg,
        rhs: Reg,
        amount: u32,
    },
    /// `cmp Xn, Wm, sxtw`, for a 32-bit product that has to sign-extend.
    CmpExtended {
        lhs: Reg,
        rhs: Reg,
    },
    /// `cmn Xn, #imm`, which is a comparison against a negative number.
    CmnImm {
        lhs: Reg,
        imm: u32,
    },
    /// `cset Xd, cond`.
    Cset {
        dst: Reg,
        cond: Cond,
    },
    /// `sxtw Xd, Wn`.
    Sxtw {
        dst: Reg,
        src: Reg,
    },
    /// `ldr Xt, [Xn]` or `ldr Xt, [Xn, #offset]`.
    ///
    /// `None` is not the same as `Some(0)`: they encode alike and print
    /// differently, and the printing is what the backend already wrote.
    Load {
        dst: Reg,
        base: Reg,
        offset: Option<u32>,
    },
    Store {
        src: Reg,
        base: Reg,
        offset: Option<u32>,
    },
    /// `stp x29, x30, [sp, #-16]!` — the frame record going down.
    PushFrame,
    /// `ldp x29, x30, [sp], #16` — and coming back up.
    PopFrame,
    B(Target),
    Bl(String),
    /// `blr` — a call through the address in a register (`D-092`).
    Blr(Reg),
    Bcond(Cond, Target),
    Cbz(Reg, Target),
    Cbnz(Reg, Target),
    Ret,
    Brk(u16),
    /// `fmov` between two registers of any two classes it accepts.
    Fmov {
        dst: Reg,
        src: Reg,
    },
    Float {
        op: FloatOp,
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
    },
    Fcmp {
        lhs: Reg,
        rhs: Reg,
    },
}

impl fmt::Display for Inst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inst::Mov { dst, src } => write!(f, "mov {dst}, {src}"),
            Inst::Half {
                keep,
                dst,
                half,
                shift,
            } => {
                let mnemonic = if *keep { "movk" } else { "movz" };
                write!(f, "{mnemonic} {dst}, #{half}")?;
                if *shift != 0 {
                    write!(f, ", lsl #{shift}")?;
                }
                Ok(())
            }
            Inst::Arith { op, dst, lhs, rhs } => write!(f, "{op} {dst}, {lhs}, {rhs}"),
            Inst::ArithImm { op, dst, src, imm } => write!(f, "{op} {dst}, {src}, #{imm}"),
            Inst::AddLow { dst, src, label } => write!(f, "add {dst}, {src}, :lo12:{label}"),
            Inst::Adrp { dst, label } => write!(f, "adrp {dst}, {label}"),
            Inst::Cmp { lhs, rhs } => write!(f, "cmp {lhs}, {rhs}"),
            Inst::CmpShifted { lhs, rhs, amount } => write!(f, "cmp {lhs}, {rhs}, asr #{amount}"),
            Inst::CmpExtended { lhs, rhs } => write!(f, "cmp {lhs}, {rhs}, sxtw"),
            Inst::CmnImm { lhs, imm } => write!(f, "cmn {lhs}, #{imm}"),
            Inst::Cset { dst, cond } => write!(f, "cset {dst}, {cond}"),
            Inst::Sxtw { dst, src } => write!(f, "sxtw {dst}, {src}"),
            Inst::Load { dst, base, offset } => match offset {
                Some(offset) => write!(f, "ldr {dst}, [{base}, #{offset}]"),
                None => write!(f, "ldr {dst}, [{base}]"),
            },
            Inst::Store { src, base, offset } => match offset {
                Some(offset) => write!(f, "str {src}, [{base}, #{offset}]"),
                None => write!(f, "str {src}, [{base}]"),
            },
            Inst::PushFrame => f.write_str("stp x29, x30, [sp, #-16]!"),
            Inst::PopFrame => f.write_str("ldp x29, x30, [sp], #16"),
            Inst::B(target) => write!(f, "b {target}"),
            Inst::Bl(symbol) => write!(f, "bl {symbol}"),
            Inst::Blr(register) => write!(f, "blr {register}"),
            Inst::Bcond(cond, target) => write!(f, "b.{cond} {target}"),
            Inst::Cbz(register, target) => write!(f, "cbz {register}, {target}"),
            Inst::Cbnz(register, target) => write!(f, "cbnz {register}, {target}"),
            Inst::Ret => f.write_str("ret"),
            Inst::Brk(code) => write!(f, "brk #{code}"),
            Inst::Fmov { dst, src } => write!(f, "fmov {dst}, {src}"),
            Inst::Float { op, dst, lhs, rhs } => write!(f, "{op} {dst}, {lhs}, {rhs}"),
            Inst::Fcmp { lhs, rhs } => write!(f, "fcmp {lhs}, {rhs}"),
        }
    }
}

impl Instruction for Inst {
    fn undo(&self) -> Option<Self> {
        match self {
            Inst::Mov { dst, src } => Some(Inst::Mov {
                dst: *src,
                src: *dst,
            }),
            _ => None,
        }
    }

    fn encode(&self, code: &mut Code) -> Result<(), String> {
        let word = match self {
            Inst::Mov { dst, src } => {
                let (d, s) = (dst.number()?, src.number()?);
                if dst.is_stack_pointer() || src.is_stack_pointer() {
                    // A move naming `sp` is an `add` of zero: the register the
                    // ordinary move goes through is the zero register, which
                    // has the same number.
                    0x9100_0000 | (s << 5) | d
                } else if dst.is_wide() {
                    0xaa00_03e0 | (s << 16) | d
                } else {
                    0x2a00_03e0 | (s << 16) | d
                }
            }
            Inst::Half {
                keep,
                dst,
                half,
                shift,
            } => {
                if shift % 16 != 0 || *shift > 48 {
                    return Err(format!("{shift} is not a halfword boundary"));
                }
                // A 32-bit destination has only two halfwords to name, and the
                // whole register is written either way — but the encodings are
                // distinct, and the object suite compares them against `as`.
                if !dst.is_wide() && *shift > 16 {
                    return Err(format!("a 32-bit register has no halfword at {shift}"));
                }
                let base: u32 = match (*keep, dst.is_wide()) {
                    (false, true) => 0xd280_0000,
                    (false, false) => 0x5280_0000,
                    (true, true) => 0xf280_0000,
                    (true, false) => 0x7280_0000,
                };
                base | ((shift / 16) << 21) | ((*half as u32) << 5) | dst.number()?
            }
            Inst::Arith { op, dst, lhs, rhs } => {
                let (d, n, m) = (dst.number()?, lhs.number()?, rhs.number()?);
                let wide = dst.is_wide();
                match op {
                    Arith::Add | Arith::Adds | Arith::Sub | Arith::Subs => {
                        let base: u32 = match (op, wide) {
                            (Arith::Add, true) => 0x8b00_0000,
                            (Arith::Add, false) => 0x0b00_0000,
                            (Arith::Adds, true) => 0xab00_0000,
                            (Arith::Adds, false) => 0x2b00_0000,
                            (Arith::Sub, true) => 0xcb00_0000,
                            (Arith::Sub, false) => 0x4b00_0000,
                            (Arith::Subs, true) => 0xeb00_0000,
                            _ => 0x6b00_0000,
                        };
                        if lhs.is_stack_pointer() || dst.is_stack_pointer() {
                            // The shifted form cannot name `sp` at all, so a
                            // frame adjustment takes the extended form with
                            // the identity extension.
                            base | 0x0020_0000 | (m << 16) | (0b011 << 13) | (n << 5) | d
                        } else {
                            base | (m << 16) | (n << 5) | d
                        }
                    }
                    Arith::Mul => 0x9b00_7c00 | (m << 16) | (n << 5) | d,
                    Arith::Smulh => 0x9b40_7c00 | (m << 16) | (n << 5) | d,
                    Arith::Smull => 0x9b20_7c00 | (m << 16) | (n << 5) | d,
                    Arith::Sdiv => {
                        let base = if wide { 0x9ac0_0c00 } else { 0x1ac0_0c00 };
                        base | (m << 16) | (n << 5) | d
                    }
                }
            }
            Inst::ArithImm { op, dst, src, imm } => {
                if *imm > 0xfff {
                    return Err(format!("{imm} does not fit in a 12-bit immediate"));
                }
                let base: u32 = match (op, dst.is_wide()) {
                    (Arith::Add, true) => 0x9100_0000,
                    (Arith::Add, false) => 0x1100_0000,
                    (Arith::Sub, true) => 0xd100_0000,
                    (Arith::Sub, false) => 0x5100_0000,
                    (Arith::Adds, true) => 0xb100_0000,
                    (Arith::Subs, true) => 0xf100_0000,
                    _ => return Err(format!("`{op}` has no immediate form here")),
                };
                base | (imm << 10) | (src.number()? << 5) | dst.number()?
            }
            Inst::AddLow { dst, src, label } => {
                let at = code.here();
                code.relocate(at, FixupKind::AddLo12, Target::Named(label.clone()), 0);
                0x9100_0000 | (src.number()? << 5) | dst.number()?
            }
            Inst::Adrp { dst, label } => {
                let at = code.here();
                code.relocate(at, FixupKind::AdrPage21, Target::Named(label.clone()), 0);
                0x9000_0000 | dst.number()?
            }
            Inst::Cmp { lhs, rhs } => {
                let base = if lhs.is_wide() {
                    0xeb00_0000
                } else {
                    0x6b00_0000
                };
                base | (rhs.number()? << 16) | (lhs.number()? << 5) | 31
            }
            Inst::CmpShifted { lhs, rhs, amount } => {
                if *amount > 63 {
                    return Err(format!("a shift of {amount} is out of range"));
                }
                // `asr` is shift type 2.
                0xeb00_0000
                    | (0b10 << 22)
                    | (rhs.number()? << 16)
                    | (amount << 10)
                    | (lhs.number()? << 5)
                    | 31
            }
            Inst::CmpExtended { lhs, rhs } => {
                // `sxtw` is extension option 6.
                0xeb20_0000 | (rhs.number()? << 16) | (0b110 << 13) | (lhs.number()? << 5) | 31
            }
            Inst::CmnImm { lhs, imm } => {
                if *imm > 0xfff {
                    return Err(format!("{imm} does not fit in a 12-bit immediate"));
                }
                0xb100_0000 | (imm << 10) | (lhs.number()? << 5) | 31
            }
            Inst::Cset { dst, cond } => 0x9a9f_07e0 | (cond.inverted() << 12) | dst.number()?,
            Inst::Sxtw { dst, src } => 0x9340_7c00 | (src.number()? << 5) | dst.number()?,
            Inst::Load { dst, base, offset }
            | Inst::Store {
                src: dst,
                base,
                offset,
            } => {
                let offset = offset.unwrap_or(0);
                if offset % 8 != 0 {
                    return Err(format!("{offset} is not a multiple of the access size"));
                }
                let scaled = offset / 8;
                if scaled > 0xfff {
                    return Err(format!("{offset} is past the addressable frame"));
                }
                let load = matches!(self, Inst::Load { .. });
                let opcode = if load { 0xf940_0000 } else { 0xf900_0000 };
                opcode | (scaled << 10) | (base.number()? << 5) | dst.number()?
            }
            // The frame record is the one pair access this compiler emits, and
            // both halves have fixed operands: `x29`/`x30` at `sp`, sixteen
            // bytes down on the way in and back up on the way out.
            Inst::PushFrame => 0xa9bf_7bfd,
            Inst::PopFrame => 0xa8c1_7bfd,
            Inst::B(target) => {
                let at = code.here();
                code.relocate(at, FixupKind::Jump26, target.clone(), 0);
                0x1400_0000
            }
            Inst::Bl(symbol) => {
                let at = code.here();
                code.relocate(at, FixupKind::Call26, Target::Named(symbol.clone()), 0);
                0x9400_0000
            }
            // The same unconditional-branch-to-register family as `ret`, which
            // is `br x30` with the link bit clear: no target, so no relocation.
            Inst::Blr(register) => 0xd63f_0000 | (register.number()? << 5),
            Inst::Bcond(cond, target) => {
                let at = code.here();
                code.relocate(at, FixupKind::CondBr19, target.clone(), 0);
                0x5400_0000 | cond.code()
            }
            Inst::Cbz(register, target) | Inst::Cbnz(register, target) => {
                let at = code.here();
                code.relocate(at, FixupKind::CondBr19, target.clone(), 0);
                let opcode = if matches!(self, Inst::Cbz(..)) {
                    0xb400_0000
                } else {
                    0xb500_0000
                };
                opcode | register.number()?
            }
            Inst::Ret => 0xd65f_03c0,
            Inst::Brk(immediate) => 0xd420_0000 | ((*immediate as u32) << 5),
            Inst::Fmov { dst, src } => {
                let (d, s) = (dst.number()?, src.number()?);
                match (dst.is_float(), src.is_float()) {
                    (true, true) => 0x1e60_4000 | (s << 5) | d,
                    (true, false) => 0x9e67_0000 | (s << 5) | d,
                    (false, true) => 0x9e66_0000 | (s << 5) | d,
                    (false, false) => return Err("`fmov` between two general registers".into()),
                }
            }
            Inst::Float { op, dst, lhs, rhs } => {
                let base: u32 = match op {
                    FloatOp::Add => 0x1e60_2800,
                    FloatOp::Sub => 0x1e60_3800,
                    FloatOp::Mul => 0x1e60_0800,
                    FloatOp::Div => 0x1e60_1800,
                };
                base | (rhs.number()? << 16) | (lhs.number()? << 5) | dst.number()?
            }
            Inst::Fcmp { lhs, rhs } => 0x1e60_2000 | (rhs.number()? << 16) | (lhs.number()? << 5),
        };
        code.word(word);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{Assembly, Item, Section};

    fn encoding(instruction: Inst) -> u32 {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(instruction));
        let object = assembly.finish().unwrap();
        u32::from_le_bytes(object.text[0..4].try_into().unwrap())
    }

    fn x(name: &'static str) -> Reg {
        Reg(name)
    }

    /// Every word here came from `aarch64-unknown-linux-gnu-as` assembling the
    /// text on the left. They are written down rather than computed so that a
    /// change to the encoder has to disagree with the assembler out loud.
    #[test]
    fn every_form_encodes_the_way_the_assembler_encodes_it() {
        let cases: Vec<(Inst, u32)> = vec![
            (
                Inst::Mov {
                    dst: x("x19"),
                    src: x("x20"),
                },
                0xaa1403f3,
            ),
            (
                Inst::Mov {
                    dst: x("w1"),
                    src: x("w0"),
                },
                0x2a0003e1,
            ),
            (
                Inst::Mov {
                    dst: x("x29"),
                    src: x("sp"),
                },
                0x910003fd,
            ),
            (
                Inst::Mov {
                    dst: x("sp"),
                    src: x("x29"),
                },
                0x910003bf,
            ),
            (
                Inst::Half {
                    keep: false,
                    dst: x("x19"),
                    half: 20,
                    shift: 0,
                },
                0xd2800293,
            ),
            (
                Inst::Half {
                    keep: true,
                    dst: x("x3"),
                    half: 4660,
                    shift: 16,
                },
                0xf2a24683,
            ),
            (
                Inst::Half {
                    keep: false,
                    dst: x("x4"),
                    half: 1,
                    shift: 48,
                },
                0xd2e00024,
            ),
            (
                Inst::Half {
                    keep: false,
                    dst: x("w1"),
                    half: 0,
                    shift: 0,
                },
                0x52800001,
            ),
            (
                Inst::Half {
                    keep: true,
                    dst: x("w3"),
                    half: 4660,
                    shift: 16,
                },
                0x72a24683,
            ),
            (
                Inst::Arith {
                    op: Arith::Add,
                    dst: x("x0"),
                    lhs: x("x1"),
                    rhs: x("x2"),
                },
                0x8b020020,
            ),
            (
                Inst::Arith {
                    op: Arith::Add,
                    dst: x("x19"),
                    lhs: x("sp"),
                    rhs: x("x19"),
                },
                0x8b3363f3,
            ),
            (
                Inst::Arith {
                    op: Arith::Adds,
                    dst: x("x16"),
                    lhs: x("x20"),
                    rhs: x("x21"),
                },
                0xab150290,
            ),
            (
                Inst::Arith {
                    op: Arith::Subs,
                    dst: x("x9"),
                    lhs: x("x10"),
                    rhs: x("x11"),
                },
                0xeb0b0149,
            ),
            (
                Inst::Arith {
                    op: Arith::Sub,
                    dst: x("sp"),
                    lhs: x("sp"),
                    rhs: x("x16"),
                },
                0xcb3063ff,
            ),
            (
                Inst::ArithImm {
                    op: Arith::Add,
                    dst: x("x19"),
                    src: x("sp"),
                    imm: 4088,
                },
                0x913fe3f3,
            ),
            (
                Inst::ArithImm {
                    op: Arith::Sub,
                    dst: x("sp"),
                    src: x("sp"),
                    imm: 32,
                },
                0xd10083ff,
            ),
            (
                Inst::Cmp {
                    lhs: x("x1"),
                    rhs: x("x2"),
                },
                0xeb02003f,
            ),
            (
                Inst::CmpShifted {
                    lhs: x("x15"),
                    rhs: x("x16"),
                    amount: 63,
                },
                0xeb90fdff,
            ),
            (
                Inst::CmpExtended {
                    lhs: x("x16"),
                    rhs: x("w16"),
                },
                0xeb30c21f,
            ),
            (
                Inst::CmnImm {
                    lhs: x("x5"),
                    imm: 1,
                },
                0xb10004bf,
            ),
            (
                Inst::Cset {
                    dst: x("x16"),
                    cond: Cond::Eq,
                },
                0x9a9f17f0,
            ),
            (
                Inst::Cset {
                    dst: x("x16"),
                    cond: Cond::Lt,
                },
                0x9a9fa7f0,
            ),
            (
                Inst::Cset {
                    dst: x("x16"),
                    cond: Cond::Gt,
                },
                0x9a9fd7f0,
            ),
            (
                Inst::Cset {
                    dst: x("x16"),
                    cond: Cond::Mi,
                },
                0x9a9f57f0,
            ),
            (
                Inst::Sxtw {
                    dst: x("x16"),
                    src: x("w16"),
                },
                0x93407e10,
            ),
            (
                Inst::Arith {
                    op: Arith::Mul,
                    dst: x("x16"),
                    lhs: x("x1"),
                    rhs: x("x2"),
                },
                0x9b027c30,
            ),
            (
                Inst::Arith {
                    op: Arith::Smulh,
                    dst: x("x15"),
                    lhs: x("x1"),
                    rhs: x("x2"),
                },
                0x9b427c2f,
            ),
            (
                Inst::Arith {
                    op: Arith::Smull,
                    dst: x("x16"),
                    lhs: x("w1"),
                    rhs: x("w2"),
                },
                0x9b227c30,
            ),
            (
                Inst::Arith {
                    op: Arith::Sdiv,
                    dst: x("x16"),
                    lhs: x("x1"),
                    rhs: x("x2"),
                },
                0x9ac20c30,
            ),
            (
                Inst::Arith {
                    op: Arith::Sdiv,
                    dst: x("w16"),
                    lhs: x("w1"),
                    rhs: x("w2"),
                },
                0x1ac20c30,
            ),
            (
                Inst::Load {
                    dst: x("x16"),
                    base: x("sp"),
                    offset: Some(0),
                },
                0xf94003f0,
            ),
            (
                Inst::Load {
                    dst: x("x0"),
                    base: x("sp"),
                    offset: Some(32760),
                },
                0xf97fffe0,
            ),
            (
                Inst::Load {
                    dst: x("x17"),
                    base: x("x0"),
                    offset: None,
                },
                0xf9400011,
            ),
            (
                Inst::Load {
                    dst: x("x0"),
                    base: x("x16"),
                    offset: Some(8),
                },
                0xf9400600,
            ),
            (
                Inst::Store {
                    src: x("x16"),
                    base: x("sp"),
                    offset: Some(0),
                },
                0xf90003f0,
            ),
            (
                Inst::Store {
                    src: x("x0"),
                    base: x("x16"),
                    offset: Some(24),
                },
                0xf9000e00,
            ),
            (
                Inst::Store {
                    src: x("x17"),
                    base: x("x0"),
                    offset: None,
                },
                0xf9000011,
            ),
            (Inst::PushFrame, 0xa9bf7bfd),
            (Inst::PopFrame, 0xa8c17bfd),
            (Inst::Ret, 0xd65f03c0),
            (Inst::Blr(x("x16")), 0xd63f0200),
            (Inst::Blr(x("x0")), 0xd63f0000),
            (Inst::Blr(x("x30")), 0xd63f03c0),
            (Inst::Brk(1), 0xd4200020),
            (
                Inst::Fmov {
                    dst: x("d0"),
                    src: x("x16"),
                },
                0x9e670200,
            ),
            (
                Inst::Fmov {
                    dst: x("x16"),
                    src: x("d0"),
                },
                0x9e660010,
            ),
            (
                Inst::Fmov {
                    dst: x("d0"),
                    src: x("d1"),
                },
                0x1e604020,
            ),
            (
                Inst::Float {
                    op: FloatOp::Add,
                    dst: x("d0"),
                    lhs: x("d0"),
                    rhs: x("d1"),
                },
                0x1e612800,
            ),
            (
                Inst::Float {
                    op: FloatOp::Sub,
                    dst: x("d0"),
                    lhs: x("d0"),
                    rhs: x("d1"),
                },
                0x1e613800,
            ),
            (
                Inst::Float {
                    op: FloatOp::Mul,
                    dst: x("d0"),
                    lhs: x("d0"),
                    rhs: x("d1"),
                },
                0x1e610800,
            ),
            (
                Inst::Float {
                    op: FloatOp::Div,
                    dst: x("d0"),
                    lhs: x("d0"),
                    rhs: x("d1"),
                },
                0x1e611800,
            ),
            (
                Inst::Fcmp {
                    lhs: x("d0"),
                    rhs: x("d1"),
                },
                0x1e612000,
            ),
        ];
        for (instruction, expected) in cases {
            let actual = encoding(instruction.clone());
            assert_eq!(
                actual, expected,
                "`{instruction}` encoded as {actual:#010x}, not {expected:#010x}"
            );
        }
    }

    #[test]
    fn a_branch_carries_its_condition_and_its_displacement_separately() {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(Inst::Bcond(
            Cond::Vs,
            Target::Named(".Lend".into()),
        )));
        assembly.push(Item::Instruction(Inst::Ret));
        assembly.push(Item::Label(".Lend".into()));
        let object = assembly.finish().unwrap();
        assert_eq!(
            u32::from_le_bytes(object.text[0..4].try_into().unwrap()),
            0x54000046,
            "two words forward, condition vs"
        );
        assert!(object.relocations.is_empty());
    }

    #[test]
    fn an_address_is_two_instructions_and_two_relocations() {
        let mut assembly: Assembly<Inst> = Assembly::new();
        assembly.push(Item::Section(Section::RoData));
        assembly.push(Item::Label(".Lstr".into()));
        assembly.push(Item::Bytes(vec![104, 105, 0]));
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Instruction(Inst::Adrp {
            dst: x("x0"),
            label: ".Lstr".into(),
        }));
        assembly.push(Item::Instruction(Inst::AddLow {
            dst: x("x0"),
            src: x("x0"),
            label: ".Lstr".into(),
        }));
        let object = assembly.finish().unwrap();
        assert_eq!(
            u32::from_le_bytes(object.text[0..4].try_into().unwrap()),
            0x90000000
        );
        assert_eq!(
            u32::from_le_bytes(object.text[4..8].try_into().unwrap()),
            0x91000000
        );
        let kinds: Vec<_> = object
            .relocations
            .iter()
            .map(|relocation| relocation.kind)
            .collect();
        assert_eq!(kinds, vec![FixupKind::AdrPage21, FixupKind::AddLo12]);
    }

    #[test]
    fn a_name_that_is_not_a_register_fails_to_encode() {
        for name in ["rax", "x31", "q0", "", "xx"] {
            assert!(Reg(name).number().is_err(), "`{name}` was accepted");
        }
        for name in ["x0", "x30", "w7", "d31", "sp", "xzr"] {
            assert!(Reg(name).number().is_ok(), "`{name}` was refused");
        }
    }

    #[test]
    fn text_matches_what_the_backend_used_to_write() {
        let lines = [
            (
                Inst::Load {
                    dst: x("x16"),
                    base: x("sp"),
                    offset: Some(8),
                },
                "ldr x16, [sp, #8]",
            ),
            (
                Inst::Load {
                    dst: x("x16"),
                    base: x("x0"),
                    offset: None,
                },
                "ldr x16, [x0]",
            ),
            (
                Inst::Half {
                    keep: false,
                    dst: x("x0"),
                    half: 0,
                    shift: 0,
                },
                "movz x0, #0",
            ),
            (
                Inst::Half {
                    keep: true,
                    dst: x("x0"),
                    half: 7,
                    shift: 32,
                },
                "movk x0, #7, lsl #32",
            ),
            (
                Inst::Bcond(Cond::Vs, Target::Named(".Lt".into())),
                "b.vs .Lt",
            ),
            (Inst::B(Target::Forward(2)), "b 2f"),
            (Inst::PushFrame, "stp x29, x30, [sp, #-16]!"),
            (Inst::PopFrame, "ldp x29, x30, [sp], #16"),
            (
                Inst::CmpShifted {
                    lhs: x("x15"),
                    rhs: x("x16"),
                    amount: 63,
                },
                "cmp x15, x16, asr #63",
            ),
            (
                Inst::CmpExtended {
                    lhs: x("x16"),
                    rhs: x("w16"),
                },
                "cmp x16, w16, sxtw",
            ),
        ];
        for (instruction, text) in lines {
            assert_eq!(instruction.to_string(), text);
        }
    }
}
