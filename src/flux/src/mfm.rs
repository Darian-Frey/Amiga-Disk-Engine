//! Turning flux intervals into an MFM bit stream.
//!
//! # The problem
//!
//! A flux image records *when* the magnetisation reversed, not *what* was
//! written. Recovering the bits means deciding how many bit cells each
//! interval spans, and that decision needs a cell width the file does not
//! state — the drive that wrote the disk and the drive that read it did not
//! agree on speed, and neither ran at exactly its nominal rate.
//!
//! # Why a phase-locked loop rather than a fixed divisor
//!
//! Dividing by a nominal 2 µs works on a file generated from a sector image
//! and fails on a real capture. Motor speed drifts by a percent or two within
//! one revolution, and a fixed divisor accumulates that drift until intervals
//! land on the boundary between two and three cells, where a rounding error
//! becomes a wrong bit and every sector after it is lost.
//!
//! So the cell width is tracked: each interval is measured against the current
//! estimate, and the estimate is nudged toward what the interval implies. The
//! loop is deliberately slow — a small correction per interval — because a
//! fast one chases noise and a genuinely weak bit would drag it off the data
//! rate entirely.
//!
//! # What this cannot do
//!
//! Nothing here recovers a track a drive could not read. Weak bits, where the
//! same physical track reads differently each revolution, decode to *some*
//! answer here, and which answer is undefined; that is what the several stored
//! revolutions are for, and it is why the assembler decodes every stored
//! revolution rather than trusting the first.

/// MFM at 250 kbit/s: one bit cell is 2 µs, which is 80 ticks of 25 ns.
pub const NOMINAL_CELL_TICKS: u32 = 80;
/// The shortest legal MFM interval is two cells; anything less is noise.
pub const MIN_CELLS: u32 = 2;
/// The longest legal MFM interval is four cells. Longer means a gap, damage,
/// or deliberate illegality — kept rather than discarded, since a protected
/// track's whole content may be exactly that.
pub const MAX_CELLS: u32 = 4;
/// How far the cell estimate may drift from nominal before it is refused.
///
/// A capture whose implied data rate is 20% off is not a slow drive; it is a
/// misidentified format, and following it would produce confident nonsense.
pub const MAX_DRIFT: u32 = 20;

/// How strongly each interval corrects the cell estimate, as a divisor.
///
/// One sixteenth of the error per interval. Fast enough to follow motor
/// speed across a revolution, slow enough that a single wild interval moves
/// the estimate by a fraction of a percent.
const LOOP_DIVISOR: u32 = 16;

/// Fixed-point shift for the cell estimate: sixths of a tick, near enough.
///
/// The estimate is carried at 64× so that corrections smaller than one tick
/// still accumulate. Without this the loop is dead where it matters most: a
/// drive running 2% slow implies a cell one tick wider than nominal, one
/// sixteenth of one tick is zero in integer arithmetic, and the estimate never
/// moves at all. It would still *look* right — the drift stays at zero, so a
/// naive lock check would call a loop that never ran a loop that locked.
const FIXED: u32 = 64;

/// How many intervals may fall outside two-to-four cells before the decode is
/// not to be believed, as a percentage.
///
/// A clean capture has one or two per track — the first interval after the
/// index pulse is a partial cell by definition. A capture at the wrong data
/// rate has almost nothing else.
pub const MAX_OUT_OF_RANGE_PERCENT: usize = 5;

/// A bit stream being built, packed most-significant bit first.
///
/// Most significant first because that is the order [`ade_track::decode_track`]
/// scans in, and the order a sync word is written on the disk.
#[derive(Debug, Default)]
struct BitWriter {
    bytes: Vec<u8>,
    partial: u8,
    filled: u8,
}

impl BitWriter {
    fn with_capacity(bits: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bits.div_euclid(8).saturating_add(1)),
            partial: 0,
            filled: 0,
        }
    }

    fn push(&mut self, bit: bool) {
        self.partial = (self.partial << 1) | u8::from(bit);
        self.filled = self.filled.saturating_add(1);
        if self.filled == 8 {
            self.bytes.push(self.partial);
            self.partial = 0;
            self.filled = 0;
        }
    }

    /// Finish, padding the last byte with zeros.
    ///
    /// Padding with zeros is safe in a way padding with ones would not be: a
    /// run of ones can complete a sync word that was never on the disk.
    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            let shift = 8u8.saturating_sub(self.filled);
            self.bytes.push(self.partial << shift);
        }
        self.bytes
    }
}

