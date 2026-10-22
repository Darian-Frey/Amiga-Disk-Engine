//! A disassembly listing: what the Assembly view shows (F-032).
//!
//! Three sources, and the difference between them is how much is known about
//! where the code is:
//!
//! - **An executable.** The hunk header says which hunks are code. Only those
//!   are decoded; data hunks are shown as data and BSS as a size, symbols
//!   label their offsets, and every relocated address is shown as the hunk
//!   and offset it points at rather than as the zero the file stores.
//! - **A bootblock.** Boot code starts at byte 12, after the dostype, the
//!   checksum and the rootblock pointer, and runs to the end of block 1.
//! - **Anything else, when asked.** Decoded from its first byte, and headed as
//!   a guess: 68000 instructions are 2–10 bytes long, so decoding from the
//!   wrong place yields plausible nonsense, and only the person asking knows
//!   whether this is code.
//!
//! Bounded (D-006): a listing stops at [`MAX_LINES`] and says so.

use std::collections::HashMap;

use crate::hunk::{self, Executable, Hunk, Kind, Memory};
use crate::m68k::{Decoded, decode};

/// Lines a listing may hold before it stops: about a megabyte of code.
pub const MAX_LINES: usize = 400_000;

/// Bytes of boot code before the bootblock's code begins.
pub const BOOT_CODE_START: usize = 12;

/// What a line is, so a view can style it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A section heading: a hunk, the boot code.
    Heading,
    /// A symbol or label naming the next line's address.
    Label,
    /// An instruction.
    Code,
    /// Bytes shown as data: a data hunk, or a word that is no instruction.
    Data,
    /// Something said about the listing: a guess, a limit, damage.
    Note,
}

/// One line of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What it is.
    pub kind: LineKind,
    /// Where its bytes are, counted from the start of what was listed — the
    /// file, or the bootblock. `None` for a line that stands for no bytes.
    pub offset: Option<u64>,
    /// How many bytes it stands for.
    pub length: u32,
    /// The text, columns included.
    pub text: String,
}

/// A listing and how it was made.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// The lines.
    pub lines: Vec<Line>,
    /// Whether it stopped at [`MAX_LINES`].
    pub truncated: bool,
}

impl Listing {
    fn push(&mut self, kind: LineKind, offset: Option<u64>, length: u32, text: String) -> bool {
        if self.lines.len() >= MAX_LINES {
            if !self.truncated {
                self.truncated = true;
                self.lines.push(Line {
                    kind: LineKind::Note,
                    offset: None,
                    length: 0,
                    text: format!("; stopped after {MAX_LINES} lines"),
                });
            }
            return false;
        }
        self.lines.push(Line {
            kind,
            offset,
            length,
            text,
        });
        true
    }

    fn note(&mut self, text: impl Into<String>) {
        let _ = self.push(LineKind::Note, None, 0, format!("; {}", text.into()));
    }

    fn blank(&mut self) {
        let _ = self.push(LineKind::Note, None, 0, String::new());
    }
}

/// The bytes as hex, a word at a time, padded to a column.
fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    for (n, b) in bytes.iter().enumerate() {
        if n > 0 && n % 2 == 0 {
            out.push(' ');
        }
        let _ = write!(out, "{b:02x}");
    }
    format!("{out:<24}")
}

