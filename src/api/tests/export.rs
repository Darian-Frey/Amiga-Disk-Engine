//! Exported programs assemble back to themselves (D-021).
//!
//! Every executable on every corpus disk is exported, assembled with GNU
//! `as` in its MRI mode, linked with every section at address zero — where
//! the file's relocated values count from — and each hunk's bytes taken back
//! out with objcopy. They must be the file's bytes exactly: every instruction
//! written as one encodes as it was, every relocated long names the place it
//! points at, and every label sits where it says.
//!
//! GNU `as` takes a `SECTION` without the Devpac type the export writes, so
//! the type is dropped here; it decides only where a loader puts a hunk, not
//! its bytes. Local only: the corpus is not in the repository (D-010), and
//! the tools are `m68k-linux-gnu-as`, `-ld` and `-objcopy`. Any missing, the
//! test skips. Every other program is exported with a name at every place
//! the code refers to and a comment on every function (D-022), and must
//! rebuild all the same. A second test does the same with vasm (IMP-020), which also
//! sees the relocations — linked at zero, a dropped relocation leaves every
//! byte right (BUG-020). `ADE_DISASM_CORPUS=all` takes every executable; by default
//! every 10th. Measured 2026-11-08 with all: 39,348 programs, every hunk
//! rebuilt by both, in about fifteen minutes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::Image;
use ade_core::layers::object::annotations::Annotations;
use ade_core::layers::object::hunk::{self, Kind, Memory};
use ade_core::layers::object::{export, flow};

fn tool(name: &str) -> Option<String> {
    let candidate = format!("m68k-linux-gnu-{name}");
    Command::new(&candidate)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| candidate)
}

/// Names and comments for every other program (D-022): a name for every
/// function start and every place the code refers to, a comment on every
/// function — far more than anybody would write, so that names land
/// everywhere a label can, and the source must still rebuild.
fn notes_for(bytes: &[u8], n: usize) -> Annotations {
    let mut notes = Annotations::default();
    if n % 2 == 0 {
        return notes;
    }
    let Some(a) = flow::executable(bytes) else {
        return notes;
    };
    for f in &a.functions {
        let (k, at) = f.start;
        notes
            .set_name(f.start, Some(&format!("fn_{k}_{at:x}")))
            .unwrap();
        notes.set_comment(f.start, Some("a function")).unwrap();
    }
    for r in &a.references {
        let (k, at) = r.to;
        if !notes.names.contains_key(&r.to) {
            notes
                .set_name(r.to, Some(&format!("v_{k}_{at:x}")))
                .unwrap();
        }
    }
    notes
}

/// Every executable on every corpus disk, as (where, bytes).
fn executables(corpus: &Path, step: usize) -> Vec<(String, Vec<u8>)> {
    let mut images: Vec<PathBuf> = std::fs::read_dir(corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    images.sort();
    let mut found = Vec::new();
    let mut seen = 0usize;
    for path in images {
        let Ok(image) = Image::open(&path) else {
            continue;
        };
        let Ok(volume) = image.volume() else {
            continue;
        };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries {
            if !entry.kind.is_file() {
                continue;
            }
            let Ok(contents) = volume.read_file(&entry) else {
                continue;
            };
            let bytes = contents.into_bytes();
            if !hunk::is_executable(&bytes) {
                continue;
            }
            seen += 1;
            if seen % step == 0 {
                let disk = path.file_name().unwrap().to_string_lossy().into_owned();
                found.push((format!("{disk}: {name}"), bytes));
            }
        }
    }
    found
}

/// Assemble, link and split an export; each section's bytes, or why not.
fn build(
    source: &str,
    hunks: usize,
    dir: &Path,
    tools: &(String, String, String),
) -> Result<Vec<Vec<u8>>, String> {
    let (gas, ld, objcopy) = tools;
    // Devpac's typed SECTION is not GNU's; the type does not touch bytes.
    // Nor is Devpac's `OPT O-`, which MRI mode refuses; it optimises none of
    // what the export writes as instructions anyway.
    let source: String = source
        .lines()
        .filter(|l| !l.starts_with("\tOPT\t"))
        .map(|l| match l.strip_prefix("\tSECTION\t") {
            Some(rest) => format!("\tSECTION\t{}\n", rest.split(',').next().unwrap()),
            None => format!("{l}\n"),
        })
        .collect();
    std::fs::write(dir.join("p.s"), source).unwrap();
    let run = |cmd: &mut Command| -> Result<(), String> {
        let out = cmd.current_dir(dir).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(2000)
                .collect())
        }
    };
    run(Command::new(gas).args([
        "-M",
        "-m68000",
        "--register-prefix-optional",
        "-o",
        "p.o",
        "p.s",
    ]))?;
    let mut script = String::from("SECTIONS {\n");
    for k in 0..hunks {
        let _ = writeln!(script, "  hunk{k} 0 : {{ *(hunk{k}) }}");
    }
    script.push_str("  /DISCARD/ : { *(*) }\n}\n");
    std::fs::write(dir.join("p.ld"), script).unwrap();
    run(Command::new(ld).args(["--no-check-sections", "-T", "p.ld", "-o", "p.elf", "p.o"]))?;
    let mut out = Vec::new();
    for k in 0..hunks {
        let file = format!("h{k}.bin");
        let _ = std::fs::remove_file(dir.join(&file));
        // An MRI section carries no flags, and objcopy writes only what
        // would be loaded.
        let section = format!("hunk{k}");
        run(Command::new(objcopy).args([
            "-O",
            "binary",
            "-j",
            &section,
            "--set-section-flags",
            &format!("{section}=alloc,load,contents"),
            "p.elf",
            &file,
        ]))?;
        out.push(std::fs::read(dir.join(&file)).unwrap_or_default());
    }
    Ok(out)
}

