//! A 68000 instruction decoder (F-032, D-019).
//!
//! Turns bytes into instructions and nothing more: it never executes, never
//! follows a branch, never guesses where code starts. Where code starts is the
//! caller's business — an executable's hunk header says, a bootblock's layout
//! says — because a 68000 instruction is two to ten bytes long, and decoding
//! from the wrong byte produces something that looks exactly as plausible as
//! the truth.
//!
//! # What it decodes
//!
//! The 68000's whole instruction set, from Motorola's *M68000 Family
//! Programmer's Reference Manual*. Opcodes the 68000 does not implement —
//! the 68010's `moves` and `rtd`, the 68020's bitfields and long multiplies,
//! every line-F coprocessor word, line-A — are [`Decoded::Invalid`], which a
//! listing shows as `dc.w`. A program that uses them was not written for a
//! plain 68000, which is itself worth knowing (F-028).
//!
//! # Checked against GNU objdump
//!
//! The tests decode every one of the 65,536 opcode words with this and with
//! `m68k-linux-gnu-objdump -m m68k:68000`, and require the same validity, the
//! same length and the same instruction (D-019, D-002's bargain on a second
//! format). That is why the output is structured: [`Instruction`] renders as
//! Motorola syntax for people, and a test renders the same value in objdump's
//! MIT syntax to compare.
//!
//! # Untrusted input (D-006)
//!
//! Every word is read through `ade-endian`, so a truncated instruction is
//! [`Decoded::Truncated`] rather than a panic, and nothing allocates more than
//! the two operands an instruction can have.

use ade_endian::u16_at;

/// The size an instruction operates on, or the size of a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// `.b`
    Byte,
    /// `.w`
    Word,
    /// `.l`
    Long,
    /// `.s`, an 8-bit branch displacement.
    Short,
}

impl Size {
    /// The Motorola suffix, without the dot.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Byte => "b",
            Self::Word => "w",
            Self::Long => "l",
            Self::Short => "s",
        }
    }
}

/// An index register in an indexed address: `Dn` or `An`, word or long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Index {
    /// 0–7 for `d0`–`d7`, 8–15 for `a0`–`a7`.
    pub reg: u8,
    /// Whether the whole register is used rather than its low word.
    pub long: bool,
    /// 1, 2, 4 or 8. Always 1 in code written for a 68000, which ignores the
    /// field; anything else is a 68020 program, and is shown so.
    pub scale: u8,
}

/// One operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// `dn`
    D(u8),
    /// `an`
    A(u8),
    /// `(an)`
    Ind(u8),
    /// `(an)+`
    PostInc(u8),
    /// `-(an)`
    PreDec(u8),
    /// `d16(an)`
    Disp(i16, u8),
    /// `d8(an,xn)`
    Indexed(i8, u8, Index),
    /// `(xxx).w`, sign-extended as the CPU extends it.
    AbsW(i16),
    /// `(xxx).l`
    AbsL(u32),
    /// `d16(pc)`; the field is the address it reaches.
    PcDisp(u32),
    /// `d8(pc,xn)`; the address the displacement reaches, before the index.
    PcIndexed(u32, Index),
    /// `#imm`
    Imm(u32),
    /// A `movem` register list: bit 0 is `d0`, bit 15 is `a7`, whatever order
    /// the instruction stored it in.
    RegList(u16),
    /// `ccr`
    Ccr,
    /// `sr`
    Sr,
    /// `usp`
    Usp,
    /// A branch target, as an address.
    Target(u32),
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// The mnemonic without its size: `move`, `bne`, `dbf`.
    pub mnemonic: &'static str,
    /// The size suffix, where the instruction has one.
    pub size: Option<Size>,
    /// Source first, destination second, as Motorola writes them.
    pub operands: Vec<Operand>,
    /// Bytes the instruction occupies, opcode word included.
    pub len: u32,
}

/// What a run of bytes decodes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A 68000 instruction.
    Instruction(Instruction),
    /// A word that is not a 68000 instruction: shown as `dc.w`.
    Invalid(u16),
    /// A word that would be an instruction on a 68020, because its index
    /// extension uses the 68020's full format — memory indirection, or a base
    /// displacement wider than eight bits. Not a 68000 instruction, and
    /// evidence that the program was not written for one (F-028).
    Needs68020(u16),
    /// The bytes ran out part way through an instruction, or before the
    /// opcode word itself.
    Truncated,
}

impl Decoded {
    /// Bytes to step over: the instruction's length, or one word.
    #[must_use]
    pub fn len(&self) -> u32 {
        match self {
            Self::Instruction(i) => i.len,
            Self::Invalid(_) | Self::Needs68020(_) | Self::Truncated => 2,
        }
    }

    /// Whether this is an empty decode — never; present for the lint that
    /// expects `len` to have a partner.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// Decode the instruction at the start of `bytes`, which sits at `address`.
///
/// `address` is used only to turn PC-relative displacements and branches into
/// the addresses they reach.
#[must_use]
pub fn decode(bytes: &[u8], address: u32) -> Decoded {
    let Ok(op) = u16_at(bytes, 0) else {
        return Decoded::Truncated;
    };
    let mut d = Decoder {
        bytes,
        address,
        pos: 2,
        truncated: false,
        full_extension: false,
    };
    match d.instruction(op) {
        Some(mut i) if !d.truncated => {
            i.len = u32::try_from(d.pos).unwrap_or(2);
            Decoded::Instruction(i)
        }
        _ if d.truncated => Decoded::Truncated,
        _ if d.full_extension => Decoded::Needs68020(op),
        _ => Decoded::Invalid(op),
    }
}

// Addressing-mode classes, as the manual names them. A mode outside the class
// an instruction allows makes the whole word invalid.
const DN: u16 = 1 << 0;
const AN: u16 = 1 << 1;
const IND: u16 = 1 << 2;
const POST: u16 = 1 << 3;
const PRE: u16 = 1 << 4;
const D16: u16 = 1 << 5;
const D8X: u16 = 1 << 6;
const ABSW: u16 = 1 << 7;
const ABSL: u16 = 1 << 8;
const PC16: u16 = 1 << 9;
const PC8X: u16 = 1 << 10;
const IMM: u16 = 1 << 11;

const ALL: u16 = DN | AN | IND | POST | PRE | D16 | D8X | ABSW | ABSL | PC16 | PC8X | IMM;
const DATA: u16 = ALL & !AN;
const CONTROL: u16 = IND | D16 | D8X | ABSW | ABSL | PC16 | PC8X;
const ALTERABLE: u16 = DN | AN | IND | POST | PRE | D16 | D8X | ABSW | ABSL;
const DATA_ALT: u16 = ALTERABLE & !AN;
const MEM_ALT: u16 = ALTERABLE & !(DN | AN);
const CONTROL_ALT: u16 = CONTROL & ALTERABLE;

const CONDITIONS: [&str; 16] = [
    "t", "f", "hi", "ls", "cc", "cs", "ne", "eq", "vc", "vs", "pl", "mi", "ge", "lt", "gt", "le",
];

struct Decoder<'a> {
    bytes: &'a [u8],
    address: u32,
    pos: usize,
    truncated: bool,
    /// Set when a 68020 full-format extension word was what made the
    /// instruction invalid.
    full_extension: bool,
}

