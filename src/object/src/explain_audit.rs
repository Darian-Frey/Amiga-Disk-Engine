//! Every explanation checked against the instruction it explains (F-032).
//!
//! No program can say whether an English sentence about a 68000 instruction is
//! right, so there is no oracle here as there is for the decoder (objdump,
//! D-019). What there is instead is the decoded instruction itself, and a set
//! of things any correct explanation of it must say: the registers it uses,
//! the size it moves, how far a `(a0)+` moves `a0`, the condition a branch
//! tests, the operation it performs. These are checked on **every opcode
//! word**, three times over with different extension words, so a mistake in a
//! whole class of instruction cannot hide behind the examples a person thought
//! to try.
//!
//! What this cannot check is whether the sentence is well chosen — that is
//! the reviewed table in `tests/m68k_explanations.txt`.

use std::collections::BTreeSet;

use crate::explain::explain;
use crate::hardware;
use crate::m68k::{Decoded, Instruction, Names, Operand, Size, a_name, decode, number, reg_name};

/// Extension words: small positive values; negative ones and an address in
/// the custom chips; and a CIA port with a bit number that means something.
const TAILS: [[u8; 8]; 3] = [
    [0x00, 0x04, 0x00, 0x08, 0x00, 0x0c, 0x00, 0x10],
    [0xff, 0xfc, 0x00, 0xdf, 0xf0, 0x96, 0x80, 0x00],
    [0x00, 0x06, 0x00, 0xbf, 0xe0, 0x01, 0x00, 0x02],
];

/// The explanations a mnemonic that is not specifically handled would get.
/// None may appear: each would mean an instruction explained by its name.
const FALLBACKS: [&str; 20] = [
    "load an address",
    "arithmetic",
    "logic",
    "shift",
    "multiply",
    "divide",
    "compare",
    "move",
    "sign-extend",
    "exchange two registers",
    "bounds check",
    "move to a peripheral",
    "decimal arithmetic",
    "loop",
    "move several registers",
    "make a stack frame",
    "remove a stack frame",
    "halt until an interrupt",
    "raise a TRAP exception",
    "call a subroutine",
];

fn instructions() -> Vec<(u16, [u8; 8], Instruction)> {
    let mut out = Vec::new();
    for tail in TAILS {
        for op in 0..=u16::MAX {
            let mut bytes = vec![
                u8::try_from(op >> 8).unwrap_or(0),
                u8::try_from(op & 0xFF).unwrap_or(0),
            ];
            bytes.extend_from_slice(&tail);
            if let Decoded::Instruction(i) = decode(&bytes, 0x1000) {
                out.push((op, tail, i));
            }
        }
    }
    out
}

/// Whether `word` appears in `text` as a whole word: `d1` in "d1 = 0", not in
/// "d10" or "a6+d1.w" read as `6+d1`.
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The registers an operand reads or writes, by name.
fn registers(o: &Operand) -> Vec<String> {
    match o {
        Operand::D(r) => vec![reg_name(*r)],
        Operand::A(r)
        | Operand::Ind(r)
        | Operand::PostInc(r)
        | Operand::PreDec(r)
        | Operand::Disp(_, r) => {
            vec![a_name(*r)]
        }
        Operand::Indexed(_, r, x) => vec![a_name(*r), reg_name(x.reg)],
        Operand::PcIndexed(_, x) => vec![reg_name(x.reg)],
        _ => vec![],
    }
}

const fn step(size: Option<Size>, reg: u8) -> i64 {
    match size {
        Some(Size::Byte) if reg == 7 => 2,
        Some(Size::Byte) => 1,
        Some(Size::Word) => 2,
        _ => 4,
    }
}

