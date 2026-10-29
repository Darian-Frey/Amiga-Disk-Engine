//! What a program's code does with itself, found by following it (D-021):
//! its functions, who calls whom, what refers to what, and which library
//! each call through a base register reaches.
//!
//! **Only code the program can reach is read.** A walk starts where the
//! loader starts the program — the first byte of hunk 0 — and follows every
//! branch, jump and call whose target the instruction itself gives; each call
//! target starts a function, walked the same way. Bytes no walk reaches are
//! not read as code at all, so the references here are a program's, not a
//! linear sweep's guesses through its data. What a walk cannot follow — a
//! jump through a register, a jump table — is counted, not guessed at.
//!
//! **A library is named only where the code proves which one it is.** The
//! walk carries, for each register, what is known of it: nothing; a
//! library's base; or the address of a place in the program. A base is known
//! from two facts and nothing else: the long at address 4 is exec's base,
//! and `OpenLibrary` (or `OldOpenLibrary`) returns in `d0` the base of the
//! library whose name `a1` points at — read from the program's own bytes.
//! A library call keeps `d2`–`d7` and `a2`–`a6`, as the system's calling
//! convention promises; a call into the program's own code may change any
//! register, so everything known is forgotten; and where two paths meet,
//! only what both agree on survives. A base kept in memory is not followed:
//! that a variable holds `DOSBase` is a claim about every write to it, which
//! the code shown cannot prove.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::hunk::{self, Kind};
use crate::lvo::{self, Library};
use crate::m68k::{Decoded, Instruction, Operand, decode};

/// A place in a program: a hunk and an offset in it.
pub type Place = (usize, u32);

/// Instructions one program's walks may visit, revisits included, before
/// they stop: bounded, as every parse of untrusted bytes is (D-006).
pub const MAX_STEPS: usize = 4_000_000;

/// How one place refers to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    /// `bsr`, `jsr`: a call.
    Call,
    /// `jmp`, `bra`: control goes there and does not come back.
    Jump,
    /// A conditional branch, or `dbcc`.
    Branch,
    /// `lea`, `pea`, or an immediate the loader relocates: the address is
    /// taken, not what is at it.
    Address,
    /// An operand that reads what is there.
    Read,
    /// An operand that writes there.
    Write,
    /// A long in a data hunk that the loader relocates to point there.
    Pointer,
}

impl RefKind {
    /// The kind in a word.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Jump => "jump",
            Self::Branch => "branch",
            Self::Address => "address",
            Self::Read => "read",
            Self::Write => "write",
            Self::Pointer => "pointer",
        }
    }
}

/// One reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    /// The instruction, or the data long, that refers.
    pub from: Place,
    /// What it refers to.
    pub to: Place,
    /// How.
    pub kind: RefKind,
}

/// One call through a library base the walk had proved.
#[derive(Debug, Clone)]
pub struct LibraryCall {
    /// The `jsr`.
    pub at: Place,
    /// The library.
    pub library: &'static Library,
    /// The offset called, as the instruction gives it.
    pub offset: i16,
    /// The function there, when the library's table has one.
    pub function: Option<&'static str>,
    /// For exec's `OpenLibrary` and `OldOpenLibrary`, the name `a1` points
    /// at, when the walk knows where that is.
    pub opens: Option<String>,
}

impl LibraryCall {
    /// `dos Open`, or `dos -$1a4` for an offset the table does not name.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.function {
            Some(f) => match &self.opens {
                Some(name) => format!("{} {f} \"{name}\"", self.library.short()),
                None => format!("{} {f}", self.library.short()),
            },
            None => format!(
                "{} {}",
                self.library.short(),
                crate::m68k::number(i64::from(self.offset))
            ),
        }
    }
}

