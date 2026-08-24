//! Fuzzing the parse paths (F-001).
//!
//! F-001's bar: "No input — however malformed, truncated, or hostile — causes a
//! crash, hang, or unbounded memory growth; failures return typed errors." This
//! is where that is enforced.
//!
//! # Why this is hand-rolled rather than `cargo-fuzz`
//!
//! `cargo-fuzz` gives coverage-guided exploration, which is strictly better at
//! *finding* new faults — but it needs nightly, and CI runs stable. A harness CI
//! cannot run protects nothing against regressions, which is most of what a
//! fuzzer is for once the first round of bugs is fixed. So this runs on every
//! push, dependency-free, with a deterministic PRNG so any failure reproduces
//! from its seed. Coverage-guided fuzzing is a worthwhile addition later, not a
//! substitute for this.
//!
//! # Block level, not image level
//!
//! A rootblock parser reads 512 bytes. Seeding it with 880 KB images would spend
//! almost the whole budget on bytes no parser looks at, so the block parsers are
//! fuzzed with block-sized inputs and only the container and volume layers see
//! whole images.
//!
//! # What "no crash" is checked against
//!
//! A panic fails the test by itself. Hangs and unbounded growth need explicit
//! assertions, and the workspace forbids `unsafe`, so an allocator hook is not
//! available. Instead each operation is held to **structural bounds** that
//! unbounded growth would necessarily violate — a file cannot exceed the volume,
//! a listing cannot exceed the block count — plus a wall-clock budget per case.
//! That catches the runaway without needing to measure the heap: ADFlib's 29 GB
//! blow-up on a real disk would have tripped every one of these.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding over data it constructs"
)]

use std::time::{Duration, Instant};

use ade_core::layers::{
    block::{self, Geometry},
    container::{RawImage, sniff},
    filesystem::{
        bootblock::Bootblock, dircache, dostype::Dostype, entry::Entry, rootblock::Rootblock,
        volume::Volume,
    },
};
use ade_fixtures::{Volume as Fixture, corrupt};

/// Iterations per target. Raise for a deep local run:
/// `ADE_FUZZ_ITERS=200000 cargo test -p ade-core --test fuzz`.
fn iterations(default: u32) -> u32 {
    std::env::var("ADE_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// No single case may take longer than this. A hang is a failure, and CI timing
/// out tells you nothing about which input caused it.
const CASE_BUDGET: Duration = Duration::from_secs(2);

/// xorshift64*. Deterministic, so a failing seed reproduces exactly, and small
/// enough not to be a dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 24) as u8
    }
    /// Bytes biased towards the values that break parsers: 0, 0xFF, and the
    /// high bits of length and pointer fields.
    fn nasty_byte(&mut self) -> u8 {
        match self.below(8) {
            0 | 1 => 0x00,
            2 | 3 => 0xFF,
            4 => 0x80,
            5 => 0x7F,
            _ => self.byte(),
        }
    }
}

/// Corrupt a buffer in place, using strategies that reach deeper than random
/// bytes do: a parser rejects noise at the first field, so most of the budget
/// must go on inputs that stay *plausible* until something subtle is wrong.
fn mutate(buf: &mut [u8], rng: &mut Rng) {
    if buf.is_empty() {
        return;
    }
    match rng.below(6) {
        // Flip a single bit. Finds off-by-one and boundary handling.
        0 => {
            let i = rng.below(buf.len());
            buf[i] ^= 1 << rng.below(8);
        }
        // Overwrite one byte with a nasty value.
        1 => {
            let i = rng.below(buf.len());
            buf[i] = rng.nasty_byte();
        }
        // Smash a whole 32-bit field — the shape of every pointer, length and
        // type in the format.
        2 => {
            let i = rng.below(buf.len().saturating_sub(4).max(1));
            for k in 0..4.min(buf.len() - i) {
                buf[i + k] = rng.nasty_byte();
            }
        }
        // Set a field to a plausible-but-wrong block number. Written through
        // ade-endian rather than `to_be_bytes`, because C-001 admits no
        // exemptions and the lint caught this exact line.
        3 => {
            let i = rng.below(buf.len().saturating_sub(4).max(1));
            let v = (rng.next_u64() % 4000) as u32;
            let _ = ade_core::layers::endian::put_u32(buf, i, v);
        }
        // Zero a run.
        4 => {
            let i = rng.below(buf.len());
            let n = rng.below(64).min(buf.len() - i);
            buf[i..i + n].fill(0);
        }
        // Splice a run of random bytes.
        _ => {
            let i = rng.below(buf.len());
            let n = rng.below(64).min(buf.len() - i);
            for b in &mut buf[i..i + n] {
                *b = rng.byte();
            }
        }
    }
}

