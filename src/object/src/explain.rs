//! What an instruction does, in words (F-032).
//!
//! The Assembly view's comment column. Each explanation is built from the
//! decoded [`Instruction`] — its mnemonic, size and operands — never from the
//! text of the listing, so it cannot disagree with the line it sits beside.
//!
//! The notation is assignment-style, because it is short and exact: `d0 = 0`,
//! `word at a1 = word at a0; a0 += 2, a1 += 2`, `byte at $bfd100 |= $88`.
//! Memory is written as "a *size* at *address*", since an assembler's `(a0)`
//! is exactly the thing that is unclear to somebody learning to read one.
//!
//! Addresses the hardware gives meaning to are named from [`hardware`]:
//! `$dff096 (DMACON)`, with what the register is for after a dash, and for the
//! two CIA ports the meaning of the bit being tested — `btst #6,$bfe001` is
//! the left mouse button. Nothing is guessed: an address with no entry is a
//! number, and a library call through `a6` is an offset, because which
//! library `a6` holds is not knowable from one instruction (D-019).

use crate::hardware;
use crate::m68k::{Instruction, Names, Operand, Size, a_name, index, number, reg_name};

/// Explain `instruction`, naming what `names` can name — exactly the names
/// the instruction's own text uses, so the two cannot disagree.
#[must_use]
pub fn explain(i: &Instruction, names: &Names<'_>) -> String {
    let mut x = Explainer {
        i,
        label: names.position,
        absolute: names.absolute,
        effects: Vec::new(),
        named: None,
    };
    let main = x.main();
    let mut out = main;
    if !x.effects.is_empty() {
        out.push_str("; ");
        out.push_str(&x.effects.join(", "));
    }
    // The name is already beside the address; the dash adds what it is for.
    if let Some((_, meaning)) = x.named {
        use core::fmt::Write as _;
        let _ = write!(out, " — {meaning}");
    }
    out
}

struct Explainer<'a> {
    i: &'a Instruction,
    /// Names a place in the listing.
    label: &'a dyn Fn(u32) -> Option<String>,
    /// Names a value the loader relocates.
    absolute: &'a dyn Fn(u32) -> Option<String>,
    /// Register updates that happen as a side effect: `a0 += 2`.
    effects: Vec<String>,
    /// The first hardware address the instruction touches, and what it is.
    named: Option<(String, &'static str)>,
}

/// What a condition code means, for branches, `dbcc` and `scc`.
fn condition(cc: &str) -> &'static str {
    match cc {
        "t" => "always",
        "f" => "never",
        "hi" => "higher (unsigned >)",
        "ls" => "lower or same (unsigned <=)",
        "cc" => "carry clear (unsigned >=)",
        "cs" => "carry set (unsigned <)",
        "ne" => "not equal",
        "eq" => "equal",
        "vc" => "no overflow",
        "vs" => "overflow",
        "pl" => "positive or zero",
        "mi" => "negative",
        "ge" => "greater or equal (signed)",
        "lt" => "less than (signed)",
        "gt" => "greater than (signed)",
        "le" => "less or equal (signed)",
        _ => "the condition holds",
    }
}

const fn size_word(size: Option<Size>) -> &'static str {
    match size {
        Some(Size::Byte) => "byte",
        Some(Size::Word) => "word",
        _ => "long",
    }
}

const fn size_bytes(size: Option<Size>, reg: u8) -> i64 {
    match size {
        // A byte through the stack pointer moves it by two, keeping it even.
        Some(Size::Byte) if reg == 7 => 2,
        Some(Size::Byte) => 1,
        Some(Size::Word) => 2,
        _ => 4,
    }
}

/// Instructions that act on one byte of memory but carry no size field:
/// the bit operations, `tas`, the BCD arithmetic and the set-on-condition
/// family. Their
/// `(a0)+` moves `a0` by one, not four.
pub(crate) fn byte_sized(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "tas"
            | "nbcd"
            | "abcd"
            | "sbcd"
            | "btst"
            | "bset"
            | "bclr"
            | "bchg"
            | "st"
            | "sf"
            | "shi"
            | "sls"
            | "scc"
            | "scs"
            | "sne"
            | "seq"
            | "svc"
            | "svs"
            | "spl"
            | "smi"
            | "sge"
            | "slt"
            | "sgt"
            | "sle"
    )
}