/// Decode `code`, which sits at `base` in whatever was listed and whose first
/// byte is address `origin` as the program sees it.
fn decode_run(
    out: &mut Listing,
    code: &[u8],
    base: u64,
    origin: u32,
    labels: &HashMap<u32, String>,
    relocs: &HashMap<u32, String>,
) {
    // A first pass for where instructions start and what they point at, so a
    // branch target can be labelled where the listing reaches it. Only a
    // target that is an instruction start gets one: a label in the middle of
    // an instruction could never be printed, and naming it would mislead.
    let mut starts = std::collections::HashSet::new();
    let mut wanted = Vec::new();
    let mut at = 0usize;
    while at < code.len() {
        let address = origin.wrapping_add(u32::try_from(at).unwrap_or(0));
        starts.insert(address);
        let decoded = decode(code.get(at..).unwrap_or(&[]), address);
        if let Decoded::Instruction(i) = &decoded {
            wanted.extend(i.targets());
        }
        at = at.saturating_add(usize::try_from(decoded.len()).unwrap_or(2));
    }
    let mut labels = labels.clone();
    for t in wanted {
        if starts.contains(&t) {
            labels.entry(t).or_insert_with(|| format!("lab_{t:04x}"));
        }
    }
    let labels = &labels;

    let mut at = 0usize;
    while at < code.len() {
        let address = origin.wrapping_add(u32::try_from(at).unwrap_or(0));
        if let Some(name) = labels.get(&address)
            && !out.push(LineKind::Label, None, 0, format!("{name}:"))
        {
            return;
        }
        let rest = code.get(at..).unwrap_or(&[]);
        let decoded = decode(rest, address);
        let len = usize::try_from(decoded.len())
            .unwrap_or(2)
            .min(rest.len())
            .max(1);
        let bytes = rest.get(..len).unwrap_or(&[]);
        let offset = Some(base.saturating_add(at as u64));
        let (kind, text) = match &decoded {
            Decoded::Instruction(i) => {
                // A relocated long inside this instruction is an address the
                // loader fills in; name it by what it points at.
                let span: HashMap<u32, &String> = (2..len)
                    .filter_map(|k| {
                        let pos = address.wrapping_add(u32::try_from(k).ok()?);
                        let name = relocs.get(&pos)?;
                        let value = ade_endian::u32_at(code, at.checked_add(k)?).ok()?;
                        Some((value, name))
                    })
                    .collect();
                let label = |v: u32| -> Option<String> {
                    if let Some(prefix) = span.get(&v) {
                        return Some(format!("{prefix}+${v:x}"));
                    }
                    labels.get(&v).cloned()
                };
                (LineKind::Code, i.motorola(&label))
            }
            Decoded::Needs68020(w) => (
                LineKind::Data,
                format!(
                    "{:<8}${w:04x}        ; 68020 addressing mode: not a 68000 instruction",
                    "dc.w"
                ),
            ),
            Decoded::Invalid(w) => (
                LineKind::Data,
                format!("{:<8}${w:04x}        ; not a 68000 instruction", "dc.w"),
            ),
            Decoded::Truncated => (LineKind::Data, format!("{:<8}{}", "dc.b", byte_list(bytes))),
        };
        if !out.push(
            kind,
            offset,
            u32::try_from(len).unwrap_or(2),
            format!("{address:08x}  {}{text}", hex(bytes)),
        ) {
            return;
        }
        if let Decoded::Instruction(i) = &decoded
            && i.ends_flow()
        {
            out.blank();
        }
        at = at.saturating_add(len);
    }
}

fn byte_list(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("${b:02x}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Data shown as `dc.b`, sixteen bytes a line, with the printable ones beside.
fn data_run(out: &mut Listing, data: &[u8], base: u64) {
    for (n, chunk) in data.chunks(16).enumerate() {
        let at = n.saturating_mul(16);
        let text: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7F).contains(&b) {
                    char::from(b)
                } else {
                    '.'
                }
            })
            .collect();
        let line = format!("{at:08x}  {:<8}{:<64} ; {text}", "dc.b", byte_list(chunk));
        if !out.push(
            LineKind::Data,
            Some(base.saturating_add(at as u64)),
            u32::try_from(chunk.len()).unwrap_or(16),
            line,
        ) {
            return;
        }
    }
}

const fn memory_name(m: Memory) -> &'static str {
    match m {
        Memory::Any => "",
        Memory::Chip => ", chip memory",
        Memory::Fast => ", fast memory",
        Memory::Extended => ", extended memory flags",
    }
}

/// List an executable: its code hunks decoded, the rest shown for what it is.
///
/// `None` when the bytes are not an executable at all.
#[must_use]
pub fn executable(bytes: &[u8]) -> Option<Listing> {
    let exe: Executable = hunk::parse(bytes)?;
    let mut out = Listing::default();
    out.note(format!(
        "AmigaDOS executable: {} hunk{}",
        exe.hunks.len(),
        if exe.hunks.len() == 1 { "" } else { "s" }
    ));
    for note in &exe.notes {
        out.note(note.clone());
    }
    for h in &exe.hunks {
        out.blank();
        hunk_listing(&mut out, bytes, h);
        if out.truncated {
            break;
        }
    }
    Some(out)
}