/// Run one case under the time budget, reporting the seed if it overruns.
fn timed(seed: u64, label: &str, f: impl FnOnce()) {
    let start = Instant::now();
    f();
    let took = start.elapsed();
    assert!(
        took < CASE_BUDGET,
        "{label} took {took:?} on seed {seed} — a hang is a failure (F-001)"
    );
}

// --- block-level parsers ----------------------------------------------------

#[test]
fn fuzz_block_parsers() {
    let mut rng = Rng::new(0x5EED_0001);
    // A valid rootblock and a valid entry, as bases worth mutating.
    let fixture = {
        let mut v = Fixture::dd(0).named("Fuzz");
        v.add_file("victim", b"payload");
        v.add_dir("dir");
        v.build()
    };
    let root = 880usize * 512;
    let bases: Vec<Vec<u8>> = vec![
        fixture[root..root + 512].to_vec(),
        fixture[512 * 2..512 * 3].to_vec(),
        vec![0u8; 512],
        vec![0xFFu8; 512],
    ];

    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = bases[rng.below(bases.len())].clone();
        for _ in 0..=rng.below(4) {
            mutate(&mut buf, &mut rng);
        }
        timed(seed, "block parsers", || {
            // Every one of these must return, never panic, on any input.
            let _ = Rootblock::parse(&buf);
            let _ = Entry::parse(&buf, rng_block(&buf));
            let _ = Dostype::parse(&buf, 0);
            let _ = dircache::parse(&buf, rng_block(&buf));
            let _ = block::checksum::normal(&buf);
            let _ = block::checksum::normal_valid(&buf);
        });
    }
}

/// A block number drawn from the buffer itself, so the parser sometimes gets a
/// self-referential value.
fn rng_block(buf: &[u8]) -> u32 {
    ade_core::layers::endian::u32_at(buf, 0).unwrap_or(0)
}

#[test]
fn fuzz_parsers_on_arbitrary_lengths() {
    // Truncation is its own failure class: a parser addressing fields from the
    // end of a block must refuse a buffer that is not a block.
    let mut rng = Rng::new(0x5EED_0002);
    for i in 0..iterations(20_000) {
        let len = rng.below(1200);
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            *b = rng.nasty_byte();
        }
        timed(u64::from(i), "arbitrary lengths", || {
            let _ = Rootblock::parse(&buf);
            let _ = Entry::parse(&buf, 0);
            let _ = Dostype::parse(&buf, rng_offset(&buf));
            let _ = Bootblock::parse(&buf);
            let _ = dircache::parse(&buf, 0);
            let _ = block::checksum::normal(&buf);
            let _ = block::checksum::boot(&buf);
            let _ = sniff(&buf, buf.len() as u64);
        });
    }
}

fn rng_offset(buf: &[u8]) -> usize {
    buf.first().map_or(0, |&b| usize::from(b))
}

#[test]
fn fuzz_the_sniffer_against_declared_sizes() {
    // `sniff` takes a size the caller reports separately from the bytes. A size
    // that disagrees with reality must not mislead it into an out-of-range read.
    let mut rng = Rng::new(0x5EED_0003);
    for i in 0..iterations(20_000) {
        let len = rng.below(2048);
        let mut head = vec![0u8; len];
        for b in &mut head {
            *b = rng.nasty_byte();
        }
        let claimed = match rng.below(4) {
            0 => 0,
            1 => u64::MAX,
            2 => 901_120,
            _ => rng.next_u64(),
        };
        timed(u64::from(i), "sniffer", || {
            let d = sniff(&head, claimed);
            // Whatever it concludes, it must justify itself.
            assert!(!d.evidence.is_empty(), "a verdict with no evidence (C-008)");
        });
    }
}

// --- whole-image paths ------------------------------------------------------

