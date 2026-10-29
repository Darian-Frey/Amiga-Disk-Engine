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
    container::{RawImage, inflate, sniff},
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

#[test]
fn fuzz_the_inflater() {
    // A decompressor is the AV-005 surface: every length it acts on comes from
    // the input, and a few bytes can ask for gigabytes. The output cap here is
    // deliberately small, so a case that tries to run away is caught as a
    // failed assertion rather than by the OOM killer — which is what happened
    // to this session twice with `unadf`.
    //
    // 64 KiB is far above anything a mutated 900 KB stream legitimately
    // produces, and small enough that a case which *tries* to run away is cut
    // off cheaply. Decompression costs far more per case than parsing a block,
    // so the default count is lower than the other targets' — `ADE_FUZZ_ITERS`
    // raises it for a deep sweep.
    const CAP: usize = 64 * 1024;

    let mut rng = Rng::new(0x5EED_0007);
    // Real gzip streams as bases, so mutation lands on plausible structure
    // rather than being rejected at the magic bytes every time.
    let bases: Vec<Vec<u8>> = vec![
        gzip_of(&Fixture::dd(1).named("Fuzz").build()),
        gzip_of(&[0u8; 4096]),
        gzip_of(b"the quick brown fox jumps over the lazy dog"),
        vec![0x1F, 0x8B, 0x08, 0, 0, 0, 0, 0, 0, 0],
    ];

    for i in 0..iterations(2_000) {
        let seed = u64::from(i);
        let mut buf = bases[rng.below(bases.len())].clone();
        for _ in 0..=rng.below(8) {
            mutate(&mut buf, &mut rng);
        }
        timed(seed, "inflater", || {
            // Must return, never panic, on any input at all.
            if let Ok(out) = inflate::gunzip(&buf, CAP) {
                assert!(out.len() <= CAP, "the cap was exceeded, not enforced");
            }
            let _ = inflate::inflate(&buf, CAP);
            let _ = inflate::crc32(&buf);
        });
    }
}

#[test]
fn fuzz_the_inflater_on_arbitrary_bytes() {
    // The other half: no plausible structure at all. Most cases die at the
    // magic, but the ones that do not reach the block decoder with nonsense
    // code lengths, which is where an unchecked shift or index would live.
    const CAP: usize = 64 * 1024;

    let mut rng = Rng::new(0x5EED_0008);
    for i in 0..iterations(5_000) {
        let len = rng.below(2048);
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            *b = rng.nasty_byte();
        }
        // Half the cases wear a valid gzip header, so the body is reached.
        if len >= 10 && rng.below(2) == 0 {
            let header = [0x1F, 0x8B, 0x08, 0, 0, 0, 0, 0, 0, 0];
            buf.get_mut(..10)
                .unwrap_or(&mut [])
                .copy_from_slice(&header);
        }
        timed(u64::from(i), "inflater arbitrary", || {
            let _ = inflate::gunzip(&buf, CAP);
            let _ = inflate::inflate(&buf, CAP);
        });
    }
}

/// gzip some bytes using the system tool, so the fuzz bases are real streams
/// rather than ones this crate produced.
fn gzip_of(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut child = std::process::Command::new("gzip")
        .arg("-c")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn gzip");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(data)
        .expect("write");
    child.wait_with_output().expect("gzip").stdout
}

#[test]
fn fuzz_the_executable_parser_and_listing() {
    // F-032. An executable's hunk header is a list of counts that size reads:
    // hunk sizes, relocation counts, symbol name lengths. Each must be bounded
    // by what the file holds, and a listing must stay within its line cap
    // whatever the counts claim.
    use ade_core::layers::object::{export, flow, hunk, listing};
    let mut rng = Rng::new(0x5EED_0F32);
    let put = |v: &mut Vec<u8>, x: u32| {
        v.extend_from_slice(&[(x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, x as u8]);
    };
    // A small, valid two-hunk program with relocations and a symbol, as the
    // base worth mutating: mutation then lands on plausible structure.
    let mut base = Vec::new();
    for x in [
        0x3F3,
        0,
        2,
        0,
        1,
        2,
        1,
        0x3E9,
        2,
        0x41F9_0000,
        0x0000_4E75,
        0x3EC,
        1,
        1,
        2,
        0,
        0x3F0,
        1,
        0x6D61_696E,
        0,
        0,
        0x3F2,
        0x3EA,
        1,
        0x1234_5678,
        0x3F2,
    ] {
        put(&mut base, x);
    }
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = base.clone();
        for _ in 0..=rng.below(6) {
            mutate(&mut buf, &mut rng);
        }
        if rng.below(4) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "executable parser", || {
            let _ = hunk::parse(&buf);
            if let Some(l) = listing::executable(&buf) {
                assert!(l.lines.len() <= listing::MAX_LINES + 1);
            }
            let _ = listing::raw(&buf);
            let _ = listing::bootblock(&buf);
            // The walks and the export read the same bytes (D-021).
            if let Some(a) = flow::executable(&buf) {
                for c in &a.library_calls {
                    let _ = c.describe();
                }
            }
            let _ = flow::bootblock(&buf);
            let _ = export::executable(&buf);
        });
    }
}

