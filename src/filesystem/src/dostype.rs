//! Dostype — the four-byte filesystem magic in the bootblock.
//!
//! The identifier is `D`, `O`, `S` followed by a flags byte.
//!
//! # The flags byte is not purely a bitfield
//!
//! Bits 0..2 mean FFS, international, and dircache — but two rules break naive
//! bit decoding, and both fail *silently* if missed (see C-006):
//!
//! 1. **Dircache implies international.** `DOS\4` and `DOS\5` set the dircache
//!    bit and leave the international bit *clear*, yet international hashing
//!    applies. Reading bit 1 alone makes directory lookup miss on those disks,
//!    reporting "not found" rather than erroring.
//! 2. **`DOS\6` and `DOS\7` are dostypes, not bit patterns.** Because dircache
//!    already implies international, the combinations `0b110` and `0b111` were
//!    never used by the classic filesystems — which is exactly why LNFS (long
//!    filenames, from the AmigaOS 4-era FFS rewrite) claimed them. Decoding
//!    them as "international + dircache" is wrong: they are always
//!    international and never dircache.
//!
//! Use [`Dostype::mode`], [`Dostype::is_international`] and
//! [`Dostype::has_dircache`] rather than testing bits directly.
//!
//! Sources: Clévy's ADF FAQ §4.1, the AmigaOS wiki on DCFS/LNFS structures, and
//! the Linux AFFS driver documentation (which supports `DOS\0`..`DOS\5` only).
//! Tabulated in [SPEC.md](../../../Docs/SPEC.md).
//!
//! # What is still unverified
//!
//! The LNFS *block layout* — the 112-byte name-and-comment array and the
//! separate comment block — is summary-level only and needs a field-level pass
//! before Phase 2 implements it. Only the dostype identification above is
//! settled.

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

