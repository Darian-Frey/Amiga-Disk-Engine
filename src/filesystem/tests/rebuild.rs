//! Rebuilding the allocation bitmap (Phase 2, AV-003).
//!
//! The rebuild is computed and returned, never written: D-004 defers write
//! paths to Phase 4, and AV-003 asks for a rebuild to be *offered*. These tests
//! prove it correct without touching a disk — build the corrected bitmap, read
//! it back, and check it describes exactly the set it was built from.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests over data they construct"
)]

use std::collections::HashSet;

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock, checksum};
use ade_filesystem::{bitmap::Bitmap, volume::Volume};
use ade_fixtures::{Volume as Fixture, corrupt};

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
        let src = self
            .bytes
            .get(start..start + size)
            .ok_or(BlockError::Truncated {
                index: BlockIndex(block.index()),
            })?;
        out.copy_from_slice(src);
        Ok(())
    }
}

/// Every block the volume's tree reaches, plus the filesystem's own overhead.
fn reachable(v: &Volume<'_>, bitmap: &Bitmap) -> HashSet<u32> {
    let mut set: HashSet<u32> = HashSet::from([v.root()]);
    for &b in &bitmap.blocks {
        set.insert(b);
    }
    for (_, e) in v.walk(v.root()).unwrap().entries {
        set.insert(e.block);
        if e.kind.is_file()
            && let Ok(blocks) = v.file_blocks(&e)
        {
            set.extend(blocks);
        }
    }
    set
}

fn sample_volume() -> Vec<u8> {
    let mut f = Fixture::dd(0).named("Rebuild");
    f.add_file("small", b"x");
    f.add_file("big", &vec![7u8; 6000]);
    f.add_dir("Tools");
    f.build()
}

