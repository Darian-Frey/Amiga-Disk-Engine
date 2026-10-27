//! Workbench icons in the Decoded view (F-033), across the corpus.
//!
//! The layout is AROS's `icon.library` reader's; the unit tests in
//! `ade_object::icon` build icons to it. What the corpus adds is the check a
//! hand-built icon cannot make: that the order and the conditions are right
//! on icons real Workbenches saved, which shows as the parts accounting for
//! every byte of the file. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::layers::object::icon;

#[test]
fn every_corpus_icon_decodes_and_accounts_for_its_bytes() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut icons, mut decoded, mut whole, mut failed) = (0usize, 0usize, 0usize, Vec::new());
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(contents) = volume.read_file(entry) else {
                continue;
            };
            let intact = contents.is_complete() && contents.faults.is_empty();
            let bytes = contents.into_bytes();
            if !icon::is_icon(&bytes) {
                continue;
            }
            icons += 1;
            // Through the view's own entry point, as the window and the CLI
            // reach it.
            match ade_core::decode::file(&entry.name, &bytes) {
                Ok(d) => {
                    decoded += 1;
                    assert!(d.title.starts_with("Workbench icon"), "{}", d.title);
                    let i = icon::parse(&bytes).unwrap();
                    *kinds.entry(i.kind.describe()).or_default() += 1;
                    if i.trailing == 0 {
                        whole += 1;
                    }
                    if let Some(p) = d.picture {
                        assert_eq!(p.argb.len(), p.width * p.height);
                    }
                }
                Err(e) if intact => failed.push(format!(
                    "{}: {name}: {e}",
                    path.file_name().unwrap().to_string_lossy()
                )),
                Err(_) => {}
            }
        }
    }
    eprintln!(
        "{icons} icons: {decoded} decoded, {whole} with every byte accounted for; {} intact \
         files refused; {kinds:?}",
        failed.len()
    );
    for f in &failed {
        eprintln!("  {f}");
    }
    // Measured 2026-10-26: 12,826 icons, 12,817 decoded, 5 intact files
    // refused — each ends partway through a part the header says follows.
    assert!(icons > 12_000, "{icons}");
    assert!(decoded * 1000 >= icons * 999, "{decoded} of {icons}");
    assert!(whole * 1000 >= decoded * 999, "{whole} of {decoded}");
    assert!(failed.len() <= 10, "{failed:?}");
}
