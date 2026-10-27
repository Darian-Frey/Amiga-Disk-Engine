//! An entry whose header names a directory other than the one listing it
//! (BUG-018).
//!
//! Measured 2026-10-27 over 3,606 corpus volumes: 18 entries on Putty's three
//! disks, every one in the root and every one naming another file's header.
//! The corpus half is local only and skips without the corpus; the built
//! cases run in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::edit::Session;
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};

fn wrong_parent(bytes: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(bytes)
        .findings
        .into_iter()
        .filter(|f| f.code == "entry-wrong-parent")
        .collect()
}

/// Point `entry`'s parent field at `parent`, and reseal.
fn set_parent(img: &mut [u8], entry: u32, parent: u32) {
    use ade_core::layers::endian::put_u32;
    let at = entry as usize * 512;
    put_u32(img, at + 512 - 12, parent).unwrap();
    put_u32(img, at + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&img[at..at + 512], 20).unwrap();
    put_u32(img, at + 20, sum).unwrap();
}

#[test]
fn an_entry_naming_another_block_as_its_directory_is_a_warning() {
    let when = Stamp {
        days: 17_000,
        mins: 1,
        ticks: 0,
    };
    let dostype = Dostype::from_raw(0x444F_5301).unwrap();
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Parents", when).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(when));
    let root = s.root();
    let tools = s.make_dir(root, b"Tools").unwrap();
    let file = s.add_file(root, b"readme", b"text").unwrap();
    let inner = s.add_file(tools, b"inner", b"x").unwrap();
    let clean = s.bytes().to_vec();
    assert_eq!(wrong_parent(clean.clone()), Vec::new());

    // Putty's shape: a root file naming another file's header.
    let mut img = clean.clone();
    set_parent(&mut img, file, inner);
    let found = wrong_parent(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Warning);
    assert_eq!(found[0].block, Some(file));
    assert_eq!(
        found[0].message,
        format!(
            "readme: listed in the root, but its header names block {inner} as its directory \
             — inner, which is not a directory"
        )
    );

    // A real directory, but not the one listing it.
    let mut img = clean;
    set_parent(&mut img, inner, 880);
    let found = wrong_parent(img);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .message
            .starts_with("Tools/inner: listed in Tools, but its header names block 880")
            && found[0].message.ends_with("— the root"),
        "{}",
        found[0].message
    );
}

#[test]
fn entries_naming_another_directory_across_the_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut disks: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let n = wrong_parent(bytes).len();
        if n > 0 {
            disks.insert(path.file_name().unwrap().to_string_lossy().into_owned(), n);
        }
    }
    eprintln!("{disks:?}");
    assert_eq!(
        disks,
        BTreeMap::from([
            ("Putty_DiskA.adf".to_owned(), 5),
            ("Putty_DiskB.adf".to_owned(), 11),
            ("Putty_DiskC.adf".to_owned(), 2),
        ])
    );
}
