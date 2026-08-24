//! The rootblock: the volume's name, dates, hash table and bitmap pointers.
//!
//! Located by computation, never by reading the bootblock's pointer (C-007).
//! [`ade_block::Geometry::root_block`] does that arithmetic.
//!
//! A block at the right place is not automatically a rootblock: 19% of
//! `DOS`-prefixed images in a 4288-image survey have something else there.
//! [`Rootblock::parse`] therefore reports its identifying fields rather than
//! refusing, so a caller can say "no rootblock here" precisely.

use ade_block::checksum;
use ade_endian::{OutOfBounds, u8_at, u32_at};

use crate::datestamp::Datestamp;

/// Primary type of a rootblock: `T_HEADER`.
pub const T_HEADER: u32 = 2;
/// Secondary type of a rootblock: `ST_ROOT`.
pub const ST_ROOT: u32 = 1;
/// Value of `bm_flag` meaning the bitmap is trustworthy.
pub const BITMAP_VALID: u32 = 0xFFFF_FFFF;

/// The smallest block a volume may use.
pub const MIN_BLOCK_SIZE: usize = 512;

/// Block sizes AmigaDOS supports (C-002, C-005). Floppies are always 512;
/// hard-disk partitions carry their size in the RDB.
pub const SUPPORTED_BLOCK_SIZES: [usize; 4] = [512, 1024, 2048, 4096];

/// The volume header, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rootblock {
    /// Primary type. Should be [`T_HEADER`].
    pub block_type: u32,
    /// Secondary type. Should be [`ST_ROOT`].
    pub secondary_type: u32,
    /// Hash-table size in longs — `BSIZE/4 - 56`, so 72 for a 512-byte block.
    pub hash_table_size: u32,
    /// Whether the stored checksum matches.
    pub checksum_valid: bool,
    /// The `bm_flag` field as stored.
    pub bitmap_flag: u32,
    /// First bitmap block pointer, kept for compatibility with the common case.
    pub bitmap_block: u32,
    /// All 25 bitmap pointers stored directly in the rootblock.
    pub bitmap_pages: Vec<u32>,
    /// First bitmap *extension* block, for volumes needing more than 25.
    pub bitmap_extension: u32,
    /// Volume name, as bytes — Latin-1 on the Amiga, not UTF-8.
    pub name: Vec<u8>,
    /// Stored name length, which can disagree with what is readable.
    pub declared_name_len: u8,
    /// Last change to the root directory.
    pub root_altered: Datestamp,
    /// Last change to the volume.
    pub volume_altered: Datestamp,
    /// When the volume was formatted. Never updated afterwards.
    pub created: Datestamp,
}

impl Rootblock {
    /// Parse a candidate rootblock.
    ///
    /// # Errors
    /// [`OutOfBounds`] if the block is shorter than 512 bytes. Wrong types, a
    /// bad checksum and a nonsense name length are reported in the returned
    /// value, not as errors — deciding whether this *is* a rootblock is the
    /// caller's job, via [`Self::looks_like_a_rootblock`].
    pub fn parse(block: &[u8]) -> Result<Self, OutOfBounds> {
        let size = block.len();
        // Every field below is addressed relative to the *end* of the block, so
        // the length must be a real block size or those offsets land somewhere
        // arbitrary. A 100-byte buffer would otherwise parse "successfully"
        // into nonsense, which is the failure mode D-006 exists to prevent.
        if !SUPPORTED_BLOCK_SIZES.contains(&size) {
            return Err(OutOfBounds {
                offset: 0,
                needed: MIN_BLOCK_SIZE,
                len: size,
            });
        }
        let end = |back: usize| size.saturating_sub(back);

        let declared_name_len = u8_at(block, end(80))?;
        // Trust the declared length only as far as the field allows: the name
        // occupies 30 bytes, and a corrupt length must not read past them.
        let name_len = usize::from(declared_name_len).min(30);
        let name_start = end(79);
        let name = block
            .get(name_start..name_start.saturating_add(name_len))
            .unwrap_or(&[])
            .to_vec();

        Ok(Self {
            block_type: u32_at(block, 0)?,
            secondary_type: u32_at(block, end(4))?,
            hash_table_size: u32_at(block, 12)?,
            checksum_valid: checksum::normal_valid(block),
            bitmap_flag: u32_at(block, end(200))?,
            bitmap_block: u32_at(block, end(196))?,
            bitmap_pages: (0..crate::bitmap::ROOT_BITMAP_POINTERS)
                .map(|i| u32_at(block, end(196).saturating_add(i.saturating_mul(4))).unwrap_or(0))
                .collect(),
            bitmap_extension: u32_at(block, end(96))?,
            name,
            declared_name_len,
            root_altered: Datestamp::new(
                u32_at(block, end(92))?,
                u32_at(block, end(88))?,
                u32_at(block, end(84))?,
            ),
            volume_altered: Datestamp::new(
                u32_at(block, end(40))?,
                u32_at(block, end(36))?,
                u32_at(block, end(32))?,
            ),
            created: Datestamp::new(
                u32_at(block, end(28))?,
                u32_at(block, end(24))?,
                u32_at(block, end(20))?,
            ),
        })
    }

