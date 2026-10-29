//! Exporting a program as assembly source that builds back into it (D-021).
//!
//! What the Assembly view lists, written for an assembler rather than a
//! reader: a `SECTION` per hunk, typed `CODE`, `DATA` or `BSS` and suffixed
//! `_C` or `_F` where the hunk asks for chip or fast memory, in Devpac's
//! syntax; a label at every place anything refers to; every relocated
//! address written as the label it points at; each library call the code
//! proves written as `jsr _LVOOpenLibrary(a6)`, with its equate; and
//! everything else as it is.
//!
//! **It must assemble back to the same bytes**, and that is how it is
//! checked: the corpus test assembles every exported program with GNU `as`
//! in its MRI mode, links each section at address zero — where the file's
//! relocated values count from — and requires each hunk's bytes back
//! exactly. Devpac and vasm themselves have not been tried. Some instructions
//! are therefore written as words, with the instruction beside them as a
//! comment: one with a label inside it, which no line could carry; one an
//! assembler would encode otherwise — `move.l #1,d0` comes back as a
//! `moveq`, `0(a0)` as `(a0)`, `cmp.l #n,d0` as `cmpi.l` — and nothing in
//! the source can say "not that one"; one holding bits the 68000 ignores,
//! as a byte immediate's high byte, which no source line can set; and a
//! 68020 addressing mode, which an assembler for the 68000 refuses. A
//! relocated long no line can rebuild — into a hunk the file does not have,
//! or at an odd offset, where every assembler aligns a `dc.l` — is written
//! as the number it holds, and the source's header says how many.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::flow;
use crate::hunk::{self, Executable, Hunk, Kind, Memory};
use crate::m68k::{Decoded, Instruction, Names, Operand, decode};

/// An exported program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Export {
    /// The source.
    pub source: String,
    /// Instructions written as instructions.
    pub instructions: usize,
    /// Instructions written as words, because a label falls inside one or
    /// an assembler would encode it otherwise.
    pub as_words: usize,
    /// Relocated longs written as the numbers they hold, which a rebuilt
    /// program will not relocate: those into hunks the file does not have,
    /// and those at an odd offset, where no assembler puts a long.
    pub unrelocated: usize,
}

/// The label for offset `at` of hunk `k`.
#[must_use]
pub fn label(k: usize, at: u32) -> String {
    format!("h{k}_{at:04x}")
}

/// A label defined from its hunk's start, for one no line of the hunk can
/// carry. An offset past 2^31 is one before the start, as a PC-relative
/// reach or a relocated value can be: `h0_0000-4`, not four thousand million
/// bytes on.
fn equ(k: usize, at: u32) -> String {
    let signed = i32::from_ne_bytes(at.to_ne_bytes());
    let sign = if signed < 0 { '-' } else { '+' };
    format!(
        "{}\tequ\t{}{sign}{}",
        label(k, at),
        label(k, 0),
        signed.unsigned_abs()
    )
}

/// A hunk's bytes, as the file holds them.
fn data<'a>(bytes: &'a [u8], h: &Hunk) -> &'a [u8] {
    bytes
        .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
        .unwrap_or(&[])
}

/// Every place a label goes, by hunk: the start of each, every relocated
/// value's target, and in code every branch and PC-relative target.
fn labels(exe: &Executable, bytes: &[u8]) -> Vec<BTreeSet<u32>> {
    let mut out: Vec<BTreeSet<u32>> = exe.hunks.iter().map(|_| BTreeSet::from([0])).collect();
    for h in &exe.hunks {
        let d = data(bytes, h);
        for r in &h.relocs {
            if let (Ok(v), Some(set)) = (
                ade_endian::u32_at(d, r.offset as usize),
                out.get_mut(r.target as usize),
            ) {
                set.insert(v);
            }
        }
    }
    for (k, h) in exe.hunks.iter().enumerate() {
        if h.kind != Kind::Code {
            continue;
        }
        let d = data(bytes, h);
        let mut at = 0usize;
        while at < d.len() {
            let address = u32::try_from(at).unwrap_or(u32::MAX);
            let decoded = decode(d.get(at..).unwrap_or(&[]), address);
            if let (Decoded::Instruction(i), Some(set)) = (&decoded, out.get_mut(k)) {
                for t in i.targets() {
                    if (t as usize) < d.len() {
                        set.insert(t);
                    }
                }
            }
            at = at.saturating_add(usize::try_from(decoded.len()).unwrap_or(2));
        }
    }
    out
}

