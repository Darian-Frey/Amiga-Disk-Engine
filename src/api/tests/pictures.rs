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
    clippy::panic,
    clippy::indexing_slicing,
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

/// What ffmpeg draws of an IFF picture, as `0xRRGGBB` per pixel; `None` when
/// ffmpeg cannot be run or cannot read it.
fn ffmpeg_pixels(bytes: &[u8], scratch: &std::path::Path) -> Option<Vec<u32>> {
    let input = scratch.join("in.iff");
    let output = scratch.join("out.rgb");
    std::fs::write(&input, bytes).ok()?;
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "quiet", "-y", "-i"])
        .arg(&input)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .arg(&output)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let rgb = std::fs::read(&output).ok()?;
    // A 4-bit colour map, which ffmpeg leaves as $F0 and ADE widens to $FF
    // as AROS does: the only difference the two are allowed.
    let four_bit = rgb.iter().all(|&b| b.trailing_zeros() >= 4);
    Some(
        rgb.chunks_exact(3)
            .map(|c| {
                let widen = |v: u8| u32::from(if four_bit { v | (v >> 4) } else { v });
                (widen(c[0]) << 16) | (widen(c[1]) << 8) | widen(c[2])
            })
            .collect(),
    )
}

#[test]
fn every_corpus_acbm_is_drawn_as_ffmpeg_draws_it() {
    // An ACBM keeps its planes whole, in ABIT; AROS does not read it, and
    // ffmpeg does, independently. Measured 2026-10-29: all 83 corpus ACBM
    // pictures identical to ffmpeg's, pixel for pixel — seven of them only
    // once each chunk was read to its own length past a short FORM
    // (BUG-019).
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_err()
    {
        eprintln!("skipping: no ffmpeg");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("ade-acbm-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut found, mut identical) = (0usize, 0usize);
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
            if bytes.get(8..12) != Some(b"ACBM") || !ilbm::is_ilbm(&bytes) {
                continue;
            }
            found += 1;
            let ours = ilbm::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let theirs = ffmpeg_pixels(&bytes, &scratch)
                .unwrap_or_else(|| panic!("{}: {name}: ffmpeg could not draw it", path.display()));
            let ours: Vec<u32> = ours.argb.iter().map(|p| p & 0x00FF_FFFF).collect();
            assert!(
                ours == theirs,
                "{}: {name} differs from ffmpeg",
                path.display()
            );
            identical += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!("{found} ACBM pictures, {identical} identical to ffmpeg's");
    assert!(found >= 83, "{found}");
    assert_eq!(identical, found);
}