/// One function: a place the program calls, and the code it reaches.
#[derive(Debug, Clone, Default)]
pub struct Function {
    /// Where it starts.
    pub start: Place,
    /// Its symbol, when the program carries one there.
    pub name: Option<String>,
    /// Whether the loader starts the program here.
    pub entry: bool,
    /// Instructions the walk reached.
    pub instructions: usize,
    /// Bytes those instructions occupy.
    pub bytes: u32,
    /// Functions it calls, or jumps to in another hunk.
    pub calls: BTreeSet<Place>,
    /// Instructions that call it.
    pub callers: BTreeSet<Place>,
    /// Its calls through a library base: see [`Analysis::library_call`].
    pub library_calls: BTreeSet<Place>,
    /// Calls and jumps through a register or memory, which no walk can
    /// follow.
    pub indirect: usize,
    /// Places where the walk met bytes that are not a 68000 instruction.
    pub invalid: usize,
}

/// Everything the walks found.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// Functions, in program order.
    pub functions: Vec<Function>,
    /// Every reference, sorted by where it points.
    pub references: Vec<Reference>,
    /// Library calls, in program order.
    pub library_calls: Vec<LibraryCall>,
    /// Every instruction reached, with its length.
    pub reached: BTreeMap<Place, u32>,
    /// Whether the walks stopped at [`MAX_STEPS`].
    pub truncated: bool,
}

impl Analysis {
    /// The function starting at `place`.
    #[must_use]
    pub fn function(&self, place: Place) -> Option<&Function> {
        self.functions
            .binary_search_by_key(&place, |f| f.start)
            .ok()
            .and_then(|i| self.functions.get(i))
    }

    /// References to `place`.
    #[must_use]
    pub fn references_to(&self, place: Place) -> &[Reference] {
        let from = self.references.partition_point(|r| r.to < place);
        let to = self.references.partition_point(|r| r.to <= place);
        self.references.get(from..to).unwrap_or(&[])
    }

    /// The library call at `place`.
    #[must_use]
    pub fn library_call(&self, place: Place) -> Option<&LibraryCall> {
        self.library_calls
            .binary_search_by_key(&place, |c| c.at)
            .ok()
            .and_then(|i| self.library_calls.get(i))
    }
}

/// What is known of one register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    Unknown,
    /// A library's base, by index into [`lvo::LIBRARIES`].
    Base(usize),
    /// The address of a place in the program.
    Addr(Place),
}

/// The registers: `d0`–`d7`, then `a0`–`a7`.
type State = [Value; 16];

const UNKNOWN: State = [Value::Unknown; 16];

/// What is known of register `r`.
fn get(state: &State, r: usize) -> Value {
    state.get(r).copied().unwrap_or(Value::Unknown)
}

/// Record what is known of register `r`.
fn set(state: &mut State, r: usize, v: Value) {
    if let Some(slot) = state.get_mut(r) {
        *slot = v;
    }
}

/// Whether `t` is an address inside `code`'s bytes.
fn holds(code: &Code<'_>, t: u32) -> bool {
    t.checked_sub(code.origin)
        .is_some_and(|o| (o as usize) < code.bytes.len())
}

fn meet(a: &State, b: &State) -> State {
    let mut out = UNKNOWN;
    for ((o, x), y) in out.iter_mut().zip(a).zip(b) {
        if x == y {
            *o = *x;
        }
    }
    out
}

/// One code hunk, as the walk reads it.
struct Code<'a> {
    bytes: &'a [u8],
    /// The address its first byte is, as its instructions see it.
    origin: u32,
    /// Its relocations: the hunk and offset each relocated long holds.
    relocs: HashMap<u32, Place>,
}

const fn d(n: u8) -> usize {
    n as usize
}

const fn a(n: u8) -> usize {
    (n as usize).saturating_add(8)
}

fn is_branch(m: &str) -> bool {
    matches!(
        m,
        "bhi"
            | "bls"
            | "bcc"
            | "bcs"
            | "bne"
            | "beq"
            | "bvc"
            | "bvs"
            | "bpl"
            | "bmi"
            | "bge"
            | "blt"
            | "bgt"
            | "ble"
    )
}