impl Explainer<'_> {
    /// The size the instruction moves through memory, including for the
    /// instructions whose size is implicit.
    fn size(&self) -> Option<Size> {
        self.i
            .size
            .or_else(|| byte_sized(self.i.mnemonic).then_some(Size::Byte))
    }

    /// An address the hardware gives a name to, remembered for the dash.
    fn hardware(&mut self, address: u32) -> String {
        let shown = format!("${address:x}");
        match hardware::name(address) {
            Some(named) => {
                if self.named.is_none() {
                    self.named = Some((named.name.to_owned(), named.meaning));
                }
                format!("{shown} ({})", named.name)
            }
            None => shown,
        }
    }

    fn target(&self, address: u32) -> String {
        (self.label)(address).unwrap_or_else(|| format!("${address:x}"))
    }

    /// Where an operand points, for one that is memory.
    fn address(&mut self, o: &Operand) -> Option<String> {
        Some(match o {
            Operand::Ind(r) | Operand::PostInc(r) | Operand::PreDec(r) => a_name(*r),
            Operand::Disp(d, r) => offset(&a_name(*r), i64::from(*d)),
            Operand::Indexed(d, r, x) => {
                offset(&format!("{}+{}", a_name(*r), index(*x)), i64::from(*d))
            }
            Operand::AbsW(a) => self.hardware(u32::from_ne_bytes(i32::from(*a).to_ne_bytes())),
            Operand::AbsL(a) => (self.absolute)(*a).unwrap_or_else(|| self.hardware(*a)),
            Operand::PcDisp(t) => self.target(*t),
            Operand::PcIndexed(t, x) => format!("{}+{}", self.target(*t), index(*x)),
            _ => return None,
        })
    }

    /// Note the register update a post-increment or pre-decrement makes.
    fn effect(&mut self, o: &Operand) {
        let size = self.size();
        match o {
            Operand::PostInc(r) => {
                self.effects
                    .push(format!("{} += {}", a_name(*r), size_bytes(size, *r)));
            }
            Operand::PreDec(r) => {
                self.effects
                    .push(format!("{} -= {} first", a_name(*r), size_bytes(size, *r)));
            }
            _ => {}
        }
    }

    /// An operand as a value: a register, a number, or "the word at …".
    fn value(&mut self, o: &Operand) -> String {
        let size = self.size();
        self.effect(o);
        match o {
            Operand::D(r) => sized_reg(*r, size),
            Operand::A(r) => a_name(*r),
            Operand::Imm(v) => (self.absolute)(*v).unwrap_or_else(|| imm(*v, size)),
            Operand::Sr => "the status register".to_owned(),
            Operand::Ccr => "the condition codes".to_owned(),
            Operand::Usp => "the user stack pointer".to_owned(),
            Operand::Target(t) => self.target(*t),
            Operand::RegList(m) => list(*m),
            mem => {
                let at = self.address(mem).unwrap_or_default();
                format!("{} at {at}", size_word(size))
            }
        }
    }

    /// The instruction's two operands as values, source then destination.
    fn pair(&mut self) -> Option<(String, String)> {
        let [src, dst] = self.i.operands.as_slice() else {
            return None;
        };
        Some((self.value(src), self.value(dst)))
    }

    fn one(&mut self) -> Option<String> {
        let first = self.i.operands.first()?.clone();
        Some(self.value(&first))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per mnemonic: a dispatch table reads best as one table"
    )]
    fn main(&mut self) -> String {
        let m = self.i.mnemonic;
        let size = size_word(self.size());
        let ops = self.i.operands.clone();
        match m {
            "move" | "movea" => self.moving(&ops),
            "moveq" => {
                let v = imm_signed(ops.first());
                let d = ops.get(1).map(|o| self.value(o)).unwrap_or_default();
                format!("{d} = {v}")
            }
            "lea" => {
                let (Some(src), Some(Operand::A(r))) = (ops.first(), ops.get(1)) else {
                    return "load an address".to_owned();
                };
                let at = self.effective(src);
                // A label is a place; what lea loads is where it is.
                let at = if matches!(src, Operand::PcDisp(_) | Operand::PcIndexed(..)) {
                    format!("the address of {at}")
                } else {
                    at
                };
                format!("{} = {at}", a_name(*r))
            }
            "pea" => {
                let at = ops.first().map(|o| self.effective(o)).unwrap_or_default();
                // `effective` already says "the address in a4" for `(a4)`.
                if at.starts_with("the address") {
                    format!("push {at} onto the stack")
                } else {
                    format!("push the address {at} onto the stack")
                }
            }
            "clr" => format!("{} = 0", self.one().unwrap_or_default()),
            "tst" => format!("compare {} with 0", the(&self.one().unwrap_or_default())),
            "tas" => format!(
                "test {} and set its top bit, in one indivisible step",
                the(&self.one().unwrap_or_default())
            ),
            "neg" => {
                let d = self.one().unwrap_or_default();
                format!("{d} = -{d}")
            }
            "negx" => {
                let d = self.one().unwrap_or_default();
                format!("{d} = -{d} - X (multi-precision negate)")
            }
            "not" => format!(
                "invert every bit of {}",
                the(&self.one().unwrap_or_default())
            ),
            "nbcd" => format!(
                "negate {} as decimal (BCD) digits",
                the(&self.one().unwrap_or_default())
            ),
            "cmp" | "cmpa" | "cmpi" | "cmpm" => match self.pair() {
                Some((s, d)) => format!("compare {} with {}", the(&d), the(&s)),
                None => "compare".to_owned(),
            },
            "add" | "adda" | "addi" | "addq" => self.arith(&ops, "+="),
            "sub" | "suba" | "subi" | "subq" => self.arith(&ops, "-="),
            "addx" => self.arith(&ops, "+= X +"),
            "subx" => self.arith(&ops, "-= X +"),
            "and" | "andi" => self.logic(&ops, "&=", "keep only bits", "of"),
            "or" | "ori" => self.logic(&ops, "|=", "set bits", "in"),
            "eor" | "eori" => self.logic(&ops, "^=", "flip bits", "in"),
            "abcd" | "sbcd" => match self.pair() {
                Some((s, d)) => format!(
                    "{d} {}= {s} + X, as decimal (BCD) digits",
                    if m == "abcd" { "+" } else { "-" }
                ),
                None => "decimal arithmetic".to_owned(),
            },
            "mulu" | "muls" => match self.pair() {
                Some((s, d)) => format!(
                    "{} = {d} × {s} ({}, 16 × 16 → 32 bits)",
                    long_reg(&d),
                    if m == "mulu" { "unsigned" } else { "signed" }
                ),
                None => "multiply".to_owned(),
            },
            "divu" | "divs" => match self.pair() {
                Some((s, d)) => format!(
                    "{} /= {s} ({}): quotient in the low word, remainder in the high word",
                    long_reg(&d),
                    if m == "divu" { "unsigned" } else { "signed" }
                ),
                None => "divide".to_owned(),
            },
            "ext" => match ops.first() {
                Some(Operand::D(r)) if self.size() == Some(Size::Word) => {
                    format!("sign-extend {}'s low byte to a word", reg_name(*r))
                }
                Some(Operand::D(r)) => format!("sign-extend {}'s low word to a long", reg_name(*r)),
                _ => "sign-extend".to_owned(),
            },
            "swap" => format!(
                "swap the high and low words of {}",
                self.one().unwrap_or_default()
            ),
            "exg" => match self.pair() {
                Some((a, b)) => format!("exchange {a} and {b}"),
                None => "exchange two registers".to_owned(),
            },
            "lsl" | "lsr" | "asl" | "asr" | "rol" | "ror" | "roxl" | "roxr" => self.shift(&ops),
            "btst" | "bset" | "bclr" | "bchg" => self.bit(&ops),
            "bra" => format!("go to {}", self.one().unwrap_or_default()),
            "bsr" => format!("call the subroutine {}", self.one().unwrap_or_default()),
            "jmp" => {
                let at = ops.first().map(|o| self.effective(o)).unwrap_or_default();
                format!("jump to {at}")
            }
            "jsr" => match ops.first() {
                // A call through a6 at a negative offset is a library call on
                // the Amiga; which library is not knowable from here (D-019).
                Some(Operand::Disp(d, 6)) if *d < 0 => format!(
                    "call the library function at a6{} (a6 holds the library's base)",
                    number(i64::from(*d))
                ),
                Some(o) => {
                    let at = self.effective(&o.clone());
                    format!("call the subroutine at {at}")
                }
                None => "call a subroutine".to_owned(),
            },
            "rts" => "return from the subroutine".to_owned(),
            "rtr" => "return, restoring the condition codes from the stack".to_owned(),
            "rte" => "return from the exception or interrupt".to_owned(),
            "nop" => "do nothing".to_owned(),
            "reset" => "reset every external device (supervisor only)".to_owned(),
            "illegal" => "raise an illegal-instruction exception".to_owned(),
            "trapv" => "raise a TRAPV exception if the overflow flag is set".to_owned(),
            "trap" => match ops.first() {
                Some(Operand::Imm(n)) => format!(
                    "raise exception TRAP #{n}: run the handler whose address is at ${:x}",
                    0x80u32.saturating_add(n.saturating_mul(4))
                ),
                _ => "raise a TRAP exception".to_owned(),
            },
            "stop" => match ops.first() {
                Some(Operand::Imm(v)) => format!(
                    "set the status register to ${v:x} and halt until an interrupt{}",
                    interrupt_note(*v, true)
                ),
                _ => "halt until an interrupt".to_owned(),
            },
            "link" => match (ops.first(), ops.get(1)) {
                (Some(Operand::A(r)), Some(Operand::Imm(v))) => {
                    // The displacement is added to sp: negative reserves room
                    // for locals, which is what a compiler emits; positive is
                    // legal and moves sp the other way.
                    let shift = i32::from_ne_bytes(v.to_ne_bytes());
                    let bytes = number(i64::from(shift.unsigned_abs()));
                    let frame = a_name(*r);
                    match shift {
                        0 => {
                            format!("make a stack frame with no locals: push {frame}, {frame} = sp")
                        }
                        s if s < 0 => format!(
                            "make a stack frame with {bytes} bytes of locals: push {frame}, \
                             {frame} = sp, sp -= {bytes}"
                        ),
                        _ => format!(
                            "make a stack frame: push {frame}, {frame} = sp, then sp += {bytes}"
                        ),
                    }
                }
                _ => "make a stack frame".to_owned(),
            },
            "unlk" => match ops.first() {
                Some(Operand::A(r)) => {
                    format!("remove the stack frame: sp = {0}, then pop {0}", a_name(*r))
                }
                _ => "remove a stack frame".to_owned(),
            },
            "chk" => match self.pair() {
                Some((s, d)) => format!(
                    "raise a CHK exception if {d} is below 0 or above {}",
                    the(&s)
                ),
                None => "bounds check".to_owned(),
            },
            "movem" => self.movem(&ops),
            "movep" => match self.pair() {
                Some((s, d)) => {
                    format!("{d} = {s}, one byte to every other address (for 8-bit peripherals)")
                }
                None => "move to a peripheral".to_owned(),
            },
            m if m.starts_with("db") => self.dbcc(&ops, &m[2..]),
            m if m.starts_with('b') => {
                let t = self.one().unwrap_or_default();
                format!("branch to {t} if {}", condition(&m[1..]))
            }
            m if m.starts_with('s') => {
                let d = self.one().unwrap_or_default();
                match &m[1..] {
                    "t" => format!("{d} = $ff"),
                    "f" => format!("{d} = 0"),
                    cc => format!("{d} = $ff if {}, else 0", condition(cc)),
                }
            }
            _ => format!("{m} ({size})"),
        }
    }

    /// The address an operand stands for, as `lea`, `pea` and jumps use it.
    fn effective(&mut self, o: &Operand) -> String {
        if let Some(a) = absolute(o)
            && let Some(what) = hardware::base(a)
        {
            return format!("${a:x} ({what})");
        }
        match o {
            Operand::PcDisp(t) => self.target(*t),
            Operand::Ind(r) => format!("the address in {}", a_name(*r)),
            _ => self.address(o).unwrap_or_else(|| "an address".to_owned()),
        }
    }

    fn moving(&mut self, ops: &[Operand]) -> String {
        let [src, dst] = ops else {
            return "move".to_owned();
        };
        match (src, dst) {
            (Operand::Imm(v), Operand::Sr) => format!(
                "set the status register to ${v:x}{}",
                interrupt_note(*v, true)
            ),
            (s, Operand::Sr) => format!("set the status register to {}", self.value(s)),
            (s, Operand::Ccr) => format!("set the condition codes to {}", self.value(s)),
            (Operand::Sr, d) => format!("{} = the status register", self.value(d)),
            (Operand::A(r), Operand::Usp) => format!("user stack pointer = {}", a_name(*r)),
            (Operand::Usp, Operand::A(r)) => format!("{} = user stack pointer", a_name(*r)),
            (s, Operand::PreDec(7)) => {
                let v = self.value(s);
                // The size only where the value does not already show it:
                // `push d0 (long)`, but `push d0.w` and `push word at a0`.
                if v.contains(" at ") || v.contains(".b") || v.contains(".w") {
                    format!("push {v} onto the stack")
                } else {
                    format!("push {v} ({}) onto the stack", size_word(self.size()))
                }
            }
            (Operand::PostInc(7), d) => {
                let v = self.value(d);
                let extended = if self.i.mnemonic == "movea" && self.size() == Some(Size::Word) {
                    ", sign-extended to a long"
                } else {
                    ""
                };
                format!(
                    "pop a {} off the stack into {}{extended}",
                    size_word(self.size()),
                    the(&v)
                )
            }
            (s, d) => {
                let v = self.value(s);
                let to = self.value(d);
                let extended = if self.i.mnemonic == "movea" && self.size() == Some(Size::Word) {
                    " (sign-extended to a long)"
                } else {
                    ""
                };
                format!("{to} = {v}{extended}")
            }
        }
    }

    fn arith(&mut self, ops: &[Operand], op: &str) -> String {
        match self.pair() {
            Some((s, d)) => {
                let extended = if matches!(ops.get(1), Some(Operand::A(_)))
                    && self.size() == Some(Size::Word)
                {
                    " (sign-extended)"
                } else {
                    ""
                };
                format!("{d} {op} {s}{extended}")
            }
            None => "arithmetic".to_owned(),
        }
    }

    /// AND, OR and EOR. An immediate operand is a bit mask and reads better
    /// as one; the status register gets its interrupt level spelled out.
    fn logic(&mut self, ops: &[Operand], op: &str, words: &str, prep: &str) -> String {
        match (ops.first(), ops.get(1)) {
            (Some(Operand::Imm(v)), Some(Operand::Sr)) => {
                format!("status register {op} ${v:x}{}", interrupt_after(op, *v))
            }
            (Some(Operand::Imm(v)), Some(Operand::Ccr)) => {
                format!("condition codes {op} ${v:x}")
            }
            (Some(Operand::Imm(v)), Some(d)) => {
                let d = self.value(&d.clone());
                format!("{words} ${v:x} {prep} {}", the(&d))
            }
            _ => match self.pair() {
                Some((s, d)) => format!("{d} {op} {s}"),
                None => "logic".to_owned(),
            },
        }
    }

    fn shift(&mut self, ops: &[Operand]) -> String {
        let m = self.i.mnemonic;
        let (count, dst) = match ops {
            [d] => ("1".to_owned(), self.value(&d.clone())),
            [Operand::Imm(n), d] => (n.to_string(), self.value(&d.clone())),
            [Operand::D(r), d] => (format!("{} (mod 64)", reg_name(*r)), self.value(&d.clone())),
            _ => return "shift".to_owned(),
        };
        let bits = if count == "1" { "bit" } else { "bits" };
        let fixed: Option<u32> = count.parse().ok();
        let scale = fixed.map(|n| 1u64 << n.min(31));
        match m {
            "lsl" => format!(
                "shift {dst} left {count} {bits}{}",
                scale.map(|s| format!(" (× {s})")).unwrap_or_default()
            ),
            "asl" => format!(
                "shift {dst} left {count} {bits}, flagging signed overflow{}",
                scale.map(|s| format!(" (× {s})")).unwrap_or_default()
            ),
            "lsr" => format!(
                "shift {dst} right {count} {bits}, filling with zeros{}",
                scale
                    .map(|s| format!(" (unsigned / {s})"))
                    .unwrap_or_default()
            ),
            "asr" => format!(
                "shift {dst} right {count} {bits}, keeping the sign{}",
                scale
                    .map(|s| format!(" (signed / {s})"))
                    .unwrap_or_default()
            ),
            "rol" => format!("rotate {dst} left {count} {bits}"),
            "ror" => format!("rotate {dst} right {count} {bits}"),
            "roxl" => format!("rotate {dst} left {count} {bits} through the X flag"),
            _ => format!("rotate {dst} right {count} {bits} through the X flag"),
        }
    }

    fn bit(&mut self, ops: &[Operand]) -> String {
        let verb = match self.i.mnemonic {
            "btst" => "test",
            "bset" => "test and set",
            "bclr" => "test and clear",
            _ => "test and flip",
        };
        let [which, dst] = ops else {
            return format!("{verb} a bit");
        };
        // A data register has 32 bits and memory is tested a byte at a time.
        let modulus = if matches!(dst, Operand::D(_)) { 32 } else { 8 };
        let (bit, fixed) = match which {
            Operand::Imm(n) => {
                let b = n.checked_rem(modulus).unwrap_or(0);
                (b.to_string(), Some(b))
            }
            Operand::D(r) => (format!("{} mod {modulus}", reg_name(*r)), None),
            _ => ("a bit".to_owned(), None),
        };
        let place = match dst {
            Operand::D(r) => reg_name(*r),
            // BTST can test a bit of a constant.
            Operand::Imm(v) => format!("the value {}", number(i64::from(*v & 0xFF))),
            m => {
                self.effect(m);
                let address = absolute(m);
                let at = self.address(m).unwrap_or_default();
                if let (Some(a), Some(b)) = (address, fixed)
                    && let Some(meaning) = hardware::port_bit(a, b)
                {
                    // The bit's meaning says more than the register's.
                    self.named = None;
                    return format!("{verb} bit {bit} of the byte at {at} — {meaning}");
                }
                format!("the byte at {at}")
            }
        };
        format!("{verb} bit {bit} of {place}")
    }

    fn dbcc(&mut self, ops: &[Operand], cc: &str) -> String {
        let (Some(Operand::D(r)), Some(Operand::Target(t))) = (ops.first(), ops.get(1)) else {
            return "loop".to_owned();
        };
        let d = reg_name(*r);
        let t = self.target(*t);
        if cc == "t" {
            // The condition is always true, so it never loops: a two-word nop.
            format!("never loops ({d} and {t} are ignored): condition always true")
        } else if cc == "f" || cc == "ra" {
            format!("{d}.w -= 1; loop back to {t} until {d}.w reaches -1")
        } else {
            format!(
                "stop looping if {}; otherwise {d}.w -= 1 and loop back to {t} until it reaches -1",
                condition(cc)
            )
        }
    }

    /// MOVEM moves its address register by one size for **every** register
    /// it transfers: `movem.l (a0)+,d1-d2` leaves a0 eight bytes on.
    fn movem_effect(&mut self, o: &Operand, mask: u16) {
        let count = i64::from(mask.count_ones());
        let each: i64 = if self.size() == Some(Size::Word) {
            2
        } else {
            4
        };
        let by = number(each.saturating_mul(count));
        match o {
            Operand::PostInc(r) => self.effects.push(format!("{} += {by}", a_name(*r))),
            Operand::PreDec(r) => self.effects.push(format!("{} -= {by}", a_name(*r))),
            _ => {}
        }
    }

    fn movem(&mut self, ops: &[Operand]) -> String {
        match ops {
            [Operand::RegList(m), Operand::PreDec(7)] => {
                format!("save {} on the stack", list(*m))
            }
            [Operand::PostInc(7), Operand::RegList(m)] => {
                format!("restore {} from the stack", list(*m))
            }
            [Operand::RegList(m), dst] => {
                self.movem_effect(dst, *m);
                let at = self.address(dst).unwrap_or_default();
                format!("store {} at {at} onwards", list(*m))
            }
            [src, Operand::RegList(m)] => {
                self.movem_effect(src, *m);
                let at = self.address(src).unwrap_or_default();
                format!("load {} from {at} onwards", list(*m))
            }
            _ => "move several registers".to_owned(),
        }
    }
}

