//! `FILE_ID.DIZ` surfacing (Phase 3, F-011).
//!
//! A BBS-era convention: a short description written by whoever released the
//! disk, usually ASCII art naming the group, the title and which disk of the
//! set this is. It is the closest thing a floppy has to a label, and it is
//! often far more informative than the volume name — one corpus disk is
//! labelled `Empty` while its `FILE_ID.DIZ` reads
//! `TEST MATCH CRiCKET SAVE DISK (NEEDED) [2/2]`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::{MAX_DESCRIPTION, inspect_bytes};
use ade_fixtures::Volume as Fixture;

/// A disk carrying a description file under the given name.
fn disk_with(name: &str, body: &[u8]) -> Vec<u8> {
    let mut v = Fixture::dd(1).named("Release");
    v.add_file(name, body);
    v.add_file("other", b"not the description");
    v.build()
}

#[test]
fn a_description_is_read_and_reported() {
    let body = b"  TITLE OF THE GAME [1/2]\n  released by SOMEONE\n";
    let inspection = inspect_bytes(disk_with("FILE_ID.DIZ", body));
    let description = inspection.description.expect("description found");

    assert_eq!(description.file, "FILE_ID.DIZ");
    assert_eq!(description.text, String::from_utf8_lossy(body));
    assert_eq!(description.declared_size, body.len() as u32);
    assert!(!description.truncated);
}

#[test]
fn every_spelling_in_the_corpus_is_found() {
    // All three occur on real disks. AmigaDOS filenames are not case
    // sensitive (C-006), so this is one case-insensitive lookup rather than a
    // list of names to try.
    for name in ["FILE_ID.DIZ", "file_id.diz", "File_ID.Diz"] {
        let inspection = inspect_bytes(disk_with(name, b"a description of the disk"));
        let description = inspection
            .description
            .unwrap_or_else(|| panic!("{name} not found"));

        // The name is reported as stored, not as searched for: what is on the
        // disk is a fact about the disk.
        assert_eq!(description.file, name);
    }
}

#[test]
fn a_disk_without_one_reports_nothing() {
    let mut v = Fixture::dd(1).named("Plain");
    v.add_file("readme", b"not a description file");
    assert!(inspect_bytes(v.build()).description.is_none());
}

#[test]
fn a_directory_of_that_name_is_not_a_description() {
    // Reading a directory block as text would produce confident nonsense.
    let mut v = Fixture::dd(1).named("Tricky");
    v.add_dir("FILE_ID.DIZ");
    assert!(inspect_bytes(v.build()).description.is_none());
}

#[test]
fn an_oversized_description_is_truncated_and_says_so() {
    // The length comes off the disk, so it is capped rather than trusted
    // (AV-005, and BUG-003's lesson). Real ones are a few hundred bytes; the
    // largest in the corpus is 356.
    let body = vec![b'x'; MAX_DESCRIPTION * 2];
    let inspection = inspect_bytes(disk_with("FILE_ID.DIZ", &body));
    let description = inspection.description.expect("description found");

    assert_eq!(description.text.len(), MAX_DESCRIPTION);
    assert!(description.truncated, "truncation must be reported");
    // The declared size is still the real one — the cap is on what is read,
    // not on what is claimed.
    assert_eq!(description.declared_size, body.len() as u32);
}

#[test]
fn latin1_art_survives_intact() {
    // Release art is full of high-bit box-drawing characters. Decoding them as
    // UTF-8 would mangle the art, which here is the content.
    let body = &[
        0xB0, 0xB1, 0xDB, b'|', 0xA6, b'\n', b'T', b'I', b'T', b'L', b'E',
    ];
    let inspection = inspect_bytes(disk_with("FILE_ID.DIZ", body));
    let description = inspection.description.expect("description found");

    let chars: Vec<u32> = description.text.chars().map(|c| c as u32).collect();
    assert_eq!(
        chars[0], 0xB0,
        "Latin-1 byte should map to the same code point"
    );
    assert_eq!(chars[2], 0xDB);
    assert!(description.text.ends_with("TITLE"));
}

#[test]
fn an_empty_description_is_still_a_description() {
    // A zero-byte FILE_ID.DIZ says something: someone meant to put one there.
    let inspection = inspect_bytes(disk_with("FILE_ID.DIZ", b""));
    let description = inspection.description.expect("found");

    assert!(description.text.is_empty());
    assert_eq!(description.declared_size, 0);
}

#[test]
fn the_json_carries_it() {
    let inspection = inspect_bytes(disk_with("FILE_ID.DIZ", b"RELEASE NOTES HERE"));
    let json = inspection.to_json().to_json();

    for field in [
        "\"description\"",
        "\"file\":\"FILE_ID.DIZ\"",
        "\"declared_size\":18",
        "\"truncated\":false",
    ] {
        assert!(json.contains(field), "missing {field}\n{json}");
    }
}

#[test]
fn a_disk_that_does_not_mount_reports_no_description() {
    // No volume, no files, no description — and no panic on the way there.
    let bytes = vec![0u8; 901_120];
    assert!(inspect_bytes(bytes).description.is_none());
}