/// Whether an assembler given this instruction as text would write other
/// bytes: the forms GNU `as` and the Amiga assemblers choose for themselves,
/// and the bits the 68000 ignores, which no source line can set. `raw` is the
/// instruction's bytes.
fn reencoded(i: &Instruction, raw: &[u8]) -> bool {
    let imm = match i.operands.first() {
        Some(Operand::Imm(v)) => Some(*v),
        _ => None,
    };
    let to_data = matches!(i.operands.get(1), Some(Operand::D(_)));
    let shortened = i.operands.iter().any(|o| match o {
        // A scaled index is a 68020's; an assembler for the 68000 refuses it.
        Operand::Indexed(_, _, x) | Operand::PcIndexed(_, x) => x.scale != 1,
        // `0(an)` is written `(an)`, a word shorter.
        Operand::Disp(0, _) => true,
        _ => false,
    });
    // A byte immediate fills a word, whose high byte the 68000 ignores and
    // an assembler writes as zero. So does a bit number, and a byte for the
    // condition codes. A quick instruction keeps its number in the opcode
    // word, and has no such word.
    let quick = matches!(
        i.mnemonic,
        "addq" | "subq" | "moveq" | "asl" | "asr" | "lsl" | "lsr" | "rol" | "ror" | "roxl" | "roxr"
    );
    let byte = !quick
        && i.operands.iter().any(|o| matches!(o, Operand::Imm(_)))
        && (i.size.is_some_and(|s| s.suffix() == "b")
            || matches!(i.mnemonic, "btst" | "bchg" | "bclr" | "bset")
            || matches!(i.operands.get(1), Some(Operand::Ccr)));
    if shortened || (byte && raw.get(2).is_some_and(|&b| b != 0)) {
        return true;
    }
    match (i.mnemonic, i.size.map(crate::m68k::Size::suffix)) {
        // `move.l #n,dn` with n a byte becomes `moveq`.
        ("move", Some("l")) if to_data => imm.is_some_and(|v| {
            let v = i32::from_ne_bytes(v.to_ne_bytes());
            (-128..=127).contains(&v)
        }),
        // An immediate to `add`, `sub`, `cmp`, `and` or `or` becomes the
        // immediate form, `cmp.l #n,d0` assembled as `cmpi.l` — or the quick
        // one, `add.l #1,d0` as `addq`. `adda` and `suba` are kept.
        ("add" | "sub" | "cmp" | "and" | "or", _) => imm.is_some(),
        _ => false,
    }
}

impl Export {
    fn line(&mut self, text: &str) {
        self.source.push_str(text);
        self.source.push('\n');
    }
}

/// Bytes `from..to` of hunk `k` as data: `dc.l` for a relocated long, words
/// where the run is word-aligned, bytes otherwise, and a label line wherever
/// one falls.
fn data_run(
    out: &mut Export,
    d: &[u8],
    (k, from, to): (usize, usize, usize),
    labels: &BTreeSet<u32>,
    relocs: &HashMap<u32, (usize, u32)>,
    words: bool,
) {
    let mut at = from;
    let mut pending: Vec<String> = Vec::new();
    let flush = |out: &mut Export, pending: &mut Vec<String>, kind: &str| {
        if !pending.is_empty() {
            out.line(&format!("\t{kind}\t{}", pending.join(",")));
            pending.clear();
        }
    };
    let unit = if words { "dc.w" } else { "dc.b" };
    while at < to {
        let pos = u32::try_from(at).unwrap_or(u32::MAX);
        if at != from && labels.contains(&pos) {
            flush(out, &mut pending, unit);
            out.line(&format!("{}:", label(k, pos)));
        }
        if let Some((target, value)) = relocs.get(&pos)
            && at.saturating_add(4) <= to
            && at % 2 == 1
        {
            flush(out, &mut pending, unit);
            out.line(&format!(
                "\t; relocated, to {}: an odd offset no dc.l can sit at",
                label(*target, *value)
            ));
        }
        if let Some((target, value)) = relocs.get(&pos)
            && at.saturating_add(4) <= to
            && at % 2 == 0
        {
            flush(out, &mut pending, unit);
            // A label inside the long is defined from the hunk's start, so
            // the long can stay one relocated value.
            for inner in labels.range(pos.saturating_add(1)..pos.saturating_add(4)) {
                out.line(&equ(k, *inner));
            }
            out.line(&format!("\tdc.l\t{}", label(*target, *value)));
            at = at.saturating_add(4);
            continue;
        }
        // A word starts on an even offset, as an assembler aligns one, and
        // may not straddle a label or a relocated long.
        let next = pos.saturating_add(1);
        let step = if words
            && at % 2 == 0
            && at.saturating_add(2) <= to
            && !labels.contains(&next)
            && !relocs.contains_key(&next)
        {
            2
        } else {
            1
        };
        let value = if step == 2 {
            format!("${:04x}", ade_endian::u16_at(d, at).unwrap_or(0))
        } else {
            format!("${:02x}", d.get(at).copied().unwrap_or(0))
        };
        if step == 1 && words {
            flush(out, &mut pending, "dc.w");
            out.line(&format!("\tdc.b\t{value}"));
        } else {
            pending.push(value);
            if pending.len() == 8 {
                flush(out, &mut pending, unit);
            }
        }
        at = at.saturating_add(step);
    }
    flush(out, &mut pending, unit);
}

