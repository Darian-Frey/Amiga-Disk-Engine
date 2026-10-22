//! Disassembly on real programs (F-032, D-019).
//!
//! The opcode sweep in `ade-object` proves the decoder word by word; this
//! proves it on what Amiga programs actually contain. Every executable on
//! every corpus disk is found, its hunks parsed, and each code hunk decoded
//! by ADE and by GNU objdump from its first byte. The two must agree at every
//! address — which also means they agree on every instruction's length, or
//! the addresses would part company.
//!
//! Local only: the corpus is not in the repository (D-010), and the oracle is
//! `m68k-linux-gnu-objdump` or whatever `ADE_M68K_OBJDUMP` names. Either
//! missing, the test skips. `ADE_DISASM_CORPUS=all` takes every executable;
//! by default every 10th, which keeps an ordinary run quick.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::Image;
use ade_core::layers::object::hunk::{self, Kind};
use ade_core::layers::object::m68k::{Decoded, decode, objdump_differs};

fn objdump() -> Option<String> {
    let candidate =
        std::env::var("ADE_M68K_OBJDUMP").unwrap_or_else(|_| "m68k-linux-gnu-objdump".to_owned());
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

/// objdump's line at each address, for each of several files at once.
fn oracle(tool: &str, files: &[PathBuf]) -> Vec<BTreeMap<u32, String>> {
    let out = Command::new(tool)
        .args(["-D", "-z", "-b", "binary", "-m", "m68k:68000"])
        .args(files)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut result: Vec<BTreeMap<u32, String>> = Vec::new();
    for line in text.lines() {
        if line.contains("file format binary") {
            result.push(BTreeMap::new());
            continue;
        }
        let mut fields = line.split('\t');
        let (Some(addr), Some(_), Some(insn)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(addr) = u32::from_str_radix(addr.trim().trim_end_matches(':'), 16) else {
            continue;
        };
        if let Some(last) = result.last_mut() {
            last.insert(addr, insn.trim().to_owned());
        }
    }
    result
}

#[test]
fn real_code_hunks_decode_as_objdump_decodes_them() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("skipping: no corpus");
        return;
    }
    let Some(tool) = objdump() else {
        eprintln!("skipping: no m68k objdump (install binutils-m68k-linux-gnu)");
        return;
    };
    let step = if std::env::var("ADE_DISASM_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        10
    };
    let programs = executables(&corpus, step);
    assert!(!programs.is_empty(), "the corpus holds executables");

    let scratch = std::env::temp_dir().join(format!("ade-disasm-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();

    // Every code hunk, written out for objdump.
    let mut hunks: Vec<(String, Vec<u8>)> = Vec::new();
    let (mut parsed, mut with_notes, mut hunk_count) = (0usize, 0usize, 0usize);
    for (name, bytes) in &programs {
        let Some(exe) = hunk::parse(bytes) else {
            continue;
        };
        parsed += 1;
        if !exe.notes.is_empty() {
            with_notes += 1;
        }
        for h in &exe.hunks {
            hunk_count += 1;
            if h.kind == Kind::Code && h.file_len > 0 {
                let code = bytes[h.file_offset..h.file_offset + h.file_len].to_vec();
                hunks.push((format!("{name} hunk {}", h.index), code));
            }
        }
    }

    let (mut compared, mut skipped_after, mut invalid, mut code_bytes) =
        (0usize, 0usize, 0usize, 0usize);
    let mut needs_020 = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for batch in hunks.chunks(200) {
        let files: Vec<PathBuf> = batch
            .iter()
            .enumerate()
            .map(|(i, (_, code))| {
                let p = scratch.join(format!("h{i}.bin"));
                std::fs::write(&p, code).unwrap();
                p
            })
            .collect();
        let theirs = oracle(&tool, &files);
        assert_eq!(
            theirs.len(),
            batch.len(),
            "objdump output did not split per file"
        );
        for ((name, code), theirs) in batch.iter().zip(&theirs) {
            code_bytes += code.len();
            let mut at = 0usize;
            while at + 2 <= code.len() {
                let address = u32::try_from(at).unwrap();
                let op = u16::from(code[at]) << 8 | u16::from(code[at + 1]);
                // Where objdump knowingly differs, its idea of the length may
                // differ too, so the rest of this hunk is not comparable.
                if objdump_differs(op).is_some() {
                    skipped_after += 1;
                    break;
                }
                let decoded = decode(&code[at..], address);
                let mine = match &decoded {
                    Decoded::Instruction(i) => i.gnu(),
                    Decoded::Invalid(w) => {
                        invalid += 1;
                        format!(".short {w:#06x}")
                    }
                    // objdump decodes the 68020's full-format index words even
                    // for a 68000, so past one the two cannot be compared.
                    Decoded::Needs68020(_) => {
                        needs_020 += 1;
                        break;
                    }
                    // The last few bytes of a hunk: nothing left to compare.
                    Decoded::Truncated => break,
                };
                compared += 1;
                match theirs.get(&address) {
                    Some(t) if *t == mine => {}
                    // objdump read past the end of the hunk looking for
                    // extension words; ADE stopped at the opcode. The end.
                    Some(t) if t.contains("out of bounds") => break,
                    other => {
                        mismatches.push(format!(
                            "{name} at {address:#x}: ADE `{mine}` objdump `{}`",
                            other.map_or("(no instruction here)", String::as_str)
                        ));
                        break;
                    }
                }
                at += decoded.len() as usize;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);

    eprintln!(
        "{} executables ({parsed} parsed, {with_notes} with notes), {hunk_count} hunks, \
         {} code hunks, {code_bytes} bytes of code; {compared} instructions compared, \
         {invalid} words no 68000 instruction, {needs_020} hunks stopped at a 68020 \
         addressing mode, {skipped_after} at a word objdump reads differently",
        programs.len(),
        hunks.len()
    );
    assert!(
        mismatches.is_empty(),
        "{} code hunks disagree with objdump; first 60:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
