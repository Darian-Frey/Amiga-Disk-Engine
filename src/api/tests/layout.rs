//! Mapping what occupies each block of a disk (F-022).
//!
//! The map is what a whole-disk hex view colours from, so its invariant is not
//! "mostly right": it must tile the image exactly. A gap is a run of bytes the
//! view paints in no colour at all, which reads as ordinary data — the map
//! being wrong looks exactly like the disk being different.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over volumes they construct"
)]

use ade_core::Image;
use ade_core::layout::{Layout, Region};
use ade_fixtures::Volume as Fixture;

fn map(bytes: Vec<u8>) -> Layout {
    Layout::of(&Image::from_bytes(bytes).expect("a mountable image"))
}

/// Every region the map found, in offset order, deduplicated.
fn regions(map: &Layout) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::new();
    for span in &map.spans {
        if out.last() != Some(&span.region) {
            out.push(span.region);
        }
    }
    out
}

#[test]
fn the_spans_tile_the_image_exactly() {
    let mut v = Fixture::dd(1).named("Tiled");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    v.add_file("Tools/big", &vec![7u8; 20_000]);
    let map = map(v.build());

    let mut at = 0u64;
    let mut blocks = 0u64;
    for (i, span) in map.spans.iter().enumerate() {
        assert_eq!(span.start, at, "span {i} does not follow the one before it");
        assert!(span.end > span.start, "span {i} is empty");
        assert_eq!(
            span.end - span.start,
            span.blocks * u64::from(map.block_size),
            "span {i}'s bytes and blocks disagree"
        );
        at = span.end;
        blocks += span.blocks;
    }
    assert_eq!(
        blocks, map.blocks,
        "every block is accounted for exactly once"
    );
    assert_eq!(at, map.blocks * u64::from(map.block_size), "and every byte");
}

#[test]
fn the_structures_are_where_the_format_says() {
    let mut v = Fixture::dd(1).named("Structures");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    let map = map(v.build());

    let at = |block: u64| {
        map.spans
            .iter()
            .find(|s| block >= s.block && block < s.block + s.blocks)
            .map(|s| s.region)
    };
    assert_eq!(at(0), Some(Region::Bootblock), "block 0 boots the disk");
    assert_eq!(at(1), Some(Region::Bootblock), "and so does block 1");
    assert_eq!(at(880), Some(Region::Rootblock), "a DD floppy's rootblock");
    assert!(
        regions(&map).contains(&Region::Bitmap),
        "the bitmap is named, not left as unclaimed space"
    );
    assert!(regions(&map).contains(&Region::Directory));
    assert!(regions(&map).contains(&Region::File));
    assert!(
        regions(&map).contains(&Region::Unclaimed),
        "a disk with room left"
    );
}

#[test]
fn a_file_span_names_the_file() {
    let mut v = Fixture::dd(1).named("Owners");
    v.add_file("Distinctive", &vec![1u8; 3000]);
    let map = map(v.build());

    let owned: Vec<&str> = map
        .spans
        .iter()
        .filter(|s| s.region == Region::File)
        .filter_map(|s| s.owner.as_deref())
        .collect();
    assert!(!owned.is_empty(), "a file's blocks say which file");
    assert!(owned.iter().all(|o| *o == "Distinctive"), "{owned:?}");
}

#[test]
fn adjacent_blocks_of_one_file_become_one_span() {
    // The point of coalescing. A 20 KB file is forty blocks and one row.
    let mut v = Fixture::dd(1).named("Runs");
    v.add_file("big", &vec![9u8; 20_000]);
    let map = map(v.build());

    let longest = map
        .spans
        .iter()
        .filter(|s| s.region == Region::File)
        .map(|s| s.blocks)
        .max()
        .unwrap_or(0);
    assert!(
        longest > 10,
        "consecutive file blocks must merge, got {longest}"
    );
}

#[test]
fn two_files_side_by_side_stay_two_spans() {
    // Not merged, even when their blocks touch: the owner is what a reader
    // wants named, and one span for both would be attributed to whichever
    // came first — a confident wrong answer.
    let mut v = Fixture::dd(1).named("Neighbours");
    v.add_file("first", &vec![1u8; 2000]);
    v.add_file("second", &vec![2u8; 2000]);
    let map = map(v.build());

    let mut names: Vec<&str> = map
        .spans
        .iter()
        .filter_map(|s| s.owner.as_deref())
        .collect();
    names.dedup();
    assert!(
        names.contains(&"first") && names.contains(&"second"),
        "{names:?}"
    );
}

#[test]
fn an_image_with_no_volume_still_maps_its_bootblock() {
    // C-008: the bootblock and the filesystem are independent facts, and a
    // quarter of real images do not mount. A hex view of one still wants to
    // know where its bootblock ends — that is where protection lives, and an
    // unmountable disk is usually a protected one.
    let bytes = vec![0u8; 901_120];
    let map = map(bytes);

    assert!(!map.mounted);
    assert_eq!(map.spans.first().map(|s| s.region), Some(Region::Bootblock));
    assert_eq!(map.spans.first().map(|s| s.blocks), Some(2));
    assert!(
        map.spans
            .iter()
            .skip(1)
            .all(|s| s.region == Region::Unclaimed),
        "and everything past it is honestly unclaimed"
    );
    // Still tiles.
    assert_eq!(map.spans.iter().map(|s| s.blocks).sum::<u64>(), map.blocks);
}

#[test]
fn the_totals_add_up_to_the_disk() {
    let mut v = Fixture::dd(1).named("Totals");
    v.add_file("readme", b"hello");
    let map = map(v.build());

    let summed: u64 = map.totals().iter().map(|(_, blocks)| blocks).sum();
    assert_eq!(summed, map.blocks, "a summary that does not sum is not one");
}
