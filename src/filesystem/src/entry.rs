//! Directory entries: files, directories, and links.
//!
//! Every entry type shares a layout — name, dates, protection, comment, parent
//! and hash chain all sit at the same offsets from the block end — and differs
//! only in its secondary type and the few fields specific to it. This module
//! parses the common shape and records what kind it turned out to be.
//!
//! Source: ADF FAQ §4.4 (file header), §4.5 (directory), §4.6 (links).

use ade_endian::{OutOfBounds, u8_at, u16_at, u32_at};

use crate::datestamp::Datestamp;

/// Primary type shared by rootblock, directory, file header and link blocks.
pub const T_HEADER: u32 = 2;
/// Primary type of a file extension block.
pub const T_LIST: u32 = 16;
/// Primary type of an OFS data block.
pub const T_DATA: u32 = 8;

/// What an entry is, from its secondary type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// `ST_ROOT` (1).
    Root,
    /// `ST_USERDIR` (2).
    Directory,
    /// `ST_SOFTLINK` (3). Support was removed in AmigaDOS 3.0 and the
    /// implementation was broken before that.
    SoftLink,
    /// `ST_LINKDIR` (4). A hard link to a directory — the structure that makes
    /// traversal cycles legal rather than merely hostile (AV-001).
    HardLinkDir,
    /// `ST_FILE` (-3).
    File,
    /// `ST_LINKFILE` (-4).
    HardLinkFile,
    /// Something else. Reported rather than guessed at.
    Unknown(u32),
}

impl EntryKind {
    /// Classify a secondary type.
    #[must_use]
    pub const fn from_secondary(sec_type: u32) -> Self {
        match sec_type {
            1 => Self::Root,
            2 => Self::Directory,
            3 => Self::SoftLink,
            4 => Self::HardLinkDir,
            0xFFFF_FFFD => Self::File,         // -3
            0xFFFF_FFFC => Self::HardLinkFile, // -4
            other => Self::Unknown(other),
        }
    }

    /// Whether this entry can be descended into.
    #[must_use]
    pub const fn is_directory(self) -> bool {
        matches!(self, Self::Root | Self::Directory | Self::HardLinkDir)
    }

    /// Whether this entry holds file data.
    #[must_use]
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File | Self::HardLinkFile)
    }

    /// Whether this entry points at another entry rather than holding content.
    #[must_use]
    pub const fn is_link(self) -> bool {
        matches!(
            self,
            Self::SoftLink | Self::HardLinkDir | Self::HardLinkFile
        )
    }
}

impl core::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Root => f.write_str("volume"),
            Self::Directory => f.write_str("dir"),
            Self::SoftLink => f.write_str("softlink"),
            Self::HardLinkDir => f.write_str("linkdir"),
            Self::File => f.write_str("file"),
            Self::HardLinkFile => f.write_str("linkfile"),
            Self::Unknown(t) => write!(f, "unknown({t:#x})"),
        }
    }
}

/// Protection flags, whose owner bits are inverted relative to group and other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protection(pub u32);

impl Protection {
    /// Owner bits 0–3 are **set to forbid**, so a cleared bit permits.
    #[must_use]
    pub const fn owner_may_delete(self) -> bool {
        self.0 & 0b0001 == 0
    }
    /// See [`Self::owner_may_delete`].
    #[must_use]
    pub const fn owner_may_execute(self) -> bool {
        self.0 & 0b0010 == 0
    }
    /// See [`Self::owner_may_delete`].
    #[must_use]
    pub const fn owner_may_write(self) -> bool {
        self.0 & 0b0100 == 0
    }
    /// See [`Self::owner_may_delete`].
    #[must_use]
    pub const fn owner_may_read(self) -> bool {
        self.0 & 0b1000 == 0
    }
    /// Bit 4: the archive bit, set to mean *archived*.
    #[must_use]
    pub const fn archived(self) -> bool {
        self.0 & 0b1_0000 != 0
    }
    /// Bit 5: pure / re-entrant, so the binary may be made resident.
    #[must_use]
    pub const fn pure(self) -> bool {
        self.0 & 0b10_0000 != 0
    }
    /// Bit 6: the file is a script.
    #[must_use]
    pub const fn script(self) -> bool {
        self.0 & 0b100_0000 != 0
    }

    /// The familiar `hsparwed` rendering used by AmigaDOS `list`.
    #[must_use]
    pub fn to_amigados_string(self) -> String {
        let bit = |set: bool, c: char| if set { c } else { '-' };
        [
            bit(self.0 & 0b1000_0000 != 0, 'h'),
            bit(self.script(), 's'),
            bit(self.pure(), 'p'),
            bit(self.archived(), 'a'),
            bit(self.owner_may_read(), 'r'),
            bit(self.owner_may_write(), 'w'),
            bit(self.owner_may_execute(), 'e'),
            bit(self.owner_may_delete(), 'd'),
        ]
        .into_iter()
        .collect()
    }
}

