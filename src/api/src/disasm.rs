//! The Assembly view's engine side (F-032, D-019).
//!
//! What a front end asks is "show me this file's code" or "show me this
//! disk's boot code". Deciding whether a file *is* code, and where in it the
//! code is, belongs here rather than in a front end: the GUI and the CLI must
//! give the same answer, and the answer comes from the hunk format, not from a
//! file name.

use ade_object::hunk;
pub use ade_object::listing::{Line, LineKind, Listing};

use crate::Image;

/// How to treat a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Disassemble it only if it is an AmigaDOS executable.
    Auto,
    /// Decode it from its first byte whatever it is, headed as a guess.
    Raw,
}

/// Why a file was not disassembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotCode(pub String);

impl core::fmt::Display for NotCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotCode {}

/// A file's contents as a listing.
///
/// # Errors
/// [`NotCode`] in [`Mode::Auto`] for anything that is not an executable,
/// saying what it begins with — the view shows that, and offers [`Mode::Raw`].
pub fn file(bytes: &[u8], mode: Mode) -> Result<Listing, NotCode> {
    if mode == Mode::Raw {
        return Ok(ade_object::listing::raw(bytes));
    }
    if bytes.is_empty() {
        return Err(NotCode("the file is empty".to_owned()));
    }
    if !hunk::is_executable(bytes) {
        return Err(NotCode(
            "not an AmigaDOS executable: it does not begin with a hunk header (000003F3)"
                .to_owned(),
        ));
    }
    let (notes, problems) = crate::project::load(bytes);
    let mut listing = ade_object::listing::executable_with(bytes, &notes).ok_or_else(|| {
        NotCode("it begins like an executable, but its hunk header is unreadable".to_owned())
    })?;
    say_whose(&mut listing, bytes, &notes, &problems);
    Ok(listing)
}

/// Say at the top of a listing that names and comments in it are the
/// user's, from which file, and what of that file could not be read.
fn say_whose(listing: &mut Listing, program: &[u8], notes: &Annotations, problems: &[String]) {
    let mut said: Vec<String> = problems.iter().map(|p| format!("; project: {p}")).collect();
    if !notes.is_empty()
        && let Some(dir) = crate::project::location()
    {
        said.insert(
            0,
            format!(
                "; names and comments are yours, from {}",
                crate::project::file_in(&dir, program).display()
            ),
        );
    }
    for (n, text) in said.into_iter().enumerate() {
        listing.lines.insert(
            n,
            Line {
                kind: LineKind::Note,
                offset: None,
                length: 0,
                text,
                comment: None,
            },
        );
    }
}

/// The image's boot code as a listing.
///
/// # Errors
/// [`NotCode`] when the image's first block is not an AmigaDOS bootblock — a
/// hard disk's is its partition table, an extended ADF's is its own header.
pub fn bootblock(image: &Image) -> Result<Listing, NotCode> {
    let boot = image.read_range(0, 1024);
    if boot.len() < 1024 || boot.get(..3) != Some(b"DOS".as_slice()) {
        return Err(NotCode(
            "this image does not begin with an AmigaDOS bootblock".to_owned(),
        ));
    }
    let (notes, problems) = crate::project::load(&boot);
    let mut listing = ade_object::listing::bootblock_with(&boot, &notes);
    say_whose(&mut listing, &boot, &notes, &problems);
    Ok(listing)
}

/// One of the views beside the listing (D-021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Every function the walks found: where, how big, who calls it.
    Functions,
    /// Every place something refers to, and what refers to it, how.
    References,
    /// Who calls whom, as an indented tree from where the program starts.
    CallGraph,
    /// The call graph for Graphviz.
    CallGraphDot,
    /// Source that assembles back to the program (executables only).
    Source,
    /// Every string outside the code, and what refers to it.
    Strings,
    /// Every place the code reads, writes or takes the address of by name.
    Variables,
}

pub use ade_object::annotations::Annotations;
pub use ade_object::export::Export;
pub use ade_object::flow::{Analysis, Function, LibraryCall, Place, RefKind, Reference};

/// The label a view writes for a place: `h0_001c`, hunk then offset, as the
/// exported source labels it.
#[must_use]
pub fn place_name((k, at): Place) -> String {
    ade_object::export::label(k, at)
}

/// The executable's code walked, for the views.
///
/// # Errors
/// [`NotCode`] as [`file()`] gives it in [`Mode::Auto`].
pub fn analysis(bytes: &[u8]) -> Result<Analysis, NotCode> {
    file(bytes, Mode::Auto)?;
    ade_object::flow::executable(bytes)
        .ok_or_else(|| NotCode("its hunk header is unreadable".to_owned()))
}