#[test]
fn fuzz_the_preferences_decoder() {
    // F-033. A fixed 232 bytes, every field read through a bounds check: the
    // decoder must give an answer for any of them, and refuse any other
    // length, with every byte value in every field — the codes it has a name
    // for and the ones it reports as unknown.
    let mut rng = Rng::new(0x5EED_0F33);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = vec![0u8; 232];
        for b in &mut buf {
            *b = rng.nasty_byte();
        }
        for _ in 0..=rng.below(6) {
            mutate(&mut buf, &mut rng);
        }
        if rng.below(8) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "preferences decoder", || {
            let decoded = ade_core::decode::file(b"system-configuration", &buf);
            assert_eq!(decoded.is_ok(), buf.len() == 232, "seed {seed}");
            if let Ok(d) = decoded {
                let _ = d.lines();
            }
        });
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "a template written by hand, not with the helpers that read it (C-001's exception)"
)]
fn fuzz_the_icon_decoder() {
    // F-033. Mutations of a real-shaped icon — header, drawer window, two
    // images, a default tool and tool types — so they reach every part, plus
    // truncation. Every size and count in an icon comes from the file, and
    // none may make the decoder allocate what the file does not pay for: an
    // image of depth 0 stores no planes, which is why sides are bounded.
    let mut template = vec![0u8; 78];
    template[0..2].copy_from_slice(&[0xE3, 0x10]);
    template[3] = 1;
    template[17] = 4 | 2; // an image, and a second one when selected
    template[25] = 1; // GadgetRender
    template[29] = 1; // SelectRender
    template[47] = 1; // revision 1
    template[48] = 2; // a drawer
    template[53] = 1; // DefaultTool
    template[57] = 1; // ToolTypes
    template[69] = 1; // DrawerData
    template.extend(vec![0u8; 56]);
    for _ in 0..2 {
        let mut image = vec![0u8; 20];
        image[5] = 20;
        image[7] = 6;
        image[9] = 2;
        image[14] = 3;
        template.extend(image);
        template.extend((0u8..2 * 6 * 4).map(|i| i.wrapping_mul(37)));
    }
    for s in [&b"SYS:Utilities/More"[..], b"FILETYPE=TEXT", b"DONOTWAIT"] {
        if s == b"FILETYPE=TEXT" {
            template.extend(12u32.to_be_bytes());
        }
        template.extend((s.len() as u32 + 1).to_be_bytes());
        template.extend(s);
        template.push(0);
    }
    template.extend([0, 0, 0, 1, 0, 2]);
    assert!(
        ade_core::decode::file(b"x.info", &template).is_ok(),
        "the template decodes"
    );

    let mut rng = Rng::new(0x5EED_1C0A);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = template.clone();
        for _ in 0..=rng.below(6) {
            mutate(&mut buf, &mut rng);
        }
        // Keep the magic most of the time, or nothing reaches the parser.
        if rng.below(8) != 0 && buf.len() >= 2 {
            buf[0..2].copy_from_slice(&[0xE3, 0x10]);
        }
        if rng.below(6) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "icon decoder", || {
            if let Ok(d) = ade_core::decode::file(b"x.info", &buf) {
                if let Some(p) = &d.picture {
                    assert_eq!(p.argb.len(), p.width * p.height, "seed {seed}");
                }
                let _ = d.lines();
            }
        });
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "a template written by hand, not with the helpers that read it (C-001's exception)"
)]
fn fuzz_the_picture_decoder() {
    // F-033. Mutations of a HAM6 ILBM with a ByteRun1 body, so they reach
    // the chunk walk, the unpacker and hold-and-modify, plus truncation.
    // Sizes come from the header; none may make the decoder allocate what
    // the file does not pay for beyond the bounded picture.
    fn chunk(id: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend((data.len() as u32).to_be_bytes());
        c.extend(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }
    let mut bmhd = vec![0u8; 20];
    bmhd[0..2].copy_from_slice(&32u16.to_be_bytes());
    bmhd[2..4].copy_from_slice(&8u16.to_be_bytes());
    bmhd[8] = 6;
    bmhd[10] = 1;
    bmhd[14] = 10;
    bmhd[15] = 11;
    let cmap: Vec<u8> = (0..48u8).map(|i| i.wrapping_mul(17)).collect();
    // Each row: 6 planes of 4 bytes, as one literal and one run.
    let mut body = Vec::new();
    for y in 0..8u8 {
        body.extend([11, y, 0x55, 0xAA, y, 1, 2, 3, 4, 5, 6, 7, 8]);
        body.extend([0xF5, y.wrapping_mul(29)]);
    }
    let mut inner = b"ILBM".to_vec();
    inner.extend(chunk(*b"BMHD", &bmhd));
    inner.extend(chunk(*b"CMAP", &cmap));
    inner.extend(chunk(*b"CAMG", &0x800u32.to_be_bytes()));
    inner.extend(chunk(*b"BODY", &body));
    let mut template = b"FORM".to_vec();
    template.extend((inner.len() as u32).to_be_bytes());
    template.extend(inner);
    assert!(
        ade_core::decode::file(b"pic", &template)
            .unwrap()
            .picture
            .is_some(),
        "the template decodes"
    );

    let mut rng = Rng::new(0x5EED_1BB3);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = template.clone();
        for _ in 0..=rng.below(6) {
            mutate(&mut buf, &mut rng);
        }
        if rng.below(8) != 0 && buf.len() >= 12 {
            buf[0..4].copy_from_slice(b"FORM");
            buf[8..12].copy_from_slice(b"ILBM");
        }
        if rng.below(6) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "picture decoder", || {
            if let Ok(d) = ade_core::decode::file(b"pic", &buf) {
                if let Some(p) = &d.picture {
                    assert_eq!(p.argb.len(), p.width * p.height, "seed {seed}");
                }
                let _ = d.lines();
            }
        });
    }
}

