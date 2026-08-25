//! ADZ and HDZ — gzip-wrapped images (Phase 3, F-003).
//!
//! An ADZ is a gzip-wrapped ADF and nothing more, so the interesting questions
//! are not about Amiga structures at all. They are: does the inflater agree
//! with a real one, and does it stay bounded when the input asks it not to.
//!
//! The oracle here is the system `gzip`, which is a stronger position than the
//! rest of the project enjoys: it compresses, ADE decompresses, and the result
//! must be byte-identical to what went in. Any corpus image is a test case.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "tests over data they construct"
)]

use std::{io::Write as _, process::Command, process::Stdio};

use ade_core::layers::container::inflate::{self, InflateError};
use ade_core::{Image, MAX_DECOMPRESSED, examine, inspect_bytes};
use ade_fixtures::Volume as Fixture;

/// Compress with the system gzip, so the oracle is not our own code.
///
/// The write happens on its own thread. Feeding a large image down stdin while
/// nothing drains stdout deadlocks once gzip's output exceeds the pipe buffer:
/// gzip blocks writing, this blocks writing, and neither moves. It only shows
/// up on inputs that actually compress to more than about 64 KB — the small
/// fixtures pass happily, and the first real disk hangs.
fn gzip(data: &[u8], level: &str) -> Vec<u8> {
    let mut child = Command::new("gzip")
        .arg(level)
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn gzip");
    let mut stdin = child.stdin.take().expect("stdin");
    let payload = data.to_vec();
    let writer = std::thread::spawn(move || {
        stdin.write_all(&payload).expect("write to gzip");
        // Dropping stdin closes the pipe, which is what makes gzip finish.
    });
    let out = child.wait_with_output().expect("gzip").stdout;
    writer.join().expect("writer thread");
    out
}

#[test]
fn round_trips_every_compression_level() {
    // The levels differ in which block types and match strategies they emit,
    // so this covers fixed and dynamic Huffman blocks without having to
    // hand-build either. Stored blocks — the third type, with no Huffman
    // coding and its own length/complement check — come from
    // `round_trips_incompressible_data` instead, since no gzip level emits
    // them for compressible input.
    let image = Fixture::dd(1).named("Compressed").build();

    for level in ["-1", "-6", "-9"] {
        let compressed = gzip(&image, level);
        let out = inflate::gunzip(&compressed, MAX_DECOMPRESSED)
            .unwrap_or_else(|e| panic!("gzip {level}: {e}"));

        assert_eq!(out, image, "gzip {level} did not round-trip");
    }
}

#[test]
fn round_trips_data_that_exercises_back_references() {
    // A long run is encoded as a match whose source overlaps its own output —
    // distance 1, length 200. Copying that in bulk instead of byte at a time
    // gives the wrong answer, and gives it silently.
    let mut data = vec![0xAAu8; 200];
    data.extend_from_slice(b"then something else entirely, to break the run");
    data.extend(std::iter::repeat_n(0x55u8, 5000));
    data.extend_from_slice(b"then something else entirely, to break the run");

    let out = inflate::gunzip(&gzip(&data, "-9"), MAX_DECOMPRESSED).unwrap();

    assert_eq!(out, data);
}

#[test]
fn round_trips_incompressible_data() {
    // Data that does not compress makes gzip emit stored blocks, and makes the
    // output larger than the input — the case where a naive cap check against
    // the *compressed* size would wrongly refuse.
    // SplitMix64, whose output gzip cannot model. A weaker mixer looked
    // random and compressed 22:1, which would have left stored blocks
    // untested while the test appeared to cover them.
    let mut state = 0x1234_5678u64;
    let data: Vec<u8> = (0..70_000)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 56) as u8
        })
        .collect();

    let compressed = gzip(&data, "-9");
    // Incompressible input makes deflate fall back to stored blocks, which is
    // why this case covers that block type. If it ever compresses, the
    // coverage silently disappears — so assert it does not.
    assert!(
        compressed.len() > data.len(),
        "the data should be incompressible, so stored blocks are exercised"
    );

    let out = inflate::gunzip(&compressed, MAX_DECOMPRESSED).unwrap();

    assert_eq!(out, data);
}

#[test]
fn an_empty_stream_round_trips() {
    let out = inflate::gunzip(&gzip(&[], "-9"), MAX_DECOMPRESSED).unwrap();
    assert!(out.is_empty());
}

#[test]
fn the_output_cap_is_enforced_before_allocating() {
    // AV-005. A megabyte of zeros compresses to a few hundred bytes; asked to
    // expand it under a small cap, the inflater must refuse rather than
    // allocate and then complain.
    let bomb = gzip(&vec![0u8; 1024 * 1024], "-9");
    assert!(
        bomb.len() < 8192,
        "the bomb should be small: {}",
        bomb.len()
    );

    let err = inflate::gunzip(&bomb, 4096).unwrap_err();

    assert_eq!(err, InflateError::OutputTooLarge { limit: 4096 });
}

