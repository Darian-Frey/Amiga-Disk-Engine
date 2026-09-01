//! Recovering files nothing points at (F-030).
//!
//! The interesting property is not that files come back — it is that the disk
//! says whether to believe them. An OFS data block names the header that owns
//! it and checksums itself, so a carved file is verifiable from the disk alone.
//! These tests pin that grading, because a carver that reported everything as
//! recovered would be the thing D-002 exists to prevent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "tests over images they construct"
)]

use ade_core::Image;
use ade_core::carve::{Evidence, carve};
use ade_core::layers::endian::put_u32;
use ade_fixtures::Volume as Fixture;

/// A disk whose root directory has been wiped, leaving its files unreachable.
///
/// This is what deletion looks like from the outside: the hash table stops
/// naming the file, while the header and its data sit exactly where they were.
fn lost_directory(dostype: u8, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut v = Fixture::dd(dostype).named("Lost");
    for (name, body) in files {
        v.add_file(name, body);
    }
    let mut bytes = v.build();

    // Clear the rootblock's hash table, then put its checksum right — a
    // rootblock that fails its own checksum would not mount, and then nothing
    // would be *orphaned*, it would all just be unreadable.
    let root = 880usize * 512;
    for slot in 0..72usize {
        put_u32(&mut bytes, root + 24 + slot * 4, 0).unwrap();
    }
    let block = &mut bytes[root..root + 512];
    put_u32(block, 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(block, 20).unwrap();
    put_u32(block, 20, sum).unwrap();
    bytes
}

fn carved(bytes: Vec<u8>) -> Vec<ade_core::carve::Carved> {
    carve(&Image::from_bytes(bytes).expect("opens"))
}

#[test]
fn an_ofs_file_nothing_points_at_is_recovered_and_proves_itself() {
    // OFS data blocks carry a header naming the file they belong to, so the
    // disk confirms the recovery without anything external being asked.
    let body: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let found = carved(lost_directory(0, &[("secret", &body)]));

    let file = found
        .iter()
        .find(|c| c.name == "secret")
        .expect("the lost file");
    assert_eq!(file.evidence, Evidence::SelfEvident);
    assert_eq!(file.size, 3000);
    assert!(!file.blocks.is_empty());
}

#[test]
fn an_ffs_file_can_only_ever_be_header_only() {
    // An FFS data block is raw payload with no header at all: the name and the
    // size are sound and **nothing confirms a byte of the contents**. Claiming
    // otherwise would be the carver asserting its own correctness.
    let found = carved(lost_directory(1, &[("secret", &vec![7u8; 3000])]));

    let file = found
        .iter()
        .find(|c| c.name == "secret")
        .expect("the lost file");
    assert_eq!(file.evidence, Evidence::HeaderOnly);
    assert_eq!(file.size, 3000, "the header is still readable");
    assert!(file.blocks.is_empty(), "and no block is confirmed");
}

#[test]
fn a_file_whose_data_was_partly_reused_says_partial() {
    // The middle case, and the one that matters most: some blocks still name
    // this header and some have been taken by something else. Reporting that
    // as recovered would hand somebody a file with a hole in it.
    let body: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
    let mut bytes = lost_directory(0, &[("secret", &body)]);

    // Find the file's header, then point one of its data blocks at a different
    // owner — which is what happens when the block is reallocated.
    let header = carved(bytes.clone())
        .into_iter()
        .find(|c| c.name == "secret")
        .expect("the lost file");
    let victim = header.blocks[1];
    ade_fixtures::corrupt::data_block_owner(&mut bytes, victim, 999);

    let file = carved(bytes)
        .into_iter()
        .find(|c| c.name == "secret")
        .expect("still found");
    match file.evidence {
        Evidence::Partial { good, bad } => {
            assert!(good > 0, "the rest of the file is still confirmed");
            assert_eq!(bad, 1, "and exactly the reused block is not");
        }
        other => panic!("expected partial, got {other:?}"),
    }
    assert!(
        !file.blocks.contains(&victim),
        "a block that failed is not offered as recovered"
    );
}

#[test]
fn a_live_file_is_not_carved() {
    // Carving is about what the directory tree does *not* reach. A file it
    // does reach is an ordinary file, and reporting it as recovered would fill
    // the answer with things nobody lost.
    let mut v = Fixture::dd(0).named("Healthy");
    v.add_file("ordinary", b"still linked");
    let found = carved(v.build());
    assert!(
        found.iter().all(|c| c.name != "ordinary"),
        "{:?}",
        found.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
}

#[test]
fn a_disk_that_does_not_mount_is_still_carved() {
    // The disks worth carving include the ones with no filesystem left — 3 of
    // 600 corpus images hold 91 orphaned headers with nothing to ask. A
    // volume-only carver would refuse exactly the disks where recovery is the
    // whole point.
    let body: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let mut bytes = lost_directory(0, &[("survivor", &body)]);
    // Destroy the rootblock outright, so nothing mounts.
    bytes[880 * 512..881 * 512].fill(0);

    let image = Image::from_bytes(bytes).expect("still opens as a container");
    assert!(image.volume().is_err(), "and holds no mountable volume");

    let found = carve(&image);
    let file = found
        .iter()
        .find(|c| c.name == "survivor")
        .expect("recovered from a disk with no directory at all");
    assert_eq!(file.evidence, Evidence::SelfEvident);
}