/// A memory value in a sentence rather than an assignment: "the byte at …".
fn the(value: &str) -> String {
    if ["byte at ", "word at ", "long at "]
        .iter()
        .any(|p| value.starts_with(p))
    {
        format!("the {value}")
    } else {
        value.to_owned()
    }
}

/// A register list, or "no registers" for an empty mask.
fn list(mask: u16) -> String {
    if mask == 0 {
        return "no registers".to_owned();
    }
    Operand::RegList(mask).motorola(&Names::NONE)
}

/// The absolute address an operand names, if it names one.
fn absolute(o: &Operand) -> Option<u32> {
    match o {
        Operand::AbsW(a) => Some(u32::from_ne_bytes(i32::from(*a).to_ne_bytes())),
        Operand::AbsL(a) => Some(*a),
        _ => None,
    }
}

fn offset(base: &str, d: i64) -> String {
    match d {
        0 => base.to_owned(),
        d if d < 0 => format!("{base}{}", number(d)),
        d => format!("{base}+{}", number(d)),
    }
}

/// A data register as used at a size: `d0.w`, or `d0` for a long.
fn sized_reg(r: u8, size: Option<Size>) -> String {
    match size {
        Some(Size::Byte) => format!("{}.b", reg_name(r)),
        Some(Size::Word) => format!("{}.w", reg_name(r)),
        _ => reg_name(r),
    }
}

