//! What a program holds besides its code: its strings and its variables
//! (F-036), told apart by what [`crate::flow`]'s walks found.
//!
//! **A string** is a run of text — printable Latin-1, and the tab, newline,
//! return and escape bytes a console string carries — outside every
//! instruction a walk reached. A place whose address the code takes starts
//! one that runs, as C reads it, to its NUL, so strings may overlap. One the
//! code refers to is listed from two characters, because the reference says
//! it is used; one nothing refers to must look like text on its own — four
//! characters or more, mostly letters, digits and spaces, and either ended
//! by a NUL or six long — or packed data would bury the rest. In a code
//! hunk, whose unreached stretches are mostly code, a string must also end
//! in a NUL and have words in it. Whether it is referred to, and from where,
//! is the point of listing it here rather than in a hex view.
//!
//! **A variable** is a place an instruction the walks reached reads or
//! writes, or — in a data or BSS hunk — takes the address of, unless a
//! string starts there. Only places named by the code itself count: an
//! address relocated into the program, or reached relative to the PC. One
//! reached through a register — `$1c(a4)`, the small-data base compilers
//! keep — is not seen, because what the register holds is not proved.

use std::collections::{BTreeMap, BTreeSet};

use crate::flow::{Analysis, Place, RefKind, Reference};
use crate::hunk::{self, Kind};

/// One stretch of a program's bytes.
struct Region<'a> {
    hunk: usize,
    kind: Kind,
    /// The address its first byte is, as the program sees it.
    origin: u32,
    bytes: &'a [u8],
}

/// One string.
#[derive(Debug, Clone)]
pub struct Text {
    /// Where it starts.
    pub at: Place,
    /// Its bytes, without the NUL.
    pub bytes: Vec<u8>,
    /// Whether a NUL ends it.
    pub terminated: bool,
    /// What refers to it, or into it.
    pub references: Vec<Reference>,
    /// Whether a walk also read it as code: referred to as data and reading
    /// as a name, but in the way of a walk that ran on past something that
    /// does not return — a `trap` whose handler resets the machine. The
    /// listing there is then probably wrong, not the string.
    pub also_code: bool,
}

impl Text {
    /// The string as a quoted literal: Latin-1, with `\n`, `\t`, `\r`,
    /// `\e` and `\x9b` for the bytes a console reads as controls.
    #[must_use]
    pub fn quoted(&self) -> String {
        let mut out = String::from("\"");
        for &b in &self.bytes {
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                0x1B => out.push_str("\\e"),
                0x9B => out.push_str("\\x9b"),
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                _ => out.push(char::from(b)),
            }
        }
        out.push('"');
        out
    }
}

/// One variable.
#[derive(Debug, Clone)]
pub struct Variable {
    /// Where it is.
    pub at: Place,
    /// The kind of hunk it is in.
    pub kind: Kind,
    /// The sizes it is read and written at, in bytes.
    pub widths: BTreeSet<u8>,
    /// Instructions that read it.
    pub reads: usize,
    /// Instructions that write it.
    pub writes: usize,
    /// Instructions that take its address, and data longs that hold it.
    pub addresses: usize,
    /// Every reference to it.
    pub references: Vec<Reference>,
}

/// A byte that can be part of a string.
const fn texty(b: u8) -> bool {
    matches!(b, 0x20..=0x7E | 0xA0..=0xFF | b'\t' | b'\n' | b'\r' | 0x1B | 0x9B)
}

/// Whether a run reads as text at all: no more than a quarter of it above
/// ASCII, which Latin-1's letters and `©` are, and machine code mostly is.
fn readable(bytes: &[u8]) -> bool {
    let high = bytes.iter().filter(|&&b| b >= 0x80).count();
    high.saturating_mul(4) <= bytes.len()
}

/// Whether text nothing refers to looks enough like text to list: four
/// characters, seven in ten of them letters, digits or spaces, and a NUL
/// after it or six of them.
fn plausible(bytes: &[u8], terminated: bool) -> bool {
    let words = bytes
        .iter()
        .filter(|b| b.is_ascii_alphanumeric() || **b == b' ')
        .count();
    bytes.len() >= 4
        && words.saturating_mul(10) >= bytes.len().saturating_mul(7)
        && (terminated || bytes.len() >= 6)
}