/// A word reinterpreted as signed, which is how the CPU reads a displacement.
const fn signed16(w: u16) -> i16 {
    i16::from_ne_bytes(w.to_ne_bytes())
}

/// The low byte of a word, as a signed 8-bit displacement.
const fn low_signed(w: u16) -> i8 {
    i8::from_ne_bytes([(w & 0xFF) as u8])
}

/// A signed value's bits, for an operand that holds them unsigned.
const fn unsigned32(v: i32) -> u32 {
    u32::from_ne_bytes(v.to_ne_bytes())
}

/// A three-bit field of the opcode.
const fn bits3(op: u16, shift: u32) -> u8 {
    ((op >> shift) & 7) as u8
}

const fn size_field(op: u16) -> Option<Size> {
    match (op >> 6) & 3 {
        0 => Some(Size::Byte),
        1 => Some(Size::Word),
        2 => Some(Size::Long),
        _ => None,
    }
}

fn ins(mnemonic: &'static str, size: Option<Size>, operands: Vec<Operand>) -> Instruction {
    Instruction {
        mnemonic,
        size,
        operands,
        len: 0,
    }
}

impl Decoder<'_> {
    /// The next extension word.
    fn word(&mut self) -> Option<u16> {
        if let Ok(w) = u16_at(self.bytes, self.pos) {
            self.pos = self.pos.saturating_add(2);
            Some(w)
        } else {
            self.truncated = true;
            None
        }
    }

    fn long(&mut self) -> Option<u32> {
        let hi = self.word()?;
        let lo = self.word()?;
        Some((u32::from(hi) << 16) | u32::from(lo))
    }

    /// The address of the current extension word, which is what a
    /// PC-relative displacement is measured from.
    fn here(&self) -> u32 {
        self.address
            .wrapping_add(u32::try_from(self.pos).unwrap_or(0))
    }

    /// The effective address `mode`/`reg`, if `allowed` admits it.
    fn ea(&mut self, mode: u8, reg: u8, size: Size, allowed: u16) -> Option<Operand> {
        let class = match (mode, reg) {
            (0, _) => DN,
            (1, _) => AN,
            (2, _) => IND,
            (3, _) => POST,
            (4, _) => PRE,
            (5, _) => D16,
            (6, _) => D8X,
            (7, 0) => ABSW,
            (7, 1) => ABSL,
            (7, 2) => PC16,
            (7, 3) => PC8X,
            (7, 4) => IMM,
            _ => return None,
        };
        if allowed & class == 0 {
            return None;
        }
        Some(match class {
            DN => Operand::D(reg),
            AN => Operand::A(reg),
            IND => Operand::Ind(reg),
            POST => Operand::PostInc(reg),
            PRE => Operand::PreDec(reg),
            D16 => Operand::Disp(signed16(self.word()?), reg),
            D8X => {
                let (disp, index) = self.brief()?;
                Operand::Indexed(disp, reg, index)
            }
            ABSW => Operand::AbsW(signed16(self.word()?)),
            ABSL => Operand::AbsL(self.long()?),
            PC16 => {
                let base = self.here();
                let disp = signed16(self.word()?);
                Operand::PcDisp(base.wrapping_add_signed(i32::from(disp)))
            }
            PC8X => {
                let base = self.here();
                let (disp, index) = self.brief()?;
                Operand::PcIndexed(base.wrapping_add_signed(i32::from(disp)), index)
            }
            _ => Operand::Imm(self.immediate(size)?),
        })
    }

    /// A brief extension word: index register, scale and 8-bit displacement.
    ///
    /// Bit 8 set is the 68020's full extension format — base and outer
    /// displacements, memory indirection — which the 68000 does not have, so
    /// the word is not a 68000 instruction. The scale in bits 9–10 is shown
    /// rather than refused: a 68000 ignores it, so the instruction runs, and a
    /// scale other than 1 says the code was written for a 68020.
    fn brief(&mut self) -> Option<(i8, Index)> {
        let w = self.word()?;
        if w & 0x0100 != 0 {
            self.full_extension = true;
            return None;
        }
        let reg = u8::try_from((w >> 12) & 0xF).unwrap_or(0);
        let scale = 1u8 << ((w >> 9) & 3);
        Some((
            low_signed(w),
            Index {
                reg,
                long: w & 0x0800 != 0,
                scale,
            },
        ))
    }

    /// An immediate of `size`. A byte immediate occupies a whole word; its
    /// high byte is ignored by the CPU and so here.
    fn immediate(&mut self, size: Size) -> Option<u32> {
        match size {
            Size::Byte | Size::Short => Some(u32::from(self.word()? & 0xFF)),
            Size::Word => Some(u32::from(self.word()?)),
            Size::Long => self.long(),
        }
    }

    /// The effective address in the low six bits of `op`.
    fn ea_low(&mut self, op: u16, size: Size, allowed: u16) -> Option<Operand> {
        self.ea(bits3(op, 3), bits3(op, 0), size, allowed)
    }

    fn instruction(&mut self, op: u16) -> Option<Instruction> {
        match op >> 12 {
            0x0 => self.line0(op),
            0x1 => self.move_(op, Size::Byte),
            0x2 => self.move_(op, Size::Long),
            0x3 => self.move_(op, Size::Word),
            0x4 => self.line4(op),
            0x5 => self.line5(op),
            0x6 => self.branch(op),
            0x7 => {
                if op & 0x0100 != 0 {
                    return None;
                }
                let data = low_signed(op);
                Some(ins(
                    "moveq",
                    None,
                    vec![
                        Operand::Imm(unsigned32(i32::from(data))),
                        Operand::D(bits3(op, 9)),
                    ],
                ))
            }
            0x8 => self.logic_or_arith(op, "or", Some(("divu", "divs")), "sbcd"),
            0x9 => self.add_sub(op, "sub"),
            0xB => self.line_b(op),
            0xC => self.line_c(op),
            0xD => self.add_sub(op, "add"),
            0xE => self.shift(op),
            // Line A is reserved for emulation traps and line F for
            // coprocessors; neither is a 68000 instruction.
            _ => None,
        }
    }

    fn line0(&mut self, op: u16) -> Option<Instruction> {
        let reg = bits3(op, 9);
        let mode = bits3(op, 3);
        if op & 0x0100 != 0 {
            // Dynamic bit operations, or MOVEP when the mode is An.
            if mode == 1 {
                let disp = signed16(self.word()?);
                let mem = Operand::Disp(disp, bits3(op, 0));
                let size = if op & 0x0040 != 0 {
                    Size::Long
                } else {
                    Size::Word
                };
                let operands = if op & 0x0080 != 0 {
                    vec![Operand::D(reg), mem]
                } else {
                    vec![mem, Operand::D(reg)]
                };
                return Some(ins("movep", Some(size), operands));
            }
            let kind = (op >> 6) & 3;
            let (name, allowed) = bit_op(kind);
            // BTST Dn,#imm is legal: a byte immediate as the destination.
            let allowed = if kind == 0 { allowed | IMM } else { allowed };
            let dst = self.ea_low(op, Size::Byte, allowed)?;
            return Some(ins(name, None, vec![Operand::D(reg), dst]));
        }
        match reg {
            4 => {
                // Static bit operations: the bit number in an extension word.
                let kind = (op >> 6) & 3;
                let (name, allowed) = bit_op(kind);
                // Only the low byte is the bit number; the CPU ignores the rest.
                let bit = self.word()? & 0xFF;
                let dst = self.ea_low(op, Size::Byte, allowed & !IMM)?;
                Some(ins(name, None, vec![Operand::Imm(u32::from(bit)), dst]))
            }
            7 => None, // MOVES, 68010 and later.
            _ => {
                let name = match reg {
                    0 => "ori",
                    1 => "andi",
                    2 => "subi",
                    3 => "addi",
                    5 => "eori",
                    _ => "cmpi",
                };
                // To CCR and SR: the low six bits are 111100, which is the
                // immediate mode a data-alterable operand cannot otherwise be.
                if op & 0x3F == 0x3C && matches!(reg, 0 | 1 | 5) {
                    return match (op >> 6) & 3 {
                        0 => {
                            let imm = self.immediate(Size::Byte)?;
                            Some(ins(
                                name,
                                Some(Size::Byte),
                                vec![Operand::Imm(imm), Operand::Ccr],
                            ))
                        }
                        1 => {
                            let imm = self.immediate(Size::Word)?;
                            Some(ins(
                                name,
                                Some(Size::Word),
                                vec![Operand::Imm(imm), Operand::Sr],
                            ))
                        }
                        _ => None,
                    };
                }
                let size = size_field(op)?;
                let imm = self.immediate(size)?;
                let dst = self.ea_low(op, size, DATA_ALT)?;
                Some(ins(name, Some(size), vec![Operand::Imm(imm), dst]))
            }
        }
    }

    fn move_(&mut self, op: u16, size: Size) -> Option<Instruction> {
        let src_allowed = if size == Size::Byte { ALL & !AN } else { ALL };
        let src = self.ea_low(op, size, src_allowed)?;
        let dmode = bits3(op, 6);
        let dreg = bits3(op, 9);
        if dmode == 1 {
            if size == Size::Byte {
                return None;
            }
            return Some(ins("movea", Some(size), vec![src, Operand::A(dreg)]));
        }
        let dst = self.ea(dmode, dreg, size, DATA_ALT)?;
        Some(ins("move", Some(size), vec![src, dst]))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per opcode group of line 4, which is how the manual lays it out"
    )]
    fn line4(&mut self, op: u16) -> Option<Instruction> {
        let reg = bits3(op, 9);
        if op & 0x0100 != 0 {
            return match (op >> 6) & 7 {
                6 => {
                    let src = self.ea_low(op, Size::Word, DATA)?;
                    Some(ins("chk", Some(Size::Word), vec![src, Operand::D(reg)]))
                }
                7 => {
                    let src = self.ea_low(op, Size::Long, CONTROL)?;
                    Some(ins("lea", None, vec![src, Operand::A(reg)]))
                }
                _ => None,
            };
        }
        match op {
            0x4AFC => return Some(ins("illegal", None, vec![])),
            0x4E70 => return Some(ins("reset", None, vec![])),
            0x4E71 => return Some(ins("nop", None, vec![])),
            0x4E72 => {
                let imm = self.word()?;
                return Some(ins("stop", None, vec![Operand::Imm(u32::from(imm))]));
            }
            0x4E73 => return Some(ins("rte", None, vec![])),
            0x4E75 => return Some(ins("rts", None, vec![])),
            0x4E76 => return Some(ins("trapv", None, vec![])),
            0x4E77 => return Some(ins("rtr", None, vec![])),
            _ => {}
        }
        let sizebits = (op >> 6) & 3;
        match (op >> 8) & 0xF {
            0x0 | 0x2 | 0x4 | 0x6 if sizebits != 3 => {
                let name = match (op >> 8) & 0xF {
                    0x0 => "negx",
                    0x2 => "clr",
                    0x4 => "neg",
                    _ => "not",
                };
                let size = size_field(op)?;
                let dst = self.ea_low(op, size, DATA_ALT)?;
                Some(ins(name, Some(size), vec![dst]))
            }
            0x0 => {
                let dst = self.ea_low(op, Size::Word, DATA_ALT)?;
                Some(ins("move", Some(Size::Word), vec![Operand::Sr, dst]))
            }
            0x4 => {
                let src = self.ea_low(op, Size::Word, DATA)?;
                Some(ins("move", Some(Size::Word), vec![src, Operand::Ccr]))
            }
            0x6 => {
                let src = self.ea_low(op, Size::Word, DATA)?;
                Some(ins("move", Some(Size::Word), vec![src, Operand::Sr]))
            }
            0x8 => match sizebits {
                0 => {
                    let dst = self.ea_low(op, Size::Byte, DATA_ALT)?;
                    Some(ins("nbcd", None, vec![dst]))
                }
                1 if bits3(op, 3) == 0 => Some(ins("swap", None, vec![Operand::D(bits3(op, 0))])),
                1 => {
                    let src = self.ea_low(op, Size::Long, CONTROL)?;
                    Some(ins("pea", None, vec![src]))
                }
                _ if bits3(op, 3) == 0 => {
                    let size = if sizebits == 2 {
                        Size::Word
                    } else {
                        Size::Long
                    };
                    Some(ins("ext", Some(size), vec![Operand::D(bits3(op, 0))]))
                }
                _ => self.movem(op, false),
            },
            0xA => {
                if sizebits == 3 {
                    let dst = self.ea_low(op, Size::Byte, DATA_ALT)?;
                    return Some(ins("tas", None, vec![dst]));
                }
                let size = size_field(op)?;
                let dst = self.ea_low(op, size, DATA_ALT)?;
                Some(ins("tst", Some(size), vec![dst]))
            }
            0xC if sizebits >= 2 => self.movem(op, true),
            0xE => match (op >> 6) & 3 {
                1 => match bits3(op, 3) {
                    0 | 1 => Some(ins("trap", None, vec![Operand::Imm(u32::from(op & 0xF))])),
                    2 => {
                        let disp = self.word()?;
                        Some(ins(
                            "link",
                            None,
                            vec![
                                Operand::A(bits3(op, 0)),
                                Operand::Imm(unsigned32(i32::from(signed16(disp)))),
                            ],
                        ))
                    }
                    3 => Some(ins("unlk", None, vec![Operand::A(bits3(op, 0))])),
                    4 => Some(ins(
                        "move",
                        Some(Size::Long),
                        vec![Operand::A(bits3(op, 0)), Operand::Usp],
                    )),
                    5 => Some(ins(
                        "move",
                        Some(Size::Long),
                        vec![Operand::Usp, Operand::A(bits3(op, 0))],
                    )),
                    _ => None,
                },
                2 => {
                    let dst = self.ea_low(op, Size::Long, CONTROL)?;
                    Some(ins("jsr", None, vec![dst]))
                }
                3 => {
                    let dst = self.ea_low(op, Size::Long, CONTROL)?;
                    Some(ins("jmp", None, vec![dst]))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// MOVEM. The mask comes first, then the effective address; in the
    /// predecrement form the mask is stored reversed, `a7` in bit 0.
    fn movem(&mut self, op: u16, to_registers: bool) -> Option<Instruction> {
        let size = if op & 0x0040 != 0 {
            Size::Long
        } else {
            Size::Word
        };
        let mask = self.word()?;
        if to_registers {
            let src = self.ea_low(op, size, CONTROL | POST)?;
            Some(ins("movem", Some(size), vec![src, Operand::RegList(mask)]))
        } else {
            let dst = self.ea_low(op, size, CONTROL_ALT | PRE)?;
            let mask = if matches!(dst, Operand::PreDec(_)) {
                mask.reverse_bits()
            } else {
                mask
            };
            Some(ins("movem", Some(size), vec![Operand::RegList(mask), dst]))
        }
    }

    fn line5(&mut self, op: u16) -> Option<Instruction> {
        let cond = usize::from((op >> 8) & 0xF);
        if (op >> 6) & 3 == 3 {
            let name = *CONDITIONS.get(cond)?;
            if bits3(op, 3) == 1 {
                let base = self.here();
                let disp = signed16(self.word()?);
                return Some(ins(
                    db_name(cond),
                    None,
                    vec![
                        Operand::D(bits3(op, 0)),
                        Operand::Target(base.wrapping_add_signed(i32::from(disp))),
                    ],
                ));
            }
            let dst = self.ea_low(op, Size::Byte, DATA_ALT)?;
            return Some(ins(s_name(name), None, vec![dst]));
        }
        let size = size_field(op)?;
        let data = match bits3(op, 9) {
            0 => 8,
            n => u32::from(n),
        };
        let allowed = if size == Size::Byte {
            DATA_ALT
        } else {
            ALTERABLE
        };
        let dst = self.ea_low(op, size, allowed)?;
        let name = if op & 0x0100 != 0 { "subq" } else { "addq" };
        Some(ins(name, Some(size), vec![Operand::Imm(data), dst]))
    }

    fn branch(&mut self, op: u16) -> Option<Instruction> {
        let cond = usize::from((op >> 8) & 0xF);
        let base = self.here();
        let short = op & 0xFF;
        let (disp, size) = match short {
            0 => (i32::from(signed16(self.word()?)), Size::Word),
            // 0xFF is the 68020's 32-bit form. A 68000 has none: it takes the
            // byte as -1 and branches to an odd address, which faults. That is
            // what the CPU decodes, so it is what is shown.
            _ => (i32::from(low_signed(op)), Size::Short),
        };
        let name = match cond {
            0 => "bra",
            1 => "bsr",
            _ => b_name(cond),
        };
        Some(ins(
            name,
            Some(size),
            vec![Operand::Target(base.wrapping_add_signed(disp))],
        ))
    }

    /// OR, AND: the logical group with its multiply/divide and BCD neighbours.
    fn logic_or_arith(
        &mut self,
        op: u16,
        name: &'static str,
        muldiv: Option<(&'static str, &'static str)>,
        bcd: &'static str,
    ) -> Option<Instruction> {
        let reg = bits3(op, 9);
        let opmode = bits3(op, 6);
        match opmode {
            3 | 7 => {
                let (u, s) = muldiv?;
                let src = self.ea_low(op, Size::Word, DATA)?;
                Some(ins(
                    if opmode == 3 { u } else { s },
                    Some(Size::Word),
                    vec![src, Operand::D(reg)],
                ))
            }
            4 if bits3(op, 3) <= 1 => Some(Self::xform(op, bcd, None)),
            0..=2 => {
                let size = size_field(op)?;
                let src = self.ea_low(op, size, DATA)?;
                Some(ins(name, Some(size), vec![src, Operand::D(reg)]))
            }
            _ => {
                let size = size_field(op)?;
                let dst = self.ea_low(op, size, MEM_ALT)?;
                Some(ins(name, Some(size), vec![Operand::D(reg), dst]))
            }
        }
    }

    /// The register-to-register and memory-to-memory forms: ABCD, SBCD,
    /// ADDX, SUBX.
    fn xform(op: u16, name: &'static str, size: Option<Size>) -> Instruction {
        let rx = bits3(op, 9);
        let ry = bits3(op, 0);
        let operands = if op & 0x0008 != 0 {
            vec![Operand::PreDec(ry), Operand::PreDec(rx)]
        } else {
            vec![Operand::D(ry), Operand::D(rx)]
        };
        ins(name, size, operands)
    }

    fn add_sub(&mut self, op: u16, name: &'static str) -> Option<Instruction> {
        let reg = bits3(op, 9);
        let opmode = bits3(op, 6);
        let (a_name, x_name) = if name == "add" {
            ("adda", "addx")
        } else {
            ("suba", "subx")
        };
        match opmode {
            3 | 7 => {
                let size = if opmode == 3 { Size::Word } else { Size::Long };
                let src = self.ea_low(op, size, ALL)?;
                Some(ins(a_name, Some(size), vec![src, Operand::A(reg)]))
            }
            0..=2 => {
                let size = size_field(op)?;
                let allowed = if size == Size::Byte { ALL & !AN } else { ALL };
                let src = self.ea_low(op, size, allowed)?;
                Some(ins(name, Some(size), vec![src, Operand::D(reg)]))
            }
            _ => {
                let size = size_field(op)?;
                if bits3(op, 3) <= 1 {
                    return Some(Self::xform(op, x_name, Some(size)));
                }
                let dst = self.ea_low(op, size, MEM_ALT)?;
                Some(ins(name, Some(size), vec![Operand::D(reg), dst]))
            }
        }
    }

    fn line_b(&mut self, op: u16) -> Option<Instruction> {
        let reg = bits3(op, 9);
        let opmode = bits3(op, 6);
        match opmode {
            3 | 7 => {
                let size = if opmode == 3 { Size::Word } else { Size::Long };
                let src = self.ea_low(op, size, ALL)?;
                Some(ins("cmpa", Some(size), vec![src, Operand::A(reg)]))
            }
            0..=2 => {
                let size = size_field(op)?;
                let allowed = if size == Size::Byte { ALL & !AN } else { ALL };
                let src = self.ea_low(op, size, allowed)?;
                Some(ins("cmp", Some(size), vec![src, Operand::D(reg)]))
            }
            _ => {
                let size = size_field(op)?;
                if bits3(op, 3) == 1 {
                    return Some(ins(
                        "cmpm",
                        Some(size),
                        vec![Operand::PostInc(bits3(op, 0)), Operand::PostInc(reg)],
                    ));
                }
                let dst = self.ea_low(op, size, DATA_ALT)?;
                Some(ins("eor", Some(size), vec![Operand::D(reg), dst]))
            }
        }
    }

    fn line_c(&mut self, op: u16) -> Option<Instruction> {
        let rx = bits3(op, 9);
        let ry = bits3(op, 0);
        match (bits3(op, 6), bits3(op, 3)) {
            (5, 0) => Some(ins("exg", None, vec![Operand::D(rx), Operand::D(ry)])),
            (5, 1) => Some(ins("exg", None, vec![Operand::A(rx), Operand::A(ry)])),
            (6, 1) => Some(ins("exg", None, vec![Operand::D(rx), Operand::A(ry)])),
            _ => self.logic_or_arith(op, "and", Some(("mulu", "muls")), "abcd"),
        }
    }

    fn shift(&mut self, op: u16) -> Option<Instruction> {
        const NAMES: [[&str; 2]; 4] = [
            ["asr", "asl"],
            ["lsr", "lsl"],
            ["roxr", "roxl"],
            ["ror", "rol"],
        ];
        let left = usize::from(op & 0x0100 != 0);
        if (op >> 6) & 3 == 3 {
            // Memory form: one bit, a word, and only the four basic kinds.
            if op & 0x0800 != 0 {
                return None;
            }
            let kind = usize::from((op >> 9) & 3);
            let name = *NAMES.get(kind)?.get(left)?;
            let dst = self.ea_low(op, Size::Word, MEM_ALT)?;
            return Some(ins(name, Some(Size::Word), vec![dst]));
        }
        let size = size_field(op)?;
        let kind = usize::from((op >> 3) & 3);
        let name = *NAMES.get(kind)?.get(left)?;
        let count = bits3(op, 9);
        let first = if op & 0x0020 != 0 {
            Operand::D(count)
        } else {
            Operand::Imm(if count == 0 { 8 } else { u32::from(count) })
        };
        Some(ins(name, Some(size), vec![first, Operand::D(bits3(op, 0))]))
    }
}

/// The four bit operations, and where each may write.
const fn bit_op(kind: u16) -> (&'static str, u16) {
    match kind {
        0 => ("btst", DATA),
        1 => ("bchg", DATA_ALT),
        2 => ("bclr", DATA_ALT),
        _ => ("bset", DATA_ALT),
    }
}

fn b_name(cond: usize) -> &'static str {
    const B: [&str; 16] = [
        "bra", "bsr", "bhi", "bls", "bcc", "bcs", "bne", "beq", "bvc", "bvs", "bpl", "bmi", "bge",
        "blt", "bgt", "ble",
    ];
    B.get(cond).copied().unwrap_or("b?")
}

fn db_name(cond: usize) -> &'static str {
    const DB: [&str; 16] = [
        "dbt", "dbf", "dbhi", "dbls", "dbcc", "dbcs", "dbne", "dbeq", "dbvc", "dbvs", "dbpl",
        "dbmi", "dbge", "dblt", "dbgt", "dble",
    ];
    DB.get(cond).copied().unwrap_or("db?")
}

fn s_name(cond: &str) -> &'static str {
    match cond {
        "t" => "st",
        "f" => "sf",
        "hi" => "shi",
        "ls" => "sls",
        "cc" => "scc",
        "cs" => "scs",
        "ne" => "sne",
        "eq" => "seq",
        "vc" => "svc",
        "vs" => "svs",
        "pl" => "spl",
        "mi" => "smi",
        "ge" => "sge",
        "lt" => "slt",
        "gt" => "sgt",
        _ => "sle",
    }
}