/// The whole register a multiply or divide writes: `d0`, not `d0.w`.
fn long_reg(sized: &str) -> String {
    sized.split('.').next().unwrap_or(sized).to_owned()
}

/// An immediate at the instruction's size.
fn imm(v: u32, size: Option<Size>) -> String {
    let v = match size {
        Some(Size::Byte) => v & 0xFF,
        Some(Size::Word) => v & 0xFFFF,
        _ => v,
    };
    number(i64::from(v))
}

/// A signed immediate: `moveq`'s byte, sign-extended.
fn imm_signed(o: Option<&Operand>) -> String {
    match o {
        Some(Operand::Imm(v)) => number(i64::from(i32::from_ne_bytes(v.to_ne_bytes()))),
        _ => String::new(),
    }
}

/// What a value written to the status register does to interrupts.
fn interrupt_note(sr: u32, whole: bool) -> String {
    let level = (sr >> 8) & 7;
    let mode = if sr & 0x2000 != 0 {
        "supervisor mode"
    } else {
        "user mode"
    };
    let ints = match level {
        7 => "all interrupts off".to_owned(),
        0 => "all interrupts on".to_owned(),
        n => format!("interrupts below level {} off", n.saturating_add(1)),
    };
    if whole {
        format!(": {mode}, {ints}")
    } else {
        format!(": {ints}")
    }
}