/// The naming and caching scheme a volume uses.
///
/// Resolved from the whole flags byte rather than from individual bits,
/// because `DOS\6`/`DOS\7` are distinct dostypes rather than bit combinations.
/// See the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 30-character names, no directory cache. `DOS\0`..`DOS\3`.
    Classic,
    /// Directory cache blocks present. `DOS\4`, `DOS\5`. Always international.
    DirCache,
    /// Long filenames (LNFS). `DOS\6`, `DOS\7`. Always international, never
    /// dircache. A later extension; the Linux AFFS driver does not support it.
    LongNames,
}

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

    /// The naming and caching scheme.
    ///
    /// `DOS\6` and `DOS\7` are matched on the whole byte, not on their low
    /// bits: they are documented dostypes rather than flag combinations, so a
    /// byte that merely happens to end in `110` is treated as classic with
    /// unrecognised bits instead of being mistaken for LNFS.
    #[must_use]
    pub const fn mode(self) -> Mode {
        match self.flags {
            6 | 7 => Mode::LongNames,
            f if f & FLAG_DIRCACHE != 0 => Mode::DirCache,
            _ => Mode::Classic,
        }
    }

    /// Whether international case-folding applies to directory hashing.
    ///
    /// True when the international bit is set, **and also** whenever dircache
    /// or LNFS is in use — those imply international hashing while leaving the
    /// international bit clear (C-006).
    #[must_use]
    pub const fn is_international(self) -> bool {
        match self.mode() {
            Mode::DirCache | Mode::LongNames => true,
            Mode::Classic => self.flags & FLAG_INTL != 0,
        }
    }

    /// Whether directory cache blocks are present.
    #[must_use]
    pub const fn has_dircache(self) -> bool {
        matches!(self.mode(), Mode::DirCache)
    }

    /// The raw international bit, as stored on disk.
    ///
    /// Distinct from [`Self::is_international`]: on a dircache or LNFS volume
    /// this is `false` while international hashing still applies. Exposed so a
    /// health report can state what the disk *says* beside what it *means*.
    #[must_use]
    pub const fn intl_flag_set(self) -> bool {
        self.flags & FLAG_INTL != 0
    }

    /// Flag bits ADE does not yet interpret.
    ///
    /// Non-zero means the image uses something outside the three documented
    /// bits — surfaced rather than ignored, per D-006.
    #[must_use]
    pub const fn unrecognised_flags(self) -> u8 {
        match self.mode() {
            // LNFS uses the whole low three bits; none of them is spare.
            Mode::LongNames => self.flags & !0x07,
            Mode::Classic | Mode::DirCache => self.flags & !(FLAG_FFS | FLAG_INTL | FLAG_DIRCACHE),
        }
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
        match self.mode() {
            Mode::DirCache => f.write_str(", dircache")?,
            Mode::LongNames => f.write_str(", long names")?,
            Mode::Classic => {}
        }
        if self.unrecognised_flags() != 0 {
            write!(f, ", unknown bits {:#04x}", self.unrecognised_flags())?;
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

    /// The full table, as tabulated in SPEC.md from the ADF FAQ, the AmigaOS
    /// wiki and the Linux AFFS docs. Columns: flags, filesystem, international,
    /// dircache, mode.
    const TABLE: &[(u8, FileSystem, bool, bool, Mode)] = &[
        (0, FileSystem::Ofs, false, false, Mode::Classic),
        (1, FileSystem::Ffs, false, false, Mode::Classic),
        (2, FileSystem::Ofs, true, false, Mode::Classic),
        (3, FileSystem::Ffs, true, false, Mode::Classic),
        // DIRC implies INTL even though the INTL bit stays clear — BUG-001.
        (4, FileSystem::Ofs, true, true, Mode::DirCache),
        (5, FileSystem::Ffs, true, true, Mode::DirCache),
        // LNFS: always international, never dircache, despite bits 1 and 2.
        (6, FileSystem::Ofs, true, false, Mode::LongNames),
        (7, FileSystem::Ffs, true, false, Mode::LongNames),
    ];

    #[test]
    fn matches_the_full_dostype_table() {
        for &(flags, fs, intl, dirc, mode) in TABLE {
            let d = dostype(flags);
            assert_eq!(d.filesystem(), fs, "DOS\\{flags} filesystem");
            assert_eq!(d.is_international(), intl, "DOS\\{flags} international");
            assert_eq!(d.has_dircache(), dirc, "DOS\\{flags} dircache");
            assert_eq!(d.mode(), mode, "DOS\\{flags} mode");
            assert_eq!(d.unrecognised_flags(), 0, "DOS\\{flags} has no spare bits");
        }
    }

    #[test]
    fn dircache_is_international_with_the_intl_bit_clear() {
        // BUG-001, the regression that matters: `toupper` is the only
        // difference between the two hash functions, so getting this wrong
        // makes directory lookup miss rather than error.
        for flags in [4u8, 5] {
            let d = dostype(flags);
            assert!(d.is_international(), "DOS\\{flags} hashes as international");
            assert!(!d.intl_flag_set(), "...while the stored INTL bit is clear");
        }
    }

    #[test]
    fn lnfs_is_not_decoded_as_intl_plus_dircache() {
        // 0b110 and 0b111 were free precisely because DIRC implies INTL, so
        // LNFS took them. Bit-decoding them would report dircache wrongly.
        for flags in [6u8, 7] {
            let d = dostype(flags);
            assert_eq!(d.mode(), Mode::LongNames);
            assert!(!d.has_dircache(), "DOS\\{flags} has no directory cache");
            assert!(d.is_international());
        }
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
        // 0x32 occurs three times in a 4288-image TOSEC survey. Decode what we
        // can (bit 1 set → international) and surface the rest.
        let odd = dostype(0x32);
        assert_eq!(odd.mode(), Mode::Classic);
        assert!(odd.is_international());
        assert_eq!(odd.unrecognised_flags(), 0x30);
    }

    #[test]
    fn a_high_bit_does_not_turn_a_byte_into_lnfs() {
        // 0b0001_0110 ends in 110 but is not DOS\6.
        let d = dostype(0b0001_0110);
        assert_eq!(d.mode(), Mode::DirCache, "bit 2 is set, so dircache");
        assert_eq!(d.unrecognised_flags(), 0b0001_0000);
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
        assert_eq!(dostype(5).to_string(), "DOS\\5 (FFS, INTL, dircache)");
        assert_eq!(dostype(7).to_string(), "DOS\\7 (FFS, INTL, long names)");
        assert_eq!(
            dostype(0x32).to_string(),
            "DOS\\50 (OFS, INTL, unknown bits 0x30)"
        );
    }
}