#[allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "templates written by hand, not with the helpers that read them (C-001's exception)"
)]
/// A proportional, kerned font of 'A'..'D', as a one-hunk executable.
fn font_template() -> Vec<u8> {
    let mut h = vec![0x70, 0xFF, 0x4E, 0x75];
    h.extend([0u8; 14]);
    h.extend(0x0F80u16.to_be_bytes());
    h.extend([0u8; 2 + 4 + 32 + 20]);
    h.extend(3u16.to_be_bytes()); // YSize
    h.extend([0, 0x20]);
    h.extend([0, 4, 0, 2, 0, 1, 0, 0, b'A', b'D']);
    let at = h.len();
    h.extend([0u8; 18]);
    let bitmap = h.len() as u32;
    h.extend([0xA5, 0x5A, 0xFF, 0x00, 0x0F, 0xF0]);
    h[at..at + 4].copy_from_slice(&bitmap.to_be_bytes());
    h[at + 4..at + 6].copy_from_slice(&2u16.to_be_bytes());
    let loc = h.len() as u32;
    for i in 0..5u16 {
        h.extend((i * 3).to_be_bytes());
        h.extend(3u16.to_be_bytes());
    }
    h[at + 6..at + 10].copy_from_slice(&loc.to_be_bytes());
    for off in [10usize, 14] {
        let p = h.len() as u32;
        for v in [4i16, -1, 3, 0, 2] {
            h.extend(v.to_be_bytes());
        }
        h[at + off..at + off + 4].copy_from_slice(&p.to_be_bytes());
    }
    while h.len() % 4 != 0 {
        h.push(0);
    }
    let mut font = Vec::new();
    for v in [
        0x3F3u32,
        0,
        1,
        0,
        0,
        (h.len() / 4) as u32,
        0x3E9,
        (h.len() / 4) as u32,
    ] {
        font.extend(v.to_be_bytes());
    }
    font.extend(&h);
    font.extend(0x3F2u32.to_be_bytes());
    font
}

