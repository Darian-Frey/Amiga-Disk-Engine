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
//! On the first two, what [`crate::flow`]'s walks find is shown too (D-021):
//! a heading at each function, and each library call whose base the code
//! proves named by what it calls.
//!
//! Bounded (D-006): a listing stops at [`MAX_LINES`] and says so.

use std::collections::{BTreeMap, HashMap};

use crate::annotations::Annotations;
use crate::flow::Place;
use crate::hunk::{self, Executable, Hunk, Kind, Memory};
use crate::m68k::{Decoded, Names, Operand, decode};

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

/// Where control goes from an instruction, so a view can colour it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flow {
    /// On to the next instruction, or not an instruction.
    #[default]
    None,
    /// `bsr`, `jsr`: a call into the program, or through a register.
    Call,
    /// `rts`, `rte`, `rtr`.
    Return,
    /// `bra`, `jmp`: on, and not back.
    Jump,
    /// A conditional branch, or `dbcc`.
    Branch,
    /// `trap`, `trapv`, `illegal`: into the system's handlers.
    Trap,
    /// A call through a library base the walks proved (D-021).
    Library,
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
    /// What the line does, in words (F-032) — kept apart from `text` so a
    /// view can show or hide it without decoding again. `None` for a line
    /// with nothing to explain.
    pub comment: Option<String>,
    /// Where control goes from it.
    pub flow: Flow,
    /// The offsets of the places it refers to — a branch's target, an
    /// address it reads, a pointer it holds — so a view can go there, and
    /// find what refers to a line by looking for its offset.
    pub targets: Vec<u64>,
}

/// The column an explanation starts at when it is shown on the line.
pub const COMMENT_COLUMN: usize = 72;

impl Line {
    /// The line with its explanation, if any, as a `; comment` at
    /// [`COMMENT_COLUMN`] — or two spaces past the text when the text is
    /// longer than that.
    #[must_use]
    pub fn with_comment(&self) -> String {
        match &self.comment {
            Some(c) => {
                let width = COMMENT_COLUMN.max(self.text.chars().count().saturating_add(2));
                format!("{:<width$}; {c}", self.text)
            }
            None => self.text.clone(),
        }
    }
}

/// A listing and how it was made.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// The lines.
    pub lines: Vec<Line>,
    /// Every name a place in it goes by — its label, `h1_0042`, a symbol,
    /// the user's name — and that place's offset, so a view can go to any
    /// name another view shows.
    pub names: BTreeMap<String, u64>,
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
                    comment: None,
                    flow: Flow::None,
                    targets: Vec::new(),
                });
            }
            return false;
        }
        self.lines.push(Line {
            kind,
            offset,
            length,
            text,
            comment: None,
            flow: Flow::None,
            targets: Vec::new(),
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
    // Two spaces wider than the longest instruction's bytes (five words, 24
    // characters), so the mnemonic never runs into them.
    format!("{out:<26}")
}

/// Decode `code`, which sits at `base` in whatever was listed and whose first
/// byte is address `origin` as the program sees it.
/// What a run of code is decoded with, besides its bytes.
struct Run<'a> {
    /// Labels by address: symbols and the user's names.
    labels: &'a HashMap<u32, String>,
    /// The hunk each relocated long points into, by its address.
    relocs: &'a HashMap<u32, usize>,
    /// The user's names, by place.
    names: &'a BTreeMap<Place, String>,
    /// The run's hunk, for its labels.
    hunk: usize,
    /// A place's offset as lines carry them.
    place_offset: &'a dyn Fn(Place) -> Option<u64>,
}

/// The offsets an instruction at `at` in `code` refers to: what it reaches
/// by position, in this run, and what its relocated longs point at.
fn line_targets(
    i: &crate::m68k::Instruction,
    (code, at, len): (&[u8], usize, usize),
    (base, origin): (u64, u32),
    run: &Run<'_>,
) -> Vec<u64> {
    let mut targets: Vec<u64> = i
        .targets()
        .iter()
        .filter_map(|t| {
            let o = usize::try_from(t.checked_sub(origin)?).ok()?;
            (o < code.len()).then(|| base.saturating_add(o as u64))
        })
        .collect();
    for k in 2..len {
        let Ok(pos) = u32::try_from(at.saturating_add(k)) else {
            continue;
        };
        let pos = origin.wrapping_add(pos);
        if let Some(&t) = run.relocs.get(&pos)
            && let Ok(v) = ade_endian::u32_at(code, at.saturating_add(k))
            && let Some(o) = (run.place_offset)((t, v))
        {
            targets.push(o);
        }
    }
    targets.dedup();
    targets
}

