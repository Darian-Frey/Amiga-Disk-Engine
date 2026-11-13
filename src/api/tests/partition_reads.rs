//! Reading a partitioned hard disk as its partitions (BUG-027).
//!
//! A device has no volume of its own: its volumes are in its partitions.
//! `carve`, the layout behind `find` and the window's Map, and the damage the
//! Map draws once looked for the device's own volume, found none, and called
//! every block unclaimed — 367 live files on a sound Workbench hard disk came
//! back as lost. These hold each of them to the partitions, on a two-partition
//! device built by the fixture generator, which shares no code with ADE's
//! readers (D-010).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "test scaffolding over data it builds"
)]

use ade_core::Image;
use ade_core::carve::{self, Evidence};
use ade_core::find::{Region as FindRegion, Search};
use ade_core::layers::endian::put_u32;
use ade_core::layers::object::find::Pattern;
use ade_core::layout::{Layout, Region};
use ade_fixtures::device::Device;

/// `DH0`, FFS, holding `startup` and a directory; `DH1`, OFS, holding
/// `data.bin` — OFS, so a lost file there can be confirmed by its own blocks.
fn device() -> Vec<u8> {
    let mut d = Device::new(64, 4, 32);
    d.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello from DH0");
        v.add_dir("Tools");
    });
    d.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    d.build()
}

/// Where a partition starts on the device, and its rootblock there.
fn partition(bytes: &[u8], name: &str) -> (u64, u64) {
    let inspection = ade_core::inspect_bytes(bytes.to_vec());
    let p = inspection
        .partitions
        .iter()
        .find(|p| p.name == name)
        .unwrap();
    // A partition's rootblock sits at the midpoint of its own blocks.
    let root = p.first_block + (p.blocks - 1 + u64::from(p.reserved)).div_euclid(2);
    (p.first_block, root)
}

#[test]
fn a_sound_partitioned_disk_has_nothing_to_carve() {
    let image = Image::from_bytes(device()).unwrap();
    let found = carve::carve(&image);
    assert!(found.is_empty(), "every file is live: {found:?}");
}

#[test]
fn the_layout_maps_every_partition_and_the_table() {
    let image = Image::from_bytes(device()).unwrap();
    let map = Layout::of(&image);
    assert!(map.mounted);
    let owners: Vec<&str> = map
        .spans
        .iter()
        .filter_map(|s| s.owner.as_deref())
        .collect();
    assert!(owners.contains(&"DH0:startup"), "{owners:?}");
    assert!(owners.contains(&"DH0:Tools"), "{owners:?}");
    assert!(owners.contains(&"DH1:data.bin"), "{owners:?}");
    let regions: Vec<Region> = map.spans.iter().map(|s| s.region).collect();
    for wanted in [
        Region::PartitionTable,
        Region::Bootblock,
        Region::Rootblock,
        Region::Bitmap,
        Region::Directory,
        Region::File,
    ] {
        assert!(regions.contains(&wanted), "{wanted:?} missing");
    }
    // Two rootblocks: one in each partition, each at its own midpoint.
    let bytes = device();
    for name in ["DH0", "DH1"] {
        let (_, root) = partition(&bytes, name);
        let span = map
            .spans
            .iter()
            .find(|s| s.block <= root && root < s.block + s.blocks)
            .unwrap();
        assert_eq!(span.region, Region::Rootblock, "{name}'s rootblock");
    }
}

#[test]
fn a_search_hit_is_the_file_it_is_in() {
    let image = Image::from_bytes(device()).unwrap();
    let pattern = Pattern::parse("hello from DH0", true, false).unwrap();
    let search = Search::of_image(&image, &pattern);
    let hit = search.matches.first().expect("found");
    assert_eq!(hit.region, FindRegion::File);
    assert_eq!(hit.owner.as_deref(), Some("DH0:startup"));
}

#[test]
fn a_file_lost_in_one_partition_is_carved_from_it() {
    // Clearing DH1's root hash table is what losing every entry looks like
    // from outside, as `carve.rs` tests it on a floppy; the header and its OFS
    // data blocks stay, each naming the header back.
    let mut bytes = device();
    let (first, root) = partition(&bytes, "DH1");
    let at = root as usize * 512;
    for slot in 0..72 {
        put_u32(&mut bytes, at + 24 + slot * 4, 0).unwrap();
    }
    put_u32(&mut bytes, at + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[at..at + 512], 20).unwrap();
    put_u32(&mut bytes, at + 20, sum).unwrap();

    let image = Image::from_bytes(bytes).unwrap();
    let found = carve::carve(&image);
    assert_eq!(found.len(), 1, "{found:?}");
    let lost = &found[0];
    assert_eq!(lost.name, "data.bin");
    assert_eq!(lost.partition.as_deref(), Some("DH1"));
    assert!(
        u64::from(lost.block) < root - first,
        "numbered from DH1's start"
    );
    assert_eq!(lost.evidence, Evidence::SelfEvident);
    assert_eq!(carve::contents(&image, lost), vec![0xAA; 3000]);
    assert!(carve::recovered_name(lost).starts_with("DH1-"));
}

#[test]
fn damage_in_a_partition_is_drawn_where_it_is_on_the_device() {
    // Mark DH1's bitmap all free: its rootblock, bitmap and file blocks are
    // then in use but marked free. Each must be reported inside DH1 on the
    // device, not at the same numbers counted from block 0.
    let mut bytes = device();
    let (first, root) = partition(&bytes, "DH1");
    let bitmap = u64::from(
        ade_core::layers::endian::u32_at(&bytes, root as usize * 512 + 512 - 196).unwrap(),
    );
    let at = ((first + bitmap) * 512) as usize;
    for word in 1..128 {
        put_u32(&mut bytes, at + word * 4, 0xFFFF_FFFF).unwrap();
    }
    put_u32(&mut bytes, at, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[at..at + 512], 0).unwrap();
    put_u32(&mut bytes, at, sum).unwrap();

    let inspection = ade_core::inspect_bytes(bytes.clone());
    let dh1 = inspection
        .partitions
        .iter()
        .find(|p| p.name == "DH1")
        .unwrap();
    let damaged = ade_core::image_damage(&bytes);
    assert_ne!(damaged.len(), 0);
    for (block, _) in &damaged {
        let block = u64::from(*block);
        assert!(
            dh1.first_block <= block && block < dh1.first_block + dh1.blocks,
            "block {block} is outside DH1 ({}..{})",
            dh1.first_block,
            dh1.first_block + dh1.blocks
        );
    }
    assert!(
        damaged.iter().any(|(b, _)| u64::from(*b) == root),
        "DH1's rootblock"
    );
}