#[allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "templates written by hand, not with the helpers that read them (C-001's exception)"
)]
/// An 8SVX of three octaves in stereo.
fn sound_template() -> Vec<u8> {
    let mut inner = b"8SVX".to_vec();
    let mut vhdr = vec![0u8; 20];
    vhdr[3] = 8;
    vhdr[7] = 4;
    vhdr[12..14].copy_from_slice(&8000u16.to_be_bytes());
    vhdr[14] = 3;
    vhdr[16..20].copy_from_slice(&0x10000u32.to_be_bytes());
    for (id, data) in [
        (b"VHDR", vhdr),
        (b"NAME", b"tone".to_vec()),
        (b"CHAN", 6u32.to_be_bytes().to_vec()),
        (b"BODY", (0..168u8).map(|i| i.wrapping_mul(37)).collect()),
    ] {
        inner.extend(id);
        inner.extend((data.len() as u32).to_be_bytes());
        inner.extend(data);
    }
    let mut sound = b"FORM".to_vec();
    sound.extend((inner.len() as u32).to_be_bytes());
    sound.extend(inner);
    sound
}

#[test]
#[allow(
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "templates written by hand, not with the helpers that read them (C-001's exception)"
)]
fn fuzz_the_font_and_sound_decoders() {
    // F-033. A proportional, kerned font of 'A'..'D' as a one-hunk
    // executable, and an 8SVX of three octaves in stereo: mutated, so the
    // table pointers, counts and lengths — every one of them from the file —
    // land anywhere, plus truncation. Nothing may allocate what the file
    // does not pay for beyond the bounded specimen.
    let font = font_template();
    assert!(
        ade_core::decode::file(b"8", &font)
            .unwrap()
            .picture
            .is_some(),
        "the font template decodes"
    );

    let sound = sound_template();
    assert!(
        ade_core::decode::file(b"tone", &sound)
            .unwrap()
            .picture
            .is_some(),
        "the sound template decodes"
    );

    let mut rng = Rng::new(0x5EED_F0E7);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let (name, template): (&[u8], &Vec<u8>) = if i % 2 == 0 {
            (b"8", &font)
        } else {
            (b"tone", &sound)
        };
        let mut buf = template.clone();
        for _ in 0..=rng.below(6) {
            mutate(&mut buf, &mut rng);
        }
        if rng.below(6) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "font and sound decoders", || {
            if let Ok(d) = ade_core::decode::file(name, &buf) {
                if let Some(p) = &d.picture {
                    assert_eq!(p.argb.len(), p.width * p.height, "seed {seed}");
                }
                let _ = d.lines();
            }
        });
    }
}

#[test]
fn fuzz_free_blocks_left_by_deleting() {
    // IMP-019. On an empty volume the health report reads every free block
    // and follows what looks like a deleted file's header: its data table and
    // its extension chain, all from blocks nothing vouches for. Headers are
    // planted with valid checksums, so the mutation reaches the tables and
    // the chain — counts past the table, pointers outside the volume, chains
    // that loop. The count can never exceed the volume.
    use ade_core::layers::{block::checksum, endian::put_u32};
    let dostype = Dostype::from_raw(0x444F_5301).unwrap();
    let when = ade_core::layers::filesystem::format::Stamp {
        days: 17_000,
        mins: 1,
        ticks: 0,
    };
    let blank =
        ade_core::layers::filesystem::format::blank(Geometry::DD_FLOPPY, dostype, "Free", when)
            .unwrap();
    let mut rng = Rng::new(0x5EED_F2EE);
    for i in 0..iterations(2_000) {
        let seed = u64::from(i);
        let mut img = blank.clone();
        for _ in 0..=rng.below(4) {
            let b = 882 + rng.below(800);
            let block = &mut img[b * 512..(b + 1) * 512];
            block.fill(0);
            put_u32(block, 0, 2).unwrap();
            put_u32(block, 4, b as u32).unwrap();
            put_u32(block, 8, rng.below(200) as u32).unwrap();
            for slot in 0..72 {
                let at = 512 - 204 - slot * 4;
                put_u32(block, at, (rng.next_u64() % 2000) as u32).unwrap();
            }
            put_u32(block, 512 - 8, (rng.next_u64() % 2000) as u32).unwrap();
            put_u32(block, 512 - 4, (-3i32).cast_unsigned()).unwrap();
            for _ in 0..rng.below(3) {
                mutate(block, &mut rng);
            }
            let sum = checksum::normal_at(block, 20).unwrap();
            put_u32(block, 20, sum).unwrap();
        }
        timed(seed, "free blocks left by deleting", || {
            let report = ade_core::health::examine(img);
            let held = report.bitmap.map_or(0, |b| b.free_holding_data);
            assert!(held <= 1760, "seed {seed}: {held}");
        });
    }
}

