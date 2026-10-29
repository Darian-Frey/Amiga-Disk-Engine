//! Locale catalogs in the Decoded view (F-033), across the corpus.
//!
//! The reading is AROS's `OpenCatalogA()`; the unit tests in
//! `ade_object::catalog` build catalogs to it. The corpus adds the check a
//! built file cannot: each string entry is padded to an even length and then
//! to four, and a padding rule read wrong desynchronises every entry after
//! the first odd one, leaving bytes over or running past the chunk.
//! Measured 2026-10-29: 1,860 catalogs in eleven languages, every `STRS`
//! chunk used exactly. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use ade_core::layers::object::catalog;

#[test]
fn corpus_catalogs_use_their_strings_chunk_exactly() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut catalogs, mut strings) = (0usize, 0usize);
    let mut languages = BTreeSet::new();
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
            if !c.is_complete() || !c.faults.is_empty() {
                continue;
            }
            let bytes = c.into_bytes();
            if !catalog::is_catalog(&bytes) {
                continue;
            }
            let cat = catalog::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let _ = cat.fields();
            assert_eq!(
                (cat.overrun, cat.left),
                (0, 0),
                "{}: {name}: its strings do not fill STRS exactly",
                path.display()
            );
            catalogs += 1;
            strings += cat.strings.len();
            languages.insert(cat.language.clone());
        }
    }
    eprintln!("{catalogs} catalogs, {strings} strings, languages {languages:?}");
    assert!(catalogs >= 1860, "{catalogs}");
    assert!(languages.len() >= 11, "{languages:?}");
}
