//! MFM track decoding (Phase 4).
//!
//! # The decode is self-evidencing
//!
//! Every Amiga sector carries two checksums of its own — one over the header,
//! one over the data — so a correct decode produces sectors whose own
//! arithmetic agrees, and an incorrect one does not. That is a stronger
//! position than anything else in this project: no oracle, no corpus
//! comparison, no adjudication.
//!
//! It also settled a question the sources disagree on. Descriptions of the
//! odd/even split differ over which half comes first; both orders were tried
//! against a real track and only one produced matching checksums.
//!
//! # What the tests build
//!
//! The helper here encodes sectors **without clock bits**. That is legitimate
//! for testing the decoder, which masks clock bits off rather than reading
//! them, but it means these tests say nothing about MFM legality — see SPEC
//! §What this decoder does not check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::layers::container::extended::{ExtendedAdf, TrackKind};
use ade_core::layers::endian::{put_u32, u32_at};
use ade_core::layers::track::{FORMAT_AMIGADOS, SECTOR_BYTES, decode_track};

/// Split a field into its odd and even MFM halves, clock bits left clear.
fn encode_halves(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = data.iter().map(|b| (b >> 1) & 0x55).collect();
    out.extend(data.iter().map(|b| b & 0x55));
    out
}

/// A `u32` as big-endian bytes, through the C-001 seam.
fn be_bytes(value: u32) -> [u8; 4] {
    let mut out = [0u8; 4];
    put_u32(&mut out, 0, value).expect("four bytes");
    out
}

/// XOR of the big-endian longs, keeping only data bits — the Amiga checksum.
fn checksum(mfm: &[u8]) -> u32 {
    let mut sum = 0u32;
    for at in (0..mfm.len()).step_by(4) {
        let Ok(word) = u32_at(mfm, at) else { break };
        sum ^= word;
    }
    sum & 0x5555_5555
}

/// Build one MFM sector: two sync words then the 1080-byte body.
fn sector(track: u8, number: u8, data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len(), SECTOR_BYTES);
    let mut body = Vec::new();
    body.extend(encode_halves(&[
        FORMAT_AMIGADOS,
        track,
        number,
        11 - number,
    ]));
    body.extend(encode_halves(&[0u8; 16]));

    let header_checksum = checksum(&body);
    body.extend(encode_halves(&be_bytes(header_checksum)));

    let encoded_data = encode_halves(data);
    let data_checksum = checksum(&encoded_data);
    body.extend(encode_halves(&be_bytes(data_checksum)));
    body.extend(encoded_data);

    let mut out = vec![0x44, 0x89, 0x44, 0x89];
    out.extend(body);
    out
}

/// A full standard track: eleven sectors, with a gap in front.
fn standard_track() -> Vec<u8> {
    let mut out = vec![0xAA; 200];
    for number in 0..11u8 {
        out.extend(sector(7, number, &[number.wrapping_mul(17); SECTOR_BYTES]));
    }
    out.extend(vec![0xAA; 200]);
    out
}

/// Shift a byte stream right by `bits`, as a real track's framing does.
fn shift(data: &[u8], bits: usize) -> Vec<u8> {
    assert!(bits < 8);
    if bits == 0 {
        return data.to_vec();
    }
    let mut out = vec![0u8; data.len() + 1];
    for (i, &byte) in data.iter().enumerate() {
        out[i] |= byte >> bits;
        out[i + 1] |= byte << (8 - bits);
    }
    out
}

#[test]
fn a_standard_track_decodes_completely() {
    let decoded = decode_track(&standard_track());

    assert_eq!(decoded.sectors.len(), 11);
    assert_eq!(
        decoded.sound(),
        11,
        "every sector's own checksums must agree"
    );
    assert_eq!(decoded.stray_syncs, 0);
    assert!(decoded.is_standard());

    for (number, sector) in decoded.sectors.iter().enumerate() {
        assert_eq!(sector.format, FORMAT_AMIGADOS);
        assert_eq!(sector.track, 7);
        assert_eq!(sector.sector, number as u8);
        assert_eq!(sector.data.len(), SECTOR_BYTES);
        assert!(
            sector
                .data
                .iter()
                .all(|&b| b == (number as u8).wrapping_mul(17))
        );
    }
}

#[test]
fn decoding_is_independent_of_bit_alignment() {
    // The finding that mattered. A raw track is a bit stream, and nothing makes
    // a sector start on a byte boundary of the file holding it — in one
    // `Realm of the Trolls` track every sync sits at bit offset 7 (mod 8). A
    // byte-aligned scan decoded 8% of the corpus's sectors and looked merely
    // disappointing rather than wrong.
    let track = standard_track();
    let baseline = decode_track(&track);

    for bits in 1..8 {
        let shifted = shift(&track, bits);
        let decoded = decode_track(&shifted);

        assert_eq!(
            decoded.sound(),
            baseline.sound(),
            "shifting by {bits} bits changed the result"
        );
        assert!(decoded.is_standard(), "shifted by {bits} bits");
        for (a, b) in decoded.sectors.iter().zip(baseline.sectors.iter()) {
            assert_eq!(a.data, b.data, "shifted by {bits} bits");
            assert_eq!(a.sector, b.sector);
        }
    }
}