/// What the walks of [`flow`] say about one code hunk, by offset.
#[derive(Default)]
struct Known {
    /// Library calls: the `_LVO` name to write, if one can be, and what the
    /// call is, for its comment.
    calls: HashMap<u32, (Option<String>, String)>,
    /// Function starts: how many places call each, and whether the program
    /// starts there.
    functions: HashMap<u32, (usize, bool)>,
}

/// The comment above a function.
fn heading(callers: usize, entry: bool) -> String {
    match (entry, callers) {
        (true, _) => "; ---- function: the program starts here".to_owned(),
        (false, 1) => "; ---- function: called from 1 place".to_owned(),
        (false, n) => format!("; ---- function: called from {n} places"),
    }
}

/// One code hunk: instructions, and words where an instruction cannot be.
fn code(
    out: &mut Export,
    d: &[u8],
    k: usize,
    labels: &[BTreeSet<u32>],
    relocs: &HashMap<u32, (usize, u32)>,
    known: &Known,
) {
    let here = labels.get(k).cloned().unwrap_or_default();
    let mut at = 0usize;
    while at < d.len() {
        let address = u32::try_from(at).unwrap_or(u32::MAX);
        if at != 0
            && let Some(&(callers, entry)) = known.functions.get(&address)
        {
            out.line("");
            out.line(&heading(callers, entry));
        }
        if at != 0 && here.contains(&address) {
            out.line(&format!("{}:", label(k, address)));
        }
        let decoded = decode(d.get(at..).unwrap_or(&[]), address);
        let len = usize::try_from(decoded.len())
            .unwrap_or(2)
            .min(d.len().saturating_sub(at))
            .max(1);
        let end = at.saturating_add(len);
        let inside = here
            .range(address.saturating_add(1)..u32::try_from(end).unwrap_or(u32::MAX))
            .next()
            .is_some();
        let text = match &decoded {
            Decoded::Instruction(i) if !inside && !reencoded(i, d.get(at..end).unwrap_or(&[])) => {
                instruction(i, (k, at, end), &here, relocs)
            }
            _ => None,
        };
        // A proved library call is written as its source would be.
        let text = match (text, &decoded, known.calls.get(&address)) {
            (Some(_), Decoded::Instruction(i), Some((Some(lvo), what))) => {
                Some(format!("{:<8}{lvo}(a6)\t; {what}", i.opcode()))
            }
            (Some(t), _, Some((None, what))) => Some(format!("{t}\t; {what}")),
            (t, _, _) => t,
        };
        if let Some(t) = text {
            out.instructions = out.instructions.saturating_add(1);
            out.line(&format!("\t{t}"));
        } else {
            if let Decoded::Instruction(i) = &decoded {
                out.as_words = out.as_words.saturating_add(1);
                out.line(&format!("\t; {}", i.motorola(&Names::NONE)));
            }
            data_run(out, d, (k, at, end), &here, relocs, true);
        }
        at = end;
    }
}

