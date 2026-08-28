//! SuperCard Pro flux images — see SPEC §SCP structure.
//!
//! # What an SCP holds
//!
//! Not sectors, and not even bits: a list of **intervals between magnetic flux
//! transitions**, one list per revolution, per track. Everything above this —
//! bit cells, MFM, sectors, a filesystem — is inferred from those intervals by
//! deciding where the bit cells fall, which is what [`crate::mfm`] does.
//!
//! That is the point of the format. A sector image records what a drive
//! decided the disk said; a flux image records what the disk actually did, so
//! the decisions can be made again, differently, later.
//!
//! # Two byte orders in one format
//!
//! The header, the track table and the revolution entries are
//! **little-endian**. The flux values are **big-endian**. Both go through
//! `ade-endian` (C-001), naming their order at the call site, because the
//! failure mode of guessing is not a crash: `0x009e` is 158 read one way and
//! 40448 read the other, and only one of those is a plausible interval.
//!
//! # What is parsed and what is deliberately not
//!
//! Parsed: the header, the FLAGS, the resolution, the 168-entry track table
//! and each track's revolution entries. Not parsed: the extension footer
//! (creator strings and timestamps — provenance metadata, no bearing on what
//! the disk says) and the file checksum, which covers everything from 0x10 to
//! EOF and would mean reading a 30 MB file to answer a question no caller has
//! asked. Both are recorded here rather than silently skipped.

use ade_endian::{u16_at, u32_le_at};

use core::fmt;

/// `SCP` — the file signature, at offset 0.
pub const MAGIC: &[u8; 3] = b"SCP";
/// `TRK` — the signature every track data header opens with.
pub const TRACK_MAGIC: &[u8; 3] = b"TRK";
/// Entries in the track offset table. Fixed by the format at 168.
pub const TRACK_SLOTS: usize = 168;
/// Where the track offset table begins on an ordinary floppy image.
pub const TABLE_OFFSET: usize = 0x10;
/// Where it begins when the EXTENDED-MODE flag is set (hard and tape drives).
pub const EXTENDED_TABLE_OFFSET: usize = 0x80;
/// The base time unit: one tick is 25 nanoseconds.
pub const TICK_NS: u32 = 25;
/// A flux value of zero means "no transition yet"; this much time passed.
pub const OVERFLOW_TICKS: u32 = 0x1_0000;
/// The most revolutions the format stores per track.
pub const MAX_REVOLUTIONS: usize = 5;

/// Why an SCP could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScpError {
    /// The file does not begin with `SCP`.
    NotScp,
    /// The file ends before the header does.
    Truncated {
        /// What was needed.
        needed: usize,
        /// What there was.
        len: usize,
    },
    /// The header declares more revolutions than the format allows.
    TooManyRevolutions {
        /// The declared count.
        declared: u8,
    },
}

impl fmt::Display for ScpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotScp => f.write_str("not an SCP image: no SCP signature at offset 0"),
            Self::Truncated { needed, len } => {
                write!(
                    f,
                    "truncated SCP: header needs {needed} bytes, file has {len}"
                )
            }
            Self::TooManyRevolutions { declared } => {
                write!(
                    f,
                    "SCP declares {declared} revolutions; the format allows {MAX_REVOLUTIONS}"
                )
            }
        }
    }
}

impl core::error::Error for ScpError {}

/// One stored revolution of one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revolution {
    /// Index-to-index time, in 25 ns ticks. 8,000,000 is 200 ms — 300 RPM.
    pub duration_ticks: u32,
    /// How many flux values this revolution holds.
    pub flux_count: u32,
    /// Where the flux values start, **relative to the track data header**.
    pub data_offset: u32,
}

/// One track's data header and its revolutions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Position in the offset table. This is the *physical* track: cylinder
    /// doubled, plus the head.
    pub index: usize,
    /// Byte offset of the track data header from the start of the file.
    pub header_offset: usize,
    /// The track number the header claims. Usually equal to `index`.
    ///
    /// Kept separate on purpose, and never used for placement. Two corpus
    /// extended-ADFs label every track 0, and trusting a self-declared number
    /// would pile a whole disk onto one track.
    pub declared: u8,
    /// The revolutions stored for this track.
    pub revolutions: Vec<Revolution>,
}