#[test]
fn a_sync_mark_with_nothing_behind_it_is_not_a_sector() {
    // How a custom loader marks its own data: the hardware can sync to it and
    // a standard reader finds no sector. Every raw track in `Wings of Death`
    // is like this — three sync words followed by gap.
    let mut track = vec![0xAA; 300];
    track.extend_from_slice(&[0x44, 0x89, 0x44, 0x89, 0x44, 0x89]);
    track.extend(vec![0xAA; 2000]);

    let decoded = decode_track(&track);

    assert!(decoded.sectors.is_empty(), "{:?}", decoded.sectors);
    assert!(decoded.stray_syncs > 0, "the sync mark should be counted");
}

#[test]
fn extra_sync_words_are_skipped() {
    // Two is the norm; three occurs throughout the corpus. The body begins
    // after the last of them, and mis-counting lands in the gap — which
    // decodes to a header claiming track 168 and format 0xAA.
    let mut track = vec![0xAA; 100];
    track.extend_from_slice(&[0x44, 0x89]);
    track.extend(sector(3, 0, &[0x5A; SECTOR_BYTES]));

    let decoded = decode_track(&track);

    assert_eq!(decoded.sound(), 1, "{:?}", decoded.sectors);
    assert_eq!(decoded.sectors[0].track, 3);
    assert_eq!(decoded.sectors[0].format, FORMAT_AMIGADOS);
}

#[test]
fn a_corrupt_header_fails_its_checksum_but_still_decodes() {
    // The point of reporting checksums rather than discarding: a damaged
    // sector is still evidence, and which checksum failed says where.
    let mut track = standard_track();
    let sync = track
        .windows(4)
        .position(|w| w == [0x44, 0x89, 0x44, 0x89])
        .unwrap();
    track[sync + 6] ^= 0x10;

    let decoded = decode_track(&track);
    let first = &decoded.sectors[0];

    assert!(!first.header_checksum_valid, "the damage should show");
    assert!(first.data_checksum_valid, "the data is untouched");
    assert!(!first.is_sound());
}

#[test]
fn a_corrupt_data_area_fails_only_the_data_checksum() {
    let mut track = standard_track();
    let sync = track
        .windows(4)
        .position(|w| w == [0x44, 0x89, 0x44, 0x89])
        .unwrap();
    // Well inside the data half.
    track[sync + 4 + 56 + 100] ^= 0x04;

    let decoded = decode_track(&track);
    let first = &decoded.sectors[0];

    assert!(first.header_checksum_valid);
    assert!(!first.data_checksum_valid);
}

#[test]
fn a_track_missing_a_sector_is_not_standard() {
    let mut out = vec![0xAA; 200];
    for number in 0..10u8 {
        out.extend(sector(7, number, &[0; SECTOR_BYTES]));
    }
    let decoded = decode_track(&out);

    assert_eq!(decoded.sound(), 10);
    assert!(
        !decoded.is_standard(),
        "ten sectors is not a standard track"
    );
}

#[test]
fn a_track_with_a_duplicated_sector_is_not_standard() {
    // Eleven sound sectors is not enough; they must be eleven *different* ones.
    let mut out = vec![0xAA; 200];
    for number in 0..11u8 {
        out.extend(sector(7, number.min(9), &[0; SECTOR_BYTES]));
    }
    let decoded = decode_track(&out);

    assert_eq!(decoded.sound(), 11);
    assert!(!decoded.is_standard());
}

#[test]
fn a_truncated_sector_is_not_reported_as_one() {
    let mut track = vec![0xAA; 100];
    let full = sector(1, 0, &[0xFF; SECTOR_BYTES]);
    track.extend_from_slice(&full[..500]);

    let decoded = decode_track(&track);

    assert!(decoded.sectors.is_empty());
    assert_eq!(decoded.stray_syncs, 1);
}

#[test]
fn an_empty_track_yields_nothing() {
    assert_eq!(decode_track(&[]).sectors.len(), 0);
    assert_eq!(decode_track(&[0xAA; 12668]).sectors.len(), 0);
}

#[test]
fn the_corpus_raw_tracks_decode_as_measured() {
    // The real verification. 2095 sectors across the corpus decode with both
    // of their own checksums agreeing — an outcome a wrong decoder does not
    // reach by accident.
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    let mut raw = 0usize;
    let mut standard = 0usize;
    let mut sound = 0usize;
    for entry in std::fs::read_dir(&corpus).expect("read corpus").flatten() {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        if bytes.get(..8) != Some(b"UAE-1ADF") {
            continue;
        }
        let parsed = ExtendedAdf::parse(&bytes).expect("parse");
        for track in &parsed.tracks {
            if track.kind != TrackKind::RawMfm {
                continue;
            }
            let Some(data) = parsed.track_data(&bytes, track.index) else {
                continue;
            };
            let decoded = ade_core::layers::track::decode_track(data);
            raw += 1;
            sound += decoded.sound();
            if decoded.is_standard() {
                standard += 1;
            }
            // Every sound sector must be a full one; a short read that still
            // checksummed would mean the field offsets are wrong.
            for sector in decoded.sectors.iter().filter(|s| s.is_sound()) {
                assert_eq!(sector.data.len(), SECTOR_BYTES);
                assert!(
                    sector.sector < 22,
                    "sector number {} is nonsense",
                    sector.sector
                );
            }
        }
    }

    eprintln!("raw tracks {raw}, standard {standard}, sound sectors {sound}");
    assert!(raw > 1000, "expected the corpus's raw tracks, got {raw}");
    assert!(
        sound > 2000,
        "expected ~2095 sectors to decode soundly, got {sound}"
    );
    assert!(
        standard >= 95,
        "expected ~95 fully standard tracks, got {standard}"
    );
}