/// An instruction as source, its labels and relocated values named; `None`
/// when one of them cannot be.
fn instruction(
    i: &Instruction,
    (k, at, end): (usize, usize, usize),
    here: &BTreeSet<u32>,
    relocs: &HashMap<u32, (usize, u32)>,
) -> Option<String> {
    // Every place the instruction reaches by position must carry a label.
    // A branch to an odd address is an address error, so this is data.
    if !i.targets().iter().all(|t| here.contains(t))
        || i.operands
            .iter()
            .any(|o| matches!(o, Operand::Target(t) if t % 2 == 1))
    {
        return None;
    }
    let inside: Vec<(usize, u32)> = (at.saturating_add(2)..end)
        .filter_map(|p| relocs.get(&u32::try_from(p).ok()?).copied())
        .collect();
    // A long that holds an address the loader relocates is named by it; an
    // absolute or immediate that is not relocated stays a number.
    let absolutes: Vec<u32> = i
        .operands
        .iter()
        .filter_map(|o| match o {
            Operand::AbsL(v) | Operand::Imm(v) => Some(*v),
            _ => None,
        })
        .collect();
    // Names go by value, so each value must be relocated everywhere the
    // instruction holds it or nowhere, and in one hunk.
    let mut by_value: HashMap<u32, String> = HashMap::new();
    for (t, v) in &inside {
        let held = absolutes.iter().filter(|a| *a == v).count();
        let moved = inside.iter().filter(|(_, w)| w == v).count();
        if held != moved
            || by_value
                .insert(*v, label(*t, *v))
                .is_some_and(|l| l != label(*t, *v))
        {
            return None;
        }
    }
    if inside.len()
        != absolutes
            .iter()
            .filter(|a| by_value.contains_key(a))
            .count()
    {
        return None;
    }
    let position = |t: u32| here.contains(&t).then(|| label(k, t));
    let absolute = |v: u32| by_value.get(&v).cloned();
    let names = Names {
        position: &position,
        absolute: &absolute,
    };
    // A relocated absolute address keeps its `.l`, or an assembler that
    // sees a label in the same section may reach it relative to the PC.
    // Never in a `moveq` or `link`, whose signed immediates want
    // `Instruction::motorola` itself.
    if !i
        .operands
        .iter()
        .any(|o| matches!(o, Operand::AbsL(v) if by_value.contains_key(v)))
    {
        return Some(i.motorola(&names));
    }
    let operands: Vec<String> = i
        .operands
        .iter()
        .map(|o| match o {
            Operand::AbsL(v) if by_value.contains_key(v) => {
                format!("({}).l", o.motorola(&names))
            }
            _ => o.motorola(&names),
        })
        .collect();
    Some(format!("{:<8}{}", i.opcode(), operands.join(",")))
}

/// The `SECTION` line for a hunk, typed and with its memory.
fn section(k: usize, h: &Hunk) -> String {
    let kind = match h.kind {
        Kind::Code => "CODE",
        Kind::Data => "DATA",
        Kind::Bss => "BSS",
    };
    let memory = match h.memory {
        Memory::Chip => "_C",
        Memory::Fast => "_F",
        Memory::Any | Memory::Extended => "",
    };
    format!("\tSECTION\thunk{k},{kind}{memory}")
}

/// What the walks found, by hunk, with the `_LVO` equates the calls use
/// written out: one per name, and a name two libraries give different
/// offsets left as a number, with the call named in its comment.
fn walked(bytes: &[u8], out: &mut Export) -> HashMap<usize, Known> {
    let mut by_hunk: HashMap<usize, Known> = HashMap::new();
    let Some(a) = flow::executable(bytes) else {
        return by_hunk;
    };
    let mut offsets: BTreeMap<&str, BTreeSet<i16>> = BTreeMap::new();
    for c in &a.library_calls {
        if let Some(f) = c.function {
            offsets.entry(f).or_default().insert(c.offset);
        }
    }
    let usable: BTreeMap<&str, i16> = offsets
        .iter()
        .filter(|(_, o)| o.len() == 1)
        .filter_map(|(f, o)| Some((*f, *o.first()?)))
        .collect();
    if !usable.is_empty() {
        out.line("");
        out.line("; Library vector offsets, for the calls whose library the code proves.");
        for (f, o) in &usable {
            out.line(&format!("_LVO{f}\tequ\t{o}"));
        }
    }
    for c in &a.library_calls {
        let lvo = c
            .function
            .filter(|f| usable.contains_key(f))
            .map(|f| format!("_LVO{f}"));
        by_hunk
            .entry(c.at.0)
            .or_default()
            .calls
            .insert(c.at.1, (lvo, c.describe()));
    }
    for f in &a.functions {
        by_hunk
            .entry(f.start.0)
            .or_default()
            .functions
            .insert(f.start.1, (f.callers.len(), f.entry));
    }
    by_hunk
}