/// A directory entry, as read from its block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The block this entry occupies.
    pub block: u32,
    /// Primary type. Should be [`T_HEADER`].
    pub block_type: u32,
    /// Secondary type, raw.
    pub secondary_type: u32,
    /// What the secondary type means.
    pub kind: EntryKind,
    /// Name, as stored. Latin-1, not UTF-8.
    pub name: Vec<u8>,
    /// Stored name length, which can exceed the 30-byte field.
    pub declared_name_len: u8,
    /// Comment, as stored.
    pub comment: Vec<u8>,
    /// File size in bytes. Zero for directories and links.
    pub byte_size: u32,
    /// Protection flags.
    pub protection: Protection,
    /// Owner user id.
    pub uid: u16,
    /// Owner group id.
    pub gid: u16,
    /// Last modification.
    pub altered: Datestamp,
    /// Next entry sharing this hash slot, or 0.
    pub hash_chain: u32,
    /// Parent directory block.
    pub parent: u32,
    /// First file extension block, or 0.
    pub extension: u32,
    /// First data block, or 0. Files only.
    pub first_data: u32,
    /// Data-block pointers stored in this block.
    pub high_seq: u32,
    /// For a hard link, the entry it points at.
    pub real_entry: u32,
    /// On a *target*, the newest hard link pointing at it; on a link, the next
    /// one in that chain. Zero when nothing links here (ADF FAQ §4.6).
    pub next_link: u32,
    /// Whether the checksum matched.
    pub checksum_valid: bool,
}

impl Entry {
    /// Parse an entry block.
    ///
    /// # Errors
    /// [`OutOfBounds`] if the block is not a supported block size. A wrong
    /// type, a bad checksum or an over-long name are reported in the value.
    pub fn parse(block: &[u8], at: u32) -> Result<Self, OutOfBounds> {
        let size = block.len();
        if !crate::rootblock::SUPPORTED_BLOCK_SIZES.contains(&size) {
            return Err(OutOfBounds {
                offset: 0,
                needed: crate::rootblock::MIN_BLOCK_SIZE,
                len: size,
            });
        }
        let end = |back: usize| size.saturating_sub(back);

        let declared_name_len = u8_at(block, end(80))?;
        let name = read_bcpl(block, end(79), declared_name_len, 30);
        let declared_comment_len = u8_at(block, end(184))?;
        let comment = read_bcpl(block, end(183), declared_comment_len, 79);
        let secondary_type = u32_at(block, end(4))?;

        Ok(Self {
            block: at,
            block_type: u32_at(block, 0)?,
            secondary_type,
            kind: EntryKind::from_secondary(secondary_type),
            name,
            declared_name_len,
            comment,
            byte_size: u32_at(block, end(188))?,
            protection: Protection(u32_at(block, end(192))?),
            uid: u16_at(block, end(196))?,
            gid: u16_at(block, end(194))?,
            altered: Datestamp::new(
                u32_at(block, end(92))?,
                u32_at(block, end(88))?,
                u32_at(block, end(84))?,
            ),
            hash_chain: u32_at(block, end(16))?,
            parent: u32_at(block, end(12))?,
            extension: u32_at(block, end(8))?,
            first_data: u32_at(block, 16)?,
            high_seq: u32_at(block, 8)?,
            real_entry: u32_at(block, end(44))?,
            next_link: u32_at(block, end(40))?,
            checksum_valid: ade_block::checksum::normal_valid(block),
        })
    }

    /// The name as a lossy string. Amiga names are ISO 8859-1.
    #[must_use]
    pub fn name_lossy(&self) -> String {
        self.name.iter().map(|&b| char::from(b)).collect()
    }

    /// The comment as a lossy string.
    #[must_use]
    pub fn comment_lossy(&self) -> String {
        self.comment.iter().map(|&b| char::from(b)).collect()
    }

    /// Whether the stored name length exceeded its field.
    #[must_use]
    pub fn name_length_overflows(&self) -> bool {
        self.declared_name_len > 30
    }

    /// Whether this block is shaped like a directory entry at all.
    #[must_use]
    pub fn looks_like_an_entry(&self) -> bool {
        self.block_type == T_HEADER && !matches!(self.kind, EntryKind::Unknown(_))
    }
}