fn is_db(m: &str) -> bool {
    m.starts_with("db")
}

/// Instructions that write no register they name, whatever their operands.
fn writes_nothing(m: &str) -> bool {
    matches!(
        m,
        "cmp" | "cmpa" | "cmpi" | "tst" | "btst" | "chk" | "pea" | "jmp" | "jsr" | "bsr" | "bra"
    ) || is_branch(m)
}

/// The registers an instruction may change, apart from `sp`'s own
/// bookkeeping — conservative: a register it names as a destination, one it
/// increments or decrements through, and every register a `movem` loads.
fn written(i: &Instruction) -> Vec<usize> {
    let mut out = Vec::new();
    for o in &i.operands {
        if let Operand::PostInc(r) | Operand::PreDec(r) = o {
            out.push(a(*r));
        }
    }
    if writes_nothing(i.mnemonic) {
        return out;
    }
    let reg = |o: &Operand| match o {
        Operand::D(r) => Some(d(*r)),
        Operand::A(r) => Some(a(*r)),
        _ => None,
    };
    match i.mnemonic {
        "exg" => out.extend(i.operands.iter().filter_map(reg)),
        _ if is_db(i.mnemonic) => out.extend(i.operands.first().and_then(reg)),
        "link" | "unlk" => out.extend(i.operands.first().and_then(reg)),
        _ => match i.operands.last() {
            Some(Operand::RegList(m)) => {
                out.extend((0..16).filter(|b| m & (1 << b) != 0));
            }
            Some(o) => out.extend(reg(o)),
            None => {}
        },
    }
    out
}

/// A NUL-ended name at `place`, as `OpenLibrary` reads it.
fn name_at(hunks: &[Code<'_>], (k, at): Place) -> Option<String> {
    let code = hunks.get(k)?;
    let from = usize::try_from(at.checked_sub(code.origin)?).ok()?;
    let rest = code.bytes.get(from..)?;
    let end = rest.iter().take(64).position(|&b| b == 0)?;
    let name = rest.get(..end)?;
    name.iter()
        .all(u8::is_ascii_graphic)
        .then(|| String::from_utf8_lossy(name).into_owned())
}

struct Walk<'a> {
    hunks: &'a [Code<'a>],
    exec: usize,
    steps: usize,
    truncated: bool,
    references: BTreeSet<Reference>,
    library_calls: BTreeMap<Place, LibraryCall>,
    reached: BTreeMap<Place, u32>,
    /// Calls found, as (caller instruction, callee).
    calls: Vec<(Place, Place, RefKind)>,
}