/// Export an executable, or `None` when it is not one.
#[must_use]
pub fn executable(bytes: &[u8]) -> Option<Export> {
    let exe = hunk::parse(bytes)?;
    let labels = labels(&exe, bytes);
    let mut out = Export::default();
    out.line("; Exported by ADE: source that rebuilds the program's own bytes, a SECTION");
    out.line("; per hunk. Checked with GNU as in MRI mode (-M --register-prefix-optional),");
    out.line("; given each SECTION without its type; written in Devpac's syntax, which");
    out.line("; Devpac and vasm have not been checked against.");
    // A relocation into a hunk the file does not have — an overlay's, or a
    // damaged file's — names nothing, and one at an odd offset cannot be a
    // `dc.l`, which every assembler aligns; each long stays the number it
    // holds.
    let all = || exe.hunks.iter().flat_map(|h| &h.relocs);
    let missing = all()
        .filter(|r| (r.target as usize) >= exe.hunks.len())
        .count();
    let odd = all()
        .filter(|r| (r.target as usize) < exe.hunks.len() && r.offset % 2 == 1)
        .count();
    if missing > 0 {
        out.line(&format!(
            "; {missing} relocated longs point into hunks this file does not have."
        ));
    }
    if odd > 0 {
        out.line(&format!(
            "; {odd} relocated longs sit at odd offsets, where no assembler puts one."
        ));
    }
    if missing > 0 || odd > 0 {
        out.line("; They are written as the numbers they hold, and a rebuilt program will not");
        out.line("; relocate them.");
    }
    out.unrelocated = missing.saturating_add(odd);
    let known = walked(bytes, &mut out);
    for (k, h) in exe.hunks.iter().enumerate() {
        let d = data(bytes, h);
        // This hunk's relocations, by position: the hunk and offset each
        // long points at.
        let relocs: HashMap<u32, (usize, u32)> = h
            .relocs
            .iter()
            .filter(|r| (r.target as usize) < exe.hunks.len())
            .filter_map(|r| {
                let v = ade_endian::u32_at(d, r.offset as usize).ok()?;
                Some((r.offset, (r.target as usize, v)))
            })
            .collect();
        out.line("");
        out.line(&section(k, h));
        if let Some(&(callers, entry)) = known.get(&k).and_then(|kn| kn.functions.get(&0)) {
            out.line(&heading(callers, entry));
        }
        out.line(&format!("{}:", label(k, 0)));
        let here = labels.get(k).cloned().unwrap_or_default();
        match h.kind {
            Kind::Code => code(
                &mut out,
                d,
                k,
                &labels,
                &relocs,
                known.get(&k).unwrap_or(&Known::default()),
            ),
            Kind::Data => data_run(&mut out, d, (k, 0, d.len()), &here, &relocs, false),
            Kind::Bss => {}
        }
        // Memory beyond what the file holds is zero.
        let size = usize::try_from(h.size).unwrap_or(0);
        if size > d.len() {
            out.line(&format!("\tds.b\t{}", size.saturating_sub(d.len())));
        }
        // Labels past the bytes the file holds — into the zeroed memory, or
        // beyond the hunk altogether — count from its start.
        for &at in here.range(u32::try_from(d.len()).unwrap_or(u32::MAX)..) {
            if at != 0 {
                out.line(&equ(k, at));
            }
        }
    }
    Some(out)
}

/// Each hunk's bytes as the file holds them, for comparing an assembled
/// export with.
#[must_use]
pub fn hunk_bytes(bytes: &[u8]) -> Option<BTreeMap<usize, (Kind, Vec<u8>)>> {
    let exe = hunk::parse(bytes)?;
    Some(
        exe.hunks
            .iter()
            .enumerate()
            .map(|(k, h)| (k, (h.kind, data(bytes, h).to_vec())))
            .collect(),
    )
}