/// A keymap with a plain key, a string key, a dead key and a key it changes,
/// as a one-hunk executable, with where its hunk's bytes start and how many
/// there are.
#[allow(
    clippy::disallowed_methods,
    reason = "a template written by hand, not with the helpers that read it (C-001's exception)"
)]
fn keymap_template() -> (Vec<u8>, usize, usize) {
    let mut h = vec![0u8; 46];
    let set = |h: &mut Vec<u8>, at: usize, v: u32| {
        h[at..at + 4].copy_from_slice(&v.to_be_bytes());
    };
    let mut relocs: Vec<u32> = (0..9)
        .map(|i| if i == 8 { 10 } else { 14 + i * 4 })
        .collect();
    let mut at = [0usize; 8];
    for (field, (len, fill)) in [
        (64, 0x80),
        (256, 0),
        (8, 0xFF),
        (8, 0xFF),
        (56, 0x80),
        (224, 0),
        (7, 0),
        (7, 0),
    ]
    .into_iter()
    .enumerate()
    {
        at[field] = h.len();
        h.resize(h.len() + len, fill);
        set(&mut h, 14 + field * 4, at[field] as u32);
    }
    let name = h.len();
    h.extend(b"fuzz\0");
    set(&mut h, 10, name as u32);
    h[at[0] + 0x10] = 7;
    set(&mut h, at[1] + 0x40, 0x7151_E5C5);
    let up = h.len();
    h.extend([2, 4, 2, 6, 0x9B, b'A', 0x9B, b'T']);
    h[at[4] + 0x0C] = 0x41;
    set(&mut h, at[5] + 0x30, up as u32);
    relocs.push((at[5] + 0x30) as u32);
    let dead = h.len();
    h.extend([0, b'f', 0, b'F', 8, 1, 8, 1, 0, 6, 0, 6, 0, 0, 0, 0]);
    h[at[0] + 0x23] = 0x27;
    set(&mut h, at[1] + 0x23 * 4, dead as u32);
    relocs.push((at[1] + 0x23 * 4) as u32);
    let accent = h.len();
    h.extend([1, 4, 0, b'E', b'e', 0xE8]);
    h[at[0] + 0x12] = 0x21;
    set(&mut h, at[1] + 0x12 * 4, accent as u32);
    relocs.push((at[1] + 0x12 * 4) as u32);
    while h.len() % 4 != 0 {
        h.push(0);
    }
    let longs = (h.len() / 4) as u32;
    let mut exe = Vec::new();
    for v in [0x3F3u32, 0, 1, 0, 0, longs, 0x3E9, longs] {
        exe.extend(v.to_be_bytes());
    }
    let start = exe.len();
    let len = h.len();
    exe.extend(h);
    exe.extend(0x3ECu32.to_be_bytes());
    exe.extend((relocs.len() as u32).to_be_bytes());
    exe.extend(0u32.to_be_bytes());
    for r in relocs {
        exe.extend(r.to_be_bytes());
    }
    exe.extend(0u32.to_be_bytes());
    exe.extend(0x3F2u32.to_be_bytes());
    (exe, start, len)
}