// ---------------------------------------------------------------------------
// Motorola syntax
// ---------------------------------------------------------------------------

/// A register by number, 0–7 data and 8–15 address, with `a7` as `sp`.
pub(crate) fn reg_name(reg: u8) -> String {
    match reg {
        0..=7 => format!("d{reg}"),
        15 => "sp".to_owned(),
        _ => format!("a{}", reg.saturating_sub(8)),
    }
}

pub(crate) fn a_name(reg: u8) -> String {
    reg_name(reg.saturating_add(8))
}

/// A number the way an Amiga assembler listing writes it: small ones in
/// decimal, anything else in hex with a `$`.
pub(crate) fn number(value: i64) -> String {
    if (-9..=9).contains(&value) {
        value.to_string()
    } else if value < 0 {
        format!("-${:x}", value.unsigned_abs())
    } else {
        format!("${value:x}")
    }
}

pub(crate) fn index(i: Index) -> String {
    let scale = if i.scale == 1 {
        String::new()
    } else {
        format!("*{}", i.scale)
    };
    format!(
        "{}.{}{scale}",
        reg_name(i.reg),
        if i.long { "l" } else { "w" }
    )
}

/// A register list: runs collapsed, data registers before address registers.
fn reg_list(mask: u16) -> String {
    let mut parts: Vec<String> = Vec::new();
    for bank in 0..2u8 {
        let mut r = 0u8;
        while r < 8 {
            let bit = bank.saturating_mul(8).saturating_add(r);
            if mask & (1u16 << bit) == 0 {
                r = r.saturating_add(1);
                continue;
            }
            let start = r;
            while r < 8 && mask & (1u16 << bank.saturating_mul(8).saturating_add(r)) != 0 {
                r = r.saturating_add(1);
            }
            let end = r.saturating_sub(1);
            let first = reg_name(bank.saturating_mul(8).saturating_add(start));
            if end == start {
                parts.push(first);
            } else {
                parts.push(format!(
                    "{first}-{}",
                    reg_name(bank.saturating_mul(8).saturating_add(end))
                ));
            }
        }
    }
    parts.join("/")
}

