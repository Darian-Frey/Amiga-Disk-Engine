//! SCP container parsing, against bytes built by hand.
//!
//! Every image here is assembled field by field, so a test that fails names
//! the field that broke rather than "the file did not parse". The values come
//! from [SCP] and from a real Greaseweazle-generated capture, which is where
//! the byte-order case below comes from: `0x009e` is 158 read big-endian and
//! 40448 read little-endian, and only 158 × 25 ns is a plausible MFM interval.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding: a failure to set up is a test failure"
)]
// C-001's tripwire firing here is the tripwire working. This file *writes* an
// SCP, so it needs both byte orders in the raw, and `ade-endian` offers only
// big-endian writers — it exists to read Amiga data, not to author foreign
// containers. Building the fixture by hand is also the point: an SCP assembled
// with the same helpers that parse it could agree with the parser about a
// field both had wrong.
#![allow(
    clippy::disallowed_methods,
    reason = "C-001: this file authors a container in its own byte orders"
)]

use ade_flux::scp::{MAX_REVOLUTIONS, OVERFLOW_TICKS, Scp, ScpError, TRACK_SLOTS};

/// Build an SCP holding one track of one revolution, with the flux values
/// given as raw 16-bit big-endian entries.
fn one_track(flux: &[u16], revolutions: u8) -> Vec<u8> {
    let table_bytes: usize = TRACK_SLOTS * 4;
    let tdh_at = 0x10 + table_bytes;

    let mut out = vec![0u8; tdh_at];
    out[0..3].copy_from_slice(b"SCP");
    out[0x03] = 0x25; // version 2.5
    out[0x04] = 0x04; // Commodore Amiga
    out[0x05] = revolutions;
    out[0x06] = 0;
    out[0x07] = 0;
    out[0x08] = 0b0000_0001; // index-aligned
    out[0x0B] = 0; // 25 ns

    // Track 0's offset, little-endian.
    let entry = 0x10;
    out[entry..entry + 4].copy_from_slice(&(tdh_at as u32).to_le_bytes());

    out.extend_from_slice(b"TRK");
    out.push(0); // track number
    let data_offset = 4 + 12 * u32::from(revolutions.max(1));
    for _ in 0..revolutions.max(1) {
        out.extend_from_slice(&8_000_000u32.to_le_bytes()); // 200 ms
        out.extend_from_slice(&(flux.len() as u32).to_le_bytes());
        out.extend_from_slice(&data_offset.to_le_bytes());
    }
    for value in flux {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out
}

#[test]
fn a_file_without_the_signature_is_refused() {
    let mut bytes = one_track(&[158; 4], 1);
    bytes[0] = b'X';
    assert_eq!(Scp::parse(&bytes), Err(ScpError::NotScp));
}

#[test]
fn a_file_shorter_than_its_header_is_refused() {
    let bytes = b"SCP\x25\x04\x01".to_vec();
    assert!(matches!(
        Scp::parse(&bytes),
        Err(ScpError::Truncated { needed: 0x10, .. })
    ));
}

#[test]
fn more_revolutions_than_the_format_allows_is_refused() {
    // Not merely unusual: the revolution entries are fixed-size and the sixth
    // would be read from whatever follows them.
    let mut bytes = one_track(&[158; 4], 1);
    bytes[0x05] = (MAX_REVOLUTIONS + 1) as u8;
    assert!(matches!(
        Scp::parse(&bytes),
        Err(ScpError::TooManyRevolutions { declared: 6 })
    ));
}

#[test]
fn the_header_reads_field_by_field() {
    let bytes = one_track(&[158; 8], 2);
    let scp = Scp::parse(&bytes).unwrap();
    assert_eq!(scp.version, 0x25);
    assert_eq!(scp.disk_type, 0x04);
    assert_eq!(scp.revolutions, 2);
    assert_eq!(scp.tick_ns(), 25);
    assert!(scp.index_aligned());
    assert!(!scp.normalised());
    assert!(!scp.extended_mode());
    assert_eq!(scp.tracks.len(), 1);
    assert_eq!(scp.tracks[0].revolutions.len(), 2);
    assert_eq!(scp.tracks[0].revolutions[0].duration_ticks, 8_000_000);
}

#[test]
fn flux_values_are_big_endian_while_everything_else_is_little() {
    // The single most consequential fact about this format's layout. Read the
    // wrong way, 0x009e is 40448 ticks — a millisecond, which is not an
    // interval any drive produces.
    let bytes = one_track(&[0x009e, 0x009e], 1);
    let scp = Scp::parse(&bytes).unwrap();
    let intervals = scp.intervals(&bytes, 0, 0).unwrap();
    assert_eq!(intervals, vec![158, 158]);
    // ...while the track offset that got us there was little-endian.
    assert_eq!(scp.tracks[0].header_offset, 0x10 + TRACK_SLOTS * 4);
}

#[test]
fn a_zero_flux_value_accumulates_rather_than_counting_as_an_interval() {
    // Zero does not mean "no time passed". It means no transition occurred
    // within the 16-bit range, and the interval continues into the next value.
    let bytes = one_track(&[0x0000, 0x0000, 0x0064, 0x009e], 1);
    let scp = Scp::parse(&bytes).unwrap();
    let intervals = scp.intervals(&bytes, 0, 0).unwrap();
    assert_eq!(
        intervals,
        vec![OVERFLOW_TICKS * 2 + 100, 158],
        "two overflows then 100 ticks is one long interval, not three"
    );
}

#[test]
fn a_table_entry_pointing_at_something_other_than_a_track_is_skipped() {
    // The offset table is 168 longwords of attacker-controlled data. An entry
    // pointing into the middle of flux data would otherwise yield a track of
    // plausible nonsense (AV-004).
    let mut bytes = one_track(&[158; 8], 1);
    let entry = 0x10 + 4;
    let into_flux = (bytes.len() - 4) as u32;
    bytes[entry..entry + 4].copy_from_slice(&into_flux.to_le_bytes());
    let scp = Scp::parse(&bytes).unwrap();
    assert_eq!(scp.tracks.len(), 1, "only the real TRK header counts");
}

#[test]
fn flux_running_past_the_end_of_the_file_yields_nothing() {
    // A truncated capture must not produce a short track silently: the sectors
    // it would decode are real, and the ones it would omit are indistinguishable
    // from a disk that never had them.
    let mut bytes = one_track(&[158; 16], 1);
    bytes.truncate(bytes.len() - 8);
    let scp = Scp::parse(&bytes).unwrap();
    assert_eq!(scp.intervals(&bytes, 0, 0), None);
}

#[test]
fn an_absent_track_or_revolution_is_none_not_a_panic() {
    let bytes = one_track(&[158; 8], 1);
    let scp = Scp::parse(&bytes).unwrap();
    assert_eq!(scp.intervals(&bytes, 99, 0), None);
    assert_eq!(scp.intervals(&bytes, 0, 4), None);
}

#[test]
fn a_track_number_is_a_cylinder_and_a_head() {
    let bytes = one_track(&[158; 4], 1);
    let scp = Scp::parse(&bytes).unwrap();
    assert_eq!(scp.tracks[0].cylinder(), 0);
    assert_eq!(scp.tracks[0].head(), 0);
}

#[test]
fn every_byte_of_a_hostile_file_is_survivable() {
    // D-006: no input may panic. Walk a valid file, corrupting one byte at a
    // time, and parse each result.
    let good = one_track(&[158; 32], 2);
    for i in 0..good.len().min(2048) {
        for value in [0x00u8, 0xFF, 0x80] {
            let mut bytes = good.clone();
            bytes[i] = value;
            if let Ok(scp) = Scp::parse(&bytes) {
                for track in 0..2 {
                    let _ = scp.intervals(&bytes, track, 0);
                    let _ = scp.intervals(&bytes, track, 1);
                }
            }
        }
    }
}