#[test]
fn fuzz_the_keymap_decoder() {
    // F-033. The relocations are left alone so the file stays a keymap and
    // the mutation reaches what is inside: table pointers, key types, and
    // the descriptors a string or dead key points at, with their lengths and
    // offsets — all of them the file's word, all of them bounded by the hunk.
    let (template, start, hunk_len) = keymap_template();
    assert!(
        ade_core::decode::file(b"fuzz", &template).is_ok(),
        "the keymap template decodes"
    );
    let mut rng = Rng::new(0x5EED_6E7A);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = template.clone();
        for _ in 0..=rng.below(8) {
            let body = &mut buf[start..start + hunk_len];
            mutate(body, &mut rng);
        }
        if rng.below(8) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "keymap decoder", || {
            if let Ok(d) = ade_core::decode::file(b"fuzz", &buf) {
                assert!(d.fields.len() < 512, "seed {seed}");
                let _ = d.lines();
            }
        });
    }
}

/// A four-channel module with two patterns of speed, break, jump, loop and
/// delay commands, and one looping sample.
#[allow(
    clippy::disallowed_methods,
    reason = "a template written by hand, not with the helpers that read it (C-001's exception)"
)]
fn module_template() -> Vec<u8> {
    let mut m = vec![0u8; 1084];
    m[..4].copy_from_slice(b"fuzz");
    m[20..24].copy_from_slice(b"beep");
    m[42..44].copy_from_slice(&64u16.to_be_bytes());
    m[45] = 64;
    m[46..48].copy_from_slice(&16u16.to_be_bytes());
    m[48..50].copy_from_slice(&32u16.to_be_bytes());
    m[950] = 3;
    m[952..955].copy_from_slice(&[0, 1, 0]);
    m[1080..1084].copy_from_slice(b"M.K.");
    let mut patterns = vec![0u8; 2 * 1024];
    for (p, row, ch, effect, param) in [
        (0usize, 0usize, 0usize, 0xFu8, 3u8),
        (0, 2, 1, 0xE, 0x60),
        (0, 5, 1, 0xE, 0x62),
        (0, 9, 2, 0xE, 0xE2),
        (0, 20, 3, 0xD, 0x32),
        (1, 40, 0, 0xF, 0x90),
        (1, 63, 0, 0xB, 0x02),
    ] {
        let at = p * 1024 + row * 16 + ch * 4;
        patterns[at + 2] = effect;
        patterns[at + 3] = param;
    }
    m.extend(patterns);
    m.extend((0..128u8).map(|i| i.wrapping_mul(29)));
    m
}

#[test]
fn fuzz_the_module_decoder() {
    // F-033. Mutated, so the song length, order list, sample lengths and
    // loops, and every effect the walk obeys — jumps that go nowhere,
    // breaks past the pattern, loops that never end, speeds of zero —
    // come from the file. The walk must end, and every length it reports
    // must be one the file could hold.
    let template = module_template();
    assert!(
        ade_core::decode::file(b"fuzz", &template)
            .unwrap()
            .picture
            .is_some(),
        "the module template decodes"
    );
    let mut rng = Rng::new(0x5EED_70D5);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let mut buf = template.clone();
        for _ in 0..=rng.below(8) {
            mutate(&mut buf, &mut rng);
        }
        // The tag kept, most of the time, so the mutation reaches past it.
        if rng.below(4) != 0 {
            buf[1080..1084].copy_from_slice(b"M.K.");
        }
        if rng.below(8) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "module decoder", || {
            if let Ok(m) = ade_core::layers::object::tracker::parse(&buf) {
                assert!(m.seconds.is_finite() && m.seconds >= 0.0, "seed {seed}");
                assert!(
                    m.samples.iter().all(|s| s.data.len() <= 4096),
                    "seed {seed}"
                );
            }
            if let Ok(d) = ade_core::decode::file(b"fuzz", &buf) {
                if let Some(p) = &d.picture {
                    assert_eq!(p.argb.len(), p.width * p.height, "seed {seed}");
                }
                let _ = d.lines();
            }
        });
    }
}

/// An IFF `FORM` of `kind` holding `chunks`, each padded to even.
#[allow(
    clippy::disallowed_methods,
    clippy::trivially_copy_pass_by_ref,
    reason = "a template written by hand, not with the helpers that read it (C-001's exception)"
)]
fn iff(kind: &[u8; 4], chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut inner = kind.to_vec();
    for (id, data) in chunks {
        inner.extend(*id);
        inner.extend((data.len() as u32).to_be_bytes());
        inner.extend(data);
        if data.len() % 2 == 1 {
            inner.push(0);
        }
    }
    let mut out = b"FORM".to_vec();
    out.extend((inner.len() as u32).to_be_bytes());
    out.extend(inner);
    out
}