/// How a listing names numbers in an instruction.
///
/// Two kinds, kept apart because they are different claims. A **position** —
/// a branch target, a PC-relative operand — is a place in the listing, so the
/// listing's labels name it. An **absolute** value — an immediate, an absolute
/// address — is a number until the executable's relocations say the loader
/// turns it into an address; only then may it be named. Merging the two named
/// `move.l #$c8,d2` as `#lab_00c8` because a branch target happened to sit at
/// offset `$c8`.
pub struct Names<'a> {
    /// Names a place in the listing.
    pub position: &'a dyn Fn(u32) -> Option<String>,
    /// Names a value the loader relocates.
    pub absolute: &'a dyn Fn(u32) -> Option<String>,
}

const fn nameless(_: u32) -> Option<String> {
    None
}

impl Names<'static> {
    /// No names at all: every address is a number.
    pub const NONE: Self = Self {
        position: &nameless,
        absolute: &nameless,
    };
}

impl Operand {
    /// The operand in Motorola syntax, naming what `names` can name.
    #[must_use]
    pub fn motorola(&self, names: &Names<'_>) -> String {
        let label = names.position;
        let addr = |a: u32| label(a).unwrap_or_else(|| format!("${a:x}"));
        match self {
            Self::D(r) => reg_name(*r),
            Self::A(r) => a_name(*r),
            Self::Ind(r) => format!("({})", a_name(*r)),
            Self::PostInc(r) => format!("({})+", a_name(*r)),
            Self::PreDec(r) => format!("-({})", a_name(*r)),
            Self::Disp(d, r) => format!("{}({})", number(i64::from(*d)), a_name(*r)),
            Self::Indexed(d, r, i) => {
                format!("{}({},{})", number(i64::from(*d)), a_name(*r), index(*i))
            }
            Self::AbsW(a) => format!("(${:x}).w", unsigned32(i32::from(*a))),
            Self::AbsL(a) => (names.absolute)(*a).unwrap_or_else(|| format!("(${a:x}).l")),
            Self::PcDisp(t) => format!("{}(pc)", addr(*t)),
            Self::PcIndexed(t, i) => format!("{}(pc,{})", addr(*t), index(*i)),
            Self::Imm(v) => match (names.absolute)(*v) {
                Some(name) => format!("#{name}"),
                None => format!("#{}", number(i64::from(*v))),
            },
            // An empty mask moves nothing; there is no list to write, so the
            // mask itself is shown, as objdump does.
            Self::RegList(0) => "#0".to_owned(),
            Self::RegList(m) => reg_list(*m),
            Self::Ccr => "ccr".to_owned(),
            Self::Sr => "sr".to_owned(),
            Self::Usp => "usp".to_owned(),
            Self::Target(t) => addr(*t),
        }
    }
}

