//! Searching an image, and naming what owns each hit (F-021).
//!
//! The byte search is tested in `ade-object`. What is tested here is the part
//! only a mounted volume can answer: **which file a match landed in**, and the
//! honest `null` when nothing points at that block at all.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over volumes they construct"
)]

use ade_core::find::{Region, Search};
use ade_core::layers::object::find::Pattern;
use ade_fixtures::Volume as Fixture;

fn pattern(s: &str) -> Pattern {
    Pattern::parse(s, true, false).unwrap()
}

#[test]
fn a_match_inside_a_file_names_the_file() {
    let mut v = Fixture::dd(1).named("Search");
    v.add_file("s/startup-sequence", b"C:SetPatch QUIET\nLoadWB\n");
    v.add_file("readme", b"nothing to see");
    let found = Search::run(&v.build(), &pattern("LoadWB"));

    assert_eq!(found.matches.len(), 1);
    assert_eq!(
        found.matches[0].owner.as_deref(),
        Some("s/startup-sequence"),
        "the answer a hex editor cannot give"
    );
}

#[test]
fn a_hit_in_the_bootblock_is_named_as_such_not_as_empty_space() {
    // The measurement that made `Region` exist: 103 corpus images carry the
    // string `Copylock` and 86 of them have it in block 0. Calling the
    // bootblock "unallocated" would be wrong about the most deliberately
    // written block on the disk, on precisely the search people most want to
    // run.
    let mut v = Fixture::dd(1).named("Boot");
    v.add_file("readme", b"hello");
    let mut bytes = v.build();
    bytes[100..108].copy_from_slice(b"Copylock");

    let found = Search::run(&bytes, &pattern("Copylock"));
    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].region, Region::Bootblock);
    assert_eq!(found.matches[0].owner, None, "a bootblock is not a file");
}

#[test]
fn a_bootblock_is_named_even_when_the_volume_does_not_mount() {
    // C-008: the bootblock and the filesystem are independent facts. This is
    // where that matters most — a protected disk is both the one that fails to
    // mount and the one whose hits are all in block 0, so an early return on
    // an unmountable volume would lose the answer exactly when it is wanted.
    let mut bytes = vec![0u8; 901_120];
    bytes[100..108].copy_from_slice(b"Copylock");
    let found = Search::run(&bytes, &pattern("Copylock"));

    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].region, Region::Bootblock);
}

#[test]
fn a_directory_is_named_by_its_header_block() {
    // A directory has no data blocks, but its name lives in its header.
    let mut v = Fixture::dd(1).named("Dirs");
    v.add_dir("Utilities");
    let found = Search::run(&v.build(), &pattern("Utilities"));

    let dir = found
        .matches
        .iter()
        .find(|m| m.region == Region::Directory)
        .expect("the directory header");
    assert_eq!(dir.owner.as_deref(), Some("Utilities"));
}

#[test]
fn a_match_in_a_block_nothing_points_at_says_so() {
    // Often the more interesting answer: content in space the filesystem does
    // not claim is deleted, hidden, or damage. Reporting it as owned by
    // whatever file happens to sit nearby would be a lie; reporting no owner
    // is the finding.
    let mut v = Fixture::dd(1).named("Unclaimed");
    v.add_file("readme", b"hello");
    let mut bytes = v.build();
    // A block past everything the fixture allocates.
    let at = 1500 * 512;
    bytes[at..at + 6].copy_from_slice(b"HIDDEN");

    let found = Search::run(&bytes, &pattern("HIDDEN"));
    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].owner, None);
    assert_eq!(found.matches[0].region, Region::Unclaimed);
    assert_eq!(found.matches[0].at.block, 1500);
}

#[test]
fn a_file_header_belongs_to_its_own_file() {
    // A filename lives in its header block, which is not a data block. Left
    // out of the owner map, searching for a filename would report it in an
    // unowned block — technically true of the data extent, and useless.
    let mut v = Fixture::dd(1).named("Headers");
    v.add_file("Distinctive", b"x");
    let found = Search::run(&v.build(), &pattern("Distinctive"));

    assert!(!found.matches.is_empty());
    assert_eq!(found.matches[0].owner.as_deref(), Some("Distinctive"));
}

#[test]
fn an_image_with_no_volume_still_searches() {
    // A quarter of real images do not mount. A search must still work there —
    // it simply cannot name an owner, which is exactly when someone is
    // searching in the first place.
    let mut bytes = vec![0u8; 901_120];
    bytes[4096..4100].copy_from_slice(b"FIND");
    let found = Search::run(&bytes, &pattern("FIND"));

    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].owner, None);
    assert_eq!(found.scanned, 901_120);
}

#[test]
fn the_search_reports_what_it_read_the_pattern_as() {
    // The one thing that makes the hex-or-text guess safe: it is visible.
    let hex = Pattern::parse("60 1A", false, false).unwrap();
    assert!(Search::run(&vec![0u8; 512], &hex).was_hex);

    let text = Pattern::parse("Copylock", false, false).unwrap();
    assert!(!Search::run(&vec![0u8; 512], &text).was_hex);
}

#[test]
fn every_match_is_reported_not_just_the_first_per_block() {
    let mut v = Fixture::dd(1).named("Repeats");
    v.add_file("filler", &b"MARK".repeat(64));
    let found = Search::run(&v.build(), &pattern("MARK"));
    assert_eq!(found.matches.len(), 64);
    assert!(
        found
            .matches
            .iter()
            .all(|m| m.owner.as_deref() == Some("filler")),
        "all of them inside the one file"
    );
}

#[test]
fn a_range_read_matches_the_image_it_came_from() {
    // The hex view of a whole disk reads through this, and a range that is off
    // by a block shows the wrong bytes under the right colour — which looks
    // like a bug in the map rather than in the read.
    let mut v = Fixture::dd(1).named("Ranges");
    v.add_file("readme", b"hello");
    let bytes = v.build();
    let image = ade_core::Image::from_bytes(bytes.clone()).unwrap();

    assert_eq!(image.read_range(0, 512), bytes[..512], "the first block");
    assert_eq!(
        image.read_range(1000, 24),
        bytes[1000..1024],
        "a range crossing a block boundary"
    );
    assert_eq!(image.read_range(3, 5), bytes[3..8], "unaligned and short");
    assert_eq!(image.read_range(0, bytes.len() as u64), bytes, "all of it");
}

#[test]
fn a_range_past_the_end_is_truncated_rather_than_refused() {
    // A hex view asks for a round number of bytes at the bottom of the disk
    // and should get what is there, not an error.
    let v = Fixture::dd(1).named("Edges");
    let bytes = v.build();
    let size = bytes.len() as u64;

    assert_eq!(
        image_range(&bytes, size - 10, 512).len(),
        10,
        "a short tail"
    );
    assert!(image_range(&bytes, size, 512).is_empty(), "exactly the end");
    assert!(
        image_range(&bytes, size + 4096, 512).is_empty(),
        "well past"
    );
    assert!(image_range(&bytes, 0, 0).is_empty(), "nothing asked for");
}

/// Read a range through a freshly mounted image.
fn image_range(bytes: &[u8], offset: u64, length: u64) -> Vec<u8> {
    ade_core::Image::from_bytes(bytes.to_vec())
        .map(|i| i.read_range(offset, length))
        .unwrap_or_default()
}