/// The bootblock's code walked, for the views.
///
/// # Errors
/// [`NotCode`] as [`bootblock`] gives it.
pub fn bootblock_analysis(image: &Image) -> Result<Analysis, NotCode> {
    bootblock(image)?;
    Ok(ade_object::flow::bootblock(&image.read_range(0, 1024)))
}

/// A view of a file, as lines of text.
///
/// # Errors
/// [`NotCode`] for what is not an executable.
pub fn file_view(bytes: &[u8], view: View) -> Result<Vec<String>, NotCode> {
    use ade_object::program;
    let (notes, _) = crate::project::load(bytes);
    let n = &notes;
    if matches!(view, View::Strings | View::Variables) {
        let a = analysis(bytes)?;
        let unreadable = || NotCode("its hunk header is unreadable".to_owned());
        return Ok(if view == View::Strings {
            strings(&program::strings(bytes, &a).ok_or_else(unreadable)?, &a, n)
        } else {
            variables(
                &program::variables(bytes, &a).ok_or_else(unreadable)?,
                &a,
                n,
            )
        });
    }
    if view == View::Source {
        file(bytes, Mode::Auto)?;
        let e = ade_object::export::executable_with(bytes, n).ok_or_else(|| {
            NotCode("none of its hunks can be read, so there is nothing to rebuild".to_owned())
        })?;
        return Ok(e.source.lines().map(str::to_owned).collect());
    }
    Ok(render_with(&analysis(bytes)?, view, n))
}

/// A view of the image's boot code, as lines of text.
///
/// # Errors
/// [`NotCode`] for an image with no bootblock, and for [`View::Source`],
/// which is for executables: boot code has no hunks to rebuild.
pub fn bootblock_view(image: &Image, view: View) -> Result<Vec<String>, NotCode> {
    if view == View::Source {
        return Err(NotCode(
            "boot code is not a program file, so there is no source to rebuild it from".to_owned(),
        ));
    }
    let a = bootblock_analysis(image)?;
    let boot = image.read_range(0, 1024);
    let (notes, _) = crate::project::load(&boot);
    let n = &notes;
    Ok(match view {
        View::Strings => strings(&ade_object::program::boot_strings(&boot, &a), &a, n),
        View::Variables => variables(&ade_object::program::boot_variables(&boot, &a), &a, n),
        _ => render_with(&a, view, n),
    })
}

/// An analysis as the lines of one view.
///
/// The views that need the program's bytes as well — source, strings and
/// variables — come from [`file_view`] and [`bootblock_view`], and are empty
/// here.
#[must_use]
pub fn render(a: &Analysis, view: View) -> Vec<String> {
    render_with(a, view, &Annotations::default())
}

/// [`render`], each place called by the user's name for it (D-022).
#[must_use]
pub fn render_with(a: &Analysis, view: View, n: &Annotations) -> Vec<String> {
    match view {
        View::Functions => functions(a, n),
        View::References => references(a, n),
        View::CallGraph => call_tree(a, n),
        View::CallGraphDot => call_dot(a, n),
        View::Source | View::Strings | View::Variables => Vec::new(),
    }
}