impl Instruction {
    /// The mnemonic with its size: `move.l`.
    #[must_use]
    pub fn opcode(&self) -> String {
        match self.size {
            Some(s) => format!("{}.{}", self.mnemonic, s.suffix()),
            None => self.mnemonic.to_owned(),
        }
    }

    /// The whole instruction in Motorola syntax, mnemonic padded to a column.
    #[must_use]
    pub fn motorola(&self, names: &Names<'_>) -> String {
        let operands: Vec<String> = self
            .operands
            .iter()
            .map(|o| match o {
                // A quick constant and a frame size are signed: `moveq #-1`,
                // `link a5,#-$20`, never `#$ffffffff`.
                Operand::Imm(v) if matches!(self.mnemonic, "moveq" | "link") => {
                    format!(
                        "#{}",
                        number(i64::from(i32::from_ne_bytes(v.to_ne_bytes())))
                    )
                }
                other => other.motorola(names),
            })
            .collect();
        if operands.is_empty() {
            self.opcode()
        } else {
            format!("{:<8}{}", self.opcode(), operands.join(","))
        }
    }

    /// Addresses this instruction refers to by position: branch targets and
    /// PC-relative operands. A listing labels them.
    #[must_use]
    pub fn targets(&self) -> Vec<u32> {
        self.operands
            .iter()
            .filter_map(|o| match o {
                Operand::Target(t) | Operand::PcDisp(t) | Operand::PcIndexed(t, _) => Some(*t),
                _ => None,
            })
            .collect()
    }