impl Walk<'_> {
    /// The absolute operands of `i` that the loader relocates, by value:
    /// the place each points at.
    fn relocated_operands(&self, k: usize, at: u32, i: &Instruction) -> HashMap<u32, Place> {
        let mut out = HashMap::new();
        let Some(code) = self.hunks.get(k) else {
            return out;
        };
        for p in at.saturating_add(2)..at.saturating_add(i.len) {
            if let Some(place) = code.relocs.get(&p)
                && let Some(from) = p.checked_sub(code.origin)
                && let Ok(v) = ade_endian::u32_at(code.bytes, from as usize)
            {
                out.insert(v, *place);
            }
        }
        out
    }

    /// Walk one function from `start`, with registers `entry`.
    fn function(&mut self, start: Place, entry: &State, f: &mut Function) {
        let (k, _) = start;
        let Some(code) = self.hunks.get(k) else {
            return;
        };
        let mut states: HashMap<u32, State> = HashMap::new();
        let mut queue: VecDeque<(u32, State)> = VecDeque::from([(start.1, *entry)]);
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        while let Some((at, incoming)) = queue.pop_front() {
            if self.steps >= MAX_STEPS {
                self.truncated = true;
                return;
            }
            self.steps = self.steps.saturating_add(1);
            let state = match states.get(&at) {
                Some(old) => {
                    let merged = meet(old, &incoming);
                    if merged == *old {
                        continue;
                    }
                    merged
                }
                None => incoming,
            };
            states.insert(at, state);
            let Some(from) = at.checked_sub(code.origin) else {
                continue;
            };
            let Some(rest) = code.bytes.get(from as usize..).filter(|r| !r.is_empty()) else {
                continue;
            };
            let Decoded::Instruction(i) = decode(rest, at) else {
                f.invalid = f.invalid.saturating_add(1);
                continue;
            };
            if seen.insert(at) {
                f.instructions = f.instructions.saturating_add(1);
                f.bytes = f.bytes.saturating_add(i.len);
            }
            self.reached.insert((k, at), i.len);
            let (next, after) = self.step((k, at), &i, &state, f);
            for (to, s) in next {
                queue.push_back((to, s));
            }
            if let Some(s) = after {
                queue.push_back((at.saturating_add(i.len), s));
            }
        }
    }

    /// The references one instruction makes, by operand.
    fn refer(&mut self, here: Place, ins: &Instruction, moved: &HashMap<u32, Place>) {
        let mnemonic = ins.mnemonic;
        let by_position = || match mnemonic {
            "jsr" => Some(RefKind::Call),
            "jmp" => Some(RefKind::Jump),
            "lea" | "pea" => Some(RefKind::Address),
            _ => None,
        };
        let last = ins.operands.len().saturating_sub(1);
        for (n, o) in ins.operands.iter().enumerate() {
            let data = if n == last && !writes_nothing(mnemonic) && ins.operands.len() > 1 {
                RefKind::Write
            } else {
                RefKind::Read
            };
            let (to, kind) = match o {
                Operand::Target(t) => {
                    let kind = match mnemonic {
                        "bsr" => RefKind::Call,
                        "bra" => RefKind::Jump,
                        _ => RefKind::Branch,
                    };
                    ((here.0, *t), kind)
                }
                Operand::PcDisp(t) | Operand::PcIndexed(t, _) => {
                    ((here.0, *t), by_position().unwrap_or(data))
                }
                Operand::AbsL(v) => match moved.get(v) {
                    Some(p) => (*p, by_position().unwrap_or(data)),
                    None => continue,
                },
                Operand::Imm(v) => match moved.get(v) {
                    Some(p) => (*p, RefKind::Address),
                    None => continue,
                },
                _ => continue,
            };
            self.references.insert(Reference {
                from: here,
                to,
                kind,
            });
        }
    }

    /// One instruction: its references, the state after it, and where
    /// control goes — `(targets with their states, fall-through state)`.
    fn step(
        &mut self,
        here: Place,
        ins: &Instruction,
        state: &State,
        func: &mut Function,
    ) -> (Vec<(u32, State)>, Option<State>) {
        let (k, at) = here;
        let mnemonic = ins.mnemonic;
        let moved = self.relocated_operands(k, at, ins);
        self.refer(here, ins, &moved);
        let inside = |t: u32| self.hunks.get(k).is_some_and(|c| holds(c, t));
        let mut targets = Vec::new();
        let mut after = *state;
        let forget = |after: &mut State| {
            for r in written(ins) {
                set(after, r, Value::Unknown);
            }
        };
        let target = ins.operands.iter().find_map(|o| match o {
            Operand::Target(t) | Operand::PcDisp(t) => Some((k, *t)),
            Operand::AbsL(v) => moved.get(v).copied(),
            _ => None,
        });
        match mnemonic {
            "bsr" | "jsr" => {
                after = UNKNOWN;
                match (target, ins.operands.first()) {
                    (Some(t), _) => {
                        self.calls.push((here, t, RefKind::Call));
                        func.calls.insert(t);
                    }
                    // A library's vectors are below its base; above it is
                    // the library's own data, which nothing calls.
                    (None, Some(Operand::Disp(off, 6))) if *off < 0 => match get(state, a(6)) {
                        Value::Base(lib) => {
                            after = self.library_call(here, lib, *off, state);
                            func.library_calls.insert(here);
                        }
                        _ => func.indirect = func.indirect.saturating_add(1),
                    },
                    _ => func.indirect = func.indirect.saturating_add(1),
                }
            }
            "jmp" | "bra" => match target {
                Some(t) if t.0 == k && inside(t.1) => targets.push((t.1, *state)),
                Some(t) => {
                    self.calls.push((here, t, RefKind::Jump));
                    func.calls.insert(t);
                }
                None => func.indirect = func.indirect.saturating_add(1),
            },
            _ if is_branch(mnemonic) || is_db(mnemonic) => {
                // A `dbcc` counts its register down on the way.
                forget(&mut after);
                if let Some(t) = target.filter(|t| inside(t.1)) {
                    targets.push((t.1, after));
                }
            }
            "trap" | "trapv" => after = UNKNOWN,
            _ => {
                forget(&mut after);
                self.assign(ins, state, &moved, &mut after, k);
            }
        }
        let falls = !ins.ends_flow() && !matches!(mnemonic, "rts" | "rte" | "rtr" | "illegal");
        (targets, falls.then_some(after))
    }

    /// What an instruction puts in a register that the walk can follow: a
    /// copy, exec's base from address 4, or an address in the program.
    fn assign(
        &self,
        i: &Instruction,
        before: &State,
        moved: &HashMap<u32, Place>,
        after: &mut State,
        k: usize,
    ) {
        let long = i.size.is_none_or(|s| s.suffix() == "l");
        let dest = match i.operands.get(1) {
            Some(Operand::D(r)) => d(*r),
            Some(Operand::A(r)) => a(*r),
            _ => return,
        };
        let value = match (i.mnemonic, i.operands.first()) {
            ("move" | "movea", Some(Operand::AbsW(4))) if long => Value::Base(self.exec),
            ("move" | "movea", Some(Operand::AbsL(4))) if long && !moved.contains_key(&4) => {
                Value::Base(self.exec)
            }
            ("move" | "movea", Some(Operand::D(r))) if long => get(before, d(*r)),
            ("move" | "movea", Some(Operand::A(r))) if long => get(before, a(*r)),
            ("move" | "movea", Some(Operand::Imm(v))) if long => {
                moved.get(v).map_or(Value::Unknown, |p| Value::Addr(*p))
            }
            ("lea", Some(Operand::PcDisp(t))) => Value::Addr((k, *t)),
            ("lea", Some(Operand::AbsL(v))) => {
                moved.get(v).map_or(Value::Unknown, |p| Value::Addr(*p))
            }
            ("exg", Some(o)) => {
                let src = match o {
                    Operand::D(r) => d(*r),
                    Operand::A(r) => a(*r),
                    _ => return,
                };
                set(after, dest, get(before, src));
                set(after, src, get(before, dest));
                return;
            }
            _ => return,
        };
        if let Some(slot) = after.get_mut(dest) {
            *slot = value;
        }
    }

    /// A call through a proved base: recorded, and the registers after it —
    /// the scratch ones lost, `d0` a library's base when this was exec's
    /// `OpenLibrary` given a name the table knows.
    fn library_call(&mut self, at: Place, lib: usize, offset: i16, state: &State) -> State {
        let Some(library) = lvo::LIBRARIES.get(lib) else {
            return UNKNOWN;
        };
        let function = library.function(offset);
        let opens = match get(state, a(1)) {
            Value::Addr(p)
                if lib == self.exec
                    && matches!(function, Some("OpenLibrary" | "OldOpenLibrary")) =>
            {
                name_at(self.hunks, p)
            }
            _ => None,
        };
        let mut after = *state;
        for r in [d(0), d(1), a(0), a(1)] {
            set(&mut after, r, Value::Unknown);
        }
        if let Some(opened) = opens
            .as_deref()
            .and_then(|n| lvo::LIBRARIES.iter().position(|l| l.name == n))
        {
            set(&mut after, d(0), Value::Base(opened));
        }
        self.library_calls.insert(
            at,
            LibraryCall {
                at,
                library,
                offset,
                function,
                opens,
            },
        );
        after
    }
}

