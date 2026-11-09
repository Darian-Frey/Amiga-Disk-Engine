//! Strings and variables on real programs (F-036).
//!
//! Each is checked by what the walks predict. A name the walks saw given to
//! `OpenLibrary` was read from the program's bytes at the place `a1` held,
//! so it must be in the strings, at that place, exactly, and referred to.
//! And a 68000 faults on a word or long read or written at an odd address,
//! so a variable the code reads or writes that way would mean the walks
//! followed a branch into data. The segment bar's runs must tile every
//! hunk. Local only; skips without the corpus.
//! `ADE_DISASM_CORPUS=all` takes every executable; by default every 10th.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::Image;
use ade_core::layers::object::flow::RefKind;
use ade_core::layers::object::{flow, hunk, program};

#[test]
fn strings_hold_every_library_name_and_variables_sit_where_a_68000_can_reach_them() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let step = if std::env::var("ADE_DISASM_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        10
    };
    let (mut programs, mut seen) = (0usize, 0usize);
    let (mut strings, mut referred, mut names, mut found) = (0usize, 0usize, 0usize, 0usize);
    let (mut variables, mut sized, mut odd) = (0usize, 0usize, 0usize);
    let mut segments = 0usize;
    let mut missing = Vec::new();
    let mut odd_examples = Vec::new();
    for path in &paths {
        let Ok(image) = Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !hunk::is_executable(&bytes) {
                continue;
            }
            seen += 1;
            if seen % step != 0 {
                continue;
            }
            let Some(a) = flow::executable(&bytes) else {
                continue;
            };
            programs += 1;
            let texts = program::strings(&bytes, &a).unwrap();
            strings += texts.len();
            referred += texts.iter().filter(|t| !t.references.is_empty()).count();
            for call in a.library_calls.iter().filter(|c| c.opens.is_some()) {
                let opened = call.opens.as_deref().unwrap();
                if !opened.ends_with(".library") {
                    continue;
                }
                names += 1;
                let hit = texts.iter().any(|t| {
                    t.bytes == opened.as_bytes()
                        && t.terminated
                        && t.references.iter().any(|r| r.kind == RefKind::Address)
                });
                if hit {
                    found += 1;
                } else if missing.len() < 10 {
                    missing.push(format!("{}: {name}: {opened}", path.display()));
                }
            }
            // The runs tile each hunk: from its first byte to the end of its
            // memory, in order, with no gap and no overlap.
            let exe = hunk::parse(&bytes).unwrap();
            let runs = program::segments(&bytes, &a).unwrap();
            for (k, h) in exe.hunks.iter().enumerate() {
                let mine: Vec<_> = runs.iter().filter(|r| r.hunk == k).collect();
                let mut next = 0u32;
                for r in &mine {
                    assert_eq!(r.start, next, "{}: {name}: hunk {k}", path.display());
                    next = r.start + r.length;
                }
                let end = u32::try_from(h.file_len).unwrap().max(h.size);
                assert_eq!(next, end, "{}: {name}: hunk {k}", path.display());
            }
            segments += runs.len();
            for v in program::variables(&bytes, &a).unwrap() {
                variables += 1;
                let accessed = v.reads + v.writes > 0;
                if accessed && v.widths.iter().any(|&w| w > 1) {
                    sized += 1;
                    if v.at.1 % 2 == 1 {
                        odd += 1;
                        if odd_examples.len() < 10 {
                            odd_examples.push(format!("{}: {name}: {:?}", path.display(), v.at));
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "{programs} programs: {strings} strings, {referred} referred to; {found} of {names} \
         library names found; {variables} variables, {odd} of {sized} word or long ones at odd \
         addresses; {segments} runs"
    );
    eprintln!("missing: {missing:#?}");
    eprintln!("odd: {odd_examples:#?}");
    assert!(programs > 0 && names > 0);
    assert_eq!(found, names, "{missing:#?}");
    assert!(odd * 1000 <= sized, "{odd} of {sized}: {odd_examples:#?}");
}
