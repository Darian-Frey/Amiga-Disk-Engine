//! IFF FTXT text and datatype descriptors in the Decoded view (F-033),
//! across the corpus.
//!
//! The readings are the FTXT document's and AROS's `datatypes/datatypes.h`;
//! the unit tests in `ade_object::ftxt` and `ade_object::datatype` build
//! files to them. The corpus adds a check on descriptors: an IFF datatype
//! recognises its files by `FORM`, any four bytes, and its own form type, so
//! its mask must spell its ID — which it does only if the mask's offset,
//! length and words are all read right. Measured 2026-10-29: 101 descriptors
//! in six groups and 8 texts, every one read. Local only; skips without the
//! corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::layers::object::{datatype, ftxt};

#[test]
fn corpus_texts_read_and_iff_datatypes_spell_their_form_type() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut texts, mut descriptors, mut iff) = (0usize, 0usize, 0usize);
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
            if let Some(t) = ftxt::parse(&bytes) {
                let _ = t.fields();
                texts += 1;
            }
            if !datatype::is_descriptor(&bytes) {
                continue;
            }
            let d = datatype::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let _ = d.fields();
            descriptors += 1;
            assert!(
                [
                    b"syst", b"text", b"docu", b"soun", b"inst", b"musi", b"pict", b"anim", b"movi"
                ]
                .contains(&&d.group),
                "{}: {name}: group {:?}",
                path.display(),
                d.group
            );
            if d.flags & 0x0F == 2 {
                let spelled: Vec<Option<u16>> = b"FORM"
                    .iter()
                    .map(|&c| Some(u16::from(c)))
                    .chain([None; 4])
                    .chain(
                        d.id.iter()
                            .map(|&c| Some(u16::from(c.to_ascii_uppercase()))),
                    )
                    .collect();
                assert_eq!(d.mask, spelled, "{}: {name}", path.display());
                iff += 1;
            }
        }
    }
    eprintln!(
        "{texts} texts, {descriptors} descriptors, {iff} of them IFF types spelling their form type"
    );
    assert!(
        texts >= 8 && descriptors >= 101,
        "{texts} and {descriptors}"
    );
    assert!(iff > 0);
}