/// What the operation word of a mnemonic must contain.
fn operation(m: &str) -> &'static [&'static str] {
    match m {
        "add" | "adda" | "addi" | "addq" | "addx" => &["+="],
        "sub" | "suba" | "subi" | "subq" | "subx" => &["-="],
        "and" | "andi" => &["&=", "keep only bits"],
        "or" | "ori" => &["|=", "set bits"],
        "eor" | "eori" => &["^=", "flip bits"],
        "cmp" | "cmpa" | "cmpi" | "cmpm" => &["compare"],
        "clr" | "sf" => &["= 0"],
        "neg" | "negx" => &["= -"],
        "not" => &["invert"],
        "tst" => &["with 0"],
        "tas" => &["top bit"],
        "lsl" | "asl" | "rol" | "roxl" => &["left"],
        "lsr" | "asr" | "ror" | "roxr" => &["right"],
        "mulu" | "muls" => &["×"],
        "divu" | "divs" => &["/="],
        "abcd" | "sbcd" | "nbcd" => &["decimal"],
        "btst" => &["test bit"],
        "bset" => &["test and set"],
        "bclr" => &["test and clear"],
        "bchg" => &["test and flip"],
        "bra" => &["go to"],
        "bsr" | "jsr" => &["call"],
        "jmp" => &["jump to"],
        "rts" | "rte" | "rtr" => &["return"],
        "swap" => &["swap"],
        "exg" => &["exchange"],
        "ext" => &["sign-extend"],
        "lea" | "moveq" => &[" = "],
        "movea" => &[" = ", "pop "],
        "pea" => &["push"],
        "link" | "unlk" => &["stack frame"],
        "trap" => &["TRAP #"],
        "trapv" => &["TRAPV"],
        "chk" => &["CHK"],
        "movem" => &["save ", "restore ", "store ", "load "],
        "movep" => &["every other address"],
        "stop" => &["halt"],
        "reset" => &["reset"],
        "nop" => &["nothing"],
        "illegal" => &["illegal"],
        "move" => &[
            " = ",
            "push ",
            "pop ",
            "status register",
            "condition codes",
            "user stack pointer",
        ],
        // dbt's condition is always true, so it never loops; st and sf
        // (with clr) store a constant.
        "dbt" => &["never loops"],
        "st" => &["= $ff"],
        m if m.starts_with("db") => &["loop back"],
        m if m.starts_with('b') => &["branch to"],
        m if m.starts_with('s') => &["$ff if"],
        _ => &[],
    }
}

/// The phrase a condition code must produce.
fn condition(cc: &str) -> &'static str {
    match cc {
        "hi" => "higher",
        "ls" => "lower or same",
        "cc" => "carry clear",
        "cs" => "carry set",
        "ne" => "not equal",
        "eq" => "equal",
        "vc" => "no overflow",
        "vs" => "overflow",
        "pl" => "positive",
        "mi" => "negative",
        "ge" => "greater or equal",
        "lt" => "less than",
        "gt" => "greater than",
        "le" => "less or equal",
        "t" => "always",
        _ => "never",
    }
}