/// Whether text in a code hunk is more than code that reads as letters.
/// The stretches no walk reached there are mostly code — `NuNq` is `rts`
/// then `nop`, `Nu~` an `rts` whose address a table holds — so a string
/// there must end in a NUL, hold at most one byte above ASCII (a `©`), and
/// have words in it: three lower-case letters running, or a space between
/// two runs of letters — a space alone is a common byte of code. Short and
/// referred to, letters and digits alone will do: `OK`.
fn wordlike(bytes: &[u8], terminated: bool, referred: bool) -> bool {
    let lower = bytes
        .split(|b| !b.is_ascii_lowercase())
        .any(|w| w.len() >= 3);
    let words = bytes
        .split(|b| !b.is_ascii_alphabetic())
        .filter(|w| w.len() >= 2)
        .count();
    let spaced = bytes.contains(&b' ') && words >= 2;
    let short = referred && bytes.len() <= 4 && bytes.iter().all(u8::is_ascii_alphanumeric);
    let high = bytes.iter().filter(|&&b| b >= 0x80).count();
    terminated && high <= 1 && (lower || spaced || short)
}

/// The references to places in `from..to` of hunk `k`.
fn references_into(a: &Analysis, k: usize, from: u32, to: u32) -> Vec<Reference> {
    let lo = a.references.partition_point(|r| r.to < (k, from));
    let hi = a.references.partition_point(|r| r.to < (k, to));
    a.references.get(lo..hi).unwrap_or(&[]).to_vec()
}

/// Whether `place` is a byte of an instruction a walk reached: the nearest
/// instruction starting at or before it runs over it.
fn in_code(a: &Analysis, (k, at): Place) -> bool {
    a.reached
        .range((k, 0)..=(k, at))
        .next_back()
        .is_some_and(|(&(_, start), &len)| at < start.saturating_add(len))
}