    /// Whether control never falls through to the next instruction: the
    /// listing leaves a gap after one, as an assembler programmer would.
    #[must_use]
    pub fn ends_flow(&self) -> bool {
        matches!(
            self.mnemonic,
            "rts" | "rte" | "rtr" | "jmp" | "bra" | "illegal"
        )
    }
}

// ---------------------------------------------------------------------------
// GNU (MIT) syntax, as objdump prints it
// ---------------------------------------------------------------------------

/// A register as GNU names it: `a6` is `%fp`, `a7` is `%sp`.
fn gnu_reg(r: u8) -> String {
    match r {
        0..=7 => format!("%d{r}"),
        14 => "%fp".to_owned(),
        15 => "%sp".to_owned(),
        _ => format!("%a{}", r.saturating_sub(8)),
    }
}

fn gnu_index(i: Index) -> String {
    let scale = if i.scale == 1 {
        String::new()
    } else {
        format!(":{}", i.scale)
    };
    format!(
        "{}:{}{scale}",
        gnu_reg(i.reg),
        if i.long { "l" } else { "w" }
    )
}

fn gnu_list(mask: u16) -> String {
    // objdump runs a range straight across from d7 to a0: `%d0-%fp`, where a
    // Motorola listing writes `d0-d7/a0-a6`.
    let mut parts = Vec::new();
    let mut r = 0u8;
    while r < 16 {
        if mask & (1u16 << r) == 0 {
            r = r.saturating_add(1);
            continue;
        }
        let start = r;
        while r < 16 && mask & (1u16 << r) != 0 {
            r = r.saturating_add(1);
        }
        let end = r.saturating_sub(1);
        parts.push(if end == start {
            gnu_reg(start)
        } else {
            format!("{}-{}", gnu_reg(start), gnu_reg(end))
        });
    }
    parts.join("/")
}