/// Everything wrong with one explanation, as sentences.
fn problems(i: &Instruction, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let m = i.mnemonic;

    // Said at all, and said once.
    if text.is_empty() || FALLBACKS.contains(&text) || text.starts_with(&format!("{m} (")) {
        out.push("explained by its name only".to_owned());
    }
    for bad in [
        "()",
        "  ",
        "the address the address",
        " at at ",
        "the the ",
        "byte byte",
    ] {
        if text.contains(bad) {
            out.push(format!("contains {bad:?}"));
        }
    }
    // A memory value inside a sentence takes "the": "invert every bit of
    // the byte at a0", not "of byte at a0".
    for word in ["of ", "into ", "with ", "test ", "above ", "than ", "in "] {
        for value in ["byte at ", "word at ", "long at "] {
            if text.contains(&format!(" {word}{value}"))
                || text.starts_with(&format!("{word}{value}"))
            {
                out.push(format!(
                    "says {:?} without \"the\"",
                    format!("{word}{value}")
                ));
            }
        }
    }

    // The operation.
    let ops = operation(m);
    if ops.is_empty() {
        out.push(format!("no rule for mnemonic {m}"));
    } else if !ops.iter().any(|o| text.contains(o)) {
        out.push(format!(
            "does not say what {m} does (expected one of {ops:?})"
        ));
    }

    // The condition a branch, loop or set tests.
    let branch =
        m.starts_with('b') && !matches!(m, "bra" | "bsr" | "btst" | "bset" | "bclr" | "bchg");
    let set = m.starts_with('s')
        && !matches!(
            m,
            "sub" | "suba" | "subi" | "subq" | "subx" | "sbcd" | "swap" | "stop"
        );
    let cc = if m.starts_with("db") {
        m.get(2..)
    } else if branch || set {
        m.get(1..)
    } else {
        None
    };
    if let Some(cc) = cc
        && !matches!(m, "dbf" | "dbra" | "st" | "sf")
        && !text.contains(condition(cc))
    {
        out.push(format!(
            "does not name the condition {cc} ({})",
            condition(cc)
        ));
    }

    // Every register the instruction uses. "The stack" names sp.
    for o in &i.operands {
        for r in registers(o) {
            if !has_word(text, &r) && !(r == "sp" && text.contains("stack")) {
                out.push(format!("does not mention {r}"));
            }
        }
    }
    if let Some(Operand::RegList(mask)) =
        i.operands.iter().find(|o| matches!(o, Operand::RegList(_)))
    {
        let list = Operand::RegList(*mask).motorola(&Names::NONE);
        if *mask != 0 && !text.contains(&list) {
            out.push(format!("does not list the registers {list}"));
        }
    }

    // How far an increment or decrement moves its register. MOVEM moves it
    // by every register it transfers, not by one.
    let count = i
        .operands
        .iter()
        .find_map(|o| match o {
            Operand::RegList(mask) => Some(i64::from(mask.count_ones())),
            _ => None,
        })
        .unwrap_or(1);
    for o in &i.operands {
        let (reg, sign) = match o {
            Operand::PostInc(r) => (*r, "+="),
            Operand::PreDec(r) => (*r, "-="),
            _ => continue,
        };
        // The stack forms are pushes and pops, which say so instead.
        let stack_words = ["push ", "pop ", "on the stack", "from the stack"];
        if reg == 7 && stack_words.iter().any(|w| text.contains(w)) {
            continue;
        }
        let size = i
            .size
            .or_else(|| crate::explain::byte_sized(m).then_some(Size::Byte));
        let by = step(size, reg).saturating_mul(count);
        let want = format!("{} {sign} {}", a_name(reg), number(by));
        if !text.contains(&want) {
            out.push(format!("does not say {want:?}"));
        }
    }

    // The size of what is moved through memory.
    let memory = i.operands.iter().any(|o| {
        matches!(
            o,
            Operand::Ind(_)
                | Operand::PostInc(_)
                | Operand::PreDec(_)
                | Operand::Disp(..)
                | Operand::Indexed(..)
                | Operand::AbsW(_)
                | Operand::AbsL(_)
                | Operand::PcDisp(_)
                | Operand::PcIndexed(..)
        )
    });
    let addressing = matches!(m, "lea" | "pea" | "jmp" | "jsr" | "movem" | "movep");
    if memory && !addressing {
        // Byte-only instructions act on a byte whatever their size field says.
        let size = match i
            .size
            .or_else(|| crate::explain::byte_sized(m).then_some(Size::Byte))
        {
            Some(Size::Byte) => "byte",
            Some(Size::Word) => "word",
            _ => "long",
        };
        let pushed = text.contains("push ") || text.contains("pop ");
        if !text.contains(&format!("{size} at")) && !pushed {
            out.push(format!("does not say it is a {size} in memory"));
        }
    }

    // An immediate's value, as the listing would write it.
    for o in &i.operands {
        let Operand::Imm(v) = o else { continue };
        let shown = match (m, i.size) {
            ("moveq", _) => number(i64::from(i32::from_ne_bytes(v.to_ne_bytes()))),
            // A frame is described by its size; the sign is the direction.
            ("link", _) => number(i64::from(
                i32::from_ne_bytes(v.to_ne_bytes()).unsigned_abs(),
            )),
            ("btst" | "bset" | "bclr" | "bchg", _) => {
                let modulus = if matches!(i.operands.get(1), Some(Operand::D(_))) {
                    32
                } else {
                    8
                };
                (v % modulus).to_string()
            }
            ("trap", _) => format!("#{v}"),
            ("stop", _) => format!("${v:x}"),
            ("lsl" | "lsr" | "asl" | "asr" | "rol" | "ror" | "roxl" | "roxr", _) => v.to_string(),
            (_, Some(Size::Byte)) => number(i64::from(v & 0xFF)),
            (_, Some(Size::Word)) => number(i64::from(v & 0xFFFF)),
            _ => number(i64::from(*v)),
        };
        let as_mask = format!("${v:x}");
        if !has_word(text, shown.trim_start_matches('$'))
            && !text.contains(&shown)
            && !text.contains(&as_mask)
        {
            out.push(format!("does not give the value {shown}"));
        }
    }

    // A named hardware address is named.
    for o in &i.operands {
        let address = match o {
            Operand::AbsW(a) => u32::from_ne_bytes(i32::from(*a).to_ne_bytes()),
            Operand::AbsL(a) => *a,
            _ => continue,
        };
        let named = hardware::name(address).map(|n| n.name);
        let base = hardware::base(address);
        if let Some(name) = named
            && !text.contains(name)
            && !base.is_some_and(|b| text.contains(b))
        {
            out.push(format!("does not name {name} at ${address:x}"));
        }
    }

    // A branch's destination.
    for o in &i.operands {
        if let Operand::Target(t) = o
            && !text.contains(&format!("${t:x}"))
        {
            out.push(format!("does not give the destination ${t:x}"));
        }
    }
    out
}