/// Read a BCPL string, clamping the declared length to the field width.
///
/// A corrupt length must not walk past the field (AV-004), so the clamp is the
/// defence and the discrepancy is reported separately.
fn read_bcpl(block: &[u8], start: usize, declared: u8, max: usize) -> Vec<u8> {
    let n = usize::from(declared).min(max);
    block
        .get(start..start.saturating_add(n))
        .unwrap_or(&[])
        .to_vec()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests build their own buffers"
)]
mod tests {
    use super::*;

    #[test]
    fn classifies_every_documented_secondary_type() {
        use EntryKind::{Directory, File, HardLinkDir, HardLinkFile, Root, SoftLink};
        for (raw, want) in [
            (1u32, Root),
            (2, Directory),
            (3, SoftLink),
            (4, HardLinkDir),
            (0xFFFF_FFFD, File),
            (0xFFFF_FFFC, HardLinkFile),
        ] {
            assert_eq!(EntryKind::from_secondary(raw), want, "{raw:#x}");
        }
        assert_eq!(EntryKind::from_secondary(99), EntryKind::Unknown(99));
    }

    #[test]
    fn a_hard_link_to_a_directory_is_a_directory() {
        // The AV-001 case: descending into one is legal, which is exactly why
        // traversal needs a visited-set rather than a depth limit.
        assert!(EntryKind::HardLinkDir.is_directory());
        assert!(EntryKind::HardLinkDir.is_link());
        assert!(EntryKind::HardLinkFile.is_file());
        assert!(!EntryKind::SoftLink.is_directory());
    }

    #[test]
    fn owner_protection_bits_are_inverted() {
        // Set means forbidden for bits 0-3 — the opposite of group and other.
        let none = Protection(0);
        assert!(none.owner_may_read() && none.owner_may_write());
        assert!(none.owner_may_execute() && none.owner_may_delete());

        let locked = Protection(0b1111);
        assert!(!locked.owner_may_read() && !locked.owner_may_write());
        assert_eq!(none.to_amigados_string(), "----rwed");
        assert_eq!(locked.to_amigados_string(), "--------");

        // Bit 4 archived, 5 pure, 6 script, 7 hold — each in its own position.
        assert_eq!(Protection(1 << 4).to_amigados_string(), "---arwed");
        assert_eq!(Protection(1 << 5).to_amigados_string(), "--p-rwed");
        assert_eq!(Protection(1 << 6).to_amigados_string(), "-s--rwed");
        assert_eq!(Protection(1 << 7).to_amigados_string(), "h---rwed");
        assert_eq!(Protection(0b1101_0000).to_amigados_string(), "hs-arwed");
    }

    fn entry_block(sec: u32, name: &[u8], name_len: Option<u8>) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        ade_endian::put_u32(&mut b, 0, T_HEADER).unwrap();
        ade_endian::put_u32(&mut b, 512 - 4, sec).unwrap();
        ade_endian::put_u32(&mut b, 512 - 188, 1234).unwrap();
        b[512 - 80] = name_len.unwrap_or(name.len() as u8);
        b[512 - 79..512 - 79 + name.len()].copy_from_slice(name);
        b[512 - 184] = 5;
        b[512 - 183..512 - 183 + 5].copy_from_slice(b"hello");
        let ck = ade_block::checksum::normal(&b).unwrap();
        ade_endian::put_u32(&mut b, 20, ck).unwrap();
        b
    }

    #[test]
    fn parses_a_file_entry() {
        let e = Entry::parse(&entry_block(0xFFFF_FFFD, b"readme", None), 42).unwrap();
        assert_eq!(e.block, 42);
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.name_lossy(), "readme");
        assert_eq!(e.comment_lossy(), "hello");
        assert_eq!(e.byte_size, 1234);
        assert!(e.checksum_valid);
        assert!(e.looks_like_an_entry());
    }

    #[test]
    fn a_corrupt_name_length_cannot_read_past_its_field() {
        let e = Entry::parse(&entry_block(2, b"dir", Some(255)), 1).unwrap();
        assert_eq!(e.name.len(), 30, "clamped to the 30-byte field");
        assert!(e.name_length_overflows(), "and reported");
    }

    #[test]
    fn only_real_block_sizes_parse() {
        for n in [0usize, 100, 511, 513] {
            assert!(Entry::parse(&vec![0u8; n], 0).is_err(), "{n}");
        }
    }

    #[test]
    fn an_unknown_secondary_type_is_not_an_entry() {
        let e = Entry::parse(&entry_block(0x1234, b"weird", None), 1).unwrap();
        assert_eq!(e.kind, EntryKind::Unknown(0x1234));
        assert!(!e.looks_like_an_entry());
    }
}