/// One small file of each format added 2026-10-29: an ACBM, preferences, a
/// printer driver, a catalog, an ordinary animation and an Anim Brush, a
/// score, formatted text and a datatype descriptor.
#[allow(
    clippy::disallowed_methods,
    clippy::too_many_lines,
    reason = "templates written by hand, not with the helpers that read them (C-001's \
              exception); one of each format, each in a few lines, read best together"
)]
fn more_templates() -> Vec<(&'static [u8], Vec<u8>)> {
    let bmhd = |w: u16, h: u16, d: u8| {
        let mut b = vec![0u8; 20];
        b[0..2].copy_from_slice(&w.to_be_bytes());
        b[2..4].copy_from_slice(&h.to_be_bytes());
        b[8] = d;
        b
    };
    let grey = vec![
        0, 0, 0, 0x55, 0x55, 0x55, 0xAA, 0xAA, 0xAA, 0xFF, 0xFF, 0xFF,
    ];
    let acbm = iff(
        b"ACBM",
        &[
            (b"BMHD", bmhd(16, 2, 2)),
            (b"CMAP", grey.clone()),
            (b"ABIT", vec![0x80, 0, 0x20, 0, 0x40, 0, 0x10, 0]),
        ],
    );

    let mut country = vec![0u8; 504];
    country[16..18].copy_from_slice(b"GB");
    country[20..24].copy_from_slice(&44u32.to_be_bytes());
    country[25..36].copy_from_slice(b"%A %e %B %Y");
    let mut locale = vec![0u8; 860];
    locale[16..23].copy_from_slice(b"danmark");
    locale[356..].copy_from_slice(&country);
    let mut screen = vec![0u8; 28];
    screen[16..20].copy_from_slice(&0x0002_9004u32.to_be_bytes());
    let prefs = iff(
        b"PREF",
        &[
            (b"PRHD", vec![0; 6]),
            (b"CTRY", country),
            (b"LCLE", locale),
            (b"SCRM", screen),
            (b"SERL", vec![0; 34]),
        ],
    );

    // A printer driver: MOVEQ/RTS, version, the extended data, a name.
    let mut h = vec![0x70, 0, 0x4E, 0x75, 0, 35, 0, 40];
    h.resize(8 + 62, 0);
    h[8 + 20] = 1;
    h[8 + 34..8 + 36].copy_from_slice(&120u16.to_be_bytes());
    let name_at = h.len() as u32;
    h.extend(b"Fuzz\r\nnote\0\0");
    h[8..12].copy_from_slice(&name_at.to_be_bytes());
    while h.len() % 4 != 0 {
        h.push(0);
    }
    let longs = (h.len() / 4) as u32;
    let mut printer = Vec::new();
    for v in [0x3F3u32, 0, 1, 0, 0, longs, 0x3E9, longs] {
        printer.extend(v.to_be_bytes());
    }
    printer.extend(h);
    for v in [0x3ECu32, 5, 0, 8, 12, 16, 20, 24, 0, 0x3F2] {
        printer.extend(v.to_be_bytes());
    }

    let mut strs = Vec::new();
    for (id, s) in [(0u32, &b"eins"[..]), (1, b"zwei!"), (7, b"\xFCber\n")] {
        strs.extend(id.to_be_bytes());
        strs.extend((s.len() as u32).to_be_bytes());
        strs.extend(s);
        while strs.len() % 4 != 0 {
            strs.push(0);
        }
    }
    let catalog = iff(
        b"CTLG",
        &[
            (b"FVER", b"$VER: f.catalog 1.0\0".to_vec()),
            (b"LANG", b"deutsch\0".to_vec()),
            (b"CSET", vec![0; 32]),
            (b"STRS", strs),
        ],
    );

    let first = iff(
        b"ILBM",
        &[
            (b"BMHD", bmhd(16, 4, 1)),
            (b"CMAP", vec![0, 0, 0, 255, 255, 255]),
            (b"BODY", vec![0x80, 0, 0, 0, 0x40, 0, 0, 0]),
        ],
    );
    let frame = |interleave: u8, ops: &[u8]| {
        let mut anhd = vec![0u8; 40];
        anhd[0] = 5;
        anhd[17] = 1;
        anhd[18] = interleave;
        let mut dlta = vec![0u8; 64];
        dlta[0..4].copy_from_slice(&64u32.to_be_bytes());
        dlta.extend(ops);
        dlta.push(0);
        iff(b"ILBM", &[(b"ANHD", anhd), (b"DLTA", dlta)])
    };
    let ordinary = [
        first.clone(),
        frame(0, &[2, 1, 0x81, 0x80]),
        frame(0, &[1, 0, 3, 0xC0]),
    ]
    .concat();
    let brush = [
        first,
        frame(1, &[1, 0x82, 0x80, 0x40]),
        frame(1, &[1, 0x82, 0x80, 0x40]),
    ]
    .concat();
    let wrap = |frames: Vec<u8>| {
        let mut out = b"FORM".to_vec();
        out.extend(((frames.len() + 4) as u32).to_be_bytes());
        out.extend(b"ANIM");
        out.extend(frames);
        out
    };

    let score = iff(
        b"SMUS",
        &[
            (b"SHDR", vec![0x3C, 0, 100, 2]),
            (b"NAME", b"Tune".to_vec()),
            (b"INS1", b"\x01\x01\x02\x05Piano".to_vec()),
            (
                b"TRAK",
                vec![
                    130, 0x1A, 131, 2, 60, 0x82, 64, 0x82, 67, 2, 128, 0x0A, 129, 1,
                ],
            ),
            (b"TRAK", vec![62, 1 | 0x08, 62, 0x13, 62, 0x23]),
        ],
    );
    let text = iff(
        b"FTXT",
        &[
            (b"FONS", b"\x01\0\0\0topaz.font\0".to_vec()),
            (b"CHRS", b"Hello \x9b1mbold\x9b0m\n\x1b(Bplain".to_vec()),
        ],
    );

    let mut dthd = Vec::new();
    for v in [0x32u32, 0x3D, 0x48, 0x20] {
        dthd.extend(v.to_be_bytes());
    }
    dthd.extend(b"docuamig");
    dthd.extend([0, 9, 0, 0, 0, 1, 0, 0]);
    for c in b"@database" {
        dthd.extend([0, *c]);
    }
    dthd.extend(b"AmigaGuide\0amigaguide\0#?\0");
    let descriptor = iff(
        b"DTYP",
        &[(b"NAME", b"AmigaGuide".to_vec()), (b"DTHD", dthd)],
    );

    vec![
        (&b"pic"[..], acbm),
        (b"prefs", prefs),
        (b"Fuzz", printer),
        (b"f.catalog", catalog),
        (b"a.anim", wrap(ordinary)),
        (b"b.anim", wrap(brush)),
        (b"tune", score),
        (b"text", text),
        (b"AmigaGuide", descriptor),
    ]
}