#[test]
fn a_rebuilt_bitmap_describes_exactly_what_the_tree_reaches() {
    // The round-trip that makes this checkable without writing anything.
    let m = Mem {
        bytes: sample_volume(),
        geometry: Geometry::DD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();
    let original = Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();
    let reach = reachable(&v, &original);

    let rebuilt = Bitmap::rebuild(&reach, v.geometry(), &original.blocks);
    assert_eq!(rebuilt.len(), original.blocks.len());

    // Splice the rebuilt blocks into a copy and read them back.
    let mut bytes = m.bytes.clone();
    for (block, data) in &rebuilt {
        let o = *block as usize * 512;
        bytes[o..o + 512].copy_from_slice(data);
    }
    let m2 = Mem {
        bytes,
        geometry: Geometry::DD_FLOPPY,
    };
    let v2 = Volume::mount(&m2).unwrap();
    let read_back = Bitmap::read(&m2, v2.geometry(), v2.rootblock()).unwrap();

    assert_eq!(
        read_back.allocated(),
        &reach,
        "a rebuilt bitmap must describe the set it was built from"
    );
    assert!(read_back.orphaned(&reach).is_empty());
    assert!(read_back.referenced_but_free(&reach).is_empty());
}

#[test]
fn the_rebuilt_blocks_checksum_at_offset_zero() {
    // The bitmap block is the one exception to block layout (BUG-004).
    let m = Mem {
        bytes: sample_volume(),
        geometry: Geometry::DD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();
    let original = Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();
    let reach = reachable(&v, &original);
    for (_, data) in Bitmap::rebuild(&reach, v.geometry(), &original.blocks) {
        assert!(
            checksum::sums_to_zero(&data),
            "rebuilt block fails its checksum"
        );
        let stored = ade_endian::u32_at(&data, checksum::BITMAP_OFFSET).unwrap();
        assert_eq!(
            stored,
            checksum::normal_at(&data, checksum::BITMAP_OFFSET).unwrap(),
            "the checksum must sit at offset 0, not 20"
        );
    }
}

#[test]
fn a_rebuild_repairs_a_bitmap_that_marks_live_data_free() {
    // The dangerous case, found on real disks: a block a file uses that the
    // bitmap says is available. The next write would destroy it.
    let mut f = Fixture::dd(0).named("AtRisk");
    let file = f.add_file("victim", &vec![3u8; 3000]);
    let mut img = f.build();

    let m0 = Mem {
        bytes: img.clone(),
        geometry: Geometry::DD_FLOPPY,
    };
    let v0 = Volume::mount(&m0).unwrap();
    let bm0 = Bitmap::read(&m0, v0.geometry(), v0.rootblock()).unwrap();
    let reach = reachable(&v0, &bm0);
    let bitmap_block = bm0.blocks[0];

    // Mark the file's header free while it is plainly in use.
    let idx = file - Geometry::FLOPPY_RESERVED;
    let off = bitmap_block as usize * 512 + 4 + (idx / 32) as usize * 4;
    let word = ade_endian::u32_at(&img, off).unwrap() | (1 << (idx % 32));
    ade_endian::put_u32(&mut img, off, word).unwrap();
    let ck = checksum::normal_at(
        &img[bitmap_block as usize * 512..(bitmap_block as usize + 1) * 512],
        checksum::BITMAP_OFFSET,
    )
    .unwrap();
    ade_endian::put_u32(&mut img, bitmap_block as usize * 512, ck).unwrap();

    let m1 = Mem {
        bytes: img,
        geometry: Geometry::DD_FLOPPY,
    };
    let v1 = Volume::mount(&m1).unwrap();
    let damaged = Bitmap::read(&m1, v1.geometry(), v1.rootblock()).unwrap();
    assert_eq!(
        damaged.referenced_but_free(&reach),
        vec![file],
        "the damage must be detected before it can be repaired"
    );

    // Rebuilding puts it back.
    let rebuilt = Bitmap::rebuild(&reach, v1.geometry(), &damaged.blocks);
    let mut bytes = m1.bytes.clone();
    for (block, data) in &rebuilt {
        let o = *block as usize * 512;
        bytes[o..o + 512].copy_from_slice(data);
    }
    let m2 = Mem {
        bytes,
        geometry: Geometry::DD_FLOPPY,
    };
    let v2 = Volume::mount(&m2).unwrap();
    let fixed = Bitmap::read(&m2, v2.geometry(), v2.rootblock()).unwrap();
    assert!(fixed.referenced_but_free(&reach).is_empty());
    assert!(fixed.is_allocated(file));
}

#[test]
fn a_rebuild_reclaims_orphaned_blocks() {
    let mut f = Fixture::dd(1).named("Orphans");
    f.add_file("f", b"x");
    let mut img = f.build();

    let m0 = Mem {
        bytes: img.clone(),
        geometry: Geometry::DD_FLOPPY,
    };
    let v0 = Volume::mount(&m0).unwrap();
    let bm0 = Bitmap::read(&m0, v0.geometry(), v0.rootblock()).unwrap();
    let reach = reachable(&v0, &bm0);
    let bitmap_block = bm0.blocks[0];

    // Mark three unreachable blocks as in use.
    for block in [900u32, 901, 902] {
        let idx = block - Geometry::FLOPPY_RESERVED;
        let off = bitmap_block as usize * 512 + 4 + (idx / 32) as usize * 4;
        let word = ade_endian::u32_at(&img, off).unwrap() & !(1 << (idx % 32));
        ade_endian::put_u32(&mut img, off, word).unwrap();
    }
    let ck = checksum::normal_at(
        &img[bitmap_block as usize * 512..(bitmap_block as usize + 1) * 512],
        checksum::BITMAP_OFFSET,
    )
    .unwrap();
    ade_endian::put_u32(&mut img, bitmap_block as usize * 512, ck).unwrap();

    let m1 = Mem {
        bytes: img,
        geometry: Geometry::DD_FLOPPY,
    };
    let v1 = Volume::mount(&m1).unwrap();
    let damaged = Bitmap::read(&m1, v1.geometry(), v1.rootblock()).unwrap();
    assert_eq!(damaged.orphaned(&reach), vec![900, 901, 902]);

    let rebuilt = Bitmap::rebuild(&reach, v1.geometry(), &damaged.blocks);
    let mut bytes = m1.bytes.clone();
    for (block, data) in &rebuilt {
        let o = *block as usize * 512;
        bytes[o..o + 512].copy_from_slice(data);
    }
    let m2 = Mem {
        bytes,
        geometry: Geometry::DD_FLOPPY,
    };
    let v2 = Volume::mount(&m2).unwrap();
    let fixed = Bitmap::read(&m2, v2.geometry(), v2.rootblock()).unwrap();
    assert!(fixed.orphaned(&reach).is_empty(), "orphans must be freed");
}

#[test]
fn rebuilding_a_stale_flagged_volume_does_not_depend_on_the_flag() {
    // AV-003: the flag is advisory. A rebuild is computed from the tree, so a
    // cleared flag changes nothing about the answer.
    let f = Fixture::dd(1).named("Unclean");
    let root = f.root();
    let mut img = f.build();
    corrupt::bitmap_flag_invalid(&mut img, root);

    let m = Mem {
        bytes: img,
        geometry: Geometry::DD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();
    let bm = Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();
    assert!(!bm.flagged_valid, "the flag is clear");
    let reach = reachable(&v, &bm);
    assert!(
        bm.referenced_but_free(&reach).is_empty(),
        "yet the map itself is sound — which is the point of not trusting the flag"
    );
}

#[test]
fn rebuild_covers_high_density_volumes_too() {
    let mut f = Fixture::hd(1).named("HD");
    f.add_file("big", &vec![9u8; 30_000]);
    let m = Mem {
        bytes: f.build(),
        geometry: Geometry::HD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();
    let bm = Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();
    let reach = reachable(&v, &bm);

    let rebuilt = Bitmap::rebuild(&reach, v.geometry(), &bm.blocks);
    let mut bytes = m.bytes.clone();
    for (block, data) in &rebuilt {
        let o = *block as usize * 512;
        bytes[o..o + 512].copy_from_slice(data);
    }
    let m2 = Mem {
        bytes,
        geometry: Geometry::HD_FLOPPY,
    };
    let v2 = Volume::mount(&m2).unwrap();
    let back = Bitmap::read(&m2, v2.geometry(), v2.rootblock()).unwrap();
    assert_eq!(back.allocated(), &reach);
}
