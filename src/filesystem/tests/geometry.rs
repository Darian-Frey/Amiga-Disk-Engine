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
