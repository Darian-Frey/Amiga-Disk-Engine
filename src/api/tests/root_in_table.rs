//! A rootblock listed in a directory's hash table (IMP-015).
//!
//! Measured 2026-10-25: 7 of 3,606 corpus volumes, every one the root's own
//! table leading back to the root, under the volume's name. The corpus half
//! is local only and skips without the corpus; the built cases run in CI.

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
fn a_root_in_a_hash_table_across_the_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    paths.sort();
    let (mut volumes, mut disks) = (0usize, 0usize);
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
        let mut here = Vec::new();
        for (dir_path, dir) in dirs {
            let Ok(listing) = volume.list(dir) else {
                continue;
            };
            for e in listing.entries.iter().filter(|e| e.kind == EntryKind::Root) {
                here.push(format!(
                    "in {:?}: block {} named {:?} (root is {})",
                    if dir_path.is_empty() { "/" } else { &dir_path },
                    e.block,
                    e.name_lossy(),
                    volume.root()
                ));
            }
        }
        if !here.is_empty() {
            disks += 1;
            eprintln!(
                "{}: {}",
                path.file_name().unwrap().to_string_lossy(),
                here.join("; ")
            );
        }
    }
    eprintln!("{volumes} volumes: a rootblock listed as an entry on {disks}");
    assert!(volumes > 3000, "the corpus mounted: {volumes}");
    assert!((5..50).contains(&disks), "rare, and present: {disks}");
}

// --- Built cases, which CI runs ---------------------------------------------

use ade_core::layers::block::Geometry;
use ade_core::layers::endian::put_u32;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};

fn blank() -> Vec<u8> {
    let when = Stamp {
        days: 17_000,
        mins: 1,
        ticks: 0,
    };
    format::blank(
        Geometry::DD_FLOPPY,
        Dostype::from_raw(0x444F_5300).unwrap(),
        "Echo",
        when,
    )
    .unwrap()
}

/// The root's hash slot `slot` pointed back at the root, resealed.
fn echoed(slot: usize) -> Vec<u8> {
    let mut bytes = blank();
    let root = 880 * 512;
    put_u32(&mut bytes, root + 24 + slot * 4, 880).unwrap();
    put_u32(&mut bytes, root + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[root..root + 512], 20).unwrap();
    put_u32(&mut bytes, root + 20, sum).unwrap();
    bytes
}

fn findings(bytes: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(bytes)
        .findings
        .into_iter()
        .filter(|f| f.code == "root-in-hash-table")
        .collect()
}

#[test]
fn a_root_in_its_own_table_is_reported_once_as_information() {
    assert_eq!(findings(blank()), Vec::new());
    let found = findings(echoed(0));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Info);
    assert_eq!(found[0].block, Some(880));
    assert!(
        found[0]
            .message
            .starts_with("slot 0 of the root's own hash table"),
        "{}",
        found[0].message
    );

    // And the listing keeps the row, as the volume rather than a folder.
    let image = Image::from_bytes(echoed(15)).unwrap();
    let volume = image.volume().unwrap();
    let listing = volume.list(volume.root()).unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].kind, EntryKind::Root);
    assert_eq!(volume.slots_reaching_root(volume.root()).unwrap(), vec![15]);
}
