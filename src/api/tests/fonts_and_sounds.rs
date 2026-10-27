//! Bitmap fonts and IFF 8SVX sounds in the Decoded view (F-033), across the
//! corpus.
//!
//! The layouts are AROS's; the unit tests in `ade_object::font` and
//! `ade_object::svx` build files to them. The corpus adds two checks no
//! hand-built file can make. A font's contents file and each size's file
//! are written separately and both state the size's height, so where they
//! agree the reading of both is right. And a sound's header says how many
//! bytes of samples follow, so a misread header shows as a body that does
//! not match. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::layers::object::{font, svx};

#[test]
fn corpus_fonts_and_sounds_decode_and_agree_with_themselves() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut contents, mut sizes, mut agree, mut listed) = (0usize, 0usize, 0usize, 0usize);
    let (mut fonts, mut fonts_refused, mut specimens) = (0usize, 0usize, 0usize);
    let (mut sounds, mut exact, mut waveforms) = (0usize, 0usize, 0usize);
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
            if font::is_contents(&entry.name, &bytes) {
                let Ok(list) = font::contents(&bytes) else {
                    continue;
                };
                contents += 1;
                let folder = name.rfind('/').map_or("", |i| &name[..=i]);
                for size in &list.sizes {
                    let file = format!("{folder}{}", String::from_utf8_lossy(&size.file));
                    let Ok(e) = volume.lookup(&file) else {
                        continue;
                    };
                    let Ok(c) = volume.read_file(&e) else {
                        continue;
                    };
                    if let Ok(f) = font::parse(&c.into_bytes()) {
                        listed += 1;
                        agree += usize::from(f.height == size.height);
                    }
                }
                sizes += list.sizes.len();
            }
            if font::is_font(&bytes) {
                match ade_core::decode::file(&entry.name, &bytes) {
                    Ok(d) => {
                        fonts += 1;
                        assert!(d.title.starts_with("Font"), "{}", d.title);
                        if let Some(p) = d.picture {
                            specimens += 1;
                            assert_eq!(p.argb.len(), p.width * p.height);
                        }
                    }
                    Err(_) => fonts_refused += 1,
                }
            }
            if svx::is_svx(&bytes) {
                let d = ade_core::decode::file(&entry.name, &bytes).unwrap();
                assert!(d.title.starts_with("IFF 8SVX"), "{}", d.title);
                sounds += 1;
                waveforms += usize::from(d.picture.is_some());
                let s = svx::parse(&bytes).unwrap();
                exact += usize::from(s.body.0 == s.body.1);
            }
        }
    }
    eprintln!(
        "{contents} contents files listing {sizes} sizes, {listed} of them on the disk: \
         {agree} agree on the height; {fonts} font files decoded, {fonts_refused} refused, \
         {specimens} drawn; {sounds} sounds, {exact} with the body their header describes, \
         {waveforms} drawn"
    );
    // Measured 2026-10-27: 1,426 contents files; 2,552 of 2,561 sizes agree
    // on the height; 2,575 font files, one refused; 2,599 sounds, 2,525 with
    // the body their header describes.
    assert!(contents > 1_400, "{contents}");
    assert!(agree * 100 >= listed * 99, "{agree} of {listed}");
    assert!(
        fonts > 2_500 && fonts_refused <= 2,
        "{fonts} {fonts_refused}"
    );
    assert!(sounds > 2_500, "{sounds}");
    assert!(exact * 100 >= sounds * 95, "{exact} of {sounds}");
}
