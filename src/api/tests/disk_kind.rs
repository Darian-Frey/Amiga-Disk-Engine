//! What a disk is, when it has nothing to list.
//!
//! "No rootblock at block 880" is true of a quarter of real disks and explains
//! none of them. Measured 2026-10-28 over the 4,652 corpus images: 702 boot
//! into a loader of their own, 339 are data disks another disk's loader reads,
//! 107 are games behind an empty volume, 13 are blank and 5 are raw-track
//! images of protected disks. The corpus half is local only and skips without
//! the corpus; the built cases run in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::DiskKind;
use ade_core::edit::Session;
use ade_core::layers::block::{Geometry, checksum};
use ade_core::layers::endian::put_u32;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn kind(bytes: Vec<u8>) -> DiskKind {
    ade_core::health::examine(bytes).kind
}

fn blank() -> Vec<u8> {
    let dostype = Dostype::from_raw(0x444F_5300).unwrap();
    format::blank(Geometry::DD_FLOPPY, dostype, "Kind", WHEN).unwrap()
}

/// Overwrite the rootblock with what a loader would keep there.
fn no_volume(img: &mut [u8]) {
    img[880 * 512..881 * 512].fill(0x4E);
}

/// Give the bootblock code and a valid checksum: Kickstart would run it.
fn bootable(img: &mut [u8]) {
    img[12..64].fill(0x4E);
    put_u32(img, 4, 0).unwrap();
    let sum = checksum::boot(&img[..1024]).unwrap();
    put_u32(img, 4, sum).unwrap();
}

#[test]
fn a_disk_with_no_volume_that_boots_has_its_own_loader() {
    let mut img = blank();
    no_volume(&mut img);
    bootable(&mut img);
    assert_eq!(kind(img.clone()), DiskKind::OwnLoader);

    // With a bad checksum Kickstart will not run it: another disk's loader
    // reads this one.
    img[100] ^= 1;
    assert_eq!(kind(img), DiskKind::DataDisk);
}

#[test]
fn a_disk_with_no_volume_that_cannot_boot_is_a_data_disk() {
    let mut img = blank();
    no_volume(&mut img);
    // `DOS` and a valid checksum, but no code: nothing to boot into.
    assert_eq!(kind(img.clone()), DiskKind::DataDisk);
    img[..1024].fill(0);
    assert_eq!(kind(img), DiskKind::DataDisk);
}

#[test]
fn a_disk_of_zeros_is_unformatted_not_a_data_disk() {
    assert_eq!(kind(vec![0; 901_120]), DiskKind::Unformatted);
}

#[test]
fn empty_volumes_are_told_apart_by_what_their_blocks_hold() {
    assert_eq!(kind(blank()), DiskKind::Blank);

    // A game in blocks the bitmap calls free (IMP-019).
    let mut img = blank();
    img[1500 * 512..1501 * 512].fill(0x60);
    img[1501 * 512 - 1] = 1;
    assert_eq!(kind(img), DiskKind::GameBehindEmptyRoot);

    // A deleted file's blocks, which its header still lists.
    let mut s = Session::from_bytes(blank()).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let file = s.add_file(root, b"readme", &[0x4E; 2000]).unwrap();
    assert_eq!(kind(s.bytes().to_vec()), DiskKind::Files);
    s.delete(file).unwrap();
    assert_eq!(kind(s.bytes().to_vec()), DiskKind::Emptied);
}

#[test]
fn a_device_is_a_device_and_an_unknown_file_is_unreadable() {
    let mut device = ade_fixtures::device::Device::new(64, 4, 32);
    device.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello");
    });
    assert_eq!(kind(device.build()), DiskKind::Device);
    assert_eq!(kind(b"not a disk at all".to_vec()), DiskKind::Unreadable);
}

#[test]
fn every_kind_has_words_and_a_stable_code() {
    let all = [
        DiskKind::Files,
        DiskKind::Device,
        DiskKind::Blank,
        DiskKind::Unformatted,
        DiskKind::Emptied,
        DiskKind::GameBehindEmptyRoot,
        DiskKind::OwnLoader,
        DiskKind::DataDisk,
        DiskKind::CustomTracks,
        DiskKind::Unreadable,
    ];
    let codes: std::collections::BTreeSet<_> = all.iter().map(|k| k.as_str()).collect();
    assert_eq!(codes.len(), all.len());
    for k in all {
        assert!(
            !k.summary().is_empty() && k.explanation().ends_with('.'),
            "{k:?}"
        );
        assert_eq!(
            k.has_files(),
            matches!(k, DiskKind::Files | DiskKind::Device)
        );
    }
}

#[test]
fn what_the_corpus_disks_with_nothing_to_list_are() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for path in &paths {
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        *counts.entry(kind(bytes).as_str()).or_insert(0) += 1;
    }
    eprintln!("{counts:?}");
    assert_eq!(counts.get("own-loader"), Some(&702));
    assert_eq!(counts.get("data-disk"), Some(&339));
    assert_eq!(counts.get("game-behind-empty-root"), Some(&107));
    assert_eq!(counts.get("blank"), Some(&13));
    assert_eq!(counts.get("custom-tracks"), Some(&5));
    assert_eq!(counts.get("unformatted"), None);
}