/// The run's labels, and one for every branch target that is an
/// instruction start: a first pass for where instructions start and what
/// they point at, so a branch target can be labelled where the listing
/// reaches it. A label in the middle of an instruction could never be
/// printed, and naming it would mislead.
fn with_branch_labels(
    code: &[u8],
    origin: u32,
    labels: &HashMap<u32, String>,
    hunk: usize,
) -> HashMap<u32, String> {
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
            labels
                .entry(t)
                .or_insert_with(|| crate::export::label(hunk, t));
        }
    }
    labels
}

fn decode_run(out: &mut Listing, code: &[u8], (base, origin): (u64, u32), run: &Run<'_>) {
    let Run {
        labels,
        relocs,
        names,
        hunk,
        ..
    } = *run;
    let labels = &with_branch_labels(code, origin, labels, hunk);

    let mut at = 0usize;
    while at < code.len() {
        let address = origin.wrapping_add(u32::try_from(at).unwrap_or(0));
        if let Some(name) = labels.get(&address) {
            out.names
                .insert(name.clone(), base.saturating_add(at as u64));
            if !out.push(LineKind::Label, None, 0, format!("{name}:")) {
                return;
            }
        }
        let rest = code.get(at..).unwrap_or(&[]);
        let decoded = decode(rest, address);
        let len = usize::try_from(decoded.len())
            .unwrap_or(2)
            .min(rest.len())
            .max(1);
        let bytes = rest.get(..len).unwrap_or(&[]);
        let offset = Some(base.saturating_add(at as u64));
        let (kind, text, comment) = match &decoded {
            Decoded::Instruction(i) => {
                // A relocated long inside this instruction is an address the
                // loader fills in; name it by what it points at.
                let span: HashMap<u32, usize> = (2..len)
                    .filter_map(|k| {
                        let pos = address.wrapping_add(u32::try_from(k).ok()?);
                        let target = relocs.get(&pos)?;
                        let value = ade_endian::u32_at(code, at.checked_add(k)?).ok()?;
                        Some((value, *target))
                    })
                    .collect();
                let position = |v: u32| labels.get(&v).cloned();
                // Named by the user's name for the place, or by its hunk.
                let absolute = |v: u32| {
                    span.get(&v).map(|&t| {
                        names
                            .get(&(t, v))
                            .cloned()
                            .unwrap_or_else(|| crate::export::label(t, v))
                    })
                };
                let names = crate::m68k::Names {
                    position: &position,
                    absolute: &absolute,
                };
                let words = crate::explain::explain(i, &names);
                (LineKind::Code, i.motorola(&names), Some(words))
            }
            // The note goes in the comment column, so every remark on the
            // listing lines up; a view shows it whether or not explanations
            // are on, because it is a fact about the bytes, not an explanation.
            Decoded::Needs68020(w) => (
                LineKind::Data,
                format!("{:<8}${w:04x}", "dc.w"),
                Some("68020 addressing mode: not a 68000 instruction".to_owned()),
            ),
            Decoded::Invalid(w) => (
                LineKind::Data,
                format!("{:<8}${w:04x}", "dc.w"),
                Some("not a 68000 instruction".to_owned()),
            ),
            Decoded::Truncated => (
                LineKind::Data,
                format!("{:<8}{}", "dc.b", byte_list(bytes)),
                None,
            ),
        };
        if !out.push(
            kind,
            offset,
            u32::try_from(len).unwrap_or(2),
            format!("{address:08x}  {}{text}", hex(bytes)),
        ) {
            return;
        }
        if let Some(line) = out.lines.last_mut() {
            line.comment = comment;
            if let Decoded::Instruction(i) = &decoded {
                line.flow = flow_of(i.mnemonic);
                line.targets = line_targets(i, (code, at, len), (base, origin), run);
            }
        }
        if let Decoded::Instruction(i) = &decoded
            && i.ends_flow()
        {
            out.blank();
        }
        at = at.saturating_add(len);
    }
}

