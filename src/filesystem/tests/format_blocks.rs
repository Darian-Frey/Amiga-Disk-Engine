//! A quick format as only the blocks it writes (IMP-027).
//!
//! `format::blank_blocks` writes no volume, only the blocks a format writes;
//! `format::blank_named` writes the whole volume. Laid over zeros, the first
//! must be the second byte for byte — every filesystem, floppies, odd sizes,
//! and hard disks past the 25 bitmap pages the rootblock holds — and name
//! every block the whole format makes non-zero.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "test scaffolding over volumes it formats"
)]

use ade_block::Geometry;
use ade_filesystem::dostype::Dostype;
use ade_filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

#[test]
fn the_blocks_a_format_writes_are_the_whole_format() {
    let shapes = [
        Geometry::new(80, 2, 11, 512, 2).unwrap(),
        Geometry::new(80, 2, 22, 512, 2).unwrap(),
        Geometry::new(4097, 1, 1, 512, 2).unwrap(),
        Geometry::new(256, 1, 32, 512, 2).unwrap(),
        Geometry::new(6400, 1, 32, 512, 2).unwrap(), // 100 MB: a bm_ext chain
    ];
    for geometry in shapes {
        for flags in 0..=5u32 {
            let dostype = Dostype::from_raw(0x444F_5300 | flags).unwrap();
            let whole = format::blank_named(geometry, dostype, b"Same", WHEN).unwrap();
            let blocks = format::blank_blocks(geometry, dostype, b"Same", WHEN).unwrap();
            let mut laid = vec![0u8; whole.len()];
            for (&b, raw) in &blocks {
                let at = b as usize * 512;
                laid[at..at + 512].copy_from_slice(raw);
            }
            assert!(
                laid == whole,
                "DOS\\{flags} on {} blocks",
                geometry.total_blocks()
            );
            for (i, block) in whole.chunks(512).enumerate() {
                if block.iter().any(|&x| x != 0) {
                    assert!(blocks.contains_key(&(i as u64)), "block {i} is not named");
                }
            }
            // A handful of blocks, not a volume: the point of the change.
            assert!(blocks.len() < 200, "{} blocks", blocks.len());
            for b in 0..u64::from(geometry.reserved()) {
                assert!(blocks.contains_key(&b), "reserved block {b}, zeroed");
            }
        }
    }
}
