//! Recognising content by its magic (F-020).
//!
//! # What these pin, and why each one is here
//!
//! Every rule below exists because scanning 4,652 real disks produced an
//! answer that was wrong in an instructive way. A four-byte substring search
//! over four gigabytes finds things; the work is in not lying about what they
//! are.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use ade_object::signature::{Anchor, Category, Signature, scan, scan_with};

const BLOCK: u32 = 512;

/// An image of `blocks` zero blocks with `what` written at `at`.
fn image(blocks: usize, at: usize, what: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; blocks * BLOCK as usize];
    bytes[at..at + what.len()].copy_from_slice(what);
    bytes
}

#[test]
fn an_anchored_magic_is_found_at_a_block_start() {
    let bytes = image(4, 1024, b"PP20");
    let hits = scan(&bytes, BLOCK);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "PowerPacker PP20");
    assert_eq!(hits[0].block, 2);
    assert_eq!(hits[0].offset, 1024);
    assert_eq!(hits[0].category, Category::Compressed);
}

#[test]
fn an_anchored_magic_is_ignored_mid_block() {
    // The whole reason anchoring exists. A file's header lands on a block
    // boundary, so four bytes that look like one in the middle of somebody's
    // sample data are not a file, and reporting them would drown the real
    // hits — 4.2 GB of corpus is a lot of chances for four bytes to recur.
    let bytes = image(4, 1030, b"PP20");
    assert!(scan(&bytes, BLOCK).is_empty());
}

#[test]
fn an_unanchored_magic_is_found_anywhere() {
    // A ProTracker module's `M.K.` sits 1,080 bytes in, past a title and 31
    // sample headers — never at a block start.
    let bytes = image(4, 1080, b"M.K.");
    let hits = scan(&bytes, BLOCK);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "ProTracker module (M.K.)");
    assert_eq!(
        hits[0].block, 2,
        "the block it falls in, not the one it starts"
    );
}

#[test]
fn the_more_specific_signature_wins_at_one_offset() {
    // `DMS!!ERR` and `DMS!` match the same bytes. Reporting a disk full of
    // xDMS failure filler as containing DMS archives is a confident wrong
    // answer about what damaged it — which is worse than no answer.
    let bytes = image(4, 512, b"DMS!!ERR");
    let hits = scan(&bytes, BLOCK);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "xDMS unpack failure filler");
    assert_eq!(hits[0].category, Category::Other);
}

#[test]
fn a_pattern_repeating_across_blocks_is_one_run_not_many_files() {
    // `Powerstyx.adf` carries `DMS!1.52` across 88 consecutive blocks. No file
    // header repeats like that; it is filler, and the number that matters is
    // how far the damage runs rather than how many times the bytes occur.
    let mut bytes = vec![0u8; 8 * BLOCK as usize];
    for block in 2..7 {
        let at = block * BLOCK as usize;
        bytes[at..at + 8].copy_from_slice(b"DMS!!ERR");
    }
    let hits = scan(&bytes, BLOCK);
    assert_eq!(hits.len(), 1, "five blocks of filler is one finding");
    assert_eq!(hits[0].block, 2);
    assert_eq!(hits[0].run, 5);
}

#[test]
fn two_separate_headers_are_two_findings() {
    // The other half: a run is only a run when the blocks are consecutive.
    let mut bytes = vec![0u8; 8 * BLOCK as usize];
    for block in [2usize, 5] {
        let at = block * BLOCK as usize;
        bytes[at..at + 4].copy_from_slice(b"PP20");
    }
    let hits = scan(&bytes, BLOCK);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].run, 1);
    assert_eq!(hits[1].run, 1);
}

#[test]
fn hits_come_back_in_offset_order() {
    // A reader works down the disk, and so should the report.
    let mut bytes = vec![0u8; 8 * BLOCK as usize];
    bytes[3072..3076].copy_from_slice(b"PP20");
    bytes[1024..1028].copy_from_slice(&[0x00, 0x00, 0x03, 0xF3]);
    bytes[2048..2052].copy_from_slice(b"FORM");
    let hits = scan(&bytes, BLOCK);
    let offsets: Vec<u64> = hits.iter().map(|h| h.offset).collect();
    assert_eq!(offsets, vec![1024, 2048, 3072]);
}

#[test]
fn an_empty_or_tiny_image_finds_nothing_and_does_not_panic() {
    assert!(scan(&[], BLOCK).is_empty());
    assert!(scan(&[0x00], BLOCK).is_empty());
    assert!(scan(b"PP2", BLOCK).is_empty());
}

#[test]
fn every_signature_in_the_table_can_actually_be_found() {
    // A magic nobody can match is a typo nobody notices. This plants each one
    // where its anchor says it belongs and requires the scanner to find it.
    for signature in ade_object::signature::SIGNATURES {
        let at = match signature.anchor {
            Anchor::BlockStart => BLOCK as usize,
            Anchor::Anywhere => BLOCK as usize + 37,
        };
        let bytes = image(4, at, signature.magic);
        let hits = scan(&bytes, BLOCK);
        assert!(
            hits.iter().any(|h| h.name == signature.name),
            "{} was not found where it should be",
            signature.name
        );
    }
}

#[test]
fn a_caller_can_scan_with_its_own_table() {
    let table = [Signature {
        name: "test",
        category: Category::Other,
        magic: b"ZZZZ",
        anchor: Anchor::BlockStart,
    }];
    let bytes = image(2, 512, b"ZZZZ");
    let hits = scan_with(&bytes, BLOCK, &table);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "test");
}