/// What ORing or ANDing a mask into the status register does.
fn interrupt_after(op: &str, mask: u32) -> String {
    // ANDing can clear both the interrupt level and the supervisor bit at
    // once, so every effect is named, not only the first.
    let mut notes = Vec::new();
    match op {
        "|=" if mask & 0x700 == 0x700 => {
            notes.push("raise the interrupt level to 7, all interrupts off");
        }
        "&=" => {
            if mask & 0x700 == 0 {
                notes.push("drop the interrupt level to 0, all interrupts on");
            }
            if mask & 0x2000 == 0 {
                notes.push("leave supervisor mode");
            }
        }
        _ => {}
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!(": {}", notes.join(", "))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, reason = "tests over fixed bytes")]
mod tests {
    use super::*;
    use crate::m68k::{Decoded, decode};

    fn say(bytes: &[u8]) -> String {
        match decode(bytes, 0) {
            Decoded::Instruction(i) => explain(&i, &Names::NONE),
            other => panic!("{bytes:02x?}: {other:?}"),
        }
    }

    #[test]
    fn the_instructions_in_the_screenshot_read_plainly() {
        // From 3D Pool's loader, the listing this was asked for against.
        assert_eq!(say(&[0x41, 0xfa, 0x00, 0x02]), "a0 = the address of $4");
        assert_eq!(
            say(&[0x21, 0xc8, 0x00, 0x20]),
            "long at $20 (privilege violation vector) = a0 — 68000 exception vector"
        );
        assert_eq!(
            say(&[0x00, 0x7c, 0x07, 0x00]),
            "status register |= $700: raise the interrupt level to 7, all interrupts off"
        );
        assert_eq!(
            say(&[0x00, 0x39, 0x00, 0x88, 0x00, 0xbf, 0xd1, 0x00]),
            "set bits $88 in the byte at $bfd100 (CIAB PRB) — CIA-B port B: floppy drive \
             control (motor, select, side, step)"
        );
        assert_eq!(say(&[0x43, 0xf9, 0x00, 0x00, 0x10, 0x00]), "a1 = $1000");
        assert_eq!(say(&[0x30, 0x3c, 0x00, 0x10]), "d0.w = $10");
        assert_eq!(
            say(&[0x32, 0xd8]),
            "word at a1 = word at a0; a0 += 2, a1 += 2"
        );
        assert_eq!(
            say(&[0x51, 0xc8, 0xff, 0xfc]),
            "d0.w -= 1; loop back to $fffffffe until d0.w reaches -1"
        );
        assert_eq!(say(&[0x4e, 0xf9, 0x00, 0x00, 0x10, 0x00]), "jump to $1000");
        assert_eq!(say(&[0xb1, 0xc9]), "compare a0 with a1");
        assert_eq!(
            say(&[0x6d, 0xfa]),
            "branch to $fffffffc if less than (signed)"
        );
        assert_eq!(say(&[0x4e, 0xd0]), "jump to the address in a0");
        assert_eq!(
            say(&[0x4d, 0xf9, 0x00, 0xdf, 0xf0, 0x00]),
            "a6 = $dff000 (the base of the custom chip registers)"
        );
    }