/// What decoding a revolution's flux produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitstream {
    /// The MFM bits, packed most-significant first.
    pub bits: Vec<u8>,
    /// Intervals that were shorter than two cells or longer than four.
    ///
    /// On a clean capture this is a handful — the first interval after the
    /// index pulse is a partial cell by definition. A large count means the
    /// cell estimate never locked, and the bits are not to be believed.
    pub out_of_range: usize,
    /// Intervals seen in total, the denominator for the above.
    pub intervals: usize,
    /// The cell width the loop settled on, in ticks.
    pub final_cell_ticks: u32,
}

impl Bitstream {
    /// Whether this decode is worth believing.
    ///
    /// Two conditions, and the second is the one experience added. Drift alone
    /// is not enough: flux at a data rate nothing like MFM produces intervals
    /// that are *all* out of range, so the estimate is never corrected, never
    /// drifts, and a drift-only check pronounces a total failure perfectly
    /// locked. A loop that never ran is not a loop that succeeded.
    #[must_use]
    pub fn locked(&self) -> bool {
        let drift = self.final_cell_ticks.abs_diff(NOMINAL_CELL_TICKS);
        let within_drift =
            drift.saturating_mul(100) <= NOMINAL_CELL_TICKS.saturating_mul(MAX_DRIFT);
        let usable = self
            .out_of_range
            .saturating_mul(100)
            .checked_div(self.intervals.max(1))
            .unwrap_or(100)
            <= MAX_OUT_OF_RANGE_PERCENT;
        within_drift && usable
    }
}

/// Convert one revolution's flux intervals into MFM bits.
///
/// `nominal` is the starting estimate of a bit cell in the same units as the
/// intervals; [`NOMINAL_CELL_TICKS`] is right for 25 ns ticks at 250 kbit/s.
///
/// # How an interval becomes bits
///
/// An interval spanning *n* cells means a transition happened, then *n − 1*
/// cells passed without one. In MFM's encoding that is a `1` followed by
/// *n − 1* zeros — the transition *is* the one bit.
#[must_use]
pub fn to_bits(intervals: &[u32], nominal: u32) -> Bitstream {
    let start = if nominal == 0 {
        NOMINAL_CELL_TICKS
    } else {
        nominal
    };
    // Carried at 64× so corrections below one tick are not rounded away.
    let mut cell_fixed = start.saturating_mul(FIXED);
    let mut out = BitWriter::with_capacity(intervals.len().saturating_mul(4));
    let mut out_of_range = 0usize;

    for &interval in intervals {
        let cell = cell_fixed.div_euclid(FIXED).max(1);
        // Round to the nearest whole number of cells rather than truncating:
        // an interval of 1.9 cells is a two-cell interval read by a drive
        // running slightly fast, and truncation would call it one.
        let half = cell.div_euclid(2);
        let cells = interval.saturating_add(half).checked_div(cell).unwrap_or(0);

        if !(MIN_CELLS..=MAX_CELLS).contains(&cells) {
            out_of_range = out_of_range.saturating_add(1);
            // Still emit something for a long interval: an erased region is
            // zeros on the disk, and dropping it would splice the bits either
            // side of it together as though the gap had not existed.
            if cells > MAX_CELLS {
                out.push(true);
                for _ in 1..cells.min(64) {
                    out.push(false);
                }
            }
            continue;
        }

        out.push(true);
        for _ in 1..cells {
            out.push(false);
        }

        // Nudge the estimate toward what this interval implies this cell is,
        // in the fixed-point domain so a fraction of a tick still counts.
        if let Some(implied) = interval.saturating_mul(FIXED).checked_div(cells) {
            let error = implied.abs_diff(cell_fixed).div_euclid(LOOP_DIVISOR);
            cell_fixed = if implied > cell_fixed {
                cell_fixed.saturating_add(error)
            } else {
                cell_fixed.saturating_sub(error)
            };
        }
    }

    Bitstream {
        bits: out.finish(),
        out_of_range,
        intervals: intervals.len(),
        final_cell_ticks: cell_fixed.div_euclid(FIXED),
    }
}
