//! Files large enough to need extension blocks (IMP-004).
//!
//! A file's header holds `BSIZE/4 - 56` data-block pointers — 72 at 512 bytes.
//! Anything larger stores the rest in *file extension* blocks chained from the
//! header's `extension` field (SPEC §Files).
//!
//! ADE has read that chain since Phase 1 — it is how `taterm1` was recovered
//! from a real disk — but until the generator could build one, the path had no
//! fixture, no test on a fresh clone, and no oracle. These tests close that,
//! and with it the AV-001 visited set guarding the chain against loops.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock};
use ade_filesystem::volume::{FsError, Volume};
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

/// A hardfile geometry with room for a genuinely large file.
fn hardfile(bytes: Vec<u8>) -> Mem {
    let blocks = u32::try_from(bytes.len() / 512).unwrap();
    Mem {
        bytes,
        geometry: Geometry::new(blocks, 1, 1, 512, Geometry::FLOPPY_RESERVED).unwrap(),
    }
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

#[test]
fn a_file_spanning_many_extension_blocks_round_trips() {
    for dostype in [0u8, 1] {
        let data = payload(200_000);
        let mut f = Fixture::new(512, 1, 32, dostype).named("Ext");
        f.add_file("big.bin", &data);
        let m = hardfile(f.build());
        let v = Volume::mount(&m).unwrap();
        let e = v.lookup("big.bin").unwrap();

        let got = v.read_file(&e).unwrap();
        assert_eq!(got.bytes, data, "dostype {dostype}");
        assert!(got.is_complete(), "{:?}", got.faults);

        // The point is that the chain is genuinely walked: far more blocks
        // than a single header's 72 pointers can name.
        let blocks = v.file_blocks(&e).unwrap();
        let ht = v.hash_table_size() as usize;
        assert!(
            blocks.len() > ht * 4,
            "dostype {dostype}: {} blocks, expected several extension blocks' worth",
            blocks.len()
        );
        assert_ne!(e.extension, 0, "dostype {dostype}: no extension chain");
    }
}

#[test]
fn data_stays_in_order_across_the_extension_chain() {
    // The pointer table runs backwards *within* each block while the blocks
    // themselves run forwards. Getting either wrong reverses part of the file,
    // which a symmetric payload would hide.
    let data = payload(120_000);
    let mut f = Fixture::new(512, 1, 32, 1).named("Order");
    f.add_file("ordered", &data);
    let m = hardfile(f.build());
    let v = Volume::mount(&m).unwrap();
    let got = v.read_file(&v.lookup("ordered").unwrap()).unwrap();
    assert_eq!(got.bytes, data);
    assert_eq!(got.bytes[0], data[0]);
    assert_eq!(got.bytes[119_999], data[119_999]);
}

#[test]
fn a_self_looping_extension_chain_terminates() {
    // AV-001 on the extension chain. Until IMP-004 this had no fixture.
    let mut f = Fixture::new(512, 1, 32, 1).named("Loop");
    let hdr = f.add_file("big.bin", &payload(120_000));
    let mut img = f.build();

    let m0 = hardfile(img.clone());
    let v0 = Volume::mount(&m0).unwrap();
    let ext = ade_endian::u32_at(&m0.bytes, hdr as usize * 512 + 512 - 8).unwrap();
    assert_ne!(ext, 0, "the fixture must actually have an extension block");
    drop(v0);

    corrupt::extension_chain_loop(&mut img, ext);
    let m = hardfile(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("big.bin").unwrap();
    // The point is that this returns.
    match v.read_file(&e) {
        Err(FsError::Cycle { chain, .. }) => assert_eq!(chain, "file extension"),
        other => panic!("expected a reported cycle, got {other:?}"),
    }
}

#[test]
fn a_two_block_extension_cycle_terminates() {
    // A "next != self" check catches the self-loop and misses this.
    let mut f = Fixture::new(512, 1, 32, 1).named("Cycle");
    let hdr = f.add_file("big.bin", &payload(200_000));
    let mut img = f.build();
    let first = ade_endian::u32_at(&img, hdr as usize * 512 + 512 - 8).unwrap();
    let second = ade_endian::u32_at(&img, first as usize * 512 + 512 - 8).unwrap();
    assert_ne!(second, 0, "need at least two extension blocks");

    corrupt::extension_chain_cycle(&mut img, first, second);
    let m = hardfile(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("big.bin").unwrap();
    assert!(matches!(v.read_file(&e), Err(FsError::Cycle { .. })));
    // file_blocks walks the same chain and must also terminate.
    assert!(v.file_blocks(&e).is_ok());
}

#[test]
fn an_extension_pointer_outside_the_volume_is_refused() {
    // AV-004: `extension` is a pointer off the disk like any other.
    let mut f = Fixture::new(512, 1, 32, 1).named("Wild");
    let hdr = f.add_file("big.bin", &payload(120_000));
    let mut img = f.build();
    ade_endian::put_u32(&mut img, hdr as usize * 512 + 512 - 8, 0xFFFF_FFFF).unwrap();
    let ck =
        ade_block::checksum::normal(&img[hdr as usize * 512..(hdr as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, hdr as usize * 512 + 20, ck).unwrap();

    let m = hardfile(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("big.bin").unwrap();
    assert!(matches!(v.read_file(&e), Err(FsError::Malformed { .. })));
}

#[test]
fn a_block_that_is_not_t_list_ends_the_chain_loudly() {
    let mut f = Fixture::new(512, 1, 32, 1).named("NotList");
    let hdr = f.add_file("big.bin", &payload(120_000));
    let mut img = f.build();
    let ext = ade_endian::u32_at(&img, hdr as usize * 512 + 512 - 8).unwrap();
    // Make it claim to be a data block instead.
    ade_endian::put_u32(&mut img, ext as usize * 512, 8).unwrap();
    let ck =
        ade_block::checksum::normal(&img[ext as usize * 512..(ext as usize + 1) * 512]).unwrap();
    ade_endian::put_u32(&mut img, ext as usize * 512 + 20, ck).unwrap();

    let m = hardfile(img);
    let v = Volume::mount(&m).unwrap();
    let e = v.lookup("big.bin").unwrap();
    assert!(matches!(v.read_file(&e), Err(FsError::Malformed { .. })));
}