/// Where control goes from an instruction with this mnemonic.
fn flow_of(mnemonic: &str) -> Flow {
    match mnemonic {
        "bsr" | "jsr" => Flow::Call,
        "rts" | "rte" | "rtr" => Flow::Return,
        "bra" | "jmp" => Flow::Jump,
        "trap" | "trapv" | "illegal" => Flow::Trap,
        m if crate::flow::is_branch(m) || crate::flow::is_db(m) => Flow::Branch,
        _ => Flow::None,
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
    executable_with(bytes, &Annotations::default())
}

/// List an executable with the user's names and comments in it (D-022).
#[must_use]
pub fn executable_with(bytes: &[u8], notes: &Annotations) -> Option<Listing> {
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
    // A place's offset in the file; past a hunk's file bytes, in zeroed
    // memory, it has none.
    let offset = |(k, at): Place| {
        let h = exe.hunks.get(k)?;
        (u64::from(at) < h.file_len as u64)
            .then(|| (h.file_offset as u64).saturating_add(u64::from(at)))
    };
    for (k, h) in exe.hunks.iter().enumerate() {
        out.blank();
        hunk_listing(&mut out, bytes, (k, h), notes, &offset);
        if out.truncated {
            break;
        }
    }
    if let Some(a) = crate::flow::executable(bytes) {
        annotate(&mut out, &a, offset);
    }
    remark(&mut out, notes, offset);
    Some(out)
}

/// The user's comments, each on a line of its own above the line it is
/// about — shown whether or not explanations are, being the user's own —
/// and a name for a place inside a data line, which no label line can
/// carry, said above it.
fn remark(out: &mut Listing, notes: &Annotations, offset: impl Fn(Place) -> Option<u64>) {
    if notes.is_empty() {
        return;
    }
    let comments: BTreeMap<u64, &String> = notes
        .comments
        .iter()
        .filter_map(|(p, c)| Some((offset(*p)?, c)))
        .collect();
    let names: BTreeMap<u64, &String> = notes
        .names
        .iter()
        .filter_map(|(p, n)| Some((offset(*p)?, n)))
        .collect();
    for (at, name) in &names {
        out.names.insert((*name).clone(), *at);
    }
    let old = std::mem::take(&mut out.lines);
    let mut lines = Vec::with_capacity(old.len().saturating_add(comments.len()));
    for line in old {
        if let Some(start) = line.offset {
            let end = start.saturating_add(u64::from(line.length.max(1)));
            for (_, c) in comments.range(start..end) {
                lines.push(Line {
                    kind: LineKind::Note,
                    offset: None,
                    length: 0,
                    text: format!("; {c}"),
                    comment: None,
                    flow: Flow::None,
                    targets: Vec::new(),
                });
            }
            if line.kind == LineKind::Data {
                for (at, n) in names.range(start..end) {
                    let text = if *at == start {
                        format!("{n}:")
                    } else {
                        format!(
                            "; {n} is at +{} on the line below",
                            at.saturating_sub(start)
                        )
                    };
                    lines.push(Line {
                        kind: if *at == start {
                            LineKind::Label
                        } else {
                            LineKind::Note
                        },
                        offset: None,
                        length: 0,
                        text,
                        comment: None,
                        flow: Flow::None,
                        targets: Vec::new(),
                    });
                }
            }
        }
        lines.push(line);
    }
    out.lines = lines;
}

/// What the walks of [`crate::flow`] found, put on the listing: a heading at
/// each function, and each proved library call named — in the text, as
/// `jsr _LVOOpenLibrary(a6)`, because which function a call reaches is a
/// fact about the code, not an explanation to be switched off. `offset`
/// turns a place into the offset the listing's lines carry.
fn annotate(out: &mut Listing, a: &crate::flow::Analysis, offset: impl Fn(Place) -> Option<u64>) {
    let calls: HashMap<u64, &crate::flow::LibraryCall> = a
        .library_calls
        .iter()
        .filter_map(|c| Some((offset(c.at)?, c)))
        .collect();
    let functions: HashMap<u64, &crate::flow::Function> = a
        .functions
        .iter()
        .filter_map(|f| Some((offset(f.start)?, f)))
        .collect();
    let pointers: BTreeMap<u64, u64> = a
        .references
        .iter()
        .filter(|r| r.kind == crate::flow::RefKind::Pointer)
        .filter_map(|r| Some((offset(r.from)?, offset(r.to)?)))
        .collect();
    let old = std::mem::take(&mut out.lines);
    let mut lines = Vec::with_capacity(old.len().saturating_add(functions.len()));
    for (n, mut line) in old.iter().cloned().enumerate() {
        // A function's heading goes above its label, or above the
        // instruction itself where it has none.
        let starts = match line.kind {
            LineKind::Label => old.get(n.saturating_add(1)).and_then(|l| l.offset),
            LineKind::Code
                if n == 0
                    || old
                        .get(n.wrapping_sub(1))
                        .is_none_or(|p| p.kind != LineKind::Label) =>
            {
                line.offset
            }
            _ => None,
        };
        if let Some(f) = starts.and_then(|o| functions.get(&o)) {
            let callers = f.callers.len();
            let what = match (f.entry, callers) {
                (true, _) => "the program starts here".to_owned(),
                (false, 1) => "called from 1 place".to_owned(),
                (false, n) => format!("called from {n} places"),
            };
            let name = f
                .name
                .as_deref()
                .map(|n| format!(" {n}"))
                .unwrap_or_default();
            lines.push(Line {
                kind: LineKind::Note,
                offset: None,
                length: 0,
                text: format!("; ---- function{name}: {what}"),
                comment: None,
                flow: Flow::None,
                targets: Vec::new(),
            });
        }
        if line.kind == LineKind::Code
            && let Some(c) = line.offset.and_then(|o| calls.get(&o))
        {
            let shown = Operand::Disp(c.offset, 6).motorola(&Names::NONE);
            if let Some(f) = c.function
                && let Some(at) = line.text.rfind(&shown)
            {
                line.text
                    .replace_range(at..at.saturating_add(shown.len()), &format!("_LVO{f}(a6)"));
            }
            line.comment = Some(format!("call {}", c.describe()));
            line.flow = Flow::Library;
        }
        // A data line holding a relocated long points where the loader
        // makes it point.
        if line.kind == LineKind::Data
            && let Some(start) = line.offset
        {
            let end = start.saturating_add(u64::from(line.length));
            for (_, to) in pointers.range(start..end) {
                if !line.targets.contains(to) {
                    line.targets.push(*to);
                }
            }
        }
        lines.push(line);
    }
    out.lines = lines;
    // Every place the walks know of, by the name the views give it.
    let places = a
        .references
        .iter()
        .map(|r| r.to)
        .chain(a.functions.iter().map(|f| f.start));
    for place in places {
        if let Some(o) = offset(place) {
            out.names
                .entry(crate::annotations::place_text(place))
                .or_insert(o);
        }
    }
}

fn hunk_listing(
    out: &mut Listing,
    bytes: &[u8],
    (k, h): (usize, &Hunk),
    notes: &Annotations,
    offset: &dyn Fn(Place) -> Option<u64>,
) {
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
                format!("{:08x}  {:<26}{:<8}{}", 0, "", "ds.b", h.size),
            );
        }
        Kind::Data => data_run(out, data, base),
        Kind::Code => {
            // Symbols label their places, and the user's names theirs, over
            // a symbol's (D-022).
            let mut labels: HashMap<u32, String> =
                h.symbols.iter().map(|(n, v)| (*v, n.clone())).collect();
            for (&(hunk, at), name) in &notes.names {
                if hunk == k {
                    labels.insert(at, name.clone());
                }
            }
            let relocs: HashMap<u32, usize> = h
                .relocs
                .iter()
                .map(|r| (r.offset, r.target as usize))
                .collect();
            let run = Run {
                labels: &labels,
                relocs: &relocs,
                names: &notes.names,
                hunk: k,
                place_offset: offset,
            };
            decode_run(out, data, (base, 0), &run);
        }
    }
}

