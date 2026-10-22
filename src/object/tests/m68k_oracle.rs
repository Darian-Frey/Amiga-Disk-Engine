//! The 68000 decoder against GNU objdump (F-032, D-019).
//!
//! D-002's bargain on a second format: the decoder is checked against an
//! implementation nobody here wrote, used by the tests and never linked or
//! read. Every one of the 65,536 opcode words is decoded by both, twice — once
//! followed by `4E71` extension words and once by `D040`, so displacements and
//! immediates are seen positive and negative, and index registers as data and
//! address registers, word and long.
//!
//! objdump prints GNU's MIT syntax (`%fp@(-552)`) where ADE prints Motorola's
//! (`-$228(a6)`), so the comparison is made on structure: `Instruction::gnu`
//! renders ADE's decoded instruction the way objdump would, and the text must
//! match exactly. The words skipped are `objdump_differs`, each with its
//! reason.
//!
//! The oracle is `m68k-linux-gnu-objdump` (Debian/Ubuntu
//! `binutils-m68k-linux-gnu`) on `PATH`, or whatever `ADE_M68K_OBJDUMP` names.
//! Without it the test skips, so a fresh clone still builds and tests.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::process::Command;

use ade_object::m68k::{Decoded, decode, objdump_differs};

/// Bytes per record in the sweep: an opcode word, extension words, padding.
/// Longer than any 68000 instruction, so every record starts on a boundary
/// whatever the one before it decoded to.
const RECORD: usize = 16;

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

/// objdump's line for each record, by opcode word.
fn oracle(tool: &str, pad: u16) -> BTreeMap<u16, String> {
    let mut bytes = Vec::with_capacity(65_536 * RECORD);
    for op in 0..=u16::MAX {
        bytes.extend_from_slice(&[(op >> 8) as u8, op as u8]);
        for _ in 1..RECORD / 2 {
            bytes.extend_from_slice(&[(pad >> 8) as u8, pad as u8]);
        }
    }
    let path = std::env::temp_dir().join(format!("ade-m68k-{}-{pad:04x}.bin", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let out = Command::new(tool)
        .args(["-D", "-z", "-b", "binary", "-m", "m68k:68000"])
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = BTreeMap::new();
    for line in text.lines() {
        // "   1234:\t7000           \tmoveq #0,%d0"
        let mut fields = line.split('\t');
        let (Some(addr), Some(_), Some(insn)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Ok(addr) = u32::from_str_radix(addr.trim().trim_end_matches(':'), 16) else {
            continue;
        };
        if addr as usize % RECORD == 0 {
            lines.insert((addr as usize / RECORD) as u16, insn.trim().to_owned());
        }
    }
    lines
}

#[test]
fn every_opcode_word_decodes_as_objdump_decodes_it() {
    let Some(tool) = objdump() else {
        eprintln!("skipping: no m68k objdump (install binutils-m68k-linux-gnu)");
        return;
    };
    // The brief index word in each pattern sets the scale field: 4E71 to 8,
    // D040 to 1. A 68000 ignores it; objdump shows it, and so does ADE.
    for pad in [0x4E71u16, 0xD040] {
        let theirs = oracle(&tool, pad);
        assert_eq!(theirs.len(), 65_536, "objdump output did not parse");
        let mut mismatches: Vec<String> = Vec::new();
        let mut compared = 0usize;
        for op in 0..=u16::MAX {
            if objdump_differs(op).is_some() {
                continue;
            }
            let mut bytes = vec![(op >> 8) as u8, op as u8];
            for _ in 1..RECORD / 2 {
                bytes.extend_from_slice(&[(pad >> 8) as u8, pad as u8]);
            }
            let address = u32::from(op) * RECORD as u32;
            let mine = match decode(&bytes, address) {
                Decoded::Instruction(i) => i.gnu(),
                Decoded::Invalid(w) => format!(".short {w:#06x}"),
                Decoded::Needs68020(w) => format!("68020 {w:#06x}"),
                Decoded::Truncated => "truncated".to_owned(),
            };
            compared += 1;
            if mine != theirs[&op] {
                mismatches.push(format!("{op:04x}: ADE `{mine}` objdump `{}`", theirs[&op]));
            }
        }
        assert!(compared > 60_000);
        assert!(
            mismatches.is_empty(),
            "pad {pad:04x}: {} of {compared} words disagree; first 40:\n{}",
            mismatches.len(),
            mismatches
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