    /// Whether the identifying fields are those of a rootblock.
    ///
    /// Deliberately excludes the checksum: a rootblock with a bad checksum is a
    /// damaged rootblock, which is a different finding from "not a rootblock",
    /// and collapsing the two would lose information the health report needs.
    #[must_use]
    pub fn looks_like_a_rootblock(&self) -> bool {
        self.block_type == T_HEADER && self.secondary_type == ST_ROOT
    }

    /// Whether the bitmap-valid flag is set.
    ///
    /// Advisory only. The Linux AFFS documentation warns it can be wrong after
    /// an unclean shutdown, so a cleared flag is routine rather than sinister
    /// (AV-003), and a set flag is not a guarantee.
    #[must_use]
    pub fn bitmap_flag_valid(&self) -> bool {
        self.bitmap_flag == BITMAP_VALID
    }

    /// The volume name as a lossy string.
    ///
    /// Amiga names are ISO 8859-1, so each byte maps to one code point.
    #[must_use]
    pub fn name_lossy(&self) -> String {
        self.name.iter().map(|&b| char::from(b)).collect()
    }

    /// Whether the declared name length exceeded the 30-byte field.
    #[must_use]
    pub fn name_length_overflows(&self) -> bool {
        self.declared_name_len > 30
    }
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
    use ade_endian::put_u32;

    fn rootblock(name: &[u8], name_len: Option<u8>) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        put_u32(&mut b, 0, T_HEADER).unwrap();
        put_u32(&mut b, 12, 72).unwrap();
        put_u32(&mut b, 512 - 200, BITMAP_VALID).unwrap();
        put_u32(&mut b, 512 - 196, 881).unwrap();
        put_u32(&mut b, 512 - 92, 2760).unwrap();
        put_u32(&mut b, 512 - 40, 2760).unwrap();
        put_u32(&mut b, 512 - 28, 100).unwrap();
        b[512 - 80] = name_len.unwrap_or(name.len() as u8);
        b[512 - 79..512 - 79 + name.len()].copy_from_slice(name);
        put_u32(&mut b, 512 - 4, ST_ROOT).unwrap();
        let ck = checksum::normal(&b).unwrap();
        put_u32(&mut b, 20, ck).unwrap();
        b
    }

    #[test]
    fn parses_a_well_formed_rootblock() {
        let r = Rootblock::parse(&rootblock(b"Workbench", None)).unwrap();
        assert!(r.looks_like_a_rootblock());
        assert!(r.checksum_valid);
        assert_eq!(r.name_lossy(), "Workbench");
        assert_eq!(r.hash_table_size, 72);
        assert!(r.bitmap_flag_valid());
        assert_eq!(r.bitmap_block, 881);
        assert_eq!(r.created.ymd(), (1978, 4, 11));
    }

    #[test]
    fn a_damaged_rootblock_is_distinguishable_from_a_non_rootblock() {
        let mut b = rootblock(b"Broken", None);
        b[20] ^= 0xFF; // corrupt the checksum field
        let r = Rootblock::parse(&b).unwrap();
        assert!(r.looks_like_a_rootblock(), "still identifies as one");
        assert!(!r.checksum_valid, "...but is damaged");

        let mut b = rootblock(b"Wrong", None);
        put_u32(&mut b, 512 - 4, 0x4242_4242).unwrap();
        let r = Rootblock::parse(&b).unwrap();
        assert!(
            !r.looks_like_a_rootblock(),
            "19% of real DOS images look like this"
        );
    }

    #[test]
    fn a_corrupt_name_length_cannot_read_past_the_field() {
        // AV-004 in miniature: a declared length of 255 must not walk off the
        // 30-byte name field.
        let r = Rootblock::parse(&rootblock(b"Short", Some(255))).unwrap();
        assert_eq!(r.name.len(), 30, "clamped to the field, not the claim");
        assert!(r.name_length_overflows(), "and the discrepancy is reported");
    }

    #[test]
    fn latin1_names_survive() {
        let r = Rootblock::parse(&rootblock(b"Caf\xe9", None)).unwrap();
        assert_eq!(r.name_lossy(), "Café");
    }

    #[test]
    fn a_cleared_bitmap_flag_is_reported() {
        let mut b = rootblock(b"Unclean", None);
        put_u32(&mut b, 512 - 200, 0).unwrap();
        let r = Rootblock::parse(&b).unwrap();
        assert!(!r.bitmap_flag_valid(), "AV-003 — routine after a crash");
    }

    #[test]
    fn only_real_block_sizes_are_accepted() {
        // A buffer that is not a block cannot be parsed as one: the fields are
        // addressed from the end, so a 100-byte input would decode garbage from
        // offsets that happen to be in range.
        for n in [0usize, 100, 511, 513, 1000, 3072] {
            assert!(
                Rootblock::parse(&vec![0u8; n]).is_err(),
                "{n} bytes must be refused"
            );
        }
        for n in SUPPORTED_BLOCK_SIZES {
            assert!(
                Rootblock::parse(&vec![0u8; n]).is_ok(),
                "{n} bytes is a valid block"
            );
        }
    }

    #[test]
    fn an_all_zero_block_parses_without_claiming_to_be_a_rootblock() {
        let r = Rootblock::parse(&vec![0u8; 512]).unwrap();
        assert!(!r.looks_like_a_rootblock());
        assert_eq!(r.name_lossy(), "");
    }
}