/// List a bootblock's code. `boot` is the whole bootblock, both blocks.
#[must_use]
pub fn bootblock(boot: &[u8]) -> Listing {
    bootblock_with(boot, &Annotations::default())
}

/// List a bootblock's code with the user's names and comments in it
/// (D-022); its places are the bootblock's own offsets, in hunk 0.
#[must_use]
pub fn bootblock_with(boot: &[u8], notes: &Annotations) -> Listing {
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
    let labels: HashMap<u32, String> = notes
        .names
        .iter()
        .filter(|((k, _), _)| *k == 0)
        .map(|((_, at), n)| (*at, n.clone()))
        .collect();
    // A bootblock's places are its own offsets.
    let offset = |(_, at): Place| Some(u64::from(at));
    let relocs = HashMap::new();
    let run = Run {
        labels: &labels,
        relocs: &relocs,
        names: &notes.names,
        hunk: 0,
        place_offset: &offset,
    };
    decode_run(
        &mut out,
        code,
        (
            BOOT_CODE_START as u64,
            u32::try_from(BOOT_CODE_START).unwrap_or(12),
        ),
        &run,
    );
    annotate(&mut out, &crate::flow::bootblock(boot), offset);
    remark(&mut out, notes, offset);
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
    let (labels, relocs, names) = (HashMap::new(), HashMap::new(), BTreeMap::new());
    let run = Run {
        labels: &labels,
        relocs: &relocs,
        names: &names,
        hunk: 0,
        place_offset: &|(_, at): Place| Some(u64::from(at)),
    };
    decode_run(&mut out, bytes, (0, 0), &run);
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
        // Named as every view and the export name it: hunk, then offset.
        assert!(all.contains("lea     h1_0010,a0"), "{all}");
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
    fn a_ten_byte_instruction_keeps_its_mnemonic_apart_and_every_code_line_is_explained() {
        // andi.l #$f7,($bfd100).l: five words, the longest a 68000 has. The
        // screenshot that asked for explanations also showed its bytes running
        // into the mnemonic.
        let listing = raw(&[0x02, 0xb9, 0x00, 0x00, 0x00, 0xf7, 0x00, 0xbf, 0xd1, 0x00]);
        let line = listing
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Code)
            .unwrap();
        assert!(
            line.text.contains("d100  andi.l"),
            "two spaces before the mnemonic: {}",
            line.text
        );
        let comment = line.comment.as_deref().unwrap();
        assert!(comment.contains("CIAB PRB"), "{comment}");
        assert!(
            line.with_comment().contains("; keep only bits $f7"),
            "{}",
            line.with_comment()
        );
        // Notes and data have nothing to explain.
        assert!(
            listing
                .lines
                .iter()
                .filter(|l| l.kind != LineKind::Code)
                .all(|l| l.comment.is_none())
        );
    }

    #[test]
    fn a_word_that_is_no_instruction_says_so_in_the_comment_column() {
        // Kept out of the text, so it lines up with every other remark.
        let listing = raw(&[0xa0, 0x00]);
        let line = listing
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Data)
            .unwrap();
        assert_eq!(line.comment.as_deref(), Some("not a 68000 instruction"));
        assert!(!line.text.contains(';'), "{}", line.text);
        assert_eq!(line.with_comment().find(';'), Some(COMMENT_COLUMN));
    }

    #[test]
    fn each_line_says_where_control_goes_and_what_it_refers_to() {
        let listing = raw(&[
            0x61, 0x04, // bsr.s  h0_0006
            0x67, 0x02, // beq.s  h0_0006
            0x4e, 0x75, // rts
            0x4e, 0x41, // $6: trap #1
            0x4e, 0x71, // nop
        ]);
        let code: Vec<&Line> = listing
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Code)
            .collect();
        let flows: Vec<Flow> = code.iter().map(|l| l.flow).collect();
        assert_eq!(
            flows,
            [
                Flow::Call,
                Flow::Branch,
                Flow::Return,
                Flow::Trap,
                Flow::None
            ]
        );
        assert_eq!(code[0].targets, [6]);
        assert_eq!(code[1].targets, [6]);
        assert!(code[2].targets.is_empty(), "{:?}", code[2].targets);
        // And the label's name finds its place.
        assert_eq!(listing.names.get("h0_0006"), Some(&6));
    }

    #[test]
    fn a_number_is_named_only_when_a_relocation_makes_it_an_address() {
        // Offset 4 has a label (the branch lands there), and the code also
        // holds the number 4 as an immediate and as an absolute address.
        // Neither is that label: without a relocation they are numbers. Found
        // in 3D Pool's boot code, where `move.l #$c8,d2` read `#lab_00c8`.
        let listing = raw(&[
            0x60, 0x02, // bra.s  h0_0004
            0x4e, 0x71, // nop
            0x24, 0x3c, 0x00, 0x00, 0x00, 0x04, // move.l #4,d2
            0x4e, 0xf9, 0x00, 0x00, 0x00, 0x04, // jmp ($4).l
        ]);
        let text: Vec<&str> = listing.lines.iter().map(|l| l.text.as_str()).collect();
        let all = text.join("\n");
        assert!(
            all.contains("bra.s   h0_0004"),
            "the branch is named: {all}"
        );
        assert!(
            all.contains("move.l  #4,d2"),
            "the immediate is a number: {all}"
        );
        assert!(
            all.contains("jmp     ($4).l"),
            "the absolute address is a number: {all}"
        );
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