#[test]
fn fuzz_mount_and_traverse() {
    let mut rng = Rng::new(0x5EED_0004);
    let base = {
        let mut v = Fixture::dd(1).named("Fuzz");
        for n in 0..12 {
            v.add_file(&format!("file{n:02}"), &vec![0xAB; 700 * (n + 1)]);
        }
        let d = v.add_dir("sub");
        let _ = d;
        v.build()
    };
    let geometry = Geometry::DD_FLOPPY;
    let total_bytes = geometry.total_bytes();
    let total_blocks = geometry.total_blocks();

    for i in 0..iterations(3_000) {
        let seed = u64::from(i);
        let mut img = base.clone();
        for _ in 0..=rng.below(24) {
            mutate(&mut img, &mut rng);
        }
        timed(seed, "mount and traverse", || {
            let Ok(image) = RawImage::new(img.clone(), geometry) else {
                return;
            };
            let Ok(volume) = Volume::mount(&image) else {
                return;
            };
            let Ok(listing) = volume.list(volume.root()) else {
                return;
            };

            // Unbounded growth would have to break one of these.
            assert!(
                listing.entries.len() as u64 <= total_blocks,
                "seed {seed}: {} entries in a {total_blocks}-block volume",
                listing.entries.len()
            );

            for entry in listing.entries.iter().take(32) {
                if entry.kind.is_file()
                    && let Ok(contents) = volume.read_file(entry)
                {
                    assert!(
                        contents.bytes.len() as u64 <= total_bytes,
                        "seed {seed}: file of {} bytes in a {total_bytes}-byte volume",
                        contents.bytes.len()
                    );
                }
            }

            let walked = volume.walk(volume.root()).unwrap_or_default();
            assert!(
                walked.entries.len() as u64 <= total_blocks,
                "seed {seed}: walk produced {} entries, exceeding the block count",
                walked.entries.len()
            );
            // The cap is a backstop. On a *mutated but bounded* image the
            // visited set should still be what stops the walk — if the cap
            // fires, cycle detection failed (IMP-003, AV-001).
            assert!(
                !walked.hit_limit,
                "seed {seed}: the structural cap fired — a cycle escaped the visited set"
            );
        });
    }
}

#[test]
fn fuzz_deliberately_hostile_structures() {
    // The corruptions AV-001 and AV-004 describe, layered on each other in
    // combinations no single hand-written fixture covers.
    let mut rng = Rng::new(0x5EED_0005);
    for i in 0..iterations(2_000) {
        let seed = u64::from(i);
        let mut v = Fixture::dd((rng.below(8)) as u8);
        let a = v.add_file("alpha", b"aaaa");
        let b = v.add_file("beta", &vec![0xCD; 3000]);
        let dir = v.add_dir("sub");
        let root = v.root();
        let mut img = v.build();

        for _ in 0..=rng.below(5) {
            match rng.below(8) {
                0 => corrupt::hash_chain_loop(&mut img, a),
                1 => corrupt::hash_chain_cycle(&mut img, a, b),
                2 => corrupt::directory_cycle(&mut img, dir, root),
                3 => corrupt::first_data_out_of_range(&mut img, b),
                4 => corrupt::hash_slot_out_of_range(&mut img, root, rng.below(72)),
                5 => corrupt::bitmap_flag_invalid(&mut img, root),
                6 => corrupt::block_checksum(&mut img, rng.below(1760) as u32),
                _ => corrupt::rootblock_wrong_type(&mut img, root),
            }
        }
        timed(seed, "hostile structures", || {
            let Ok(image) = RawImage::new(img.clone(), Geometry::DD_FLOPPY) else {
                return;
            };
            let Ok(volume) = Volume::mount(&image) else {
                return;
            };
            let walked = volume.walk(volume.root()).unwrap_or_default();
            assert!(
                walked.entries.len() <= 1760,
                "seed {seed}: walk exceeded the block count — a cycle escaped (AV-001)"
            );
            assert!(
                !walked.hit_limit,
                "seed {seed}: the structural cap fired on a deliberately hostile \
                 image — the visited set should have stopped it first (IMP-003)"
            );
            for (_, entry) in walked.entries.iter().take(16) {
                if entry.kind.is_file() {
                    let _ = volume.read_file(entry);
                }
            }
        });
    }
}

#[test]
fn fuzz_truncated_and_extended_images() {
    // Truncation is the commonest real-world damage, and one survey image is
    // exactly one byte over canonical.
    let mut rng = Rng::new(0x5EED_0006);
    let base = {
        let mut v = Fixture::dd(0).named("Trunc");
        v.add_file("f", &vec![7u8; 5000]);
        v.build()
    };
    for i in 0..iterations(3_000) {
        let seed = u64::from(i);
        let mut img = base.clone();
        match rng.below(3) {
            0 => img.truncate(rng.below(base.len())),
            1 => img.extend(std::iter::repeat_n(rng.byte(), rng.below(4096))),
            _ => {}
        }
        for _ in 0..=rng.below(8) {
            mutate(&mut img, &mut rng);
        }
        timed(seed, "truncated images", || {
            let size = img.len() as u64;
            let head = &img[..img.len().min(8192)];
            let _ = sniff(head, size);
            let _ = ade_core::inspect_bytes(img.clone());
        });
    }
}
