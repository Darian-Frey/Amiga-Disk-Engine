//! Geometries beyond the canonical 880 KB floppy (Phase 2).
//!
//! The corpus has none of these — all 4652 images are 80-cylinder DD — so they
//! are generated and cross-checked against ADFlib by `oracle_fixtures`. See
//! D-010's 2026-08-24 amendment for why that is a real check rather than the
//! generator agreeing with itself.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests over data they construct"
)]

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock};
use ade_filesystem::{dostype::FileSystem, volume::Volume};
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

#[test]
fn a_high_density_volume_mounts_and_round_trips() {
    let payload: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    let mut f = Fixture::hd(1).named("HighDensity");
    f.add_file("big.bin", &payload);
    f.add_dir("Tools");
    let img = f.build();
    assert_eq!(img.len(), 1_802_240, "1.76 MB");

    let m = Mem {
        bytes: img,
        geometry: Geometry::HD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();

    // The bootblock claims 880 even on HD; the real rootblock is at 1760.
    // Reading rather than computing it is C-007's trap.
    assert_eq!(v.root(), 1760);
    assert_eq!(v.rootblock().name_lossy(), "HighDensity");
    assert_eq!(v.filesystem(), FileSystem::Ffs);

    let e = v.lookup("big.bin").unwrap();
    assert_eq!(v.read_file(&e).unwrap().bytes, payload);
    assert_eq!(v.walk(v.root()).unwrap().entries.len(), 2);
}

#[test]
fn high_density_ofs_round_trips_too() {
    // OFS on HD exercises the 488-byte payload against a different geometry.
    let payload: Vec<u8> = (0..15_000u32).map(|i| (i % 97) as u8).collect();
    let mut f = Fixture::hd(0).named("HDOfs");
    f.add_file("data", &payload);
    let m = Mem {
        bytes: f.build(),
        geometry: Geometry::HD_FLOPPY,
    };
    let v = Volume::mount(&m).unwrap();
    assert_eq!(v.filesystem(), FileSystem::Ofs);
    let e = v.lookup("data").unwrap();
    let got = v.read_file(&e).unwrap();
    assert_eq!(got.bytes, payload);
    assert!(got.faults.is_empty(), "{:?}", got.faults);
}

#[test]
fn extra_cylinder_geometries_mount() {
    // 81-83 cylinder images occur in the wild (SPEC §Corpus observations), and
    // the corpus holds five of them.
    for cyl in [81u32, 82, 83] {
        let geometry = Geometry::new(cyl, 2, 11, 512, Geometry::FLOPPY_RESERVED).unwrap();
        let mut f = Fixture::new(cyl, 2, 11, 1).named("Extra");
        f.add_file("f", b"payload");
        let m = Mem {
            bytes: f.build(),
            geometry,
        };
        let v = Volume::mount(&m).unwrap();
        assert_eq!(v.lookup("f").unwrap().byte_size, 7, "{cyl} cylinders");
        // rootKey = (reserved + highKey) / 2, not half the block count.
        assert_eq!(u64::from(v.root()), geometry.root_block().0);
    }
}

#[test]
fn the_rootblock_moves_with_the_geometry() {
    // Three geometries, three different rootblock locations, none of them read
    // from the bootblock (C-007).
    for (cyl, heads, sectors, want) in [
        (80u32, 2u32, 11u32, 880u64),
        (80, 2, 22, 1760),
        (83, 2, 11, 913),
    ] {
        let g = Geometry::new(cyl, heads, sectors, 512, Geometry::FLOPPY_RESERVED).unwrap();
        assert_eq!(g.root_block().0, want, "{cyl}x{heads}x{sectors}");
    }
}

#[test]
fn a_hardfile_mounts_and_needs_several_bitmap_blocks() {
    // 8 MB in the UAE shape: 1 head, 32 sectors. One 512-byte bitmap block
    // covers 4064 blocks, so this needs five — the case that used to panic the
    // fixture generator (BUG-006).
    let mut f = Fixture::new(512, 1, 32, 1).named("Hardfile");
    f.add_file("readme", b"a hardfile, not a floppy");
    f.add_dir("Tools");
    let img = f.build();
    assert_eq!(img.len(), 8_388_608);

    // A raw volume carries no geometry; only the block count matters.
    let geometry = Geometry::new(16_384, 1, 1, 512, Geometry::FLOPPY_RESERVED).unwrap();
    let m = Mem {
        bytes: img,
        geometry,
    };
    let v = Volume::mount(&m).unwrap();
    assert_eq!(v.root(), 8192, "(2 + 16383) / 2");
    assert_eq!(v.rootblock().name_lossy(), "Hardfile");
    assert_eq!(v.walk(v.root()).unwrap().entries.len(), 2);

    let bm = ade_filesystem::bitmap::Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();
    assert_eq!(
        bm.blocks.len(),
        5,
        "five bitmap blocks for 16382 covered blocks"
    );
    assert!(bm.bad_checksums.is_empty());
    assert!(!bm.incomplete, "the map must cover the whole volume");
}

#[test]
fn a_hardfile_bitmap_survives_a_rebuild() {
    // Multi-block bitmaps are where the per-block windowing could go wrong.
    use std::collections::HashSet;
    let mut f = Fixture::new(512, 1, 32, 0).named("BigOfs");
    // Under 72 blocks: the fixture generator cannot build file extension
    // blocks yet (IMP-004), and the point here is the multi-block *bitmap*.
    f.add_file("data", &vec![5u8; 30_000]);
    let geometry = Geometry::new(16_384, 1, 1, 512, Geometry::FLOPPY_RESERVED).unwrap();
    let m = Mem {
        bytes: f.build(),
        geometry,
    };
    let v = Volume::mount(&m).unwrap();
    let bm = ade_filesystem::bitmap::Bitmap::read(&m, v.geometry(), v.rootblock()).unwrap();

    let mut reach: HashSet<u32> = HashSet::from([v.root()]);
    reach.extend(bm.blocks.iter().copied());
    for (_, e) in v.walk(v.root()).unwrap().entries {
        reach.insert(e.block);
        if let Ok(b) = v.file_blocks(&e) {
            reach.extend(b);
        }
    }
    assert!(bm.referenced_but_free(&reach).is_empty());
    assert!(bm.orphaned(&reach).is_empty());

    let rebuilt = ade_filesystem::bitmap::Bitmap::rebuild(&reach, v.geometry(), &bm.blocks);
    assert_eq!(rebuilt.len(), 5);
    let mut bytes = m.bytes.clone();
    for (block, data) in &rebuilt {
        let o = *block as usize * 512;
        bytes[o..o + 512].copy_from_slice(data);
    }
    let m2 = Mem { bytes, geometry };
    let v2 = Volume::mount(&m2).unwrap();
    let back = ade_filesystem::bitmap::Bitmap::read(&m2, v2.geometry(), v2.rootblock()).unwrap();
    assert_eq!(
        back.allocated(),
        &reach,
        "a five-block bitmap must round-trip too"
    );
}
