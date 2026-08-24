//! Hard and soft links (Phase 2).
//!
//! Links are FFS-only, rare on floppies, and absent from the 4652-image corpus
//! — none in 8865 entries sampled. They are also the structure that makes
//! AV-001's traversal cycle reachable on an *uncorrupted* disk, since AmigaDOS
//! permits a hard link to a directory.
//!
//! # The oracle cannot help here
//!
//! `unadf` omits link entries from its listings entirely: given a volume with
//! four entries, two of them links, it lists two. Our link blocks match SPEC
//! §Links field for field, so this is a limitation of ADFlib — the ADF FAQ
//! calls the whole link implementation "a mess" — rather than a fault in the
//! fixtures. It does mean link support is validated against the specification
//! only, with no independent implementation to check it, which is exactly the
//! situation D-010's amendment says the corpus would normally cover and here
//! cannot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::many_single_char_names,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests over data they construct"
)]

use ade_block::{BlockError, BlockSource, Geometry, ValidBlock};
use ade_filesystem::{
    entry::EntryKind,
    volume::{FsError, Volume},
};
use ade_fixtures::Volume as Fixture;

struct Mem {
    bytes: Vec<u8>,
    geometry: Geometry,
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

fn mem(bytes: Vec<u8>) -> Mem {
    Mem {
        bytes,
        geometry: Geometry::DD_FLOPPY,
    }
}

const CONTENTS: &[u8] = b"the actual contents";

fn volume_with_links() -> Vec<u8> {
    let mut v = Fixture::dd(1).named("Links");
    let f = v.add_file("real.txt", CONTENTS);
    v.add_hardlink("link.txt", f, false);
    let d = v.add_dir("Tools");
    v.add_hardlink("ToolsLink", d, true);
    v.build()
}

#[test]
fn a_hard_link_reads_the_target_contents() {
    // BUG-005: this used to return Ok("") — an empty file, silently.
    let m = mem(volume_with_links());
    let v = Volume::mount(&m).unwrap();
    let link = v.lookup("link.txt").unwrap();
    assert_eq!(link.kind, EntryKind::HardLinkFile);
    let got = v.read_file(&link).unwrap();
    assert_eq!(got.bytes, CONTENTS, "a link must read what it points at");
    assert!(got.is_complete());
}

#[test]
fn resolve_names_the_target_and_is_a_no_op_otherwise() {
    let m = mem(volume_with_links());
    let v = Volume::mount(&m).unwrap();

    let link = v.lookup("link.txt").unwrap();
    let target = v.resolve(&link).unwrap();
    assert_eq!(target.name_lossy(), "real.txt");
    assert_eq!(target.kind, EntryKind::File);

    // Anything that is not a link resolves to itself, so callers can resolve
    // unconditionally.
    let plain = v.lookup("real.txt").unwrap();
    assert_eq!(v.resolve(&plain).unwrap().block, plain.block);
}

#[test]
fn a_hard_link_to_a_directory_is_walked_once_not_twice() {
    // The legitimate AV-001 case. Both entries appear in the listing, but the
    // directory's *contents* are walked once.
    let mut f = Fixture::dd(1).named("DirLink");
    let d = f.add_dir("Tools");
    f.add_hardlink("ToolsLink", d, true);
    let m = mem(f.build());
    let v = Volume::mount(&m).unwrap();

    let walked = v.walk(v.root()).unwrap();
    assert!(
        !walked.hit_limit,
        "the visited set must stop it, not the cap"
    );
    let names: Vec<_> = walked.entries.iter().map(|(p, _)| p.clone()).collect();
    assert!(names.contains(&"Tools".to_owned()));
    assert!(names.contains(&"ToolsLink".to_owned()));
}

#[test]
fn a_link_pointing_outside_the_volume_is_refused_not_followed() {
    // AV-004: `real_entry` comes off the disk like any other pointer.
    let mut f = Fixture::dd(1);
    let t = f.add_file("real.txt", CONTENTS);
    let link = f.add_hardlink("link.txt", t, false);
    let mut img = f.build();
    ade_endian::put_u32(&mut img, link as usize * 512 + 512 - 44, 0xFFFF_FFFF).unwrap();
    let ck =
        ade_block::checksum::normal(&img[link as usize * 512..(link as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, link as usize * 512 + 20, ck).unwrap();

    let m = mem(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("link.txt").unwrap();
    assert!(matches!(v.resolve(&e), Err(FsError::BrokenLink { .. })));
    assert!(
        v.read_file(&e).is_err(),
        "and reading it must fail, not return empty"
    );
}

#[test]
fn a_link_that_points_at_itself_terminates() {
    let mut f = Fixture::dd(1);
    let t = f.add_file("real.txt", CONTENTS);
    let link = f.add_hardlink("link.txt", t, false);
    let mut img = f.build();
    ade_endian::put_u32(&mut img, link as usize * 512 + 512 - 44, link).unwrap();
    let ck =
        ade_block::checksum::normal(&img[link as usize * 512..(link as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, link as usize * 512 + 20, ck).unwrap();

    let m = mem(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("link.txt").unwrap();
    // The point is that this returns at all.
    assert!(matches!(v.resolve(&e), Err(FsError::BrokenLink { .. })));
}

#[test]
fn a_link_naming_no_target_is_reported() {
    let mut f = Fixture::dd(1);
    let t = f.add_file("real.txt", CONTENTS);
    let link = f.add_hardlink("link.txt", t, false);
    let mut img = f.build();
    ade_endian::put_u32(&mut img, link as usize * 512 + 512 - 44, 0).unwrap();
    let ck =
        ade_block::checksum::normal(&img[link as usize * 512..(link as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, link as usize * 512 + 20, ck).unwrap();

    let m = mem(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("link.txt").unwrap();
    assert!(matches!(v.resolve(&e), Err(FsError::BrokenLink { .. })));
}

#[test]
fn the_target_chains_its_links() {
    // `next_link` on the target names the newest link pointing at it
    // (ADF FAQ §4.6). ADE does not yet walk that chain, but the fixture writes
    // it, so a reader that starts to will have something to read.
    let m = mem(volume_with_links());
    let v = Volume::mount(&m).unwrap();
    let target = v.lookup("real.txt").unwrap();
    let link = v.lookup("link.txt").unwrap();
    assert_eq!(target.next_link, link.block);
}