impl Instruction {
    /// The instruction exactly as GNU objdump prints it for `m68k:68000`.
    ///
    /// Not for people — Motorola syntax is what an Amiga programmer reads.
    /// It exists so the tests can compare ADE with objdump character for
    /// character (D-019), which includes copying objdump's quirks: an 8-bit
    /// index displacement in bare hex sign-extended to 64 bits, and immediates
    /// signed at the instruction's size.
    #[must_use]
    pub fn gnu(&self) -> String {
        let signed = |v: u32| -> String {
            match self.size {
                Some(Size::Byte | Size::Short) => {
                    format!("#{}", low_signed(u16::try_from(v & 0xFF).unwrap_or(0)))
                }
                Some(Size::Word) => {
                    format!("#{}", signed16(u16::try_from(v & 0xFFFF).unwrap_or(0)))
                }
                _ => format!("#{}", i32::from_ne_bytes(v.to_ne_bytes())),
            }
        };
        let operand = |o: &Operand| match o {
            Operand::D(r) => gnu_reg(*r),
            Operand::A(r) => gnu_reg(r.saturating_add(8)),
            Operand::Ind(r) => format!("{}@", gnu_reg(r.saturating_add(8))),
            Operand::PostInc(r) => format!("{}@+", gnu_reg(r.saturating_add(8))),
            Operand::PreDec(r) => format!("{}@-", gnu_reg(r.saturating_add(8))),
            Operand::Disp(d, r) => format!("{}@({d})", gnu_reg(r.saturating_add(8))),
            Operand::Indexed(d, r, x) => format!(
                "{}@({:x},{})",
                gnu_reg(r.saturating_add(8)),
                u64::from_ne_bytes(i64::from(*d).to_ne_bytes()),
                gnu_index(*x)
            ),
            Operand::AbsW(a) => format!("{:#x}", unsigned32(i32::from(*a))),
            Operand::AbsL(a) | Operand::Target(a) => format!("{a:#x}"),
            Operand::PcDisp(t) => format!("%pc@({t:#x})"),
            Operand::PcIndexed(t, x) => format!("%pc@({t:#x},{})", gnu_index(*x)),
            // A static bit number is a byte; objdump signs it, so bit 162
            // prints as #-94.
            Operand::Imm(v) if matches!(self.mnemonic, "btst" | "bchg" | "bclr" | "bset") => {
                format!("#{}", low_signed(u16::try_from(*v & 0xFF).unwrap_or(0)))
            }
            // STOP's operand is SR's 16-bit value; objdump signs it.
            Operand::Imm(v) if self.mnemonic == "stop" => {
                format!("#{}", signed16(u16::try_from(*v & 0xFFFF).unwrap_or(0)))
            }
            Operand::Imm(v) => signed(*v),
            Operand::RegList(0) => "#0".to_owned(),
            Operand::RegList(m) => gnu_list(*m),
            Operand::Ccr => "%ccr".to_owned(),
            Operand::Sr => "%sr".to_owned(),
            Operand::Usp => "%usp".to_owned(),
        };
        let size = match (self.mnemonic, self.size) {
            ("link", _) => "w",
            (_, Some(s)) => s.suffix(),
            (_, None) => "",
        };
        let ops: Vec<String> = self.operands.iter().map(operand).collect();
        if ops.is_empty() {
            format!("{}{size}", self.mnemonic)
        } else {
            format!("{}{size} {}", self.mnemonic, ops.join(","))
        }
    }
}