impl Track {
    /// The cylinder this track sits on, if it is a two-sided floppy layout.
    #[must_use]
    pub const fn cylinder(&self) -> usize {
        self.index / 2
    }

    /// The head this track was read with, in the same layout.
    #[must_use]
    pub const fn head(&self) -> usize {
        self.index % 2
    }
}

/// A parsed SCP image: everything except the flux values themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scp {
    /// Version byte, `(version << 4) | revision`. Greaseweazle writes 0.
    pub version: u8,
    /// The disk-type byte — manufacturer in the upper nibble, subclass below.
    ///
    /// **Not to be trusted for format detection.** Greaseweazle writes 0x80
    /// ("other") for an Amiga disk it has just encoded as AmigaDOS MFM, so a
    /// reader that dispatched on this byte would refuse its own output.
    pub disk_type: u8,
    /// Revolutions stored per track, per the header.
    pub revolutions: u8,
    /// First and last track slots the file claims to use.
    pub track_range: (u8, u8),
    /// The raw FLAGS byte.
    pub flags: u8,
    /// Bits per flux value. Zero means the default of 16.
    pub bit_cell_width: u8,
    /// 0 = both heads, 1 = side 0 only, 2 = side 1 only.
    pub heads: u8,
    /// Time resolution as a multiplier of 25 ns; 0 is 25 ns.
    pub resolution: u8,
    /// The checksum the file declares over everything from 0x10 to EOF.
    ///
    /// Recorded, not verified: checking it means reading the whole file, which
    /// for a 30 MB image is a cost nothing has asked for. A caller that wants
    /// the guarantee can compute it.
    pub declared_checksum: u32,
    /// Every track the offset table actually points at.
    pub tracks: Vec<Track>,
}

impl Scp {
    /// Flux data begins at the index pulse (FLAGS bit 0).
    #[must_use]
    pub const fn index_aligned(&self) -> bool {
        self.flags & 0b0000_0001 != 0
    }

    /// The capture came from a 360 RPM drive (FLAGS bit 2).
    #[must_use]
    pub const fn rpm_360(&self) -> bool {
        self.flags & 0b0000_0100 != 0
    }

    /// The flux has been normalised rather than kept as captured (FLAGS bit 3).
    #[must_use]
    pub const fn normalised(&self) -> bool {
        self.flags & 0b0000_1000 != 0
    }

    /// An extension footer follows the track data (FLAGS bit 5).
    #[must_use]
    pub const fn has_footer(&self) -> bool {
        self.flags & 0b0010_0000 != 0
    }

    /// The image describes a hard or tape drive rather than a floppy (bit 6).
    #[must_use]
    pub const fn extended_mode(&self) -> bool {
        self.flags & 0b0100_0000 != 0
    }

    /// Something other than SuperCard Pro hardware wrote this (FLAGS bit 7).
    #[must_use]
    pub const fn foreign_creator(&self) -> bool {
        self.flags & 0b1000_0000 != 0
    }

    /// Nanoseconds per tick, from the resolution byte.
    #[must_use]
    pub const fn tick_ns(&self) -> u32 {
        TICK_NS.saturating_mul((self.resolution as u32).saturating_add(1))
    }

