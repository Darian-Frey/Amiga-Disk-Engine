//! Content identification against TOSEC datfiles (Phase 5, F-013).
//!
//! A disk image says almost nothing about itself: its filename is whatever the
//! last person to touch it chose, and its volume label is often `Empty`. What
//! survives is the bytes, so identification means hashing content and asking a
//! dataset what it is.
//!
//! # What these tests are careful about
//!
//! CRC32 is a **content hash, not an identity**. Measured across the real
//! dataset there are 71 collisions among 88,833 entries, and size does not
//! separate them. None involves an `.adf` today, but the property is measured
//! rather than guaranteed — so `identify` returns every match and the tests
//! pin that it does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "tests over data they construct"
)]

use std::fmt::Write as _;
use std::path::PathBuf;

use ade_core::layers::block::checksum::crc32;
use ade_core::layers::catalogue::{Catalogue, parse};

/// A datfile with the given `(name, size, crc)` entries.
fn datfile(entries: &[(&str, u64, u32)]) -> String {
    let mut out =
        String::from("<?xml version=\"1.0\"?>\n<datafile>\n<header><name>Test</name></header>\n");
    for (name, size, crc) in entries {
        let _ = write!(
            out,
            "<game name=\"{name}\">\n<rom name=\"{name}.adf\" size=\"{size}\" crc=\"{crc:08x}\" md5=\"deadbeef\" sha1=\"cafe\"/>\n</game>\n"
        );
    }
    out.push_str("</datafile>\n");
    out
}

#[test]
fn a_datfile_parses_into_entries() {
    let text = datfile(&[("Alpha (1990)(Someone)", 901_120, 0x1234_5678)]);
    let entries = parse(&text, "Test Set");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Alpha (1990)(Someone).adf");
    assert_eq!(entries[0].size, 901_120);
    assert_eq!(entries[0].crc32, 0x1234_5678);
    assert_eq!(entries[0].md5.as_deref(), Some("deadbeef"));
    assert_eq!(entries[0].sha1.as_deref(), Some("cafe"));
    assert_eq!(entries[0].source, "Test Set");
}

#[test]
fn the_ampersand_entity_is_resolved() {
    // The only entity that actually occurs in the Amiga set — 8772 times.
    let text = "<rom name=\"Rock &amp; Roll (1989).adf\" size=\"901120\" crc=\"00000001\"/>";
    let entries = parse(text, "s");

    assert_eq!(entries[0].name, "Rock & Roll (1989).adf");
}

#[test]
fn attributes_are_read_by_name_not_by_position() {
    // Attribute order is not guaranteed by XML, and a positional scanner would
    // silently mis-read a datfile that happened to differ.
    let text = "<rom crc=\"0000002a\" name=\"Backwards.adf\" sha1=\"aa\" size=\"512\"/>";
    let entries = parse(text, "s");

    assert_eq!(entries[0].name, "Backwards.adf");
    assert_eq!(entries[0].crc32, 42);
    assert_eq!(entries[0].size, 512);
}

#[test]
fn an_entry_without_a_crc_is_skipped_not_guessed() {
    let text = "<rom name=\"NoHash.adf\" size=\"901120\"/>\n\
                <rom name=\"Good.adf\" size=\"512\" crc=\"00000005\"/>";
    let entries = parse(text, "s");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Good.adf");
}

#[test]
fn identification_matches_on_content() {
    let bytes = b"the contents of a small disk".to_vec();
    let mut catalogue = Catalogue::default();
    catalogue.add(
        &datfile(&[("Known (1991)(Publisher)", bytes.len() as u64, crc32(&bytes))]),
        "Test Set",
    );

    let matches = catalogue.identify(&bytes);

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "Known (1991)(Publisher).adf");
}

#[test]
fn unknown_content_matches_nothing() {
    let mut catalogue = Catalogue::default();
    catalogue.add(&datfile(&[("Known", 4, 0x1234_5678)]), "s");

    assert!(catalogue.identify(b"never seen before").is_empty());
}

#[test]
fn every_entry_sharing_a_hash_is_returned() {
    // The property that matters. A content hash is not an identity: 71
    // collisions exist in the real dataset, so a caller must be given all of
    // them rather than an arbitrary one.
    let bytes = b"colliding content".to_vec();
    let hash = crc32(&bytes);
    let mut catalogue = Catalogue::default();
    catalogue.add(
        &datfile(&[
            ("First Claimant", bytes.len() as u64, hash),
            ("Second Claimant", bytes.len() as u64, hash),
        ]),
        "s",
    );

    let matches = catalogue.identify(&bytes);

    assert_eq!(matches.len(), 2, "both must be reported");
    let mut names: Vec<&str> = matches.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["First Claimant.adf", "Second Claimant.adf"]);
}

#[test]
fn a_hash_match_with_the_wrong_size_is_rejected() {
    // Cheap extra evidence: it cannot separate the dataset's real collisions,
    // which share a length, but it rules out an unrelated file.
    let bytes = b"some content".to_vec();
    let mut catalogue = Catalogue::default();
    catalogue.add(&datfile(&[("Wrong Size", 999_999, crc32(&bytes))]), "s");

    assert!(catalogue.identify(&bytes).is_empty());
}

#[test]
fn the_real_dataset_identifies_the_real_corpus() {
    // The acceptance criterion, against the actual TOSEC datfiles and the
    // actual corpus. Skips cleanly when either is absent.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let datfiles = root.join("datfiles");
    let disks = root.join("disks");
    if !datfiles.is_dir() || !disks.is_dir() {
        eprintln!("no datfiles or no corpus — skipping");
        return;
    }

    let catalogue = Catalogue::load_dir(&datfiles).expect("load datfiles");
    assert!(
        catalogue.len() > 80_000,
        "expected the Amiga sets, got {}",
        catalogue.len()
    );

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&disks)
        .expect("read corpus")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("adf"))
        .collect();
    paths.sort();

    let mut identified = 0usize;
    let mut ambiguous = 0usize;
    let mut checked = 0usize;
    // A deterministic spread rather than the whole corpus: this test is about
    // the matching being right, and `batch.rs` already covers the full pass.
    let step = (paths.len() / 300).max(1);
    for path in paths.iter().step_by(step).take(300) {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        checked += 1;
        let matches = catalogue.identify(&bytes);
        if !matches.is_empty() {
            identified += 1;
            // Every match must actually be consistent with the file.
            for entry in &matches {
                assert_eq!(entry.size, bytes.len() as u64);
                assert_eq!(entry.crc32, crc32(&bytes));
                assert!(!entry.name.is_empty());
                assert!(!entry.source.is_empty());
            }
        }
        if matches.len() > 1 {
            ambiguous += 1;
        }
    }

    eprintln!(
        "identify: {identified} of {checked} named, {ambiguous} ambiguous, \
         from {} entries in {} datfiles",
        catalogue.len(),
        catalogue.files()
    );
    assert!(
        identified * 100 / checked >= 90,
        "expected ~98% identified, got {identified} of {checked}"
    );
    assert_eq!(
        ambiguous, 0,
        "no Amiga floppy in this dataset shares a CRC32 — if that changes, \
         the collision handling needs real exercise"
    );
}