    #[test]
    fn amiga_idioms_say_what_they_are_for() {
        assert_eq!(
            say(&[0x2c, 0x78, 0x00, 0x04]),
            "a6 = long at $4 (ExecBase) — pointer to exec.library, the one fixed address in \
             AmigaOS"
        );
        assert_eq!(
            say(&[0x08, 0x39, 0x00, 0x06, 0x00, 0xbf, 0xe0, 0x01]),
            "test bit 6 of the byte at $bfe001 (CIAA PRA) — /FIR0: port 1 fire button (left \
             mouse button)"
        );
        assert_eq!(
            say(&[0x4e, 0xae, 0xfd, 0xd8]),
            "call the library function at a6-$228 (a6 holds the library's base)"
        );
        assert_eq!(say(&[0x70, 0xff]), "d0 = -1");
        assert_eq!(
            say(&[0x48, 0x54]),
            "push the address in a4 onto the stack",
            "said once, not twice"
        );
        assert_eq!(
            say(&[0x48, 0xe7, 0xff, 0xfe]),
            "save d0-d7/a0-a6 on the stack"
        );
        assert_eq!(
            say(&[0x4c, 0xdf, 0x7f, 0xff]),
            "restore d0-d7/a0-a6 from the stack"
        );
        assert_eq!(say(&[0x2f, 0x00]), "push d0 (long) onto the stack");
        assert_eq!(
            say(&[0x33, 0xfc, 0x7f, 0xff, 0x00, 0xdf, 0xf0, 0x96]),
            "word at $dff096 (DMACON) = $7fff — DMA control: switch DMA channels on and off"
        );
        assert_eq!(say(&[0xe3, 0x88]), "shift d0 left 1 bit (× 2)");
        assert_eq!(
            say(&[0xc0, 0xc1]),
            "d0 = d0.w × d1.w (unsigned, 16 × 16 → 32 bits)"
        );
    }

    #[test]
    fn every_instruction_has_an_explanation_and_none_panics() {
        // Every opcode word with a full tail, so every instruction the decoder
        // can produce is explained at least once.
        let tail = [0x00u8, 0x04, 0x00, 0xdf, 0xf0, 0x96, 0x12, 0x34];
        for op in 0..=u16::MAX {
            let mut bytes = vec![
                u8::try_from(op >> 8).unwrap(),
                u8::try_from(op & 0xFF).unwrap(),
            ];
            bytes.extend_from_slice(&tail);
            if let Decoded::Instruction(i) = decode(&bytes, 0x1000) {
                let words = explain(&i, &Names::NONE);
                assert!(!words.is_empty(), "{op:04x}");
                assert!(!words.contains("()"), "{op:04x}: {words}");
            }
        }
    }
}
