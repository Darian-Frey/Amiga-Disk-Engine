//! Mounting, listing, path lookup and file reading, against generated volumes.
//!
//! Fixtures are built by `ade-fixtures`, an independent statement of the format
//! (D-010). No image is read from the repository.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::redundant_closure_for_method_calls,
    reason = "tests over data they construct"
)]

use ade_block::{BlockError, BlockSource, Geometry, ValidBlock};
use ade_filesystem::{
    entry::EntryKind,
    volume::{FsError, Volume},
};
use ade_fixtures::{Volume as Fixture, corrupt};

/// A block source over bytes. Deliberately local rather than borrowed from
/// `ade-container`: these tests exercise the filesystem layer, and pulling in a
/// sibling would blur which layer a failure came from.
struct Mem {
    bytes: Vec<u8>,
    geometry: Geometry,
}

impl Mem {
    fn dd(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            geometry: Geometry::DD_FLOPPY,
        }
    }
}

impl BlockSource for Mem {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }
    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        let size = self.geometry.block_size() as usize;
        let start = block.index() as usize * size;
        out.copy_from_slice(
            self.bytes
                .get(start..start + size)
                .ok_or(BlockError::Truncated {
                    index: ade_block::BlockIndex(block.index()),
                })?,
        );
        Ok(())
    }
}

#[test]
fn mounts_and_lists_a_volume() {
    let mut f = Fixture::dd(1).named("Workbench");
    f.add_file("readme", b"hello world");
    f.add_dir("Tools");
    f.add_file("startup-sequence", b"echo hi");
    let mem = Mem::dd(f.build());

    let v = Volume::mount(&mem).expect("mount");
    assert_eq!(v.rootblock().name_lossy(), "Workbench");
    assert_eq!(v.hash_table_size(), 72);

    let listing = v.list(v.root()).expect("list");
    assert!(listing.is_clean(), "{:?}", listing.faults);
    let mut names: Vec<_> = listing.entries.iter().map(|e| e.name_lossy()).collect();
    names.sort();
    assert_eq!(names, ["Tools", "readme", "startup-sequence"]);
    assert_eq!(listing.directories().count(), 1);
    assert_eq!(listing.files().count(), 2);
}

#[test]
fn reads_a_file_back_byte_for_byte() {
    for dostype in [0u8, 1] {
        let payload: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let mut f = Fixture::dd(dostype);
        f.add_file("data.bin", &payload);
        let mem = Mem::dd(f.build());
        let v = Volume::mount(&mem).expect("mount");
        let entry = v.lookup("data.bin").expect("lookup");
        assert_eq!(entry.byte_size as usize, payload.len());
        let got = v.read_file(&entry).expect("read");
        assert!(got.is_complete(), "generated fixtures must read complete");
        let got = got.into_bytes();
        assert_eq!(got, payload, "dostype {dostype} round-trip");
    }
}

#[test]
fn an_empty_file_reads_as_empty() {
    let mut f = Fixture::dd(1);
    f.add_file("empty", b"");
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("empty").unwrap();
    assert_eq!(v.read_file(&e).unwrap().into_bytes(), Vec::<u8>::new());
}

#[test]
fn data_blocks_are_read_in_order_not_reversed() {
    // The table runs backwards; iterating it forwards returns the file
    // reversed, which a symmetric payload would hide.
    let payload: Vec<u8> = (0..3000u32).map(|i| (i / 100) as u8).collect();
    let mut f = Fixture::dd(1);
    f.add_file("ordered", &payload);
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("ordered").unwrap();
    let got = v.read_file(&e).unwrap().into_bytes();
    assert_eq!(got[0], 0, "first byte");
    assert_eq!(got[got.len() - 1], payload[payload.len() - 1], "last byte");
    assert_eq!(got, payload);
}

#[test]
fn same_hash_chains_are_followed() {
    // Names chosen to collide are hard to guarantee, so add enough entries that
    // collisions are certain in a 72-slot table, then check all are found.
    let mut f = Fixture::dd(1);
    let names: Vec<String> = (0..40).map(|i| format!("file{i:03}")).collect();
    for n in &names {
        f.add_file(n, b"x");
    }
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    let listing = v.list(v.root()).unwrap();
    assert!(listing.is_clean());
    assert_eq!(listing.entries.len(), 40, "every entry must be reachable");
    for n in &names {
        assert!(v.lookup(n).is_ok(), "{n} not found");
    }
}