/// Walk code from `entries`, each a place and the registers it starts with.
fn walk(
    hunks: &[Code<'_>],
    entries: &[(Place, State)],
    symbols: &HashMap<Place, String>,
) -> Analysis {
    let exec = lvo::LIBRARIES
        .iter()
        .position(|l| l.name == "exec.library")
        .unwrap_or(0);
    let mut w = Walk {
        hunks,
        exec,
        steps: 0,
        truncated: false,
        references: BTreeSet::new(),
        library_calls: BTreeMap::new(),
        reached: BTreeMap::new(),
        calls: Vec::new(),
    };
    let mut functions: BTreeMap<Place, Function> = BTreeMap::new();
    let mut queue: VecDeque<(Place, State, bool)> =
        entries.iter().map(|(p, s)| (*p, *s, true)).collect();
    while let Some((start, state, entry)) = queue.pop_front() {
        if functions.contains_key(&start) || w.truncated {
            continue;
        }
        let is_code = hunks.get(start.0).is_some_and(|c| holds(c, start.1));
        if !is_code {
            continue;
        }
        let mut f = Function {
            start,
            name: symbols.get(&start).cloned(),
            entry,
            ..Function::default()
        };
        let before = w.calls.len();
        w.function(start, &state, &mut f);
        for &(_, callee, _) in w.calls.get(before..).unwrap_or(&[]) {
            queue.push_back((callee, UNKNOWN, false));
        }
        functions.insert(start, f);
    }
    // Callers, now every function is known; and a call to a place that is
    // not code — past a hunk's end, into data — is no function's.
    for &(site, callee, _) in &w.calls {
        if let Some(f) = functions.get_mut(&callee) {
            f.callers.insert(site);
        }
    }
    let starts: BTreeSet<Place> = functions.keys().copied().collect();
    for f in functions.values_mut() {
        f.calls.retain(|c| starts.contains(c));
    }
    let library_calls: Vec<LibraryCall> = w.library_calls.into_values().collect();
    let functions: Vec<Function> = functions.into_values().collect();
    let mut references: Vec<Reference> = w.references.into_iter().collect();
    references.sort_by_key(|r| (r.to, r.from, r.kind));
    Analysis {
        functions,
        references,
        library_calls,
        reached: w.reached,
        truncated: w.truncated,
    }
}

/// Analyse an executable, or `None` when it is not one.
#[must_use]
pub fn executable(bytes: &[u8]) -> Option<Analysis> {
    let exe = hunk::parse(bytes)?;
    let hunks: Vec<Code<'_>> = exe
        .hunks
        .iter()
        .map(|h| {
            let data = if h.kind == Kind::Code {
                bytes
                    .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
                    .unwrap_or(&[])
            } else {
                &[]
            };
            let all = bytes
                .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
                .unwrap_or(&[]);
            let relocs = h
                .relocs
                .iter()
                .filter(|r| (r.target as usize) < exe.hunks.len())
                .filter_map(|r| {
                    let v = ade_endian::u32_at(all, r.offset as usize).ok()?;
                    Some((r.offset, (r.target as usize, v)))
                })
                .collect();
            Code {
                bytes: data,
                origin: 0,
                relocs,
            }
        })
        .collect();
    let symbols: HashMap<Place, String> = exe
        .hunks
        .iter()
        .enumerate()
        .flat_map(|(k, h)| h.symbols.iter().map(move |(n, v)| ((k, *v), n.clone())))
        .collect();
    let entries: Vec<(Place, State)> = match exe.hunks.first() {
        Some(h) if h.kind == Kind::Code => vec![((0, 0), UNKNOWN)],
        _ => Vec::new(),
    };
    let mut out = walk(&hunks, &entries, &symbols);
    // A relocated long in a data hunk points where the loader makes it.
    for (k, h) in exe.hunks.iter().enumerate() {
        if h.kind != Kind::Data {
            continue;
        }
        let all = bytes
            .get(h.file_offset..h.file_offset.saturating_add(h.file_len))
            .unwrap_or(&[]);
        for r in &h.relocs {
            if (r.target as usize) < exe.hunks.len()
                && let Ok(v) = ade_endian::u32_at(all, r.offset as usize)
            {
                out.references.push(Reference {
                    from: (k, r.offset),
                    to: (r.target as usize, v),
                    kind: RefKind::Pointer,
                });
            }
        }
    }
    out.references.sort_by_key(|r| (r.to, r.from, r.kind));
    Some(out)
}

/// Analyse a bootblock's code, which AmigaDOS enters at byte 12 with `a6`
/// holding exec's base. Places are offsets in the bootblock, in hunk 0.
#[must_use]
pub fn bootblock(boot: &[u8]) -> Analysis {
    let origin = u32::try_from(crate::listing::BOOT_CODE_START).unwrap_or(12);
    let code = boot.get(crate::listing::BOOT_CODE_START..).unwrap_or(&[]);
    let hunks = [Code {
        bytes: code,
        origin,
        relocs: HashMap::new(),
    }];
    let exec = lvo::LIBRARIES
        .iter()
        .position(|l| l.name == "exec.library")
        .unwrap_or(0);
    let mut entry = UNKNOWN;
    set(&mut entry, a(6), Value::Base(exec));
    walk(&hunks, &[((0, origin), entry)], &HashMap::new())
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

    /// A one-hunk executable holding `code`, padded to a long.
    fn program(code: &[u8]) -> Vec<u8> {
        let mut code = code.to_vec();
        while code.len() % 4 != 0 {
            code.push(0);
        }
        let n = u32::try_from(code.len() / 4).unwrap();
        let mut out = Vec::new();
        for l in [0x3F3u32, 0, 1, 0, 0, n, 0x3E9, n] {
            out.extend_from_slice(&l.to_be_bytes());
        }
        out.extend_from_slice(&code);
        out.extend_from_slice(&0x3F2u32.to_be_bytes());
        out
    }

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    #[test]
    fn exec_from_address_4_and_the_library_it_opens_are_named() {
        let mut code = words(&[
            0x2C78, 0x0004, // movea.l $4.w,a6
            0x43FA, 0x0012, // lea $18(pc),a1
            0x7000, // moveq #0,d0
            0x4EAE, 0xFDD8, // jsr -552(a6): OpenLibrary
            0x2C40, // movea.l d0,a6
            0x4EAE, 0xFFC4, // jsr -60(a6): dos Output
            0x4E75, 0x4E71, // rts; nop
        ]);
        code.extend_from_slice(b"dos.library\0");
        let a = executable(&program(&code)).unwrap();
        let named: Vec<String> = a.library_calls.iter().map(LibraryCall::describe).collect();
        assert_eq!(
            named,
            ["exec OpenLibrary \"dos.library\"", "dos Output"],
            "{a:?}"
        );
        assert!(
            a.references_to((0, 0x18))
                .iter()
                .any(|r| r.kind == RefKind::Address && r.from == (0, 4))
        );
        let entry = a.function((0, 0)).unwrap();
        assert!(entry.entry);
        assert_eq!(entry.library_calls.len(), 2);
        // The name is data the walk never reaches.
        assert!(!a.reached.contains_key(&(0, 0x18)));
    }

    #[test]
    fn a_call_into_the_program_forgets_the_base() {
        let code = words(&[
            0x2C78, 0x0004, // movea.l $4.w,a6
            0x6100, 0x0008, // bsr.w $e
            0x4EAE, 0xFF7C, // jsr -132(a6): a6 may have changed
            0x4E75, // rts
            0x4E75, // $e: rts
        ]);
        let a = executable(&program(&code)).unwrap();
        assert!(a.library_calls.is_empty(), "{:?}", a.library_calls);
        let entry = a.function((0, 0)).unwrap();
        assert_eq!(entry.indirect, 1);
        assert_eq!(entry.calls, BTreeSet::from([(0, 0xe)]));
        let callee = a.function((0, 0xe)).unwrap();
        assert_eq!(callee.callers, BTreeSet::from([(0, 4)]));
        assert!(!callee.entry);
    }

    #[test]
    fn where_paths_meet_only_what_both_know_survives() {
        // One path sets a6 and the other does not.
        let split = words(&[
            0x4A80, // tst.l d0
            0x6704, // beq.s $8
            0x2C78, 0x0004, // movea.l $4.w,a6
            0x4EAE, 0xFF7C, // $8: jsr -132(a6)
            0x4E75,
        ]);
        let a = executable(&program(&split)).unwrap();
        assert!(a.library_calls.is_empty(), "{:?}", a.library_calls);
        // Both paths set it.
        let both = words(&[
            0x2C78, 0x0004, // movea.l $4.w,a6
            0x4A80, // tst.l d0
            0x6702, // beq.s $8
            0x7001, // moveq #1,d0
            0x4EAE, 0xFF7C, // $8: jsr -132(a6): Forbid
            0x4E75,
        ]);
        let a = executable(&program(&both)).unwrap();
        let named: Vec<String> = a.library_calls.iter().map(LibraryCall::describe).collect();
        assert_eq!(named, ["exec Forbid"]);
    }

    #[test]
    fn a_register_written_another_way_is_no_longer_known() {
        let code = words(&[
            0x2C78, 0x0004, // movea.l $4.w,a6
            0x2C5F, // movea.l (sp)+,a6
            0x4EAE, 0xFF7C, // jsr -132(a6)
            0x4E75,
        ]);
        let a = executable(&program(&code)).unwrap();
        assert!(a.library_calls.is_empty(), "{:?}", a.library_calls);
    }

    #[test]
    fn a_bootblock_starts_with_exec_in_a6() {
        let mut boot = vec![0u8; 1024];
        boot[..4].copy_from_slice(b"DOS\0");
        boot[12..18].copy_from_slice(&words(&[0x4EAE, 0xFFA0, 0x4E75])); // jsr -96(a6); rts
        let a = bootblock(&boot);
        let named: Vec<String> = a.library_calls.iter().map(LibraryCall::describe).collect();
        assert_eq!(named, ["exec FindResident"]);
        assert_eq!(a.library_calls[0].at, (0, 12));
    }

    #[test]
    fn bytes_no_walk_reaches_are_not_read_as_code() {
        let code = words(&[0x4E75, 0x4EAE, 0xFDD8]); // rts; then data
        let a = executable(&program(&code)).unwrap();
        assert_eq!(a.reached.len(), 1);
        assert!(a.references.is_empty(), "{:?}", a.references);
        assert_eq!(a.functions.len(), 1);
    }

    #[test]
    fn the_table_holds_offsets_known_from_amigaos() {
        let exec = lvo::library("exec.library").unwrap();
        assert_eq!(exec.function(-552), Some("OpenLibrary"));
        assert_eq!(exec.function(-198), Some("AllocMem"));
        assert_eq!(exec.function(-30), Some("Supervisor"));
        let dos = lvo::library("dos.library").unwrap();
        assert_eq!(dos.function(-48), Some("Write"));
        let gfx = lvo::library("graphics.library").unwrap();
        assert_eq!(gfx.function(-60), Some("Text"));
        let int = lvo::library("intuition.library").unwrap();
        assert_eq!(int.function(-204), Some("OpenWindow"));
        let locale = lvo::library("locale.library").unwrap();
        assert_eq!(locale.function(-36), Some("CloseCatalog"));
        assert_eq!(exec.function(-31), None);
    }
}
