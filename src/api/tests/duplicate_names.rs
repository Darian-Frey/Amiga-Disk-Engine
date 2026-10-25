//! Two entries with one name in a directory, across the corpus (IMP-016).
//!
//! AmigaDOS cannot create them and disks carry them. Only one of a group can
//! be opened by name; this measures how many groups there are and which one
//! a lookup reaches. Measured 2026-10-25: 11 groups on 5 of 3,606 volumes, 8
//! reachable by name from none, and 9 where ADE's own `lookup` picks a
//! different entry from AmigaDOS (BUG-013), printed as it runs. The corpus
//! half is local only and skips without the corpus; the built cases run in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::Image;
use ade_core::layers::filesystem::entry::EntryKind;

#[test]
fn duplicate_names_across_the_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    paths.sort();

    let (mut volumes, mut groups, mut disks, mut unreachable, mut ade_differs) =
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
        let dirs = std::iter::once((String::new(), volume.root())).chain(
            walk.entries
                .iter()
                .filter(|(_, e)| e.kind == EntryKind::Directory)
                .map(|(p, e)| (p.clone(), e.block)),
        );
        let mut here = 0usize;
        for (dir_path, dir) in dirs {
            let Ok(found) = volume.duplicates(dir) else {
                continue;
            };
            for d in &found {
                here += 1;
                if d.reached.is_none() {
                    unreachable += 1;
                }
                // What ADE's own lookup returns for the same name.
                let name: String = d.name.iter().map(|&b| char::from(b)).collect();
                let full = if dir_path.is_empty() {
                    name.clone()
                } else {
                    format!("{dir_path}/{name}")
                };
                let ade = volume.lookup(&full).ok().map(|e| e.block);
                if ade != d.reached {
                    ade_differs += 1;
                    eprintln!(
                        "{}: {full:?} blocks {:?}: AmigaDOS reaches {:?}, ADE's lookup {:?}",
                        path.file_name().unwrap().to_string_lossy(),
                        d.blocks,
                        d.reached,
                        ade
                    );
                }
            }
        }
        if here > 0 {
            disks += 1;
            groups += here;
        }
    }
    eprintln!(
        "{volumes} volumes: {groups} duplicate-name groups on {disks} disks, {unreachable} \
         reachable by name from none, {ade_differs} where ADE's lookup picks another"
    );
    assert!(volumes > 3000, "the corpus mounted: {volumes}");
    // Measured 2026-10-25: 11 groups on 5 disks. A folding or grouping bug
    // would make distinct names collide everywhere, or find none at all.
    assert!(
        (5..100).contains(&groups),
        "duplicate names are rare and real: {groups}"
    );
}

// --- Built cases, which CI runs ---------------------------------------------

use ade_core::edit::Session;
use ade_core::layers::block::Geometry;
use ade_core::layers::endian::{put_u32, u32_at};
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::layers::filesystem::hash::hash_name;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};
const ROOT: usize = 880 * 512;
const HT: u32 = 72;

fn reseal(bytes: &mut [u8], block: usize) {
    let at = block * 512;
    put_u32(bytes, at + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[at..at + 512], 20).unwrap();
    put_u32(bytes, at + 20, sum).unwrap();
}

fn slot_at(slot: u32) -> usize {
    ROOT + 24 + slot as usize * 4
}

/// A plain-hashing disk holding `aaa` and `bbb`, with `bbb` then renamed
/// `AAA` — the same name under case folding — and the header blocks of both.
fn twins() -> (Vec<u8>, u32, u32) {
    let blank = format::blank(
        Geometry::DD_FLOPPY,
        Dostype::from_raw(0x444F_5300).unwrap(),
        "Twins",
        WHEN,
    )
    .unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let first = s.add_file(root, b"aaa", b"the one AmigaDOS opens").unwrap();
    let second = s.add_file(root, b"bbb", b"a stale copy").unwrap();
    let mut bytes = s.bytes().to_vec();
    let header = second as usize * 512;
    bytes[header + 432] = 3;
    bytes[header + 433..header + 436].copy_from_slice(b"AAA");
    reseal(&mut bytes, second as usize);
    (bytes, first, second)
}

fn findings(bytes: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(bytes)
        .findings
        .into_iter()
        .filter(|f| f.code == "duplicate-name")
        .collect()
}

#[test]
fn two_entries_of_one_name_are_a_warning_naming_the_one_amigados_opens() {
    let (mut bytes, first, second) = twins();
    // Put the renamed copy down the right chain, after the original: both are
    // where a lookup looks, and the first match is the original.
    let right = hash_name(b"aaa", HT, false);
    let stale = hash_name(b"bbb", HT, false);
    assert_eq!(u32_at(&bytes, slot_at(right)).unwrap(), first);
    put_u32(&mut bytes, slot_at(stale), 0).unwrap();
    put_u32(&mut bytes, first as usize * 512 + 496, second).unwrap();
    reseal(&mut bytes, first as usize);
    reseal(&mut bytes, 880);

    let found = findings(bytes);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Warning);
    assert_eq!(found[0].block, Some(first));
    assert!(
        found[0]
            .message
            .contains(&format!("AmigaDOS opens block {first}")),
        "{}",
        found[0].message
    );
    assert!(
        found[0].message.contains("2 entries"),
        "{}",
        found[0].message
    );
}

#[test]
fn copies_all_in_wrong_slots_are_reachable_from_none() {
    // `AAA` left in `bbb`'s slot, and the original moved to another wrong one:
    // a listing shows both, a lookup finds neither.
    let (mut bytes, first, _) = twins();
    let right = hash_name(b"aaa", HT, false);
    put_u32(&mut bytes, slot_at(right), 0).unwrap();
    put_u32(&mut bytes, slot_at((right + 7) % HT), first).unwrap();
    reseal(&mut bytes, 880);

    let found = findings(bytes);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].message.contains("can open none of them"),
        "{}",
        found[0].message
    );
}

#[test]
fn distinct_names_are_not_duplicates() {
    // The unedited disk, and one entry reached from two slots — a merged
    // chain — which `list` lists once.
    let blank = format::blank(
        Geometry::DD_FLOPPY,
        Dostype::from_raw(0x444F_5300).unwrap(),
        "Twins",
        WHEN,
    )
    .unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let only = s.add_file(root, b"aaa", b"x").unwrap();
    let mut bytes = s.bytes().to_vec();
    assert_eq!(findings(bytes.clone()), Vec::new());
    let right = hash_name(b"aaa", HT, false);
    put_u32(&mut bytes, slot_at((right + 3) % HT), only).unwrap();
    reseal(&mut bytes, 880);
    assert_eq!(
        findings(bytes),
        Vec::new(),
        "one entry reached twice is one entry"
    );
}
