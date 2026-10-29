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
//! test skips. `ADE_DISASM_CORPUS=all` takes every executable; by default
//! every 10th. Measured 2026-10-29 with all: 39,363 programs, every hunk
//! rebuilt, in about nine minutes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::Image;
use ade_core::layers::object::export;
use ade_core::layers::object::hunk::{self, Kind};

fn tool(name: &str) -> Option<String> {
    let candidate = format!("m68k-linux-gnu-{name}");
    Command::new(&candidate)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| candidate)
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
    let source: String = source
        .lines()
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
    for (place, bytes) in executables(&corpus, step) {
        let Some(e) = export::executable(&bytes) else {
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