fn strings_in(regions: &[Region<'_>], a: &Analysis) -> Vec<Text> {
    let mut out = Vec::new();
    for r in regions {
        if r.kind == Kind::Bss {
            continue;
        }
        let place = |j: usize| {
            r.origin
                .saturating_add(u32::try_from(j).unwrap_or(u32::MAX))
        };
        let free =
            |j: usize| r.bytes.get(j).is_some_and(|&b| texty(b)) && !in_code(a, (r.hunk, place(j)));
        let mut i = 0usize;
        while i < r.bytes.len() {
            if !free(i) {
                i = i.saturating_add(1);
                continue;
            }
            let mut j = i;
            while free(j) {
                j = j.saturating_add(1);
            }
            // A place whose address the code takes is where a string starts,
            // and like a C string it runs to the NUL: `graphics.library` and
            // the `.library` inside it can both be strings. Text before the
            // first such place is a string of its own, ending there — or an
            // unreferenced `rts` reading as `Nu` would glue itself on. A read
            // or a write is not a start: programs reuse a name's bytes as
            // storage once the library is open.
            let mut starts: Vec<usize> = references_into(a, r.hunk, place(i), place(j))
                .iter()
                .filter(|x| matches!(x.kind, RefKind::Address | RefKind::Pointer))
                .filter_map(|x| usize::try_from(x.to.1.checked_sub(r.origin)?).ok())
                .filter(|&c| c >= i && c < j)
                .collect();
            starts.sort_unstable();
            starts.dedup();
            let mut pieces: Vec<(usize, usize)> = Vec::new();
            let lead = starts.first().copied().unwrap_or(j);
            if lead > i {
                pieces.push((i, lead));
            }
            pieces.extend(starts.iter().map(|&s| (s, j)));
            for (from, to) in pieces {
                let bytes = r.bytes.get(from..to).unwrap_or(&[]).to_vec();
                let terminated = r.bytes.get(to) == Some(&0);
                let references = references_into(a, r.hunk, place(from), place(to));
                let referred = !references.is_empty();
                let listed = readable(&bytes)
                    && ((referred && bytes.len() >= 2) || plausible(&bytes, terminated))
                    && (r.kind != Kind::Code || wordlike(&bytes, terminated, referred));
                if listed {
                    out.push(Text {
                        at: (r.hunk, place(from)),
                        bytes,
                        terminated,
                        references,
                        also_code: false,
                    });
                }
            }
            i = j;
        }
        // Names the code points at as data that a walk ran through — at
        // their start, or part way along, where it branched into them.
        for x in a.references.iter().filter(|x| {
            x.to.0 == r.hunk
                && matches!(x.kind, RefKind::Address | RefKind::Read | RefKind::Pointer)
        }) {
            let Some(start) =
                x.to.1
                    .checked_sub(r.origin)
                    .and_then(|o| usize::try_from(o).ok())
            else {
                continue;
            };
            let rest = r.bytes.get(start..).unwrap_or(&[]);
            let len = rest.iter().take_while(|&&b| texty(b)).count();
            let bytes = rest.get(..len).unwrap_or(&[]).to_vec();
            let terminated = rest.get(len) == Some(&0);
            let end = x.to.1.saturating_add(u32::try_from(len).unwrap_or(0));
            let ran_through = (x.to.1..end).any(|p| in_code(a, (r.hunk, p)));
            if ran_through
                && bytes.len() >= 4
                && readable(&bytes)
                && wordlike(&bytes, terminated, true)
                && !out.iter().any(|t: &Text| t.at == x.to)
            {
                out.push(Text {
                    at: x.to,
                    bytes,
                    terminated,
                    references: references_into(a, r.hunk, x.to.1, end),
                    also_code: true,
                });
            }
        }
    }
    out.sort_by_key(|t| t.at);
    out
}

fn variables_in(regions: &[Region<'_>], a: &Analysis, strings: &[Text]) -> Vec<Variable> {
    let kinds: BTreeMap<usize, Kind> = regions.iter().map(|r| (r.hunk, r.kind)).collect();
    let texts: BTreeSet<Place> = strings.iter().map(|t| t.at).collect();
    let mut by_place: BTreeMap<Place, Variable> = BTreeMap::new();
    for r in &a.references {
        let Some(&kind) = kinds.get(&r.to.0) else {
            continue;
        };
        // Code is not a variable — unless nothing reached it as code, as a
        // program keeping its variables in its code hunk does.
        if in_code(a, r.to) {
            continue;
        }
        let counts = match r.kind {
            RefKind::Read | RefKind::Write => true,
            RefKind::Address | RefKind::Pointer => kind != Kind::Code && !texts.contains(&r.to),
            RefKind::Call | RefKind::Jump | RefKind::Branch => false,
        };
        if !counts {
            continue;
        }
        let v = by_place.entry(r.to).or_insert_with(|| Variable {
            at: r.to,
            kind,
            widths: BTreeSet::new(),
            reads: 0,
            writes: 0,
            addresses: 0,
            references: Vec::new(),
        });
        match r.kind {
            RefKind::Read => v.reads = v.reads.saturating_add(1),
            RefKind::Write => v.writes = v.writes.saturating_add(1),
            _ => v.addresses = v.addresses.saturating_add(1),
        }
        v.widths.extend(r.width);
        v.references.push(*r);
    }
    // A place only addressed, never read or written by name, in a code hunk
    // was filtered above; the rest stand.
    by_place.into_values().collect()
}

fn executable_regions(bytes: &[u8]) -> Option<Vec<Region<'_>>> {
    let exe = hunk::parse(bytes)?;
    Some(
        exe.hunks
            .iter()
            .enumerate()
            .map(|(k, h)| Region {
                hunk: k,
                kind: h.kind,
                origin: 0,
                bytes: bytes
                    .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
                    .unwrap_or(&[]),
            })
            .collect(),
    )
}

fn boot_regions(boot: &[u8]) -> Vec<Region<'_>> {
    vec![Region {
        hunk: 0,
        kind: Kind::Code,
        origin: u32::try_from(crate::listing::BOOT_CODE_START).unwrap_or(12),
        bytes: boot.get(crate::listing::BOOT_CODE_START..).unwrap_or(&[]),
    }]
}

/// An executable's strings, given its walks; `None` when it is not one.
#[must_use]
pub fn strings(bytes: &[u8], a: &Analysis) -> Option<Vec<Text>> {
    Some(strings_in(&executable_regions(bytes)?, a))
}

/// An executable's variables, given its walks; `None` when it is not one.
#[must_use]
pub fn variables(bytes: &[u8], a: &Analysis) -> Option<Vec<Variable>> {
    let regions = executable_regions(bytes)?;
    let texts = strings_in(&regions, a);
    Some(variables_in(&regions, a, &texts))
}

/// A bootblock's strings, given its walks.
#[must_use]
pub fn boot_strings(boot: &[u8], a: &Analysis) -> Vec<Text> {
    strings_in(&boot_regions(boot), a)
}

