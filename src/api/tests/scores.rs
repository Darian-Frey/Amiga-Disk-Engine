//! IFF SMUS scores in the Decoded view (F-033), across the corpus.
//!
//! The reading is the SMUS document's; the unit tests in `ade_object::smus`
//! build scores to it. There is no other reader here to compare with, but a
//! score checks itself: its tracks play together, so they mostly last the
//! same number of quarter notes to the 26,880th — which they do only if
//! every duration bit is read right. Measured 2026-10-29: 257 scores, and of
//! the 245 with more than one track, 171 with every track the same length;
//! the rest stop a track early rather than write its final rests. Reading
//! dotted notes wrong leaves 57, triplets wrong 128. Local only; skips
//! without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::layers::object::smus;

#[test]
fn corpus_scores_read_and_their_tracks_end_together() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut scores, mut several, mut together) = (0usize, 0usize, 0usize);
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
            if !smus::is_score(&bytes) {
                continue;
            }
            let s = smus::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let _ = s.fields();
            scores += 1;
            let lengths: Vec<u64> = s
                .tracks
                .iter()
                .filter(|t| t.notes + t.rests > 0)
                .map(|t| t.length)
                .collect();
            if lengths.len() > 1 {
                several += 1;
                if lengths.windows(2).all(|w| w.first() == w.last()) {
                    together += 1;
                }
            }
        }
    }
    eprintln!("{scores} scores; {together} of {several} with their tracks the same length");
    assert!(scores >= 257, "{scores}");
    assert!(together >= 171, "{together} of {several}");
}
