//! Extended ADF (`UAE-1ADF`) — the container that carries raw MFM.
//!
//! A plain ADF holds decoded sectors and therefore cannot represent a disk that
//! is not made of ordinary sectors. Copy protection is precisely that: tracks
//! with the wrong sector count, deliberate CRC errors, non-standard sync words,
//! or data in the gaps. Extended ADF exists to carry those tracks as raw MFM.
//!
//! # The layout is empirical
//!
//! There is no published specification. The structure below was derived from
//! the eleven extended ADFs in the corpus and checked arithmetically against
//! every one of them. Two things that a plausible reading gets wrong are
//! recorded in SPEC and enforced here:
//!
//! - **`space` is the file allocation; `length` is the meaningful extent.** For
//!   a type-0 track `length` is 45056 bits — 5632 bytes, one standard DD track
//!   — in all 428 observed, while `space` varies by writer (5632, 12650 and
//!   12668 all occur). A reader that takes `space` as the data size reads
//!   thousands of bytes of padding as sectors.
//! - **A track may be empty.** 154 observed type-1 tracks have `space` and
//!   `length` both zero: an unformatted or unrecorded track, which is a fact
//!   about the disk rather than a defect in the file.
//!
//! Mixed track types within one image are normal and are the *signature* of
//! copy protection — `Deep Space` carries track 0 as standard sectors and the
//! remaining 165 as raw MFM.

use ade_endian::{u16_at, u32_at};

/// The magic every extended ADF opens with.
pub const MAGIC: &[u8; 8] = b"UAE-1ADF";

/// Bytes of file header before the track table.
const HEADER_BYTES: usize = 12;

/// Bytes per track-table entry.
const ENTRY_BYTES: usize = 12;

/// Bytes in one standard double-density track: 11 sectors of 512.
pub const STANDARD_TRACK_BYTES: usize = 11 * 512;

/// A sanity bound on the track count, so a corrupt header cannot make the
/// reader allocate a table from an attacker-controlled number (AV-005).
///
/// A double-sided 83-cylinder disk is 166 tracks; 1024 is far above anything
/// real and far below anything harmful.
const MAX_TRACKS: usize = 1024;

/// What a track holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Decoded AmigaDOS sectors, exactly as a plain ADF stores them.
    Sectors,
    /// Raw MFM, as captured. This is the reason the format exists.
    RawMfm,
    /// Something else. Reported rather than guessed at — the type field is two
    /// bytes and only 0 and 1 have ever been observed.
    Unknown(u16),
}

impl core::fmt::Display for TrackKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Sectors => f.write_str("sectors"),
            Self::RawMfm => f.write_str("raw MFM"),
            Self::Unknown(t) => write!(f, "unknown type {t}"),
        }
    }
}

/// One track's entry in the table, plus where its data sits in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Index in the table: cylinder × 2 + head.
    pub index: usize,
    /// What the track holds.
    pub kind: TrackKind,
    /// Bytes allocated to the track in the file. **Not** the data length.
    pub space: u32,
    /// Meaningful extent, in **bits**.
    pub length_bits: u32,
    /// Where the data begins in the file.
    pub offset: usize,
    /// Whether the file actually extends far enough to hold it.
    pub present: bool,
}

impl Track {
    /// The meaningful extent in whole bytes, rounded up from the bit count.
    #[must_use]
    pub const fn length_bytes(&self) -> usize {
        self.length_bits.div_ceil(8) as usize
    }

    /// Whether the track holds nothing at all — unformatted, or not captured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.space == 0 && self.length_bits == 0
    }
}

/// A parsed extended ADF: its track table and what is wrong with it.
#[derive(Debug, Clone)]
pub struct ExtendedAdf {
    /// Every track the table declares, in order.
    pub tracks: Vec<Track>,
    /// Problems found. A truncated file is reported and the tracks that *are*
    /// present are kept — half a disk is still worth reading.
    pub faults: Vec<String>,
}

/// Why an extended ADF could not be parsed at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtendedError {
    /// The magic is not `UAE-1ADF`.
    NotExtendedAdf,
    /// The file ends inside the header or the track table.
    Truncated,
    /// The track count exceeds any plausible disk.
    TooManyTracks {
        /// What the header claimed.
        declared: usize,
    },
}