/// A bootblock's variables, given its walks.
#[must_use]
pub fn boot_variables(boot: &[u8], a: &Analysis) -> Vec<Variable> {
    let regions = boot_regions(boot);
    let texts = strings_in(&regions, a);
    variables_in(&regions, a, &texts)
}

/// What a stretch of a program is, for drawing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Instructions a walk reached.
    Reached,
    /// Bytes of a code hunk no walk reached: code nothing calls by name, or
    /// data kept among the code.
    Unreached,
    /// A string.
    Text,
    /// A data hunk's other bytes.
    Data,
    /// Zeroed memory: BSS, or a hunk's memory past its file's bytes.
    Bss,
}

/// One run of bytes of one class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The hunk, 0 for boot code.
    pub hunk: usize,
    /// Where in it the run starts, as the program sees the address.
    pub start: u32,
    /// How long it is.
    pub length: u32,
    /// What it is.
    pub class: Class,
    /// Where the run's first byte is in the file, or the bootblock — the
    /// offset the listing's lines carry; `None` for memory the file does not
    /// hold.
    pub offset: Option<u64>,
}

/// Classify each byte of `regions` and run the classes together.
fn segments_in(
    regions: &[Region<'_>],
    sizes: &[u32],
    file_offsets: &[u64],
    a: &Analysis,
) -> Vec<Segment> {
    let texts = strings_in(regions, a);
    let mut out: Vec<Segment> = Vec::new();
    // One byte more of a run, or the first of a new one.
    let push = |out: &mut Vec<Segment>, hunk: usize, at: u32, class: Class, offset: Option<u64>| {
        if let Some(last) = out.last_mut()
            && last.hunk == hunk
            && last.class == class
            && last.start.saturating_add(last.length) == at
        {
            last.length = last.length.saturating_add(1);
            return;
        }
        out.push(Segment {
            hunk,
            start: at,
            length: 1,
            class,
            offset,
        });
    };
    for (n, r) in regions.iter().enumerate() {
        let in_text: Vec<(u32, u32)> = texts
            .iter()
            .filter(|t| t.at.0 == r.hunk)
            .map(|t| {
                (
                    t.at.1,
                    t.at.1
                        .saturating_add(u32::try_from(t.bytes.len()).unwrap_or(0)),
                )
            })
            .collect();
        let base = file_offsets.get(n).copied().unwrap_or(0);
        let len = u32::try_from(r.bytes.len()).unwrap_or(u32::MAX);
        // The strings are in order, so one cursor walks them with the bytes.
        let mut next = 0usize;
        for i in 0..len {
            let at = r.origin.saturating_add(i);
            while in_text.get(next).is_some_and(|&(_, e)| e <= at) {
                next = next.saturating_add(1);
            }
            let text = in_text.get(next).is_some_and(|&(s, e)| s <= at && at < e);
            let class = if r.kind == Kind::Bss {
                Class::Bss
            } else if in_code(a, (r.hunk, at)) {
                Class::Reached
            } else if text {
                Class::Text
            } else if r.kind == Kind::Code {
                Class::Unreached
            } else {
                Class::Data
            };
            push(
                &mut out,
                r.hunk,
                at,
                class,
                Some(base.saturating_add(u64::from(i))),
            );
        }
        // Memory the file does not hold: a BSS hunk, or the zeroed tail.
        let size = sizes.get(n).copied().unwrap_or(0);
        if size > len {
            out.push(Segment {
                hunk: r.hunk,
                start: r.origin.saturating_add(len),
                length: size.saturating_sub(len),
                class: Class::Bss,
                offset: None,
            });
        }
    }
    out
}

/// An executable as runs of reached code, unreached code, strings, data and
/// zeroed memory; `None` when it is not one.
#[must_use]
pub fn segments(bytes: &[u8], a: &Analysis) -> Option<Vec<Segment>> {
    let exe = hunk::parse(bytes)?;
    let regions = executable_regions(bytes)?;
    let sizes: Vec<u32> = exe.hunks.iter().map(|h| h.size).collect();
    let offsets: Vec<u64> = exe.hunks.iter().map(|h| h.file_offset as u64).collect();
    Some(segments_in(&regions, &sizes, &offsets, a))
}

/// A bootblock's code as runs, offsets counted from the bootblock's start.
#[must_use]
pub fn boot_segments(boot: &[u8], a: &Analysis) -> Vec<Segment> {
    let regions = boot_regions(boot);
    let size = regions
        .first()
        .map_or(0, |r| u32::try_from(r.bytes.len()).unwrap_or(0));
    segments_in(
        &regions,
        &[size],
        &[crate::listing::BOOT_CODE_START as u64],
        a,
    )
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::disallowed_methods,
    reason = "test scaffolding: programs built by hand, so the bytes cannot agree with the \
              reader about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;
    use crate::flow;

    /// An executable of the given hunks, each `(type, bytes)`, with one
    /// relocation list for hunk 0: `(offset, target)`.
    fn program(hunks: &[(u32, &[u8])], relocs: &[(u32, u32)]) -> Vec<u8> {
        let longs = |b: &[u8]| u32::try_from(b.len().div_ceil(4)).unwrap();
        let mut out = Vec::new();
        let n = u32::try_from(hunks.len()).unwrap();
        for l in [0x3F3u32, 0, n, 0, n - 1] {
            out.extend_from_slice(&l.to_be_bytes());
        }
        for (_, b) in hunks {
            out.extend_from_slice(&longs(b).to_be_bytes());
        }
        for (k, (t, b)) in hunks.iter().enumerate() {
            out.extend_from_slice(&t.to_be_bytes());
            out.extend_from_slice(&longs(b).to_be_bytes());
            let mut padded = b.to_vec();
            padded.resize(b.len().div_ceil(4) * 4, 0);
            out.extend_from_slice(&padded);
            if k == 0 && !relocs.is_empty() {
                out.extend_from_slice(&0x3ECu32.to_be_bytes());
                for (offset, target) in relocs {
                    for l in [1, *target, *offset] {
                        out.extend_from_slice(&l.to_be_bytes());
                    }
                }
                out.extend_from_slice(&0u32.to_be_bytes());
            }
            out.extend_from_slice(&0x3F2u32.to_be_bytes());
        }
        out
    }

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn a_string_the_code_points_at_starts_where_it_points() {
        // lea $8(pc),a0; rts; then "Nu" glued onto "hello world".
        let mut code = words(&[0x41FA, 0x0006, 0x4E75, 0x4E75]);
        code.extend_from_slice(b"hello world\0");
        // The rts at 6 is not reached: the walk stops at the one at 4.
        let bytes = program(&[(0x3E9, &code)], &[]);
        let a = flow::executable(&bytes).unwrap();
        let texts = strings(&bytes, &a).unwrap();
        let found: Vec<(u32, String)> = texts.iter().map(|t| (t.at.1, t.quoted())).collect();
        assert_eq!(found, [(8, "\"hello world\"".to_owned())], "{texts:?}");
        assert_eq!(texts[0].references[0].kind, RefKind::Address);
    }

    #[test]
    fn code_nothing_reaches_does_not_read_as_strings() {
        // rts, then rts nop rts nop and a NUL: "NuNqNuNq", unreferenced.
        let code = words(&[0x4E75, 0x4E75, 0x4E71, 0x4E75, 0x4E71, 0x0000]);
        let bytes = program(&[(0x3E9, &code)], &[]);
        let a = flow::executable(&bytes).unwrap();
        assert!(strings(&bytes, &a).unwrap().is_empty());
    }

    #[test]
    fn text_in_a_data_hunk_is_listed_whether_or_not_it_is_used() {
        let code = words(&[0x4E75]);
        let bytes = program(&[(0x3E9, &code), (0x3EA, b"Insert disk 2\0")], &[]);
        let a = flow::executable(&bytes).unwrap();
        let texts = strings(&bytes, &a).unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].at, (1, 0));
        assert!(texts[0].references.is_empty() && texts[0].terminated);
    }

    #[test]
    fn a_variable_is_what_the_code_reads_and_writes_by_name() {
        // move.w (h1+4).l,d0; move.l d0,(h1+8).l; rts
        let code = words(&[0x3039, 0, 4, 0x23C0, 0, 8, 0x4E75]);
        let bytes = program(&[(0x3E9, &code), (0x3EA, &[0; 12])], &[(2, 1), (8, 1)]);
        let a = flow::executable(&bytes).unwrap();
        let vars = variables(&bytes, &a).unwrap();
        let seen: Vec<(Place, Kind, Vec<u8>, usize, usize)> = vars
            .iter()
            .map(|v| {
                (
                    v.at,
                    v.kind,
                    v.widths.iter().copied().collect(),
                    v.reads,
                    v.writes,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ((1, 4), Kind::Data, vec![2], 1, 0),
                ((1, 8), Kind::Data, vec![4], 0, 1)
            ]
        );
    }
}