    /// Parse the header and track table. Flux values are left in the file.
    ///
    /// # Errors
    /// [`ScpError`] if the signature is absent, the header is truncated, or
    /// the revolution count exceeds what the format allows.
    pub fn parse(bytes: &[u8]) -> Result<Self, ScpError> {
        let header = bytes.get(..0x10).ok_or(ScpError::Truncated {
            needed: 0x10,
            len: bytes.len(),
        })?;
        if header.get(..3) != Some(MAGIC.as_slice()) {
            return Err(ScpError::NotScp);
        }

        let at = |i: usize| header.get(i).copied().unwrap_or(0);
        let flags = at(0x08);
        let revolutions = at(0x05);
        if usize::from(revolutions) > MAX_REVOLUTIONS {
            return Err(ScpError::TooManyRevolutions {
                declared: revolutions,
            });
        }

        let extended = flags & 0b0100_0000 != 0;
        let table = if extended {
            EXTENDED_TABLE_OFFSET
        } else {
            TABLE_OFFSET
        };

        let mut scp = Self {
            version: at(0x03),
            disk_type: at(0x04),
            revolutions,
            track_range: (at(0x06), at(0x07)),
            flags,
            bit_cell_width: at(0x09),
            heads: at(0x0A),
            resolution: at(0x0B),
            declared_checksum: u32_le_at(bytes, 0x0C).unwrap_or(0),
            tracks: Vec::new(),
        };

        // A revolution count of zero would make every track empty rather than
        // malformed, so read at least one: the entries are there regardless,
        // and a header that lies about its own count should not silently
        // discard the data it points at.
        let revs = usize::from(revolutions).max(1);
        for slot in 0..TRACK_SLOTS {
            let Some(entry) = table.checked_add(slot.saturating_mul(4)) else {
                break;
            };
            let Ok(offset) = u32_le_at(bytes, entry) else {
                break;
            };
            if offset == 0 {
                continue;
            }
            let header_offset = offset as usize;
            // The signature is the check that the offset is real. A table
            // entry pointing into flux data would otherwise yield a track of
            // plausible-looking nonsense.
            if bytes.get(header_offset..header_offset.saturating_add(3))
                != Some(TRACK_MAGIC.as_slice())
            {
                continue;
            }

            let declared = bytes
                .get(header_offset.saturating_add(3))
                .copied()
                .unwrap_or(0);
            let mut list = Vec::with_capacity(revs);
            for rev in 0..revs {
                let base = header_offset
                    .saturating_add(4)
                    .saturating_add(rev.saturating_mul(12));
                let (Ok(duration_ticks), Ok(flux_count), Ok(data_offset)) = (
                    u32_le_at(bytes, base),
                    u32_le_at(bytes, base.saturating_add(4)),
                    u32_le_at(bytes, base.saturating_add(8)),
                ) else {
                    break;
                };
                // An all-zero entry is an unused revolution slot, not a
                // zero-length revolution.
                if flux_count == 0 && data_offset == 0 {
                    continue;
                }
                list.push(Revolution {
                    duration_ticks,
                    flux_count,
                    data_offset,
                });
            }
            scp.tracks.push(Track {
                index: slot,
                header_offset,
                declared,
                revolutions: list,
            });
        }
        Ok(scp)
    }

    /// The intervals between flux transitions, in ticks, for one revolution.
    ///
    /// Returns `None` when the track or revolution does not exist, or when the
    /// flux data runs past the end of the file — a truncated capture yields
    /// nothing for the damaged track rather than a short one silently.
    ///
    /// # Overflow
    ///
    /// A stored value of zero does not mean "no time passed": it means no
    /// transition occurred within the 16-bit range, so 65,536 ticks are
    /// accumulated and the next value continues the same interval. Treating
    /// zero as an interval produces a stream of impossible transitions; the
    /// long gaps it stands for are how an unformatted or erased region reads.
    #[must_use]
    pub fn intervals(&self, bytes: &[u8], track: usize, revolution: usize) -> Option<Vec<u32>> {
        let track = self.tracks.iter().find(|t| t.index == track)?;
        let rev = track.revolutions.get(revolution)?;

        let start = track.header_offset.checked_add(rev.data_offset as usize)?;
        let count = rev.flux_count as usize;
        let len = count.checked_mul(2)?;
        let end = start.checked_add(len)?;
        let raw = bytes.get(start..end)?;

        let mut out = Vec::with_capacity(count);
        let mut carry: u32 = 0;
        for i in 0..count {
            let Ok(value) = u16_at(raw, i.saturating_mul(2)) else {
                break;
            };
            if value == 0 {
                carry = carry.saturating_add(OVERFLOW_TICKS);
                continue;
            }
            out.push(carry.saturating_add(u32::from(value)));
            carry = 0;
        }
        Some(out)
    }
}
