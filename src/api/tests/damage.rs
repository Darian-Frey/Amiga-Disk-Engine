//! What the map shows wrong with a disk's blocks (F-027), and its counts.
//!
//! The built cases damage a disk ADE wrote, one way at a time, and require
//! the block to be marked as exactly that. The corpus half requires the
//! counts to add up and every damaged block to be on the disk. Measured
//! 2026-11-10 over every 10th of the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding"
)]

use std::path::PathBuf;

use ade_core::edit::Session;
use ade_core::layers::block::Geometry;
use ade_core::layers::block::checksum::normal_at;
use ade_core::layers::endian::{put_u32, u32_at};
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::{Damage, damaged_blocks, examine};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};
const BITMAP: usize = 881 * 512;

/// A disk holding a directory and two files, made by ADE's writer.
fn disk() -> Vec<u8> {
    let dostype = Dostype::from_raw(0x444F_5300).unwrap();
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Map", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let dir = s.make_dir(root, b"Dir").unwrap();
    s.add_file(root, b"hello.txt", &[7u8; 3000]).unwrap();
    s.add_file(dir, b"inner", &[9u8; 700]).unwrap();
    s.bytes().to_vec()
}

/// Flip the bitmap bit for `block`, set meaning free, and reseal it.
fn flip(bytes: &mut [u8], block: u32) {
    let index = (block - 2) as usize;
    let at = BITMAP + 4 + (index / 32) * 4;
    let word = u32_at(bytes, at).unwrap() ^ (1 << (index % 32));
    put_u32(bytes, at, word).unwrap();
    put_u32(bytes, BITMAP, 0).unwrap();
    let sum = normal_at(&bytes[BITMAP..BITMAP + 512], 0).unwrap();
    put_u32(bytes, BITMAP, sum).unwrap();
}

/// The blocks of the file at `path`: its header, then its data.
fn blocks_of(bytes: &[u8], path: &str) -> Vec<u32> {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    volume.file_blocks(&entry).unwrap()
}

#[test]
fn a_sound_disk_has_nothing_to_show() {
    let damaged = damaged_blocks(&examine(disk()));
    assert!(damaged.is_empty(), "{damaged:?}");
}

#[test]
fn each_kind_of_damage_marks_its_block() {
    let original = disk();
    let hello = blocks_of(&original, "hello.txt");

    // Marked used, and nothing reaches it.
    let mut bytes = original.clone();
    flip(&mut bytes, 1500);
    assert_eq!(damaged_blocks(&examine(bytes)), [(1500, Damage::Orphaned)]);

    // In use by a file, marked free.
    let mut bytes = original.clone();
    flip(&mut bytes, hello[1]);
    assert_eq!(
        damaged_blocks(&examine(bytes)),
        [(hello[1], Damage::AtRisk)]
    );

    // A file header whose checksum does not match.
    let mut bytes = original.clone();
    let header = hello[0] as usize * 512;
    bytes[header + 0x1B0] ^= 0x40; // a byte of the file's name
    let damaged = damaged_blocks(&examine(bytes));
    assert!(
        damaged.contains(&(hello[0], Damage::Checksum)),
        "{damaged:?}"
    );

    // Two at once, on two blocks, each kept apart and in block order.
    let mut bytes = original.clone();
    flip(&mut bytes, 1500);
    flip(&mut bytes, hello[1]);
    let damaged = damaged_blocks(&examine(bytes));
    assert_eq!(
        damaged,
        [(hello[1], Damage::AtRisk), (1500, Damage::Orphaned)]
    );
}

#[test]
fn the_worse_damage_names_a_block_wrong_two_ways() {
    // The order a legend lists them in, and the one that wins a block.
    let mut kinds = vec![
        Damage::HoldsData,
        Damage::Orphaned,
        Damage::AtRisk,
        Damage::Structure,
        Damage::Checksum,
        Damage::CrossLinked,
    ];
    kinds.sort();
    assert_eq!(kinds.first(), Some(&Damage::CrossLinked));
    assert_eq!(kinds.last(), Some(&Damage::HoldsData));
    for k in kinds {
        assert!(!k.name().is_empty() && !k.describes().is_empty());
    }
}

#[test]
fn a_file_in_one_run_is_not_in_pieces() {
    let image = ade_core::Image::from_bytes(disk()).unwrap();
    let (files, pieces) = ade_core::layout::Layout::of(&image).files();
    assert_eq!((files, pieces), (2, 0));
}

#[test]
fn over_the_corpus_the_counts_add_up_and_damage_is_on_the_disk() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut disks, mut damaged, mut by_kind) = (0usize, 0usize, std::collections::BTreeMap::new());
    for path in paths.iter().step_by(10) {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let size = bytes.len() as u64;
        let health = examine(bytes);
        let Some(b) = &health.bitmap else { continue };
        disks += 1;
        let blocks = damaged_blocks(&health);
        if !blocks.is_empty() {
            damaged += 1;
        }
        for (block, kind) in &blocks {
            assert!(
                u64::from(*block) * 512 < size,
                "{}: block {block} is past the disk",
                path.display()
            );
            assert!(
                u64::from(*block) < u64::from(b.covered) + 2,
                "{}",
                path.display()
            );
            *by_kind.entry(kind.name()).or_insert(0usize) += 1;
        }
        // One kind a block: the list holds each block once.
        let mut seen: Vec<u32> = blocks.iter().map(|(b, _)| *b).collect();
        seen.dedup();
        assert_eq!(seen.len(), blocks.len(), "{}", path.display());
    }
    eprintln!("{disks} volumes, {damaged} with damage to show: {by_kind:?}");
    assert!(disks > 0);
}
