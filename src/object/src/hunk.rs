//! AmigaDOS executables: the hunk format (F-032).
//!
//! An executable is not one run of code. It is a header listing *hunks* —
//! code, data and BSS, each loaded separately wherever memory allows — then
//! each hunk's bytes followed by the relocations that patch addresses into it
//! once the loader knows where everything went, and optionally symbols. A
//! disassembler that ignores this decodes data as instructions and prints
//! every address as the zero the file stores in place of it.
//!
//! Source: the AmigaDOS Manual (3rd ed.) chapter on the binary file
//! structure, and the AmigaOS `dos/doshunks.h` hunk numbers.
//!
//! # Untrusted input (D-006)
//!
//! Every count in the format comes off the disk and sizes a read. Each is
//! checked against the bytes actually remaining before it is used, the hunk
//! table is capped, and a block of a kind ADE does not parse ends the parse
//! with a note rather than a guess at its length.

use ade_endian::{u16_at, u32_at};

/// `HUNK_HEADER`: the four bytes every executable starts with.
pub const HUNK_HEADER: u32 = 0x3F3;

const HUNK_NAME: u32 = 0x3E8;
const HUNK_CODE: u32 = 0x3E9;
const HUNK_DATA: u32 = 0x3EA;
const HUNK_BSS: u32 = 0x3EB;
const HUNK_RELOC32: u32 = 0x3EC;
const HUNK_RELOC16: u32 = 0x3ED;
const HUNK_RELOC8: u32 = 0x3EE;
const HUNK_SYMBOL: u32 = 0x3F0;
const HUNK_DEBUG: u32 = 0x3F1;
const HUNK_END: u32 = 0x3F2;
const HUNK_OVERLAY: u32 = 0x3F5;
const HUNK_BREAK: u32 = 0x3F6;
const HUNK_DREL32: u32 = 0x3F7;
const HUNK_DREL16: u32 = 0x3F8;
const HUNK_DREL8: u32 = 0x3F9;
const HUNK_RELOC32SHORT: u32 = 0x3FC;

/// More hunks than any real program has; a table claiming more is damage.
const MAX_HUNKS: u32 = 4096;

/// What a hunk holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Instructions.
    Code,
    /// Initialised data.
    Data,
    /// Zeroed memory: a size and no bytes.
    Bss,
}

impl Kind {
    /// The name an Amiga programmer uses.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Code => "CODE",
            Self::Data => "DATA",
            Self::Bss => "BSS",
        }
    }
}

/// Where a hunk asks to be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memory {
    /// Anywhere.
    Any,
    /// Chip RAM, which the custom chips can reach: graphics, sound.
    Chip,
    /// Fast RAM.
    Fast,
    /// Both flags set: attributes given in a further long.
    Extended,
}

impl Memory {
    const fn from_flags(long: u32) -> Self {
        match long >> 30 {
            0 => Self::Any,
            1 => Self::Chip,
            2 => Self::Fast,
            _ => Self::Extended,
        }
    }
}

/// One address the loader patches: the long at `offset` in this hunk holds
/// an offset into hunk `target`, to which the target's load address is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reloc {
    /// Where in this hunk the address sits.
    pub offset: u32,
    /// Which hunk it points into.
    pub target: u32,
}

/// One hunk of an executable.
#[derive(Debug, Clone)]
pub struct Hunk {
    /// Its number, counted from the header's first hunk.
    pub index: u32,
    /// Code, data or BSS.
    pub kind: Kind,
    /// Where it asks to be loaded.
    pub memory: Memory,
    /// The name `HUNK_NAME` gave it, if any.
    pub name: Option<String>,
    /// Bytes of memory it occupies, from the header table.
    pub size: u32,
    /// Where its bytes start in the file. Zero for BSS.
    pub file_offset: usize,
    /// How many bytes the file holds for it. Zero for BSS.
    pub file_len: usize,
    /// 32-bit relocations into this hunk. The 16- and 8-bit kinds are
    /// skipped: they patch relative offsets that a listing need not resolve.
    pub relocs: Vec<Reloc>,
    /// Symbols, as `(name, offset)`.
    pub symbols: Vec<(String, u32)>,
}

/// A parsed executable.
#[derive(Debug, Clone, Default)]
pub struct Executable {
    /// Every hunk, in order.
    pub hunks: Vec<Hunk>,
    /// What could not be parsed, and where it stopped.
    pub notes: Vec<String>,
}