/// References, grouped by how they refer: `read from h0_0010, h0_0024;
/// write from h0_0030`.
fn how_referred(refs: &[Reference], n: &Annotations) -> String {
    let mut by_kind: std::collections::BTreeMap<RefKind, Vec<String>> =
        std::collections::BTreeMap::new();
    for r in refs {
        by_kind.entry(r.kind).or_default().push(n.label(r.from));
    }
    by_kind
        .iter()
        .map(|(k, from)| {
            // A long list is cut: the cross-references view has it whole.
            let shown: Vec<String> = from.iter().take(8).cloned().collect();
            let more = from.len().saturating_sub(shown.len());
            let tail = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            format!("{} from {}{tail}", k.name(), shown.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn strings(texts: &[ade_object::program::Text], a: &Analysis, n: &Annotations) -> Vec<String> {
    let used = texts.iter().filter(|t| !t.references.is_empty()).count();
    let mut out = vec![format!(
        "; {} string{} outside the code the walks reached; {used} of them referred to by the code",
        texts.len(),
        if texts.len() == 1 { "" } else { "s" },
    )];
    caveats(a, &mut out);
    out.push(
        "; one nothing refers to is listed only if it looks like text: it may be data that does"
            .to_owned(),
    );
    out.push(String::new());
    for t in texts {
        let mut line = format!("{:<12}{}", n.label(t.at), t.quoted());
        if !t.references.is_empty() {
            line = format!("{line:<60}  ; {}", how_referred(&t.references, n));
        }
        if t.also_code {
            line.push_str("; a walk also read it as code, so the listing there is likely wrong");
        }
        out.push(line);
    }
    out
}

fn variables(vars: &[ade_object::program::Variable], a: &Analysis, n: &Annotations) -> Vec<String> {
    let mut out = vec![format!(
        "; {} variable{}: places the code reads, writes or takes the address of by name",
        vars.len(),
        if vars.len() == 1 { "" } else { "s" },
    )];
    caveats(a, &mut out);
    out.push(
        "; one reached only through a register — a small-data base like $1c(a4) — is not seen"
            .to_owned(),
    );
    out.push(String::new());
    out.push(format!(
        "{:<12}{:<6}{:<11}{:>6}{:>7}{:>9}  referred to",
        "place", "hunk", "size", "reads", "writes", "address"
    ));
    for v in vars {
        let size = if v.widths.is_empty() {
            "-".to_owned()
        } else {
            v.widths
                .iter()
                .map(|w| match w {
                    1 => "byte",
                    2 => "word",
                    _ => "long",
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        out.push(format!(
            "{:<12}{:<6}{:<11}{:>6}{:>7}{:>9}  {}",
            n.label(v.at),
            v.kind.name().to_ascii_lowercase(),
            size,
            v.reads,
            v.writes,
            v.addresses,
            how_referred(&v.references, n)
        ));
    }
    out
}

/// What a walk says about itself: what it could not follow, and a stop.
fn caveats(a: &Analysis, out: &mut Vec<String>) {
    let indirect: usize = a.functions.iter().map(|f| f.indirect).sum();
    if indirect > 0 {
        out.push(format!(
            "; {indirect} call{} or jump{} through a register or memory, which no walk can follow: \
             what they reach is not listed",
            if indirect == 1 { "" } else { "s" },
            if indirect == 1 { "" } else { "s" },
        ));
    }
    if a.truncated {
        out.push(format!(
            "; the walks stopped after {} steps",
            ade_object::flow::MAX_STEPS
        ));
    }
}

fn function_name(f: &Function, n: &Annotations) -> String {
    let mut name = n.label(f.start);
    if let Some(symbol) = &f.name {
        name = format!("{name} {symbol}");
    }
    if f.entry {
        name.push_str(" (entry)");
    }
    name
}

fn functions(a: &Analysis, n: &Annotations) -> Vec<String> {
    let mut out = vec![format!(
        "; {} function{}, found by following the code from where it starts; {} library call{} \
         whose library the code proves",
        a.functions.len(),
        if a.functions.len() == 1 { "" } else { "s" },
        a.library_calls.len(),
        if a.library_calls.len() == 1 { "" } else { "s" },
    )];
    caveats(a, &mut out);
    out.push(String::new());
    out.push(format!(
        "{:<28}{:>7}{:>14}{:>9}{:>7}{:>10}",
        "function", "bytes", "instructions", "callers", "calls", "library"
    ));
    for f in &a.functions {
        out.push(format!(
            "{:<28}{:>7}{:>14}{:>9}{:>7}{:>10}",
            function_name(f, n),
            f.bytes,
            f.instructions,
            f.callers.len(),
            f.calls.len(),
            f.library_calls.len()
        ));
    }
    if !a.library_calls.is_empty() {
        out.push(String::new());
        out.push("; library calls".to_owned());
        for c in &a.library_calls {
            out.push(format!("{:<12}{}", n.label(c.at), c.describe()));
        }
    }
    out
}

fn references(a: &Analysis, n: &Annotations) -> Vec<String> {
    let targets: std::collections::BTreeSet<Place> = a.references.iter().map(|r| r.to).collect();
    let mut out = vec![format!(
        "; {} place{} referred to, by {} reference{} from code the walks reached and from \
         relocated data",
        targets.len(),
        if targets.len() == 1 { "" } else { "s" },
        a.references.len(),
        if a.references.len() == 1 { "" } else { "s" },
    )];
    caveats(a, &mut out);
    out.push(String::new());
    for to in targets {
        let mut by_kind: std::collections::BTreeMap<RefKind, Vec<String>> =
            std::collections::BTreeMap::new();
        for r in a.references_to(to) {
            by_kind.entry(r.kind).or_default().push(n.label(r.from));
        }
        let what: Vec<String> = by_kind
            .iter()
            .map(|(k, from)| format!("{} from {}", k.name(), from.join(", ")))
            .collect();
        let mut name = n.label(to);
        if a.function(to).is_some() {
            name.push_str(" (function)");
        }
        out.push(format!("{name:<22}{}", what.join("; ")));
    }
    out
}

/// The library calls a function makes, as `exec: Forbid, Permit`.
fn library_leaves(a: &Analysis, f: &Function) -> Vec<String> {
    let mut by_library: std::collections::BTreeMap<&str, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for at in &f.library_calls {
        if let Some(c) = a.library_call(*at) {
            let name = c.function.map_or_else(
                || ade_object::m68k::number(i64::from(c.offset)),
                str::to_owned,
            );
            by_library
                .entry(c.library.short())
                .or_default()
                .insert(name);
        }
    }
    by_library
        .iter()
        .map(|(l, fs)| format!("{l}: {}", fs.iter().cloned().collect::<Vec<_>>().join(", ")))
        .collect()
}

/// Lines a call tree may hold before it stops.
const MAX_TREE: usize = 20_000;

fn call_tree(a: &Analysis, n: &Annotations) -> Vec<String> {
    let mut out = vec![
        "; who calls whom, from where the program starts; a function already shown is not \
         shown again"
            .to_owned(),
    ];
    caveats(a, &mut out);
    out.push(String::new());
    let mut shown = std::collections::BTreeSet::new();
    let roots: Vec<&Function> = a.functions.iter().filter(|f| f.entry).collect();
    for root in roots {
        // Depth first, with an explicit stack: a deep chain of calls must not
        // become a deep Rust stack (D-006).
        let mut stack: Vec<(Place, usize)> = vec![(root.start, 0)];
        while let Some((at, depth)) = stack.pop() {
            if out.len() >= MAX_TREE {
                out.push(format!("; stopped after {MAX_TREE} lines"));
                return out;
            }
            let Some(f) = a.function(at) else { continue };
            let indent = "  ".repeat(depth);
            if !shown.insert(at) {
                out.push(format!("{indent}{} (above)", function_name(f, n)));
                continue;
            }
            out.push(format!("{indent}{}", function_name(f, n)));
            for leaf in library_leaves(a, f) {
                out.push(format!("{indent}  - {leaf}"));
            }
            for callee in f.calls.iter().rev() {
                stack.push((*callee, depth.saturating_add(1)));
            }
        }
    }
    out
}

fn call_dot(a: &Analysis, n: &Annotations) -> Vec<String> {
    let mut out = vec![
        "digraph calls {".to_owned(),
        "  node [shape=box, fontname=\"monospace\"];".to_owned(),
    ];
    for f in &a.functions {
        let shape = if f.entry { ", style=bold" } else { "" };
        out.push(format!(
            "  \"{}\" [label=\"{}\"{shape}];",
            n.label(f.start),
            function_name(f, n).replace('"', "'")
        ));
        for callee in &f.calls {
            out.push(format!(
                "  \"{}\" -> \"{}\";",
                n.label(f.start),
                n.label(*callee)
            ));
        }
        for leaf in library_leaves(a, f) {
            out.push(format!(
                "  \"{0}\" -> \"{0}:{1}\"; \"{0}:{1}\" [label=\"{1}\", shape=note];",
                n.label(f.start),
                leaf.replace('"', "'")
            ));
        }
    }
    out.push("}".to_owned());
    out
}

pub use ade_object::program::{Class, Segment};

/// A program as runs of reached code, unreached code, strings, data and
/// zeroed memory, for a bar beside the listing.
///
/// # Errors
/// [`NotCode`] as [`analysis`] gives it.
pub fn file_segments(bytes: &[u8]) -> Result<Vec<Segment>, NotCode> {
    let a = analysis(bytes)?;
    ade_object::program::segments(bytes, &a)
        .ok_or_else(|| NotCode("its hunk header is unreadable".to_owned()))
}

/// The boot code as runs, as [`file_segments`] gives a program's.
///
/// # Errors
/// [`NotCode`] as [`bootblock`] gives it.
pub fn bootblock_segments(image: &Image) -> Result<Vec<Segment>, NotCode> {
    let a = bootblock_analysis(image)?;
    Ok(ade_object::program::boot_segments(
        &image.read_range(0, 1024),
        &a,
    ))
}