#[test]
fn a_corrupt_stream_fails_rather_than_returning_garbage() {
    // The failure that matters is not "it errored" but "it did not quietly
    // hand back plausible bytes". Every mutation either errors or reproduces
    // the original exactly — never something in between.
    let image = Fixture::dd(1).named("Corruptible").build();
    let compressed = gzip(&image, "-6");

    let mut errored = 0;
    let mut identical = 0;
    for offset in (32..compressed.len().min(4000)).step_by(37) {
        let mut damaged = compressed.clone();
        damaged[offset] ^= 0xFF;
        match inflate::gunzip(&damaged, MAX_DECOMPRESSED) {
            Err(_) => errored += 1,
            Ok(out) => {
                assert_eq!(
                    out, image,
                    "byte {offset}: returned different bytes without an error"
                );
                identical += 1;
            }
        }
    }

    assert!(errored > 0, "damage should be detected");
    eprintln!("corruption: {errored} rejected, {identical} harmless (header padding)");
}

#[test]
fn the_checksum_is_verified_not_merely_carried() {
    // gzip's CRC32 is the only thing standing between a subtly corrupt stream
    // and a silently wrong disk image. Break the trailer and nothing else.
    let compressed = gzip(&Fixture::dd(1).named("Summed").build(), "-6");
    let mut damaged = compressed.clone();
    let crc_at = damaged.len() - 8;
    damaged[crc_at] ^= 0x01;

    let err = inflate::gunzip(&damaged, MAX_DECOMPRESSED).unwrap_err();

    assert!(
        matches!(err, InflateError::ChecksumMismatch { .. }),
        "expected a checksum mismatch, got {err}"
    );
}

#[test]
fn the_declared_length_is_verified_too() {
    let compressed = gzip(&Fixture::dd(1).named("Sized").build(), "-6");
    let mut damaged = compressed.clone();
    let len = damaged.len();
    damaged[len - 4] ^= 0x01;

    let err = inflate::gunzip(&damaged, MAX_DECOMPRESSED).unwrap_err();

    assert!(
        matches!(err, InflateError::LengthMismatch { .. }),
        "expected a length mismatch, got {err}"
    );
}

#[test]
fn a_truncated_stream_is_reported_as_truncated() {
    let compressed = gzip(&Fixture::dd(1).named("Cut").build(), "-6");
    let cut = &compressed[..compressed.len() / 2];

    let err = inflate::gunzip(cut, MAX_DECOMPRESSED).unwrap_err();

    // Either the trailer is missing or the stream ends mid-block; both are
    // truncation and both must be named as such rather than guessed past.
    assert!(
        matches!(err, InflateError::Truncated { .. }),
        "expected truncation, got {err}"
    );
}

#[test]
fn a_non_gzip_file_is_refused_by_the_gzip_reader() {
    let err = inflate::gunzip(b"not gzip at all, really", MAX_DECOMPRESSED).unwrap_err();
    assert_eq!(err, InflateError::NotGzip);
}

#[test]
fn a_gzip_header_with_a_name_and_comment_is_skipped() {
    // gzip -N stores the original filename; the optional fields sit between
    // the header and the data, and mis-skipping them starts the inflater a few
    // bytes into a Huffman block, which fails in a confusing way.
    let data = b"payload that is long enough to compress into something".repeat(4);
    let path = std::env::temp_dir().join(format!("ade-named-{}.bin", std::process::id()));
    std::fs::write(&path, &data).unwrap();
    let out = Command::new("gzip")
        .args(["-N", "-9", "-c"])
        .arg(&path)
        .output()
        .expect("gzip");
    let _ = std::fs::remove_file(&path);

    // FNAME must actually be set, or this tests nothing.
    assert_eq!(out.stdout[3] & 0b0000_1000, 0b0000_1000, "FNAME not set");

    assert_eq!(
        inflate::gunzip(&out.stdout, MAX_DECOMPRESSED).unwrap(),
        data
    );
}