#[test]
fn every_explanation_agrees_with_its_instruction() {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut mnemonics: BTreeSet<&'static str> = BTreeSet::new();
    for (op, tail, i) in instructions() {
        checked += 1;
        mnemonics.insert(i.mnemonic);
        let text = explain(&i, &Names::NONE);
        for p in problems(&i, &text) {
            if failures.len() < 60 {
                failures.push(format!(
                    "{op:04x} [{}] {}  →  {text:?}: {p}",
                    tail.iter().fold(String::new(), |mut hex, b| {
                        use core::fmt::Write as _;
                        let _ = write!(hex, "{b:02x}");
                        hex
                    }),
                    i.motorola(&Names::NONE)
                ));
            } else {
                break;
            }
        }
    }
    assert!(checked > 130_000, "checked {checked}");
    assert!(mnemonics.len() > 80, "{mnemonics:?}");
    assert!(
        failures.is_empty(),
        "{} problems in {checked} explanations; first {}:\n{}",
        failures.len(),
        failures.len(),
        failures.join("\n")
    );
}

/// The kind of an operand, for grouping instructions into forms.
const fn kind(o: &Operand) -> &'static str {
    match o {
        Operand::D(_) => "Dn",
        Operand::A(_) => "An",
        Operand::Ind(_) => "(An)",
        Operand::PostInc(_) => "(An)+",
        Operand::PreDec(_) => "-(An)",
        Operand::Disp(..) => "d(An)",
        Operand::Indexed(..) => "d(An,Xn)",
        Operand::AbsW(_) => "abs.w",
        Operand::AbsL(_) => "abs.l",
        Operand::PcDisp(_) => "d(pc)",
        Operand::PcIndexed(..) => "d(pc,Xn)",
        Operand::Imm(_) => "#imm",
        Operand::RegList(_) => "list",
        Operand::Ccr => "ccr",
        Operand::Sr => "sr",
        Operand::Usp => "usp",
        Operand::Target(_) => "target",
    }
}

/// One example of every form the decoder produces — an instruction, its
/// size, and the kinds of its operands — with its explanation.
///
/// The stack forms (`-(sp)`, `(sp)+`) are forms of their own, because they
/// are explained differently: as a push or a pop.
fn reference() -> String {
    use core::fmt::Write as _;
    let mut forms: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for (op, _, i) in instructions() {
        let kinds: Vec<String> = i
            .operands
            .iter()
            .map(|o| match o {
                Operand::PostInc(7) => "(sp)+".to_owned(),
                Operand::PreDec(7) => "-(sp)".to_owned(),
                o => kind(o).to_owned(),
            })
            .collect();
        let key = format!("{:<9} {}", i.opcode(), kinds.join(","));
        forms.entry(key).or_insert_with(|| {
            format!(
                "{op:04x}  {:<28}  {}",
                i.motorola(&Names::NONE),
                explain(&i, &Names::NONE)
            )
        });
    }
    let mut out = String::from(
        "# Every 68000 instruction form ADE decodes, with its explanation (F-032).\n\
         # Reviewed against the M68000 Programmer's Reference Manual. Regenerate with\n\
         # ADE_BLESS=1 cargo test -p ade-object --lib explain_audit, and review the diff.\n",
    );
    for (key, line) in &forms {
        let _ = writeln!(out, "{key:<34}{line}");
    }
    out
}

#[test]
fn every_instruction_form_is_explained_as_reviewed() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/m68k_explanations.txt");
    let now = reference();
    if std::env::var("ADE_BLESS").is_ok() {
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let reviewed = std::fs::read_to_string(&path).unwrap_or_default();
    if reviewed == now {
        return;
    }
    let old: Vec<&str> = reviewed.lines().collect();
    let new: Vec<&str> = now.lines().collect();
    let changed: Vec<String> = new
        .iter()
        .filter(|l| !old.contains(l))
        .take(20)
        .map(|l| format!("  now: {l}"))
        .collect();
    panic!(
        "{} instruction explanations differ from the reviewed table ({} lines now, {} reviewed). \
         Review them, then regenerate with ADE_BLESS=1.\n{}",
        new.iter().filter(|l| !old.contains(l)).count(),
        new.len(),
        old.len(),
        changed.join("\n")
    );
}
