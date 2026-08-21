//! Dostype — the four-byte filesystem magic in the bootblock.
//!
//! The identifier is `D`, `O`, `S` followed by a flags byte. ADE models the
//! flags byte by its documented bit meanings rather than by enumerating eight
//! named constants, because the bits are what the on-disk format actually
//! encodes and they compose independently.
//!
//! # A deliberate gap
//!
//! [SPEC.md](../../../Docs/SPEC.md) defers the authoritative dostype table to
//! Phase 1/2, to be tabulated against the AFFS driver documentation. In
//! particular the long-filename (LNFS) variants and the exact naming of
//! `DOS\6`/`DOS\7` are **not** settled here, and this module does not pretend
//! otherwise: it decodes the three bits it can justify and preserves the raw
//! value for everything else. Guessing the table now would put an unverified
//! claim somewhere it would later be trusted.

use ade_endian::{OutOfBounds, u32_at};

/// The `DOS` prefix common to every Amiga dostype.
pub const DOS_PREFIX: [u8; 3] = *b"DOS";

/// The prefix positioned in the high three bytes of the dostype word.
///
/// Comparing against this avoids decomposing the word into bytes, which would
/// mean a byte-order conversion outside `ade-endian` (C-001).
const DOS_PREFIX_WORD: u32 = 0x444F_5300;

/// Mask selecting the prefix bytes.
const PREFIX_MASK: u32 = 0xFFFF_FF00;

/// Bit 0 of the flags byte: Fast File System rather than Old File System.
pub const FLAG_FFS: u8 = 0b001;
/// Bit 1: international mode, altering case-folding in directory hashing.
pub const FLAG_INTL: u8 = 0b010;
/// Bit 2: directory cache blocks are present.
pub const FLAG_DIRCACHE: u8 = 0b100;

/// Which of the two filesystems a volume uses.
///
/// The distinction is load-bearing at the block layer, not merely cosmetic:
/// OFS data blocks spend 24 bytes on a header and carry 488 bytes of payload,
/// where FFS uses the full 512 (C-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystem {
    /// Old File System — 24-byte data-block header, 488 usable bytes.
    Ofs,
    /// Fast File System — no data-block header, 512 usable bytes.
    Ffs,
}

impl FileSystem {
    /// Usable payload bytes in one data block, for a given block size.
    ///
    /// Returns `None` if the block is too small to hold an OFS header, which
    /// a hostile or corrupt geometry could claim.
    #[must_use]
    pub const fn payload_bytes(self, block_size: u32) -> Option<u32> {
        match self {
            Self::Ffs => Some(block_size),
            Self::Ofs => block_size.checked_sub(Self::OFS_HEADER_BYTES),
        }
    }

    /// Bytes of metadata at the head of an OFS data block.
    pub const OFS_HEADER_BYTES: u32 = 24;
}

/// A parsed dostype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dostype {
    raw: u32,
    flags: u8,
}

/// Why a four-byte value is not a dostype ADE recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DostypeError {
    /// The value did not begin with `DOS`.
    ///
    /// Other prefixes exist in the wild — `PFS`, `SFS`, `KICK` — and are out
    /// of scope for v1, so they are reported rather than guessed at.
    NotDos {
        /// The raw four bytes, for reporting.
        raw: u32,
    },
    /// The bootblock was too short to hold a dostype.
    Truncated(OutOfBounds),
}

impl core::fmt::Display for DostypeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotDos { raw } => write!(f, "not an Amiga dostype: {raw:#010x}"),
            Self::Truncated(e) => write!(f, "reading dostype: {e}"),
        }
    }
}

impl core::error::Error for DostypeError {}

impl Dostype {
    /// Decode the dostype at `offset` in a bootblock.
    pub fn parse(buf: &[u8], offset: usize) -> Result<Self, DostypeError> {
        let raw = u32_at(buf, offset).map_err(DostypeError::Truncated)?;
        Self::from_raw(raw)
    }