impl core::fmt::Display for ExtendedError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotExtendedAdf => f.write_str("not an extended ADF"),
            Self::Truncated => f.write_str("the file ends inside its own track table"),
            Self::TooManyTracks { declared } => {
                write!(f, "{declared} tracks declared, which no disk has")
            }
        }
    }
}

impl core::error::Error for ExtendedError {}

impl ExtendedAdf {
    /// Parse the header and track table.
    ///
    /// Track data is not copied: each [`Track`] records where its bytes are, so
    /// a multi-megabyte image is described without being duplicated.
    ///
    /// # Errors
    /// [`ExtendedError`] when the file is not one, or is too damaged to have a
    /// table at all. Damage *past* the table is a fault, not an error.
    pub fn parse(bytes: &[u8]) -> Result<Self, ExtendedError> {
        if bytes.get(..8) != Some(MAGIC) {
            return Err(ExtendedError::NotExtendedAdf);
        }
        let declared = usize::from(u16_at(bytes, 10).map_err(|_| ExtendedError::Truncated)?);
        if declared > MAX_TRACKS {
            return Err(ExtendedError::TooManyTracks { declared });
        }

        let table_end = HEADER_BYTES
            .checked_add(
                declared
                    .checked_mul(ENTRY_BYTES)
                    .ok_or(ExtendedError::Truncated)?,
            )
            .ok_or(ExtendedError::Truncated)?;
        if bytes.len() < table_end {
            return Err(ExtendedError::Truncated);
        }

        let mut tracks = Vec::with_capacity(declared);
        let mut faults = Vec::new();
        let mut offset = table_end;

        for index in 0..declared {
            let at = HEADER_BYTES.saturating_add(index.saturating_mul(ENTRY_BYTES));
            let raw_type = u16_at(bytes, at.saturating_add(2)).unwrap_or(0);
            let space = u32_at(bytes, at.saturating_add(4)).unwrap_or(0);
            let length_bits = u32_at(bytes, at.saturating_add(8)).unwrap_or(0);

            let kind = match raw_type {
                0 => TrackKind::Sectors,
                1 => TrackKind::RawMfm,
                other => {
                    faults.push(format!("track {index}: unknown track type {other}"));
                    TrackKind::Unknown(other)
                }
            };

            let end = offset.saturating_add(space as usize);
            let present = end <= bytes.len();
            if !present && space > 0 {
                faults.push(format!(
                    "track {index}: the file ends before its data — {} bytes short",
                    end.saturating_sub(bytes.len())
                ));
            }
            // `length` must not exceed what is allocated; a writer that says
            // otherwise is describing data it did not store.
            if length_bits.div_ceil(8) as usize > space as usize {
                faults.push(format!(
                    "track {index}: claims {length_bits} bits but only {space} bytes are allocated"
                ));
            }

            tracks.push(Track {
                index,
                kind,
                space,
                length_bits,
                offset,
                present,
            });
            offset = end;
        }

        if offset < bytes.len() {
            faults.push(format!(
                "{} trailing bytes past the last track",
                bytes.len().saturating_sub(offset)
            ));
        }

        Ok(Self { tracks, faults })
    }

    /// The bytes of one track, or `None` if the file does not reach them.
    ///
    /// Bounded by `length` rather than `space`: `space` is the allocation and
    /// includes padding, which for a type-0 track would be read as sectors.
    #[must_use]
    pub fn track_data<'a>(&self, bytes: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let track = self.tracks.get(index)?;
        if !track.present || track.is_empty() {
            return None;
        }
        let wanted = match track.kind {
            // A standard track is a known size; the length field agrees in
            // every observed image, but the constant is the authority.
            TrackKind::Sectors => STANDARD_TRACK_BYTES.min(track.space as usize),
            _ => track.length_bytes().min(track.space as usize),
        };
        let end = track.offset.checked_add(wanted)?;
        bytes.get(track.offset..end)
    }

    /// How many tracks hold each kind, for reporting.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut sectors = 0usize;
        let mut raw = 0usize;
        let mut empty = 0usize;
        for track in &self.tracks {
            if track.is_empty() {
                empty = empty.saturating_add(1);
            } else if track.kind == TrackKind::Sectors {
                sectors = sectors.saturating_add(1);
            } else {
                raw = raw.saturating_add(1);
            }
        }
        (sectors, raw, empty)
    }
}