#[test]
fn lookup_is_case_insensitive() {
    let mut f = Fixture::dd(1);
    f.add_file("ReadMe", b"x");
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    for probe in ["ReadMe", "readme", "README", "rEaDmE"] {
        assert!(v.lookup(probe).is_ok(), "{probe}");
    }
    assert!(matches!(v.lookup("nope"), Err(FsError::NotFound { .. })));
}

#[test]
fn nested_paths_resolve() {
    let mut f = Fixture::dd(1);
    f.add_dir("Tools");
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    let d = v.lookup("Tools").unwrap();
    assert_eq!(d.kind, EntryKind::Directory);
    assert!(v.lookup("/Tools/").is_ok(), "leading and trailing slashes");
}

// --- AV-001: cycles ---------------------------------------------------------

#[test]
fn a_self_referential_hash_chain_terminates() {
    let mut f = Fixture::dd(1);
    let a = f.add_file("alpha", b"a");
    let mut img = f.build();
    corrupt::hash_chain_loop(&mut img, a);

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    // The point is that this returns at all.
    let listing = v.list(v.root()).expect("list must terminate");
    assert!(!listing.cycles.is_empty(), "the cycle must be reported");
    assert!(
        listing.entries.iter().any(|e| e.name_lossy() == "alpha"),
        "entries found before the cycle are still good data"
    );
}

#[test]
fn a_two_block_hash_cycle_terminates() {
    // A "next != self" check would catch the self-loop above and miss this one.
    let mut f = Fixture::dd(1);
    let a = f.add_file("alpha", b"a");
    let b = f.add_file("beta", b"b");
    let mut img = f.build();
    corrupt::hash_chain_cycle(&mut img, a, b);

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let listing = v.list(v.root()).expect("list must terminate");
    assert!(!listing.cycles.is_empty());
}

#[test]
fn a_directory_cycle_terminates_a_tree_walk() {
    // The legitimate case: a hard link to a directory. AmigaDOS allows it, so
    // this is a valid disk, not a corrupt one.
    let mut f = Fixture::dd(1);
    let tools = f.add_dir("Tools");
    f.add_file("readme", b"x");
    let root = f.root();
    let mut img = f.build();
    corrupt::directory_cycle(&mut img, tools, root);

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let walked = v.walk(v.root()).expect("walk must terminate");
    assert!(!walked.is_empty());
    assert!(
        walked.len() < 100,
        "a visited set must stop the walk, not a depth limit: got {} entries",
        walked.len()
    );
}

// --- AV-004: out-of-range pointers ------------------------------------------

#[test]
fn an_out_of_range_hash_slot_is_reported_not_dereferenced() {
    let f = Fixture::dd(1);
    let root = f.root();
    let mut img = f.build();
    corrupt::hash_slot_out_of_range(&mut img, root, 0);

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let listing = v.list(v.root()).expect("list");
    assert!(
        listing
            .faults
            .iter()
            .any(|e| matches!(e, FsError::Malformed { .. })),
        "the wild pointer must be reported"
    );
}

#[test]
fn an_out_of_range_data_pointer_is_refused() {
    let mut f = Fixture::dd(1);
    let file = f.add_file("bad", &vec![0u8; 2000]);
    let mut img = f.build();
    corrupt::first_data_out_of_range(&mut img, file);

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("bad").unwrap();
    // Reading may succeed with short data or fail; it must not panic or read
    // outside the image.
    let _ = v.read_file(&e);
}

#[test]
fn mounting_a_volume_without_a_rootblock_fails_cleanly() {
    let f = Fixture::dd(0);
    let root = f.root();
    let mut img = f.build();
    corrupt::rootblock_wrong_type(&mut img, root);
    let mem = Mem::dd(img);
    assert!(matches!(
        Volume::mount(&mem),
        Err(FsError::NoRootblock { block: 880 })
    ));
}

#[test]
fn a_zeroed_volume_does_not_panic() {
    let mem = Mem::dd(corrupt::zeroed_volume());
    assert!(Volume::mount(&mem).is_err());
}