    /// Decode an already-extracted big-endian word.
    #[allow(clippy::cast_possible_truncation, reason = "masked to a single byte")]
    pub const fn from_raw(raw: u32) -> Result<Self, DostypeError> {
        if raw & PREFIX_MASK != DOS_PREFIX_WORD {
            return Err(DostypeError::NotDos { raw });
        }
        Ok(Self {
            raw,
            flags: (raw & 0xFF) as u8,
        })
    }

    /// The raw four-byte value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }

    /// The flags byte following the `DOS` prefix.
    #[must_use]
    pub const fn flags(self) -> u8 {
        self.flags
    }

    /// Which filesystem this volume uses.
    #[must_use]
    pub const fn filesystem(self) -> FileSystem {
        if self.flags & FLAG_FFS == 0 {
            FileSystem::Ofs
        } else {
            FileSystem::Ffs
        }
    }

    /// Whether international case-folding applies to directory hashing.
    #[must_use]
    pub const fn is_international(self) -> bool {
        self.flags & FLAG_INTL != 0
    }

    /// Whether directory cache blocks are present.
    #[must_use]
    pub const fn has_dircache(self) -> bool {
        self.flags & FLAG_DIRCACHE != 0
    }

    /// Flag bits ADE does not yet interpret.
    ///
    /// Non-zero means the image uses something outside the three documented
    /// bits — surfaced rather than ignored, per D-006.
    #[must_use]
    pub const fn unrecognised_flags(self) -> u8 {
        self.flags & !(FLAG_FFS | FLAG_INTL | FLAG_DIRCACHE)
    }
}

impl core::fmt::Display for Dostype {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DOS\\{}", self.flags)?;
        match self.filesystem() {
            FileSystem::Ofs => f.write_str(" (OFS")?,
            FileSystem::Ffs => f.write_str(" (FFS")?,
        }
        if self.is_international() {
            f.write_str(", INTL")?;
        }
        if self.has_dircache() {
            f.write_str(", dircache")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    fn dostype(flags: u8) -> Dostype {
        Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap()
    }

    #[test]
    fn decodes_the_documented_bits() {
        assert_eq!(dostype(0).filesystem(), FileSystem::Ofs);
        assert_eq!(dostype(1).filesystem(), FileSystem::Ffs);
        assert!(!dostype(1).is_international());
        assert!(dostype(2).is_international());
        assert!(dostype(4).has_dircache());
        assert!(dostype(5).has_dircache());
        assert_eq!(dostype(5).filesystem(), FileSystem::Ffs);
    }

    #[test]
    fn rejects_foreign_prefixes() {
        // PFS and SFS are real, and out of scope for v1 — reported, not guessed.
        assert!(matches!(
            Dostype::parse(b"PFS\x00", 0),
            Err(DostypeError::NotDos { .. })
        ));
        assert!(matches!(
            Dostype::parse(b"", 0),
            Err(DostypeError::Truncated(_))
        ));
    }

    #[test]
    fn surfaces_bits_it_cannot_explain() {
        assert_eq!(dostype(0b0000_0111).unrecognised_flags(), 0);
        assert_eq!(dostype(0b0010_0000).unrecognised_flags(), 0b0010_0000);
    }

    #[test]
    fn ofs_loses_twenty_four_bytes_per_block() {
        // C-005 — the difference the block layer is parameterised on.
        assert_eq!(FileSystem::Ofs.payload_bytes(512), Some(488));
        assert_eq!(FileSystem::Ffs.payload_bytes(512), Some(512));
        assert_eq!(
            FileSystem::Ofs.payload_bytes(16),
            None,
            "too small for a header"
        );
    }

    #[test]
    fn displays_readably() {
        assert_eq!(dostype(0).to_string(), "DOS\\0 (OFS)");
        assert_eq!(dostype(3).to_string(), "DOS\\3 (FFS, INTL)");
        assert_eq!(dostype(5).to_string(), "DOS\\5 (FFS, dircache)");
    }
}
