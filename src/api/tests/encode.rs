//! MFM encoding and extended-ADF writing (Phase 4).
//!
//! The inverse of everything in `mfm.rs` and `extended.rs`, and verified
//! against them: what this writes, those read back byte for byte.
//!
//! # Why a round-trip is the right test here
//!
//! D-004 wants a read path proven before its write path ships, and the read
//! path is proven — 2095 corpus sectors decoding with both of their own
//! checksums agreeing. That makes the decoder a trustworthy judge of the
//! encoder, and the two together a closed loop: a real disk encoded to raw MFM
//! and read back must be the same disk, or one of them is wrong.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::layers::container::extended::{
    ExtendedAdf, STANDARD_TRACK_BYTES, TrackKind, TrackSource, write,
};
use ade_core::layers::track::{
    SECTOR_BYTES, SECTOR_MFM_BYTES, clock_violations, decode_track, encode_sector, encode_track,
};
use ade_core::{assemble, inspect_bytes};

/// Eleven sectors of distinguishable data.
fn sectors() -> Vec<Vec<u8>> {
    (0..11u8)
        .map(|i| {
            (0..SECTOR_BYTES)
                .map(|j| (usize::from(i).wrapping_mul(31).wrapping_add(j)) as u8)
                .collect()
        })
        .collect()
}

#[test]
fn an_encoded_sector_is_the_documented_size() {
    let data = vec![0x5Au8; SECTOR_BYTES];
    let encoded = encode_sector(3, 0, 11, &data).expect("encodes");

    assert_eq!(encoded.len(), SECTOR_MFM_BYTES, "1088 bytes per SPEC §MFM");
}

#[test]
fn a_sector_that_is_not_512_bytes_is_refused() {
    assert!(encode_sector(0, 0, 11, &[0u8; 100]).is_none());
    assert!(encode_sector(0, 0, 11, &[]).is_none());
}

#[test]
fn an_encoded_track_round_trips_through_the_decoder() {
    let data = sectors();
    let refs: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
    let track = encode_track(7, &refs).expect("encodes");

    let decoded = decode_track(&track);

    assert_eq!(decoded.sectors.len(), 11);
    assert_eq!(decoded.sound(), 11, "every checksum must agree");
    assert!(decoded.is_standard());
    assert_eq!(decoded.stray_syncs, 0);
    for (index, sector) in decoded.sectors.iter().enumerate() {
        assert_eq!(sector.track, 7);
        assert_eq!(sector.sector, index as u8);
        assert_eq!(sector.data, data[index], "sector {index} differs");
    }
}

#[test]
fn encoded_data_is_legal_mfm_and_the_sync_words_are_not() {
    // The encoder computes clock bits rather than leaving them clear, which a
    // test helper can skip and a real encoder cannot. The sync words are
    // excluded deliberately: their illegality is what makes them findable.
    let data = vec![0xC3u8; SECTOR_BYTES];
    let encoded = encode_sector(1, 0, 11, &data).expect("encodes");

    // Lead-in (4) plus two sync words (4) then the body.
    let body = &encoded[8..];
    assert_eq!(clock_violations(body), 0, "the body must be legal MFM");

    let decoded = decode_track(&encoded);
    assert_eq!(decoded.sound(), 1);
    assert_eq!(decoded.sectors[0].clock_violations, 0);
}

#[test]
fn the_gap_between_sectors_is_legal_mfm() {
    let data = sectors();
    let refs: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
    let track = encode_track(0, &refs).expect("encodes");

    // The leading gap, before any sync.
    assert_eq!(clock_violations(&track[..32]), 0);
}

#[test]
fn a_real_disk_survives_a_full_raw_mfm_round_trip() {
    // The strongest test available: a whole disk encoded track by track into
    // raw MFM, written as an extended ADF, read back, decoded and reassembled.
    // Byte-identical or one of the two is wrong.
    let mut fixture = ade_fixtures::Volume::dd(1).named("RoundTrip");
    fixture.add_file("startup", b"through MFM and back again");
    fixture.add_dir("Tools");
    fixture.add_file("data.bin", &vec![0xA5u8; 20_000]);
    let plain = fixture.build();

    let mut encoded: Vec<Vec<u8>> = Vec::new();
    for track in 0..160usize {
        let base = track * STANDARD_TRACK_BYTES;
        let slices: Vec<&[u8]> = (0..11)
            .map(|s| &plain[base + s * SECTOR_BYTES..base + (s + 1) * SECTOR_BYTES])
            .collect();
        encoded.push(encode_track(track as u8, &slices).expect("encodes"));
    }
    let sources: Vec<TrackSource<'_>> = encoded
        .iter()
        .map(|data| TrackSource::RawMfm {
            data,
            length_bits: (data.len() * 8) as u32,
        })
        .collect();
    let extended = write(&sources).expect("writes");

    let parsed = ExtendedAdf::parse(&extended).expect("parses");
    assert!(parsed.faults.is_empty(), "{:?}", parsed.faults);
    assert_eq!(parsed.counts(), (0, 160, 0), "all raw, none empty");

    let assembly = assemble(&parsed, &extended);
    assert_eq!(assembly.sectors_placed, 1760);
    assert_eq!(assembly.from_raw_tracks, 160);
    assert_eq!(
        assembly.bytes, plain,
        "a disk through MFM and back must be the same disk"
    );

    // And it mounts under its own name, which the byte comparison already
    // implies but is the thing a user would actually notice.
    let inspection = inspect_bytes(extended);
    assert_eq!(
        inspection.volume.map(|v| v.rootblock.name_lossy()),
        Some("RoundTrip".to_owned())
    );
}

#[test]
fn a_written_container_declares_what_it_holds() {
    let sector_track = vec![0x11u8; STANDARD_TRACK_BYTES];
    let empty = [0u8; SECTOR_BYTES];
    let refs = [&empty[..]; 11];
    let raw = encode_track(0, &refs).expect("encodes");
    let written = write(&[
        TrackSource::Sectors(&sector_track),
        TrackSource::RawMfm {
            data: &raw,
            length_bits: (raw.len() * 8) as u32,
        },
        TrackSource::Empty,
    ])
    .expect("writes");

    let parsed = ExtendedAdf::parse(&written).expect("parses");

    assert!(parsed.faults.is_empty(), "{:?}", parsed.faults);
    assert_eq!(parsed.tracks[0].kind, TrackKind::Sectors);
    assert_eq!(parsed.tracks[1].kind, TrackKind::RawMfm);
    assert!(parsed.tracks[2].is_empty());
    assert_eq!(parsed.counts(), (1, 1, 1));

    // The length field of a sector track is 45056 bits whatever its
    // allocation — the corpus is unanimous on that across 428 tracks.
    assert_eq!(parsed.tracks[0].length_bits, 45056);
}

#[test]
fn a_written_container_reads_back_its_own_data() {
    let first = vec![0xABu8; STANDARD_TRACK_BYTES];
    let written = write(&[TrackSource::Sectors(&first)]).expect("writes");
    let parsed = ExtendedAdf::parse(&written).expect("parses");

    assert_eq!(parsed.track_data(&written, 0), Some(&first[..]));
}

#[test]
fn writing_more_tracks_than_a_disk_has_is_refused() {
    let sources = vec![TrackSource::Empty; 5000];
    assert!(write(&sources).is_err());
}

#[test]
fn an_empty_container_is_still_valid() {
    let written = write(&[]).expect("writes");
    let parsed = ExtendedAdf::parse(&written).expect("parses");

    assert!(parsed.tracks.is_empty());
    assert!(parsed.faults.is_empty());
}
