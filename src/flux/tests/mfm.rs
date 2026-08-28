//! Flux intervals to MFM bits.
//!
//! # The round trip is the real test
//!
//! `ade-track` can *encode* an AmigaDOS track, so a whole disk's worth of
//! known-correct MFM is available without a capture, a drive or an external
//! tool. Turning those bits back into flux intervals is the inverse of what
//! this module does — the distance between two transitions, in cells — so
//! encode, invert, decode, and the sectors must come back identical.
//!
//! That closes the loop the way `encode_track` closed MFM's: the check is
//! against something independently produced, not against this module's own
//! idea of what it should have done.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use ade_flux::mfm::{MAX_CELLS, NOMINAL_CELL_TICKS, to_bits};
use ade_track::{decode_track, encode_track};

/// The inverse of [`to_bits`]: a bit stream becomes the intervals a drive
/// would have seen reading it, at a given cell width.
///
/// A `1` bit is a transition. The interval is the time since the previous one,
/// so this counts cells between set bits.
fn to_flux(bits: &[u8], cell: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut since = 0u32;
    let mut started = false;
    for byte in bits {
        for shift in (0..8).rev() {
            let set = (byte >> shift) & 1 == 1;
            since += 1;
            if set {
                if started {
                    out.push(since * cell);
                }
                started = true;
                since = 0;
            }
        }
    }
    out
}

/// A track of eleven sectors, filled so no two are alike.
fn sample_track() -> Vec<u8> {
    let sectors: Vec<Vec<u8>> = (0u32..11)
        .map(|s| {
            (0u32..512)
                .map(|i| u8::try_from(s.wrapping_mul(7).wrapping_add(i) % 256).unwrap_or(0))
                .collect()
        })
        .collect();
    let refs: Vec<&[u8]> = sectors.iter().map(Vec::as_slice).collect();
    encode_track(0, &refs).unwrap()
}

#[test]
fn an_interval_of_two_cells_is_one_bit_then_a_zero() {
    // MFM's encoding in one assertion: the transition *is* the one bit, and
    // the cells that pass without one are the zeros.
    let stream = to_bits(&[NOMINAL_CELL_TICKS * 2], NOMINAL_CELL_TICKS);
    assert_eq!(stream.bits, vec![0b1000_0000]);
}

#[test]
fn three_and_four_cell_intervals_lengthen_the_run_of_zeros() {
    let three = to_bits(&[NOMINAL_CELL_TICKS * 3], NOMINAL_CELL_TICKS);
    let four = to_bits(&[NOMINAL_CELL_TICKS * 4], NOMINAL_CELL_TICKS);
    assert_eq!(three.bits, vec![0b1000_0000]);
    assert_eq!(four.bits, vec![0b1000_0000]);
    // The bits themselves are padded to a byte, so compare what was written:
    // three cells is "100", four is "1000".
    assert_eq!(to_bits(&[240, 240], 80).bits, vec![0b1001_0000]);
    assert_eq!(to_bits(&[320, 320], 80).bits, vec![0b1000_1000]);
}

#[test]
fn an_interval_is_rounded_to_the_nearest_cell_not_truncated() {
    // A drive running 5% fast produces 1.9 cells where the disk holds 2.
    // Truncation would call that one cell and corrupt every bit after it.
    let fast = to_bits(&[152, 152, 152], 80);
    let exact = to_bits(&[160, 160, 160], 80);
    assert_eq!(fast.bits, exact.bits);
    assert_eq!(fast.out_of_range, 0);
}

#[test]
fn intervals_too_short_to_be_legal_are_counted_and_dropped() {
    // Shorter than two cells is not an MFM interval at all — it is noise, and
    // emitting a bit for it would shift everything after it.
    let stream = to_bits(&[10, 160, 160], 80);
    assert_eq!(stream.out_of_range, 1);
    assert_eq!(stream.bits, vec![0b1010_0000]);
}

#[test]
fn an_interval_too_long_to_be_legal_still_takes_up_its_time() {
    // An erased or unformatted region is a long gap. Dropping it would splice
    // the bits either side together as though it had never been there.
    let stream = to_bits(&[80 * 10, 160], 80);
    assert_eq!(stream.out_of_range, 1);
    let ones = stream.bits.iter().map(|b| b.count_ones()).sum::<u32>();
    assert_eq!(ones, 2, "the gap is one transition then silence");
    assert!(stream.bits.len() * 8 > MAX_CELLS as usize);
}

#[test]
fn the_loop_follows_a_drive_running_slow() {
    // Two percent slow, sustained — well inside what a real drive does, and
    // exactly what a fixed divisor accumulates into a wrong bit.
    let slow: Vec<u32> = (0..2000).map(|_| 163).collect();
    let stream = to_bits(&slow, NOMINAL_CELL_TICKS);
    assert_eq!(stream.out_of_range, 0);
    assert!(stream.locked(), "a 2% drift is not a lock failure");
    assert!(
        stream.final_cell_ticks > NOMINAL_CELL_TICKS,
        "the estimate should have moved toward the slower rate"
    );
}

#[test]
fn a_rate_nothing_like_mfm_is_reported_as_unlocked() {
    // Not a crash and not silence: a capture at the wrong data rate decodes to
    // *something*, and the only honest answer is to say the lock failed.
    let wrong: Vec<u32> = (0..2000).map(|_| 500).collect();
    let stream = to_bits(&wrong, NOMINAL_CELL_TICKS);
    assert!(!stream.locked());
}

#[test]
fn a_track_survives_the_round_trip_through_flux() {
    // The whole point: encode a real AmigaDOS track, turn it into the flux a
    // drive would have seen, read it back, and get the same sectors.
    let track = sample_track();
    let flux = to_flux(&track, NOMINAL_CELL_TICKS);
    let stream = to_bits(&flux, NOMINAL_CELL_TICKS);

    let before = decode_track(&track);
    let after = decode_track(&stream.bits);
    assert_eq!(before.sound(), 11, "the encoder should produce 11 sectors");
    assert_eq!(after.sound(), 11, "and all 11 should survive the flux");

    for (a, b) in before.sectors.iter().zip(after.sectors.iter()) {
        assert_eq!(a.sector, b.sector);
        assert_eq!(a.data, b.data);
    }
}

#[test]
fn a_track_survives_a_drive_running_off_speed() {
    // The same round trip with the flux stretched 3%, which is a real drive on
    // a cold morning rather than a hostile input.
    let track = sample_track();
    let flux: Vec<u32> = to_flux(&track, NOMINAL_CELL_TICKS)
        .iter()
        .map(|i| i * 103 / 100)
        .collect();
    let stream = to_bits(&flux, NOMINAL_CELL_TICKS);
    assert_eq!(decode_track(&stream.bits).sound(), 11);
}

#[test]
fn empty_flux_yields_empty_bits() {
    let stream = to_bits(&[], NOMINAL_CELL_TICKS);
    assert!(stream.bits.is_empty());
    assert_eq!(stream.out_of_range, 0);
}

#[test]
fn a_zero_cell_estimate_falls_back_rather_than_dividing_by_zero() {
    let stream = to_bits(&[160, 160], 0);
    assert_eq!(stream.bits, vec![0b1010_0000]);
}