fn hunk_listing(out: &mut Listing, bytes: &[u8], h: &Hunk) {
    let name = h
        .name
        .as_deref()
        .map(|n| format!(" \"{n}\""))
        .unwrap_or_default();
    let _ = out.push(
        LineKind::Heading,
        None,
        0,
        format!(
            "; hunk {}{name}: {}, {} bytes{}",
            h.index,
            h.kind.name(),
            h.size,
            memory_name(h.memory)
        ),
    );
    let data = bytes
        .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
        .unwrap_or(&[]);
    let base = h.file_offset as u64;
    match h.kind {
        Kind::Bss => {
            let _ = out.push(
                LineKind::Data,
                None,
                0,
                format!("{:08x}  {:<24}{:<8}{}", 0, "", "ds.b", h.size),
            );
        }
        Kind::Data => data_run(out, data, base),
        Kind::Code => {
            let labels: HashMap<u32, String> =
                h.symbols.iter().map(|(n, v)| (*v, n.clone())).collect();
            let relocs: HashMap<u32, String> = h
                .relocs
                .iter()
                .map(|r| (r.offset, format!("hunk{}", r.target)))
                .collect();
            decode_run(out, data, base, 0, &labels, &relocs);
        }
    }
}

/// List a bootblock's code. `boot` is the whole bootblock, both blocks.
#[must_use]
pub fn bootblock(boot: &[u8]) -> Listing {
    let mut out = Listing::default();
    let code = boot.get(BOOT_CODE_START..).unwrap_or(&[]);
    if code.iter().all(|&b| b == 0) {
        out.note("the bootblock holds no boot code: it is zeros after the header");
        return out;
    }
    let _ = out.push(
        LineKind::Heading,
        None,
        0,
        "; boot code, from byte 12 of the bootblock".to_owned(),
    );
    out.note("AmigaDOS calls this with a6 = ExecBase and a1 = the trackdisk IORequest");
    out.blank();
    decode_run(
        &mut out,
        code,
        BOOT_CODE_START as u64,
        u32::try_from(BOOT_CODE_START).unwrap_or(12),
        &HashMap::new(),
        &HashMap::new(),
    );
    out
}

/// List arbitrary bytes as code from their first byte — a guess, and headed
/// as one.
#[must_use]
pub fn raw(bytes: &[u8]) -> Listing {
    let mut out = Listing::default();
    out.note("not an executable: decoded from the first byte as if it were code.");
    out.note("If it is not code, or the code starts elsewhere, what follows is noise.");
    out.blank();
    decode_run(&mut out, bytes, 0, 0, &HashMap::new(), &HashMap::new());
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests"
)]
mod tests {
    use super::*;

    fn longs(values: &[u32]) -> Vec<u8> {
        let mut out = vec![0u8; values.len() * 4];
        for (i, v) in values.iter().enumerate() {
            ade_endian::put_u32(&mut out, i * 4, *v).unwrap();
        }
        out
    }

    #[test]
    fn a_relocated_address_is_shown_as_where_it_points() {
        let bytes = longs(&[
            0x3F3,
            0,
            2,
            0,
            1,
            2,
            1,
            0x3E9,
            2,
            0x41F9_0000,
            0x0010_4E75, // lea (h1+$10).l,a0 ; rts
            0x3EC,
            1,
            1,
            2,
            0,
            0x3F0,
            1,
            0x6D61_696E,
            0,
            0, // main at 0
            0x3F2,
            0x3EA,
            1,
            0x4869_2100, // "Hi!\0"
            0x3F2,
        ]);
        let listing = executable(&bytes).unwrap();
        let text: Vec<&str> = listing.lines.iter().map(|l| l.text.as_str()).collect();
        let all = text.join("\n");
        assert!(all.contains("main:"), "{all}");
        assert!(all.contains("lea     hunk1+$10,a0"), "{all}");
        assert!(all.contains("; hunk 1: DATA, 4 bytes"), "{all}");
        assert!(all.contains("Hi!."), "{all}");
        // The code line points at the file bytes it came from.
        let lea = listing
            .lines
            .iter()
            .find(|l| l.text.contains("lea"))
            .unwrap();
        assert_eq!((lea.offset, lea.length), (Some(36), 6));
    }

    #[test]
    fn an_empty_bootblock_says_so_and_a_guess_says_so() {
        assert!(
            bootblock(&[0u8; 1024]).lines[0]
                .text
                .contains("no boot code")
        );
        assert!(
            raw(&[0x4e, 0x75]).lines[0]
                .text
                .contains("not an executable")
        );
    }

    #[test]
    fn a_listing_is_bounded() {
        let listing = raw(&vec![0x4e; 2 * MAX_LINES + 100]);
        assert!(listing.truncated);
        assert!(listing.lines.len() <= MAX_LINES + 1);
    }
}