/// Whether bytes begin like an AmigaDOS executable.
#[must_use]
pub fn is_executable(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == Ok(HUNK_HEADER)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn long(&mut self) -> Option<u32> {
        let v = u32_at(self.bytes, self.pos).ok()?;
        self.pos = self.pos.checked_add(4)?;
        Some(v)
    }

    fn word(&mut self) -> Option<u16> {
        let v = u16_at(self.bytes, self.pos).ok()?;
        self.pos = self.pos.checked_add(2)?;
        Some(v)
    }

    /// Skip `n` longs, if that many remain.
    fn skip_longs(&mut self, n: u32) -> Option<()> {
        let bytes = usize::try_from(n).ok()?.checked_mul(4)?;
        let end = self.pos.checked_add(bytes)?;
        if end > self.bytes.len() {
            return None;
        }
        self.pos = end;
        Some(())
    }

    /// A name of `n` longs, NUL-padded, as Latin-1.
    fn name(&mut self, n: u32) -> Option<String> {
        let start = self.pos;
        self.skip_longs(n)?;
        let raw = self.bytes.get(start..self.pos)?;
        Some(
            raw.iter()
                .take_while(|&&b| b != 0)
                .map(|&b| char::from(b))
                .collect(),
        )
    }
}

/// Parse an executable.
///
/// Returns `None` if the bytes do not begin with a readable header. Damage
/// after that is reported in [`Executable::notes`], with every hunk read
/// before it kept: half a program is still worth showing.
#[must_use]
pub fn parse(bytes: &[u8]) -> Option<Executable> {
    let mut r = Reader { bytes, pos: 0 };
    if r.long()? != HUNK_HEADER {
        return None;
    }
    // Resident library names, a list ended by a zero length. Executables
    // never use them, but the format allows them.
    loop {
        let n = r.long()?;
        if n == 0 {
            break;
        }
        r.skip_longs(n)?;
    }
    let _table_size = r.long()?;
    let first = r.long()?;
    let last = r.long()?;
    let count = last.checked_sub(first)?.checked_add(1)?;
    if count > MAX_HUNKS {
        return None;
    }
    let mut sizes = Vec::new();
    for _ in 0..count {
        let raw = r.long()?;
        let memory = Memory::from_flags(raw);
        if memory == Memory::Extended {
            let _attributes = r.long()?;
        }
        sizes.push(((raw & 0x3FFF_FFFF).checked_mul(4)?, memory));
    }

    let mut exe = Executable::default();
    let mut name: Option<String> = None;
    let mut current: Option<Hunk> = None;
    let mut index = first;
    while r.pos < bytes.len() {
        let Some(block) = r.long() else {
            exe.notes.push(format!(
                "{} stray bytes at the end",
                bytes.len().saturating_sub(r.pos)
            ));
            break;
        };
        let block_type = block & 0x3FFF_FFFF;
        let result = match block_type {
            HUNK_NAME => r.long().and_then(|n| r.name(n)).map(|n| {
                name = Some(n);
            }),
            HUNK_CODE | HUNK_DATA | HUNK_BSS => {
                if let Some(done) = current.take() {
                    exe.hunks.push(done);
                    index = index.saturating_add(1);
                }
                let planned = usize::try_from(index.saturating_sub(first))
                    .ok()
                    .and_then(|i| sizes.get(i).copied());
                start_hunk(&mut r, block, index, planned, name.take()).map(|h| {
                    current = Some(h);
                })
            }
            HUNK_RELOC32 | HUNK_RELOC16 | HUNK_RELOC8 | HUNK_DREL16 | HUNK_DREL8 => {
                let keep = block_type == HUNK_RELOC32;
                long_relocs(&mut r, current.as_mut().filter(|_| keep))
            }
            HUNK_RELOC32SHORT | HUNK_DREL32 => short_relocs(&mut r, current.as_mut()),
            HUNK_SYMBOL => symbols(&mut r, current.as_mut()),
            HUNK_DEBUG => r.long().and_then(|n| r.skip_longs(n)),
            HUNK_END | HUNK_BREAK => Some(()),
            HUNK_OVERLAY => {
                exe.notes
                    .push("an overlay follows; only the root hunks are shown".to_owned());
                break;
            }
            other => {
                exe.notes.push(format!(
                    "stopped at an unrecognised block type {other:#x} at byte {}",
                    r.pos.saturating_sub(4)
                ));
                break;
            }
        };
        if result.is_none() {
            exe.notes.push(format!(
                "a {block_type:#x} block at byte {} runs past the end of the file",
                r.pos
            ));
            break;
        }
    }
    if let Some(done) = current {
        exe.hunks.push(done);
    }
    Some(exe)
}

