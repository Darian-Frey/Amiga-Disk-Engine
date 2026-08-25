//! Extended ADF (`UAE-1ADF`) — Phase 4's container.
//!
//! A plain ADF holds decoded sectors and so cannot represent a disk that is not
//! made of ordinary sectors. Copy protection is exactly that, and extended ADF
//! exists to carry those tracks as raw MFM.
//!
//! The layout has no published specification: it was derived from the eleven
//! extended ADFs in the corpus. These tests pin the two readings a plausible
//! implementation gets wrong — `space` versus `length`, and empty tracks — and
//! check the whole thing against the real images where they are available.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::inspect_bytes;
use ade_core::layers::container::extended::{
    ExtendedAdf, ExtendedError, STANDARD_TRACK_BYTES, TrackKind,
};
use ade_core::layers::endian::{put_u16, put_u32};

/// Build an extended ADF from `(type, space, length_bits, fill)` tracks.
///
/// Written through the C-001 seam like everything else that touches
/// big-endian data.
fn build(tracks: &[(u16, u32, u32, u8)]) -> Vec<u8> {
    let mut out = vec![0u8; 12];
    out[..8].copy_from_slice(b"UAE-1ADF");
    put_u16(&mut out, 8, 0).unwrap();
    put_u16(&mut out, 10, tracks.len() as u16).unwrap();
    for (kind, space, length, _) in tracks {
        let at = out.len();
        out.extend_from_slice(&[0u8; 12]);
        put_u16(&mut out, at, 0).unwrap();
        put_u16(&mut out, at + 2, *kind).unwrap();
        put_u32(&mut out, at + 4, *space).unwrap();
        put_u32(&mut out, at + 8, *length).unwrap();
    }
    for (_, space, _, fill) in tracks {
        out.extend(std::iter::repeat_n(*fill, *space as usize));
    }
    out
}

