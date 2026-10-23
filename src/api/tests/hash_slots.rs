//! Entries filed under the wrong hash slot, across the corpus (IMP-012).
//!
//! A misplaced entry is listed by every reader that walks a directory and
//! found by none that looks a name up. Measured 2026-10-23: 30 such entries on
//! 6 of 3,606 volumes, none of them explained by the other hash function —
//! hand edits and damage, not the C-006 mistake.
//!
//! The assertions are what the measurement found, held as a regression check:
//! a broken hash function in ADE would make most entries look misplaced, and
//! would make the other function fit. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::Image;
use ade_core::layers::filesystem::entry::EntryKind;

#[test]
fn misplaced_entries_are_rare_and_never_the_c006_mistake() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    paths.sort();

    let (mut volumes, mut entries, mut misplaced, mut other_mode, mut disks) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let Ok(image) = Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        volumes += 1;
        entries += walk.entries.len();
        let dirs = std::iter::once(volume.root()).chain(
            walk.entries
                .iter()
                .filter(|(_, e)| e.kind == EntryKind::Directory)
                .map(|(_, e)| e.block),
        );
        let mut here = 0usize;
        for dir in dirs {
            let Ok(found) = volume.misplaced(dir) else {
                continue;
            };
            here += found.len();
            other_mode += found.iter().filter(|m| m.other_mode).count();
        }
        if here > 0 {
            disks += 1;
            misplaced += here;
        }
    }
    eprintln!(
        "{volumes} volumes, {entries} entries: {misplaced} misplaced on {disks} disks, \
         {other_mode} fitting the other hash function"
    );
    assert!(volumes > 3000, "the corpus mounted: {volumes}");
    assert_eq!(other_mode, 0, "no corpus entry shows the C-006 mistake");
    assert!(
        (misplaced as f64) < entries as f64 * 0.001,
        "misplacement is rare on real disks; {misplaced} of {entries} suggests ADE's hash broke"
    );
}

// --- Built cases, which CI runs ---------------------------------------------

use ade_core::edit::Session;
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::layers::filesystem::hash::hash_name;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

/// A disk holding one file called `name`, and the file's header block.
fn disk_with(flags: u8, name: &[u8]) -> (Vec<u8>, u32) {
    let dostype = Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap();
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Slots", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let block = s.add_file(root, name, b"contents").unwrap();
    (s.bytes().to_vec(), block)
}

/// Move whatever the root's hash slot `from` holds into slot `to`, and reseal.
fn move_slot(bytes: &mut [u8], from: u32, to: u32) {
    use ade_core::layers::endian::{put_u32, u32_at};
    let root = 880 * 512;
    let at = |slot: u32| root + 24 + slot as usize * 4;
    let entry = u32_at(bytes, at(from)).unwrap();
    put_u32(bytes, at(from), 0).unwrap();
    put_u32(bytes, at(to), entry).unwrap();
    put_u32(bytes, root + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[root..root + 512], 20).unwrap();
    put_u32(bytes, root + 20, sum).unwrap();
}

fn wrong_slot_findings(bytes: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(bytes)
        .findings
        .into_iter()
        .filter(|f| f.code == "entry-wrong-slot")
        .collect()
}

#[test]
fn a_file_in_the_wrong_slot_is_a_warning_naming_it() {
    let (bytes, block) = disk_with(1, b"hello.txt");
    assert!(
        wrong_slot_findings(bytes.clone()).is_empty(),
        "placed correctly"
    );

    let mut moved = bytes;
    let right = hash_name(b"hello.txt", 72, false);
    move_slot(&mut moved, right, (right + 5) % 72);
    let found = wrong_slot_findings(moved);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Warning);
    assert_eq!(found[0].block, Some(block));
    assert!(
        found[0].message.starts_with("hello.txt:"),
        "{}",
        found[0].message
    );
    assert!(!found[0].message.contains("C-006"), "{}", found[0].message);
}

#[test]
fn a_name_filed_by_the_other_hash_function_says_so() {
    // `für` hashes differently under the international and plain functions.
    // On an international volume, put it where the plain function would: the
    // mistake a writer makes when it gets C-006 wrong.
    let name = b"f\xFCr";
    let intl = hash_name(name, 72, true);
    let plain = hash_name(name, 72, false);
    assert_ne!(intl, plain);
    let (mut bytes, _) = disk_with(3, name);
    move_slot(&mut bytes, intl, plain);
    let found = wrong_slot_findings(bytes);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].message.contains("C-006"), "{}", found[0].message);
}

#[test]
fn a_root_listed_in_its_own_table_is_not_a_misplaced_name() {
    // Eight corpus disks point a root hash slot back at the rootblock. That is
    // a different structure, and must not be reported as a misfiled entry.
    use ade_core::layers::endian::put_u32;
    let (mut bytes, _) = disk_with(1, b"x");
    let root = 880 * 512;
    put_u32(&mut bytes, root + 24, 880).unwrap();
    put_u32(&mut bytes, root + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[root..root + 512], 20).unwrap();
    put_u32(&mut bytes, root + 20, sum).unwrap();
    let found = wrong_slot_findings(bytes);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_entry_also_reachable_from_its_right_slot_is_not_reported() {
    // Found on Emetic Skimmer: one slot's chain runs into another's, so an
    // entry sits in a wrong slot *and* down its right slot's chain. A lookup
    // by name hashes to the right slot and finds it; reporting it would be a
    // false alarm.
    use ade_core::layers::endian::{put_u32, u32_at};
    let (mut bytes, block) = disk_with(1, b"hello.txt");
    let right = hash_name(b"hello.txt", 72, false);
    // A lower slot than the right one, so a walk meets the wrong slot first:
    // the order in which the first version of this check got it wrong.
    assert_ne!(right, 0);
    let wrong: u32 = 0;
    let root = 880 * 512;
    assert_eq!(
        u32_at(&bytes, root + 24 + right as usize * 4).unwrap(),
        block
    );
    put_u32(&mut bytes, root + 24 + wrong as usize * 4, block).unwrap();
    put_u32(&mut bytes, root + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[root..root + 512], 20).unwrap();
    put_u32(&mut bytes, root + 20, sum).unwrap();
    let found = wrong_slot_findings(bytes);
    assert!(found.is_empty(), "{found:?}");
}
