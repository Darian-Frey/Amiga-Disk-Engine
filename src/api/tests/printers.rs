//! Printer drivers in the Decoded view (F-033), across the corpus.
//!
//! The layout is AROS's `devices/prtbase.h`; the unit tests in
//! `ade_object::printer` build drivers to it. The corpus adds the printers:
//! a driver names the printer it drives, and the file is named for the
//! printer too, so a misplaced name pointer or a misread structure shows as
//! names that do not agree. Measured 2026-10-29: 2,173 drivers, every one
//! read, and 2,154 naming the printer their file is named for — the rest
//! renamed copies, third-party drivers and one test build. Local only; skips
//! without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::layers::object::printer;

/// Letters and digits, lower case: `CBM_MPS1000` and `CBM MPS1000` alike.
fn plain(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[test]
fn corpus_drivers_name_the_printers_their_files_are_named_for() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut drivers, mut agree) = (0usize, 0usize);
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
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
            if !printer::is_driver(&bytes) {
                continue;
            }
            let d = printer::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let _ = d.fields();
            drivers += 1;
            let file = name.rsplit('/').next().unwrap_or(name);
            let (file, printer) = (
                plain(file.split('.').next().unwrap_or(file)),
                plain(&d.name),
            );
            let head = |s: &str| s.chars().take(5).collect::<String>();
            if file.starts_with(&head(&printer)) || printer.starts_with(&head(&file)) {
                agree += 1;
            }
        }
    }
    eprintln!("{drivers} printer drivers, {agree} naming their file's printer");
    assert!(drivers >= 2173, "{drivers}");
    assert!(agree * 100 >= drivers * 99, "{agree} of {drivers}");
}