#[test]
fn fuzz_the_decoders_added_for_systems_files_and_media() {
    // F-033, 2026-10-29. Every template decodes, and then each is mutated
    // and cut short: chunk lengths, offsets into a chunk, counts of planes,
    // strings, frames, events and mask words all come from the file, and
    // none may read outside it, run without end, or allocate past what it
    // holds.
    let templates = more_templates();
    for (name, bytes) in &templates {
        assert!(
            ade_core::decode::file(name, bytes).is_ok(),
            "the {} template decodes",
            String::from_utf8_lossy(name)
        );
    }
    let mut rng = Rng::new(0x5EED_0A11);
    for i in 0..iterations(20_000) {
        let seed = u64::from(i);
        let (name, template) = &templates[i as usize % templates.len()];
        let mut buf = template.clone();
        for _ in 0..=rng.below(8) {
            mutate(&mut buf, &mut rng);
        }
        if rng.below(6) == 0 {
            buf.truncate(rng.below(buf.len() + 1));
        }
        timed(seed, "systems files and media", || {
            if let Ok(d) = ade_core::decode::file(name, &buf) {
                if let Some(p) = &d.picture {
                    assert_eq!(p.argb.len(), p.width * p.height, "seed {seed}");
                }
                let _ = d.lines();
            }
        });
    }
}