#[test]
fn crc32_matches_the_known_vector() {
    // The standard check value for "123456789" under the reflected 0xEDB88320
    // polynomial. If this is wrong, every gzip stream is rejected.
    assert_eq!(inflate::crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(inflate::crc32(b""), 0);
}

#[test]
fn an_adz_inspects_as_the_adf_inside_it() {
    let image = Fixture::dd(1).named("Wrapped").build();
    let compressed = gzip(&image, "-9");

    let plain = inspect_bytes(image.clone());
    let wrapped = inspect_bytes(compressed.clone());

    // The container reported is the *inner* one: what the user has is an Amiga
    // floppy, and gzip is how it was stored.
    assert_eq!(
        wrapped.detection.kind.to_string(),
        plain.detection.kind.to_string()
    );
    assert_eq!(
        wrapped.size, plain.size,
        "the size is the image's, not the file's"
    );
    assert_eq!(
        wrapped.volume.map(|v| v.rootblock.name_lossy()),
        Some("Wrapped".to_owned())
    );

    // But the wrapper is reported, not hidden.
    let compression = wrapped.compression.expect("compression reported");
    assert_eq!(compression.compressed_size, compressed.len() as u64);
    assert_eq!(compression.decompressed_size, Some(image.len() as u64));
    assert!(compression.error.is_none());
    assert!(plain.compression.is_none(), "a plain ADF is not compressed");
}

#[test]
fn an_adz_mounts_and_lists() {
    let mut fixture = Fixture::dd(1).named("Mountable");
    fixture.add_file("startup", b"hello from inside a gzip");
    fixture.add_dir("Tools");
    let compressed = gzip(&fixture.build(), "-9");

    let path = std::env::temp_dir().join(format!("ade-adz-{}.adz", std::process::id()));
    std::fs::write(&path, &compressed).unwrap();
    let image = Image::open(&path).unwrap();
    let volume = image.volume().unwrap();
    let listing = volume.list(volume.root()).unwrap();
    let _ = std::fs::remove_file(&path);

    assert_eq!(volume.rootblock().name_lossy(), "Mountable");
    assert_eq!(listing.entries.len(), 2);
}

#[test]
fn health_reads_through_the_wrapper() {
    // `examine` reads blocks directly rather than through the inspection, so
    // it needs unwrapping of its own. Passing it the compressed bytes reported
    // a truncated image on a sound disk.
    let mut fixture = Fixture::dd(1).named("Healthy");
    fixture.add_file("startup", b"hello");
    let image = fixture.build();

    let plain = examine(image.clone());
    let wrapped = examine(gzip(&image, "-9"));

    assert_eq!(wrapped.files, plain.files);
    assert_eq!(wrapped.directories, plain.directories);
    assert_eq!(wrapped.bytes_recovered, plain.bytes_recovered);
    assert_eq!(
        wrapped.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
        plain.findings.iter().map(|f| f.code).collect::<Vec<_>>()
    );
}

#[test]
fn an_undecompressable_wrapper_is_reported_not_swallowed() {
    // The file really is a gzip; it just cannot be read. Reporting "unknown
    // container" would be a worse answer than saying why.
    let mut damaged = gzip(&Fixture::dd(1).named("Broken").build(), "-6");
    let len = damaged.len();
    damaged[len / 2] ^= 0xFF;
    damaged[len / 2 + 1] ^= 0xFF;

    let inspection = inspect_bytes(damaged);
    let compression = inspection.compression.expect("still identified as gzip");

    assert!(compression.decompressed_size.is_none());
    assert!(compression.error.is_some(), "the reason should be recorded");
    assert!(inspection.volume.is_none());
}

#[test]
fn the_corpus_round_trips_through_gzip() {
    // The strongest check available: real images, compressed by a real gzip,
    // decompressed by ours, compared byte for byte. Unlike the D-002 oracle
    // this has no interpretive gap at all — the answer is either identical or
    // wrong.
    //
    // Skips cleanly when `disks/` is absent, so a fresh clone passes offline.
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&corpus)
        .expect("read corpus")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("adf"))
        .collect();
    paths.sort();

    // A deterministic spread rather than the first N, so a failure is
    // reproducible and the sample is not all one publisher.
    let step = (paths.len() / 20).max(1);
    let sample: Vec<_> = paths.iter().step_by(step).take(20).collect();

    let mut checked = 0usize;
    for path in sample {
        let original = std::fs::read(path).expect("read image");
        let compressed = gzip(&original, "-6");
        let out = inflate::gunzip(&compressed, MAX_DECOMPRESSED)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        assert_eq!(
            out.len(),
            original.len(),
            "{}: length differs",
            path.display()
        );
        assert!(out == original, "{}: contents differ", path.display());
        checked += 1;
    }

    eprintln!("gzip round-trip: {checked} real images, byte-identical");
    assert!(checked >= 10, "expected a real sample, got {checked}");
}

#[test]
fn converting_an_adz_reproduces_the_adf_exactly() {
    // The end-to-end claim `ade convert` makes: what comes out is what went
    // in. Checked here at the library level; `cli/tests/convert.rs` checks the
    // command, including the refusals.
    let image = Fixture::dd(1).named("Converted").build();
    let compressed = gzip(&image, "-9");

    let out = inflate::gunzip(&compressed, MAX_DECOMPRESSED).unwrap();

    assert_eq!(out, image);
}