/// Opcode words where objdump's answer is not the 68000's, and why. Each is a
/// property of objdump rather than a choice to agree with it less; the tests
/// skip these and nothing else.
#[must_use]
pub fn objdump_differs(op: u16) -> Option<&'static str> {
    if op >> 12 == 0xF {
        // objdump decodes FPU and MMU coprocessor words even when told the
        // CPU is a 68000, which has no coprocessor interface.
        return Some("line F: objdump decodes coprocessor words a 68000 lacks");
    }
    if op == 0x4AFC || op == 0x4AFD {
        // ILLEGAL followed by a word reads as SVR4's `swbeg` switch marker.
        return Some("swbeg: objdump reads ILLEGAL plus a word as an SVR4 marker");
    }
    if op & 0xF1F8 == 0x5108 {
        // SUBQ.B to an address register; the manual allows only word and
        // long for An, and objdump accepts it for SUBQ but not ADDQ.
        return Some("subq.b to An: objdump accepts a size the manual forbids");
    }
    None
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests over fixed bytes"
)]
mod tests {
    use super::*;

    fn text(bytes: &[u8]) -> String {
        match decode(bytes, 0) {
            Decoded::Instruction(i) => i.motorola(&Names::NONE),
            other => panic!("{bytes:02x?}: {other:?}"),
        }
    }

    #[test]
    fn the_everyday_instructions_read_as_an_amiga_programmer_writes_them() {
        assert_eq!(text(&[0x70, 0x00]), "moveq   #0,d0");
        assert_eq!(text(&[0x70, 0xff]), "moveq   #-1,d0");
        assert_eq!(text(&[0x4e, 0x55, 0xff, 0xe0]), "link    a5,#-$20");
        assert_eq!(text(&[0x2f, 0x00]), "move.l  d0,-(sp)");
        assert_eq!(text(&[0x4e, 0xae, 0xfd, 0xd8]), "jsr     -$228(a6)");
        assert_eq!(text(&[0x2c, 0x78, 0x00, 0x04]), "movea.l ($4).w,a6");
        assert_eq!(text(&[0x4e, 0x75]), "rts");
        assert_eq!(text(&[0x48, 0xe7, 0xff, 0xfe]), "movem.l d0-d7/a0-a6,-(sp)");
        assert_eq!(text(&[0x4c, 0xdf, 0x7f, 0xff]), "movem.l (sp)+,d0-d7/a0-a6");
        assert_eq!(text(&[0x41, 0xfa, 0x00, 0x10]), "lea     $12(pc),a0");
        // Found in 22 corpus programs: a MOVEM with an empty mask.
        assert_eq!(text(&[0x48, 0xe7, 0x00, 0x00]), "movem.l #0,-(sp)");
        assert_eq!(text(&[0x66, 0xfe]), "bne.s   $0");
        assert_eq!(text(&[0x51, 0xc8, 0xff, 0xfe]), "dbf     d0,$0");
    }

    #[test]
    fn what_the_68000_lacks_is_invalid_and_what_runs_out_is_truncated() {
        // A 68020 full-format index word is named as such; line A and line F
        // are simply not instructions.
        assert_eq!(
            decode(&[0x20, 0x30, 0x01, 0x00], 0),
            Decoded::Needs68020(0x2030)
        );
        // A scaled index is shown, not refused: a 68000 ignores the scale.
        assert_eq!(
            text(&[0x20, 0x30, 0x06, 0xf0]),
            "move.l  -$10(a0,d0.w*8),d0"
        );
        // A branch byte of FF is the 68020's long form; a 68000 reads -1.
        assert_eq!(text(&[0x60, 0xff]), "bra.s   $1");
        assert_eq!(decode(&[0xa0, 0x00], 0), Decoded::Invalid(0xa000));
        assert_eq!(decode(&[0xf2, 0x00], 0), Decoded::Invalid(0xf200));
        assert_eq!(decode(&[0x4e, 0xb9, 0x00], 0), Decoded::Truncated);
        assert_eq!(decode(&[0x4e], 0), Decoded::Truncated);
    }

    #[test]
    fn no_word_and_no_tail_can_make_it_panic() {
        // Every opcode word, followed by every length of tail from nothing to
        // the longest an instruction can need.
        let tail = [0x12u8, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0];
        for op in 0..=u16::MAX {
            let mut bytes = vec![
                u8::try_from(op >> 8).unwrap(),
                u8::try_from(op & 0xFF).unwrap(),
            ];
            for n in 0..=tail.len() {
                bytes.truncate(2);
                bytes.extend_from_slice(&tail[..n]);
                let d = decode(&bytes, 0x1000);
                assert!(d.len() >= 2 && d.len() <= 10);
            }
        }
    }
}