#[test]
fn a_track_table_parses() {
    let image = build(&[(0, 5632, 45056, 0xAA), (1, 8000, 63000, 0xBB)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();

    assert!(parsed.faults.is_empty(), "{:?}", parsed.faults);
    assert_eq!(parsed.tracks.len(), 2);
    assert_eq!(parsed.tracks[0].kind, TrackKind::Sectors);
    assert_eq!(parsed.tracks[1].kind, TrackKind::RawMfm);
    assert_eq!(parsed.counts(), (1, 1, 0));
}

#[test]
fn data_is_bounded_by_length_not_by_space() {
    // The reading that matters. In the corpus a type-0 track has length 45056
    // bits — 5632 bytes, one standard DD track — in all 428 observed, while
    // `space` is 5632, 12650 or 12668 depending on the writer. Taking `space`
    // as the data size reads seven kilobytes of padding as sectors.
    let image = build(&[(0, 12668, 45056, 0xAA)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();
    let data = parsed.track_data(&image, 0).expect("track present");

    assert_eq!(
        data.len(),
        STANDARD_TRACK_BYTES,
        "a standard track is 11 sectors of 512, whatever the allocation says"
    );
}

#[test]
fn a_raw_track_uses_its_own_length() {
    // Raw MFM tracks vary in length legitimately — that is what makes them
    // worth capturing — so here the length field is the authority.
    //
    // Note the corpus's usual pair, space 12768 with length 102138 bits, is
    // *not* a useful case: 102138 bits rounds up to exactly 12768 bytes, so
    // the two readings agree and the test would pass either way. The gap has
    // to be a whole-byte one to prove anything.
    let image = build(&[(1, 12768, 63_000, 0xBB)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();
    let data = parsed.track_data(&image, 0).expect("track present");

    assert_eq!(data.len(), 63_000_usize.div_ceil(8));
    assert!(data.len() < 12768, "shorter than its allocation");
}

#[test]
fn an_empty_track_holds_nothing_and_is_not_a_fault() {
    // 154 observed tracks have space and length both zero: unformatted, or
    // never captured. That is a fact about the disk, not damage to the file.
    let image = build(&[(1, 0, 0, 0), (0, 5632, 45056, 0xAA)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();

    assert!(parsed.faults.is_empty(), "{:?}", parsed.faults);
    assert!(parsed.tracks[0].is_empty());
    assert!(parsed.track_data(&image, 0).is_none());
    assert_eq!(
        parsed.counts(),
        (1, 0, 1),
        "the empty one counts separately"
    );
}

#[test]
fn a_truncated_file_keeps_what_it_has() {
    // `Demolition.adf` in the corpus is genuinely short: it declares 166
    // tracks and the file ends inside track 163. Failing the whole read would
    // throw away 163 good tracks.
    let mut image = build(&[(0, 5632, 45056, 0xAA), (0, 5632, 45056, 0xBB)]);
    image.truncate(image.len() - 3000);

    let parsed = ExtendedAdf::parse(&image).unwrap();

    assert_eq!(parsed.tracks.len(), 2, "the table still parses");
    assert!(parsed.tracks[0].present);
    assert!(!parsed.tracks[1].present);
    assert!(parsed.track_data(&image, 0).is_some());
    assert!(parsed.track_data(&image, 1).is_none());
    assert!(
        parsed.faults.iter().any(|f| f.contains("ends before")),
        "{:?}",
        parsed.faults
    );
}

#[test]
fn a_length_exceeding_its_allocation_is_reported() {
    // A writer claiming more data than it stored. The read is still bounded by
    // what is there; the discrepancy is a fault rather than a crash.
    let image = build(&[(1, 100, 8000, 0xCC)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();

    assert!(
        parsed.faults.iter().any(|f| f.contains("only 100 bytes")),
        "{:?}",
        parsed.faults
    );
    assert_eq!(parsed.track_data(&image, 0).map(<[u8]>::len), Some(100));
}

#[test]
fn a_wild_track_count_is_refused_before_allocating() {
    // AV-005: the count is two bytes off the front of the file and sizes a
    // table. 65535 tracks is not a disk.
    let mut image = vec![0u8; 12];
    image[..8].copy_from_slice(b"UAE-1ADF");
    put_u16(&mut image, 10, 0xFFFF).unwrap();

    assert_eq!(
        ExtendedAdf::parse(&image).unwrap_err(),
        ExtendedError::TooManyTracks { declared: 65535 }
    );
}

#[test]
fn a_file_ending_inside_its_table_is_an_error_not_a_fault() {
    // Without a table there is nothing to report about, so this is the one
    // case that fails outright rather than degrading.
    let mut image = build(&[(0, 5632, 45056, 0xAA), (0, 5632, 45056, 0xBB)]);
    image.truncate(20);

    assert_eq!(
        ExtendedAdf::parse(&image).unwrap_err(),
        ExtendedError::Truncated
    );
}

#[test]
fn a_plain_adf_is_not_an_extended_one() {
    let plain = ade_fixtures::Volume::dd(1).named("Plain").build();
    assert_eq!(
        ExtendedAdf::parse(&plain).unwrap_err(),
        ExtendedError::NotExtendedAdf
    );
}

#[test]
fn an_unknown_track_type_is_reported_rather_than_guessed() {
    let image = build(&[(7, 1000, 8000, 0xDD)]);
    let parsed = ExtendedAdf::parse(&image).unwrap();

    assert_eq!(parsed.tracks[0].kind, TrackKind::Unknown(7));
    assert!(
        parsed
            .faults
            .iter()
            .any(|f| f.contains("unknown track type 7"))
    );
}

#[test]
fn the_inspection_reports_the_table() {
    let image = build(&[(0, 5632, 45056, 0xAA), (1, 8000, 63000, 0xBB), (1, 0, 0, 0)]);
    let inspection = inspect_bytes(image);
    let table = inspection.tracks.expect("track table reported");

    assert_eq!(table.declared, 3);
    assert_eq!(table.sectors, 1);
    assert_eq!(table.raw_mfm, 1);
    assert_eq!(table.empty, 1);
    // A raw-track container has no volume, and the reason must say so rather
    // than claim the format is unimplemented.
    assert!(inspection.volume.is_none());
    let why = inspection.volume_absent.expect("a reason");
    assert!(why.contains("holds tracks, not a volume"), "{why}");
}

#[test]
fn a_plain_adf_reports_no_track_table() {
    let plain = ade_fixtures::Volume::dd(1).named("Plain").build();
    assert!(inspect_bytes(plain).tracks.is_none());
}

#[test]
fn the_corpus_extended_adfs_all_parse() {
    // Eleven real images, which is the entire published basis for this layout.
    // Skips cleanly when `disks/` is absent.
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    let mut checked = 0usize;
    let mut truncated = 0usize;
    for entry in std::fs::read_dir(&corpus).expect("read corpus").flatten() {
        let path = entry.path();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if bytes.get(..8) != Some(b"UAE-1ADF") {
            continue;
        }
        let parsed =
            ExtendedAdf::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        // The table must describe the file: every present track's data must be
        // reachable, and the arithmetic must not run past the end.
        for track in &parsed.tracks {
            if track.present && !track.is_empty() {
                assert!(
                    parsed.track_data(&bytes, track.index).is_some(),
                    "{}: track {} claims to be present but cannot be read",
                    path.display(),
                    track.index
                );
            }
        }
        if parsed.tracks.iter().any(|t| !t.present) {
            truncated += 1;
        }
        checked += 1;
    }

    eprintln!("extended ADFs: {checked} parsed, {truncated} truncated");
    assert!(
        checked >= 10,
        "expected the corpus's extended ADFs, got {checked}"
    );
    assert_eq!(
        truncated, 1,
        "exactly one corpus image is short: Demolition"
    );
}
