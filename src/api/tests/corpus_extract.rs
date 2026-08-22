//! Walk and extract across the local corpus, when one is present.
//!
//! Generated fixtures prove ADE implements the specification; this proves it
//! survives contact with real disks, which the survey showed differ (D-010).
//! Skips cleanly when `disks/` is absent, so a fresh clone passes offline.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test scaffolding over data it controls"
)]

use std::{collections::BTreeMap, fs, path::PathBuf};

use ade_core::Image;

fn corpus() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    d.is_dir().then_some(d)
}

/// A deterministic sample, so a failure is reproducible.
fn sample(paths: &mut Vec<PathBuf>, n: usize) {
    paths.sort();
    let step = (paths.len() / n.max(1)).max(1);
    let picked: Vec<_> = paths.iter().step_by(step).take(n).cloned().collect();
    *paths = picked;
}

#[test]
fn extracts_every_file_from_a_corpus_sample() {
    let Some(root) = corpus() else {
        eprintln!("no corpus — skipping (D-010: images are never committed)");
        return;
    };
    let mut paths: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    let total = paths.len();
    sample(&mut paths, 400);

    let mut disks = 0usize;
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut short: BTreeMap<String, (u32, usize)> = BTreeMap::new();
    let mut read_errors = 0usize;
    let mut cycles = 0usize;

    for path in &paths {
        let Ok(image) = Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        disks += 1;
        let Ok(walked) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walked {
            if !entry.kind.is_file() {
                continue;
            }
            match volume.read_file(&entry) {
                Ok(data) => {
                    files += 1;
                    bytes += data.bytes.len() as u64;
                    if !data.is_complete() {
                        let disk = path.file_name().unwrap().to_string_lossy().into_owned();
                        short.insert(
                            format!("{disk}:{name}"),
                            (entry.byte_size, data.bytes.len()),
                        );
                    }
                }
                Err(e) => {
                    read_errors += 1;
                    if matches!(e, ade_filesystem::volume::FsError::Cycle { .. }) {
                        cycles += 1;
                    }
                }
            }
        }
    }

    eprintln!(
        "corpus: {total} images, {} sampled, {disks} mounted",
        paths.len()
    );
    eprintln!("  files extracted: {files}  ({:.1} MB)", bytes as f64 / 1e6);
    eprintln!("  read errors: {read_errors} (of which cycles: {cycles})");
    eprintln!("  size mismatches: {}", short.len());
    for (k, (want, got)) in short.iter().take(10) {
        eprintln!("    {k}: declared {want}, got {got}");
    }

    assert!(disks > 50, "expected to mount many disks, mounted {disks}");
    assert!(files > 500, "expected many files, extracted {files}");

    // A file shorter than its declared size means the data blocks ran out:
    // truncation, a broken chain, or a bug here. It should be rare — a high
    // rate would mean the reader is wrong rather than the disks damaged.
    let rate = short.len() as f64 / files as f64;
    assert!(
        rate < 0.02,
        "{} of {files} files came up short ({:.2}%) — a reader bug, not disk damage",
        short.len(),
        rate * 100.0
    );
}
