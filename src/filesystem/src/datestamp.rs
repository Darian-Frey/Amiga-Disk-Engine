//! Amiga datestamps: days since 1978-01-01, minutes past midnight, ticks.
//!
//! A tick is 1/50 s. Valid ranges are `0 <= mins < 1440` and `0 <= ticks <
//! 3000`, and a `days` of zero is treated as illegal by most Amiga software
//! (ADF FAQ §4.2).
//!
//! ADE **does not normalise** out-of-range values. A datestamp claiming 90
//! minutes past midnight is a finding about the image, and silently folding it
//! into an hour would destroy the evidence (D-006, F-010).

use core::fmt;

/// Days from the Unix epoch (1970-01-01) to the Amiga epoch (1978-01-01).
const AMIGA_EPOCH_OFFSET_DAYS: i64 = 2922;

/// A raw datestamp, exactly as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Datestamp {
    /// Days since 1978-01-01.
    pub days: u32,
    /// Minutes past midnight.
    pub mins: u32,
    /// Ticks (1/50 s) past the minute.
    pub ticks: u32,
}

/// Why a datestamp is not a valid point in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateFault {
    /// `days` is zero, which Amiga software treats as unset.
    DayZero,
    /// `mins` is 1440 or more.
    MinutesOutOfRange,
    /// `ticks` is 3000 or more.
    TicksOutOfRange,
}

impl fmt::Display for DateFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DayZero => f.write_str("day 0, which Amiga software treats as unset"),
            Self::MinutesOutOfRange => f.write_str("minutes past midnight is 1440 or more"),
            Self::TicksOutOfRange => f.write_str("ticks is 3000 or more"),
        }
    }
}

impl Datestamp {
    /// Build a datestamp from its three raw fields.
    #[must_use]
    pub const fn new(days: u32, mins: u32, ticks: u32) -> Self {
        Self { days, mins, ticks }
    }

    /// Every way this datestamp is out of range, or empty if it is sound.
    #[must_use]
    pub fn faults(self) -> Vec<DateFault> {
        let mut f = Vec::new();
        if self.days == 0 {
            f.push(DateFault::DayZero);
        }
        if self.mins >= 1440 {
            f.push(DateFault::MinutesOutOfRange);
        }
        if self.ticks >= 3000 {
            f.push(DateFault::TicksOutOfRange);
        }
        f
    }

    /// Whether the stamp is within its documented ranges.
    #[must_use]
    pub fn is_sound(self) -> bool {
        self.faults().is_empty()
    }

    /// Calendar date as `(year, month, day)`, from the day count alone.
    ///
    /// Computed even when [`Self::faults`] is non-empty — a caller reporting a
    /// bad stamp usually still wants to show what it decodes to.
    #[must_use]
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(i64::from(self.days).saturating_add(AMIGA_EPOCH_OFFSET_DAYS))
    }

    /// Time of day as `(hour, minute, second)`.
    #[must_use]
    pub fn hms(self) -> (u32, u32, u32) {
        (
            self.mins.checked_div(60).unwrap_or(0),
            self.mins.checked_rem(60).unwrap_or(0),
            self.ticks.checked_div(50).unwrap_or(0),
        )
    }
}

impl fmt::Display for Datestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        let (hh, mm, ss) = self.hms();
        write!(f, "{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
    }
}

/// Days since the Unix epoch to a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is exact over the whole range and
/// avoids a date-library dependency for what is twenty lines of arithmetic.
#[allow(
    clippy::many_single_char_names,
    clippy::arithmetic_side_effects,
    reason = "transcription of Hinnant's civil_from_days over a bounded input: \
              `days` is a u32, so every intermediate stays far inside i64 and \
              cannot overflow. Rewriting it in checked form would make it \
              unverifiable against the published original for no gain."
)]
#[must_use]
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u32::try_from(m).unwrap_or(0),
        u32::try_from(d).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_zero_is_the_amiga_epoch() {
        assert_eq!(Datestamp::new(0, 0, 0).ymd(), (1978, 1, 1));
    }

    #[test]
    fn known_dates_decode() {
        // 1978-01-01 + 365 days = 1979-01-01 (1978 was not a leap year).
        assert_eq!(Datestamp::new(365, 0, 0).ymd(), (1979, 1, 1));
        // The Amiga 1000 launch, 1985-07-23, is day 2760.
        assert_eq!(Datestamp::new(2760, 0, 0).ymd(), (1985, 7, 23));
        // A leap day, to catch off-by-one in the era arithmetic.
        assert_eq!(Datestamp::new(789, 0, 0).ymd(), (1980, 2, 29));
        assert_eq!(Datestamp::new(790, 0, 0).ymd(), (1980, 3, 1));
    }

    #[test]
    fn time_of_day_decodes() {
        assert_eq!(Datestamp::new(1, 0, 0).hms(), (0, 0, 0));
        assert_eq!(Datestamp::new(1, 754, 1250).hms(), (12, 34, 25));
        assert_eq!(Datestamp::new(1, 1439, 2999).hms(), (23, 59, 59));
    }

    #[test]
    fn faults_are_reported_not_normalised() {
        assert_eq!(Datestamp::new(0, 0, 0).faults(), vec![DateFault::DayZero]);
        assert_eq!(
            Datestamp::new(1, 1440, 0).faults(),
            vec![DateFault::MinutesOutOfRange]
        );
        assert_eq!(
            Datestamp::new(1, 100, 3000).faults(),
            vec![DateFault::TicksOutOfRange]
        );
        assert_eq!(
            Datestamp::new(0, 9999, 9999).faults().len(),
            3,
            "every fault is listed, not just the first"
        );
        assert!(Datestamp::new(1, 0, 0).is_sound());

        // A 90-minute "hour" still decodes, rather than being folded away.
        assert_eq!(Datestamp::new(1, 1500, 0).hms(), (25, 0, 0));
    }

    #[test]
    fn displays_as_iso() {
        assert_eq!(
            Datestamp::new(2760, 754, 1250).to_string(),
            "1985-07-23 12:34:25"
        );
    }
}