/// A code, data or BSS block: the start of the next hunk. `planned` is its
/// size and memory from the header table, when the table has an entry for it.
fn start_hunk(
    r: &mut Reader<'_>,
    block: u32,
    index: u32,
    planned: Option<(u32, Memory)>,
    name: Option<String>,
) -> Option<Hunk> {
    let kind = match block & 0x3FFF_FFFF {
        HUNK_CODE => Kind::Code,
        HUNK_DATA => Kind::Data,
        _ => Kind::Bss,
    };
    let longs = r.long()?;
    let (size, memory) = planned.unwrap_or((longs.saturating_mul(4), Memory::from_flags(block)));
    let file_offset = r.pos;
    if kind != Kind::Bss {
        r.skip_longs(longs & 0x3FFF_FFFF)?;
    }
    Some(Hunk {
        index,
        kind,
        memory,
        name,
        size,
        file_offset: if kind == Kind::Bss { 0 } else { file_offset },
        file_len: r.pos.saturating_sub(file_offset),
        relocs: Vec::new(),
        symbols: Vec::new(),
    })
}

/// `HUNK_RELOC32` and its relatives: groups of `count, hunk, offsets…`,
/// ended by a zero count.
fn long_relocs(r: &mut Reader<'_>, mut into: Option<&mut Hunk>) -> Option<()> {
    loop {
        let count = r.long()?;
        if count == 0 {
            return Some(());
        }
        let target = r.long()?;
        // Bounded by what remains before anything is read or kept.
        let needed = usize::try_from(count).ok()?.checked_mul(4)?;
        if r.pos.checked_add(needed)? > r.bytes.len() {
            return None;
        }
        for _ in 0..count {
            let offset = r.long()?;
            if let Some(h) = into.as_deref_mut() {
                h.relocs.push(Reloc { offset, target });
            }
        }
    }
}

/// The 16-bit form, `HUNK_RELOC32SHORT`: words instead of longs, then padding
/// to a long boundary.
fn short_relocs(r: &mut Reader<'_>, mut into: Option<&mut Hunk>) -> Option<()> {
    loop {
        let count = r.word()?;
        if count == 0 {
            break;
        }
        let target = r.word()?;
        for _ in 0..count {
            let offset = r.word()?;
            if let Some(h) = into.as_deref_mut() {
                h.relocs.push(Reloc {
                    offset: u32::from(offset),
                    target: u32::from(target),
                });
            }
        }
    }
    if r.pos % 4 != 0 {
        r.word()?;
    }
    Some(())
}

/// `HUNK_SYMBOL`: `length, name, value` triples ended by a zero length.
fn symbols(r: &mut Reader<'_>, mut into: Option<&mut Hunk>) -> Option<()> {
    loop {
        let n = r.long()?;
        if n == 0 {
            return Some(());
        }
        let name = r.name(n & 0x00FF_FFFF)?;
        let value = r.long()?;
        if let Some(h) = into.as_deref_mut() {
            h.symbols.push((name, value));
        }
    }
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
    fn a_two_hunk_program_parses_with_its_relocation_and_symbol() {
        let bytes = longs(&[
            HUNK_HEADER,
            0,
            2,
            0,
            1,
            2,
            0x4000_0001, // code 8 bytes, data 4 bytes in chip
            HUNK_CODE,
            2,
            0x41F9_0000,
            0x0000_4E75, // lea (h1+0).l,a0 ; rts
            HUNK_RELOC32,
            1,
            1,
            2,
            0,
            HUNK_SYMBOL,
            1,
            0x6D61_696E,
            0,
            0, // "main" at 0
            HUNK_END,
            HUNK_DATA,
            1,
            0x1234_5678,
            HUNK_END,
        ]);
        let exe = parse(&bytes).unwrap();
        assert!(exe.notes.is_empty(), "{:?}", exe.notes);
        assert_eq!(exe.hunks.len(), 2);
        let code = &exe.hunks[0];
        assert_eq!((code.kind, code.size, code.file_len), (Kind::Code, 8, 8));
        assert_eq!(
            code.relocs,
            vec![Reloc {
                offset: 2,
                target: 1
            }]
        );
        assert_eq!(code.symbols, vec![("main".to_owned(), 0)]);
        let data = &exe.hunks[1];
        assert_eq!(
            (data.kind, data.memory, data.index),
            (Kind::Data, Memory::Chip, 1)
        );
    }

    #[test]
    fn a_count_that_runs_off_the_end_is_a_note_not_a_read() {
        let bytes = longs(&[HUNK_HEADER, 0, 1, 0, 0, 1, HUNK_CODE, 0x00FF_FFFF]);
        let exe = parse(&bytes).unwrap();
        assert!(exe.hunks.is_empty());
        assert_eq!(exe.notes.len(), 1);
        // And a hunk table claiming a billion hunks is not believed at all.
        assert!(parse(&longs(&[HUNK_HEADER, 0, 0, 0, 0x3FFF_FFFF])).is_none());
    }
}
