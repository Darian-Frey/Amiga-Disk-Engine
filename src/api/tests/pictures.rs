//! IFF ILBM pictures in the Decoded view (F-033), across the corpus.
//!
//! The reading is AROS's `ilbm.datatype`'s; the unit tests in
//! `ade_object::ilbm` build pictures to it. What the corpus adds is the check
//! a hand-built picture cannot make: that a real `BODY`, unpacked row by row
//! as the header describes, ends where the chunk ends — a misread row length,
//! plane count or ByteRun1 rule leaves bytes over or runs out early, on
//! thousands of files at once. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::layers::object::ilbm;

#[test]
fn every_corpus_picture_decodes_and_uses_its_body() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut files, mut pictures, mut palettes, mut exact) = (0usize, 0usize, 0usize, 0usize);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let mut modes: BTreeMap<&str, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (_, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(contents) = volume.read_file(entry) else {
                continue;
            };
            if !contents.is_complete() || !contents.faults.is_empty() {
                continue;
            }
            let bytes = contents.into_bytes();
            if !ilbm::is_ilbm(&bytes) {
                continue;
            }
            files += 1;
            match ade_core::decode::file(&entry.name, &bytes) {
                Ok(d) if d.picture.is_none() => palettes += 1,
                Ok(d) => {
                    pictures += 1;
                    let p = d.picture.unwrap();
                    assert_eq!(p.argb.len(), p.width * p.height);
                    let i = ilbm::parse(&bytes).unwrap();
                    // The pad byte of an odd-length chunk is not counted.
                    if i.body_left <= 1 {
                        exact += 1;
                    }
                    let m = i.mode.unwrap_or(0);
                    *modes
                        .entry(if i.depth == 24 {
                            "true colour"
                        } else if m & 0x800 != 0 {
                            "hold-and-modify"
                        } else if m & 0x80 != 0 {
                            "extra-half-brite"
                        } else {
                            "indexed"
                        })
                        .or_default() += 1;
                }
                Err(e) => {
                    let why: String = e.0.chars().take_while(|c| !c.is_ascii_digit()).collect();
                    *refused.entry(why).or_default() += 1;
                }
            }
        }
    }
    eprintln!(
        "{files} intact ILBM files: {pictures} pictures ({exact} using their BODY exactly), \
         {palettes} colour maps; {modes:?}; refused {refused:?}"
    );
    // Measured 2026-10-26: 4,664 pictures, 4,637 using their BODY to the
    // byte; refusals are game formats wearing an ILBM header (other
    // compressions, no BMHD, bodies packed some other way).
    assert!(pictures > 4_500, "{pictures}");
    assert!(exact * 100 >= pictures * 99, "{exact} of {pictures}");
    assert!(modes.get("hold-and-modify").copied().unwrap_or(0) > 50);
    assert!(modes.get("extra-half-brite").copied().unwrap_or(0) > 100);
}
