//! What a file is, for its icon in the tree, against what its name and place
//! say it should be: `.info` files are icons, `c/` holds commands, `.library`
//! files are libraries, `s/startup-sequence` is text. The kind is read from
//! contents, so a name is independent evidence. Measured 2026-11-10 over
//! every 10th corpus disk; local only, and skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::case_sensitive_file_extension_comparisons,
    reason = "test scaffolding; the names compared are lowered first"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::Image;
use ade_core::decode::{FileKind, kind};

#[test]
fn a_files_kind_agrees_with_what_its_name_says() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut all: BTreeMap<&str, usize> = BTreeMap::new();
    // For each expectation: how many files the name puts there, how many the
    // contents agree with, and a few that do not.
    let mut checks: BTreeMap<&str, (usize, usize, Vec<String>)> = BTreeMap::new();
    for path in paths.iter().step_by(10) {
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
            let k = kind(&entry.name, &bytes);
            *all.entry(k.name()).or_default() += 1;
            let lower = name.to_ascii_lowercase();
            // The older `$F34C` `.info` is not an icon but Workbench 1.x's
            // record of a drawer, listing its files' names; with no source to
            // read it from (F-033), it is data, and not counted here.
            let old_record = bytes.starts_with(&[0xF3, 0x4C]);
            let expect: Option<(&str, &[FileKind])> =
                if lower.ends_with(".info") && !bytes.is_empty() && !old_record {
                    Some((".info is an icon", &[FileKind::Icon]))
                } else if lower.starts_with("c/") && !bytes.is_empty() {
                    Some(("c/ holds programs", &[FileKind::Program, FileKind::Packed]))
                } else if lower.ends_with(".library") && !bytes.is_empty() {
                    Some((
                        ".library is a library",
                        &[FileKind::Library, FileKind::Packed],
                    ))
                } else if lower == "s/startup-sequence" {
                    Some(("s/startup-sequence is text", &[FileKind::Text]))
                } else {
                    None
                };
            if let Some((what, kinds)) = expect {
                let e = checks.entry(what).or_default();
                e.0 += 1;
                if kinds.contains(&k) {
                    e.1 += 1;
                } else if e.2.len() < 5 {
                    e.2.push(format!("{}: {name}: {}", path.display(), k.name()));
                }
            }
        }
    }
    eprintln!("kinds: {all:?}");
    for (what, (n, agree, odd)) in &checks {
        eprintln!("{what}: {agree} of {n}; e.g. {odd:#?}");
    }
    for (what, (n, agree, _)) in &checks {
        assert!(*n > 0, "{what}: none in the sample");
        // Nineteen in twenty: a name is evidence, not proof — a `c/` file
        // can be a script, a `.info` can be empty or damaged.
        assert!(agree * 20 >= n * 19, "{what}: {agree} of {n}");
    }
}
