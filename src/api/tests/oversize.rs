//! A volume can be smaller than the file that holds it (BUG-009).
//!
//! A drive could generally seek past cylinder 79, so images of 81, 82 and 83
//! cylinders occur. Those are **not larger volumes**: they are ordinary
//! 80-cylinder filesystems in files carrying extra tracks, and their rootblock
//! is still at 880. Computing it from the file's own block count lands a
//! reader on block 902 for a 1804-block image, where it finds a file header.
//!
//! Measured before it was fixed: five corpus images were unmountable this way,
//! four oversized and one truncated. ADFlib mounts the oversized ones as
//! `Floppy 880 KBytes ... between sectors [0-1759]`, which is what settled
//! what the right answer was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over images they construct"
)]

use ade_core::Image;
use ade_fixtures::Volume as Fixture;

/// A sound DD volume with `extra` bytes of track data after it.
fn with_extra_tracks(extra: usize) -> Vec<u8> {
    let mut v = Fixture::dd(1).named("Oversize");
    v.add_file("readme", b"still here");
    let mut bytes = v.build();
    assert_eq!(bytes.len(), 901_120, "a DD floppy to start with");
    // Not zeros: an extra cylinder holds whatever the drive wrote there, and
    // zeros would let a bug pass by looking like absence.
    bytes.extend(std::iter::repeat_n(0xA5u8, extra));
    bytes
}

#[test]
fn a_disk_with_extra_cylinders_mounts_as_the_volume_it_holds() {
    // 81, 82 and 83 cylinders, the counts that occur in the wild.
    for cylinders in [81usize, 82, 83] {
        let extra = (cylinders - 80) * 2 * 11 * 512;
        let image = Image::from_bytes(with_extra_tracks(extra)).expect("opens");

        assert_eq!(
            image.geometry().total_bytes(),
            901_120,
            "{cylinders} cylinders: the volume is the 880 KB one inside"
        );
        assert_eq!(image.geometry().root_block().0, 880, "{cylinders}");

        let volume = image.volume().expect("mounts");
        assert_eq!(volume.rootblock().name_lossy(), "Oversize", "{cylinders}");
        let walk = volume.walk(volume.root()).expect("walks");
        assert!(
            walk.entries.iter().any(|(p, _)| p == "readme"),
            "{cylinders}: and its contents are readable"
        );
    }
}

#[test]
fn an_ordinary_disk_is_not_touched() {
    // The adoption tries the file's own geometry first, so nothing that
    // mounted before can mount differently now. This is the property that
    // matters more than the rescue: measured over 4,652 corpus images, five
    // began mounting and **none stopped**.
    let mut v = Fixture::dd(1).named("Normal");
    v.add_file("readme", b"unchanged");
    let bytes = v.build();
    let image = Image::from_bytes(bytes).expect("opens");

    assert_eq!(image.geometry().total_bytes(), 901_120);
    assert_eq!(image.volume().unwrap().rootblock().name_lossy(), "Normal");
}

#[test]
fn an_hd_disk_with_extra_tracks_stays_an_hd_disk() {
    // Candidates are tried largest first. Tried the other way round, an
    // oversized HD image would be read as a DD volume the moment block 880
    // happened to hold something rootblock-shaped — and on an HD disk block
    // 880 is ordinary data.
    let mut v = Fixture::new(80, 2, 22, 1).named("HighDensity");
    v.add_file("readme", b"high density");
    let mut bytes = v.build();
    assert_eq!(bytes.len(), 1_802_240);
    bytes.extend(std::iter::repeat_n(0xA5u8, 2 * 22 * 512));

    let image = Image::from_bytes(bytes).expect("opens");
    assert_eq!(image.geometry().total_bytes(), 1_802_240, "still HD");
    assert_eq!(image.geometry().root_block().0, 1760);
    assert_eq!(
        image.volume().unwrap().rootblock().name_lossy(),
        "HighDensity"
    );
}

#[test]
fn a_file_with_no_volume_anywhere_is_still_refused() {
    // The adoption may rescue an image but must never invent one. A candidate
    // is taken only when a real rootblock is actually at its position.
    let bytes = vec![0xA5u8; 923_648];
    let image = Image::from_bytes(bytes).expect("opens as a container");
    assert!(
        image.volume().is_err(),
        "nothing here is a volume, and saying so is the answer"
    );
}

#[test]
fn a_truncated_dump_is_not_rescued_and_that_is_the_honest_answer() {
    // One corpus image is this shape: 1738 blocks, TOSEC-tagged `[u]` for
    // underdumped, with its rootblock at 880. The volume it belongs to is a DD
    // floppy, and the file cannot cover one — mounting it would mean claiming
    // an extent the bytes do not have. A file that does not contain a whole
    // volume is reported as not containing one.
    let mut v = Fixture::dd(1).named("Cut");
    v.add_file("readme", b"most of a disk");
    let mut bytes = v.build();
    bytes.truncate(1738 * 512);

    if let Ok(image) = Image::from_bytes(bytes) {
        assert!(
            image.geometry().total_bytes() < 901_120,
            "if it opens at all, it is not claiming to be a whole DD floppy"
        );
        assert!(image.volume().is_err(), "and it holds no mountable volume");
    }
}
