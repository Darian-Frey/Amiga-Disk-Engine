//! IFF ANIM animations in the Decoded view (F-033), across the corpus.
//!
//! The reading is the ANIM document's; the unit tests in `ade_object::anim`
//! build animations to it. Two independent checks, one for each kind. An
//! ordinary animation — changes stored into the frame two back — is played
//! by ffmpeg too, and every frame must be ffmpeg's, pixel for pixel. A DPaint
//! Anim Brush — changes XORed into the frame before — ffmpeg does not read,
//! as the document says no player of its day did; but the document says its
//! last change brings back its first frame, and that is checked instead.
//! Measured 2026-10-29: 29 ordinary animations identical to ffmpeg in every
//! frame, and 45 brushes ending on their first frame. Local only; skips
//! without the corpus or without ffmpeg.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::cast_possible_truncation,
    reason = "test scaffolding; a pixel's channels taken apart by casting"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::layers::object::anim;

/// Every frame ffmpeg draws, as RGB bytes, one output frame per frame read.
fn ffmpeg_frames(bytes: &[u8], scratch: &Path) -> Option<Vec<u8>> {
    let input = scratch.join("in.anim");
    let output = scratch.join("out.rgb");
    std::fs::write(&input, bytes).ok()?;
    let ok = Command::new("ffmpeg")
        .args(["-v", "quiet", "-y", "-i"])
        .arg(&input)
        .args([
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
        ])
        .arg(&output)
        .status()
        .ok()?
        .success();
    ok.then(|| std::fs::read(&output).ok()).flatten()
}

/// ADE's frame against ffmpeg's: the same, or different only as the two are
/// known to differ — a 4-bit colour map ADE widens, `$F0` to `$FF`, as
/// AROS does, across the picture or, in hold-and-modify, colour by colour.
fn agree(ours: &[u8], theirs: &[u8]) -> bool {
    let widen = |x: u8| x | (x >> 4);
    ours == theirs
        || (theirs.iter().all(|&x| x.trailing_zeros() >= 4)
            && ours.iter().zip(theirs).all(|(&a, &b)| a == widen(b)))
        || ours
            .iter()
            .zip(theirs)
            .all(|(&a, &b)| a == if b.trailing_zeros() >= 4 { widen(b) } else { b })
}

#[test]
fn corpus_animations_play_as_ffmpeg_plays_them_and_brushes_come_back_round() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("ade-anims-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut ordinary, mut brushes) = (0usize, 0usize);
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
            if !anim::is_anim(&bytes) {
                continue;
            }
            let mut frames: Vec<Vec<u8>> = Vec::new();
            let a = anim::play(&bytes, |_, argb| {
                frames.push(
                    argb.iter()
                        .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
                        .collect(),
                );
                true
            })
            .unwrap_or_else(|e| panic!("{}: {name}: {e}", path.display()));
            let _ = a.fields();
            if a.brush {
                assert_eq!(
                    frames.last(),
                    frames.first(),
                    "{}: {name}: a brush's last change must bring back its first frame",
                    path.display()
                );
                brushes += 1;
                continue;
            }
            let theirs = ffmpeg_frames(&bytes, &scratch)
                .unwrap_or_else(|| panic!("{}: {name}: ffmpeg could not play it", path.display()));
            let size = frames.first().map_or(0, Vec::len);
            assert_eq!(
                theirs.len(),
                size * frames.len(),
                "{}: {name}: frame count",
                path.display()
            );
            for (n, (ours, theirs)) in frames
                .iter()
                .zip(theirs.chunks_exact(size.max(1)))
                .enumerate()
            {
                assert!(
                    agree(ours, theirs),
                    "{}: {name}: frame {n} differs from ffmpeg",
                    path.display()
                );
            }
            ordinary += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!("{ordinary} animations identical to ffmpeg, {brushes} brushes coming back round");
    assert!(ordinary >= 29 && brushes >= 45, "{ordinary} and {brushes}");
}
