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
    volume::{DataFaultKind, FsError, Volume},
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
    assert!(!walked.entries.is_empty());
    assert!(
        walked.entries.len() < 100,
        "a visited set must stop the walk, not a depth limit: got {} entries",
        walked.entries.len()
    );
    // The structural cap is the backstop, not the mechanism. If it fired, the
    // visited set failed and this test would be passing for the wrong reason
    // (IMP-003).
    assert!(
        !walked.hit_limit,
        "the cycle was stopped by the cap, not by the visited set"
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

// --- IMP-002: OFS data-block validation -------------------------------------

/// Build a one-file OFS volume and return (image, file header block, first
/// data block).
fn ofs_with_file(payload: &[u8]) -> (Vec<u8>, u32, u32) {
    let mut f = Fixture::dd(0);
    let hdr = f.add_file("victim", payload);
    let img = f.build();
    // The first data block pointer lives at BSIZE-204 in the header.
    let o = hdr as usize * 512 + 512 - 204;
    // Through ade-endian, not `to_be_bytes`: C-001 has no exemptions, in test
    // code either.
    let first = ade_endian::u32_at(&img, o).expect("first data pointer");
    (img, hdr, first)
}

fn read_faults(img: Vec<u8>) -> Vec<ade_filesystem::volume::DataFault> {
    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).expect("mount");
    let e = v.lookup("victim").expect("lookup");
    v.read_file(&e).expect("read").faults
}

#[test]
fn a_clean_ofs_file_reports_no_faults() {
    let (img, _, _) = ofs_with_file(&vec![0x5A; 3000]);
    let contents = {
        let mem = Mem::dd(img);
        let v = Volume::mount(&mem).unwrap();
        let e = v.lookup("victim").unwrap();
        v.read_file(&e).unwrap()
    };
    assert!(contents.faults.is_empty(), "{:?}", contents.faults);
    assert!(contents.is_complete());
}

#[test]
fn a_zeroed_data_block_is_reported_as_zeroed() {
    let (mut img, _, first) = ofs_with_file(&vec![1u8; 3000]);
    corrupt::zero_block(&mut img, first);
    let faults = read_faults(img);
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert_eq!(faults[0].kind, DataFaultKind::Zeroed);
    assert_eq!(faults[0].first_block, first);
}

#[test]
fn a_non_data_block_is_reported_with_the_type_it_claims() {
    let (mut img, _, first) = ofs_with_file(&vec![2u8; 3000]);
    corrupt::data_block_type(&mut img, first, 0x6db6_6db6);
    let faults = read_faults(img);
    assert_eq!(
        faults[0].kind,
        DataFaultKind::NotADataBlock { found: 0x6db6_6db6 },
        "the real disk carried exactly this value"
    );
}

#[test]
fn a_cross_linked_block_is_reported() {
    let (mut img, hdr, first) = ofs_with_file(&vec![3u8; 3000]);
    corrupt::data_block_owner(&mut img, first, 999);
    let faults = read_faults(img);
    assert_eq!(
        faults[0].kind,
        DataFaultKind::WrongOwner {
            expected: hdr,
            found: 999
        }
    );
}

#[test]
fn an_out_of_sequence_block_is_reported() {
    let (mut img, _, first) = ofs_with_file(&vec![4u8; 3000]);
    corrupt::data_block_seq(&mut img, first, 42);
    let faults = read_faults(img);
    assert_eq!(
        faults[0].kind,
        DataFaultKind::OutOfSequence {
            expected: 1,
            found: 42
        }
    );
}

#[test]
fn an_oversized_length_is_reported_and_clamped() {
    let (mut img, _, first) = ofs_with_file(&vec![5u8; 3000]);
    corrupt::data_block_oversized(&mut img, first, 0xFFFF_FFFF);
    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("victim").unwrap();
    let c = v.read_file(&e).unwrap();
    assert_eq!(
        c.faults[0].kind,
        DataFaultKind::OversizedLength {
            declared: 0xFFFF_FFFF,
            capacity: 488
        }
    );
    // Clamped, not trusted: the read must not run past the volume.
    assert!(c.bytes.len() <= 3000, "got {} bytes", c.bytes.len());
}

#[test]
fn repeated_faults_coalesce_into_one_summary() {
    // A cracked disk can have dozens of bad blocks in a row; one entry each
    // would bury the finding.
    let (mut img, _, first) = ofs_with_file(&vec![6u8; 6000]);
    for n in 0..5u32 {
        corrupt::zero_block(&mut img, first + n);
    }
    let faults = read_faults(img);
    assert_eq!(faults.len(), 1, "must summarise, not enumerate: {faults:?}");
    assert_eq!(faults[0].count, 5);
    assert_eq!(faults[0].first_block, first);
}

#[test]
fn faults_do_not_prevent_recovery() {
    // The whole point of D-012: read the bytes, flag the doubt.
    let (mut img, _, first) = ofs_with_file(&vec![7u8; 3000]);
    corrupt::data_block_seq(&mut img, first, 999);
    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("victim").unwrap();
    let c = v.read_file(&e).unwrap();
    assert!(!c.faults.is_empty(), "the doubt is flagged");
    assert_eq!(c.bytes.len(), 3000, "...and the data still comes back");
    assert!(c.is_full_length());
    assert!(
        !c.is_complete(),
        "complete means sound as well as full-length"
    );
}

#[test]
fn ffs_has_no_data_block_faults_to_find() {
    // C-005's asymmetry: FFS data blocks carry no header, so there is nothing
    // to validate and nothing to report.
    let mut f = Fixture::dd(1);
    f.add_file("victim", &vec![9u8; 3000]);
    let mem = Mem::dd(f.build());
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("victim").unwrap();
    assert!(v.read_file(&e).unwrap().faults.is_empty());
}

#[test]
fn a_file_header_claiming_four_gigabytes_must_not_allocate_it() {
    // AV-005. `byte_size` is a u32 read straight off the disk, so a hostile or
    // corrupt header can claim 4 GB on an 880 KB floppy.
    let (mut img, hdr, _) = ofs_with_file(&vec![1u8; 2000]);
    ade_endian::put_u32(&mut img, hdr as usize * 512 + 512 - 188, u32::MAX).unwrap();
    // Re-checksum so the entry still parses as valid.
    let ck =
        ade_block::checksum::normal(&img[hdr as usize * 512..(hdr as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, hdr as usize * 512 + 20, ck).unwrap();

    let mem = Mem::dd(img);
    let v = Volume::mount(&mem).unwrap();
    let e = v.lookup("victim").unwrap();
    let c = v.read_file(&e).unwrap();
    assert!(
        c.bytes.len() <= 901_120,
        "recovered {} bytes from an 880 KB volume",
        c.bytes.len()
    );
}