#[test]
fn every_exported_program_assembles_back_to_its_own_bytes() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("skipping: no corpus");
        return;
    }
    let (Some(gas), Some(ld), Some(objcopy)) = (tool("as"), tool("ld"), tool("objcopy")) else {
        eprintln!("skipping: no m68k binutils");
        return;
    };
    let tools = (gas, ld, objcopy);
    let step = if std::env::var("ADE_DISASM_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        10
    };
    let dir = std::env::temp_dir().join(format!("ade-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (mut programs, mut instructions, mut words) = (0usize, 0usize, 0usize);
    let mut failures = Vec::new();
    for (n, (place, bytes)) in executables(&corpus, step).into_iter().enumerate() {
        let Some(e) = export::executable_with(&bytes, &notes_for(&bytes, n)) else {
            continue;
        };
        let hunks = export::hunk_bytes(&bytes).unwrap();
        programs += 1;
        instructions += e.instructions;
        words += e.as_words;
        match build(&e.source, hunks.len(), &dir, &tools) {
            Err(why) => {
                failures.push(format!("{place}: {why}"));
                if std::env::var_os("ADE_EXPORT_KEEP").is_some() {
                    let n = failures.len();
                    std::fs::copy(dir.join("p.s"), dir.join(format!("err{n}.s"))).unwrap();
                }
            }
            Ok(built) => {
                for (k, (kind, want)) in &hunks {
                    let got = &built[*k];
                    let got = &got[..got.len().min(want.len())];
                    if *kind != Kind::Bss && got != want.as_slice() {
                        let at = got
                            .iter()
                            .zip(want)
                            .position(|(a, b)| a != b)
                            .unwrap_or(got.len());
                        failures.push(format!(
                            "{place}: hunk {k} differs at ${at:x} of ${:x}",
                            want.len()
                        ));
                        if std::env::var_os("ADE_EXPORT_KEEP").is_some() {
                            let n = failures.len();
                            std::fs::copy(dir.join("p.s"), dir.join(format!("fail{n}.s"))).unwrap();
                            std::fs::write(dir.join(format!("fail{n}.h{k}.at{at}")), want).unwrap();
                        }
                        break;
                    }
                }
            }
        }
    }
    if std::env::var_os("ADE_EXPORT_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!(
        "{programs} programs, {instructions} instructions written as instructions, {words} as words"
    );
    assert!(
        failures.is_empty(),
        "{} of {programs} failed:\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(programs > 0);
}

/// vasm, from `ADE_VASM` or `vasmm68k_mot` on the path.
fn vasm() -> Option<String> {
    let candidate = std::env::var("ADE_VASM").unwrap_or_else(|_| "vasmm68k_mot".to_owned());
    // vasm has no `--version`; given no input it says so and fails, which
    // is enough to know it runs.
    Command::new(&candidate)
        .arg("-quiet")
        .output()
        .ok()
        .filter(|o| String::from_utf8_lossy(&o.stderr).contains("no input file"))
        .map(|_| candidate)
}

/// One hunk as the loader sees it: kind, memory, the bytes it loads, and
/// its relocations as (offset, target hunk).
type Loaded = (Kind, Memory, Vec<u8>, BTreeSet<(u32, u32)>);

/// A program as the loader sees it: each hunk's kind, memory, the bytes it
/// loads — the file's, then zeros to its size — and its relocations. Those
/// the export says it cannot rebuild are left out: odd, into a missing
/// hunk, or past the end of their own.
fn loaded(bytes: &[u8]) -> Option<Vec<Loaded>> {
    let exe = hunk::parse(bytes)?;
    let count = u32::try_from(exe.hunks.len()).ok()?;
    Some(
        exe.hunks
            .iter()
            .map(|h| {
                let mut memory = bytes
                    .get(h.file_offset..h.file_offset + h.file_len)
                    .unwrap_or(&[])
                    .to_vec();
                memory.resize(memory.len().max(h.size as usize), 0);
                let end = (h.size as usize).max(h.file_len);
                let relocs = h
                    .relocs
                    .iter()
                    .filter(|r| {
                        r.target < count && r.offset % 2 == 0 && r.offset as usize + 4 <= end
                    })
                    .map(|r| (r.offset, r.target))
                    .collect();
                // The export cannot say "both flags and a further long";
                // that hunk is rebuilt as one for any memory.
                let mem = if h.memory == Memory::Extended {
                    Memory::Any
                } else {
                    h.memory
                };
                (h.kind, mem, memory, relocs)
            })
            .collect(),
    )
}

/// The export, given to vasm, must build an executable the loader cannot
/// tell from the original (IMP-020) — as Devpac would take it, and with
/// vasm's defaults. This checks what the GNU round
/// trip cannot: the section types, the chip and fast flags, and that vasm
/// relocates every address the original's loader would.
#[test]
fn every_exported_program_rebuilds_with_vasm() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("skipping: no corpus");
        return;
    }
    let Some(vasm) = vasm() else {
        eprintln!("skipping: no vasmm68k_mot (set ADE_VASM)");
        return;
    };
    let step = if std::env::var("ADE_DISASM_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        10
    };
    let dir = std::env::temp_dir().join(format!("ade-vasm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let keep = std::env::var_os("ADE_EXPORT_KEEP").is_some();
    let mut programs = 0usize;
    let mut failures = Vec::new();
    for (n, (place, bytes)) in executables(&corpus, step).into_iter().enumerate() {
        let Some(e) = export::executable_with(&bytes, &notes_for(&bytes, n)) else {
            continue;
        };
        let want = loaded(&bytes).unwrap();
        programs += 1;
        std::fs::write(dir.join("p.s"), &e.source).unwrap();
        for options in VASM_OPTIONS {
            let _ = std::fs::remove_file(dir.join("p.exe"));
            let out = Command::new(&vasm)
                .args(options)
                .args(["-o", "p.exe", "p.s"])
                .current_dir(&dir)
                .output()
                .unwrap();
            let why = if out.status.success() {
                let built = std::fs::read(dir.join("p.exe")).unwrap_or_default();
                match loaded(&built) {
                    None => Some("vasm's output does not parse".to_owned()),
                    Some(got) if got.len() != want.len() => {
                        Some(format!("{} hunks, not {}", got.len(), want.len()))
                    }
                    Some(got) => got.iter().zip(&want).enumerate().find_map(|(k, (g, w))| {
                        if (g.0, g.1) != (w.0, w.1) {
                            Some(format!(
                                "hunk {k}: {:?} {:?}, not {:?} {:?}",
                                g.0, g.1, w.0, w.1
                            ))
                        } else if g.2 != w.2 {
                            let at = g.2.iter().zip(&w.2).position(|(a, b)| a != b);
                            Some(format!(
                                "hunk {k}: {} bytes, not {}; first difference at {at:x?}",
                                g.2.len(),
                                w.2.len()
                            ))
                        } else if g.3 != w.3 {
                            let extra: Vec<_> = g.3.difference(&w.3).take(3).collect();
                            let lost: Vec<_> = w.3.difference(&g.3).take(3).collect();
                            Some(format!(
                                "hunk {k}: relocations differ: added {extra:x?}, lost {lost:x?}"
                            ))
                        } else {
                            None
                        }
                    }),
                }
            } else {
                Some(
                    String::from_utf8_lossy(&out.stderr)
                        .chars()
                        .take(1500)
                        .collect(),
                )
            };
            if let Some(why) = why {
                failures.push(format!("{place} ({}): {why}", options.join(" ")));
                if keep {
                    let n = failures.len();
                    std::fs::copy(dir.join("p.s"), dir.join(format!("fail{n}.s"))).unwrap();
                    std::fs::write(dir.join(format!("fail{n}.orig")), &bytes).unwrap();
                }
                break;
            }
        }
    }
    if !keep {
        let _ = std::fs::remove_dir_all(&dir);
    }
    eprintln!("{programs} programs rebuilt by vasm");
    assert!(
        failures.is_empty(),
        "{} of {programs} failed:\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(programs > 0);
}

/// How vasm is run: an executable out, no symbols, no banner — once as
/// Devpac would take the source, and once with vasm's own defaults, which
/// optimise unless the source says not to. The export's `OPT O-` is what
/// makes the second agree with the first.
const VASM_OPTIONS: [&[&str]; 2] = [
    &["-Fhunkexe", "-devpac", "-nosym", "-quiet"],
    &["-Fhunkexe", "-nosym", "-quiet"],
];
