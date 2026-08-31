//! Creating a blank AmigaDOS volume (F-019).
//!
//! # The first write path, and why it is allowed to be
//!
//! D-004 defers writing until "every write path ships only after its read path
//! is proven on fixtures". That condition is met for OFS and FFS — 4,652
//! corpus images read, 99.36% byte-identical agreement with ADFlib over 3,900
//! extracted files — and D-004's own consequence line puts write "from Phase
//! 4/5", which is where the project is.
//!
//! It is also the safest write there is. Formatting produces a **new** image;
//! it does not touch an existing one, which is the irreversible damage D-004
//! is actually about. Adding a file to a disk somebody already owns is a
//! different feature with a different risk, and is not this.
//!
//! # Written from SPEC, deliberately not from the fixture generator
//!
//! `ade-fixtures` already builds volumes, and reusing it here would have been
//! quicker. It would also have destroyed what makes it useful: D-010 keeps the
//! generator dependent on nothing so that a misreading in a layer crate cannot
//! cancel out against it. If this module were derived from the generator, the
//! two would share every mistake.
//!
//! So this is written from SPEC §Rootblock, §Bitmap and §Bootblock — the
//! primary-source documents — and validated three ways: ADE reads back what it
//! wrote, ADFlib reads it (D-002's oracle), and the fixture generator's
//! equivalent volume must agree with it structurally. Three independent
//! statements of one format.
//!
//! # What a blank disk is not
//!
//! It is not bootable. AmigaDOS's own `format` leaves the boot code zeroed
//! unless `install` is run afterwards, and ADE will not write boot code it
//! would then refuse to execute or interpret (AV-002). The dostype and the
//! checksum are written; the 1,012 bytes after them are zeros.

use ade_block::{Geometry, checksum};
use ade_endian::put_u32;

use crate::dostype::Dostype;

/// The volume name a disk gets when the caller does not choose one.
pub const DEFAULT_NAME: &str = "Empty";

/// The longest volume name AmigaDOS stores — 30 bytes, per SPEC §Rootblock.
pub const MAX_NAME: usize = 30;

/// Why a volume could not be formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The name is longer than the 30 bytes the rootblock holds.
    NameTooLong {
        /// How long it was.
        len: usize,
    },
    /// The name holds a byte AmigaDOS does not allow in one.
    NameInvalid {
        /// The offending byte.
        byte: u8,
    },
    /// The geometry describes a volume too small to hold a filesystem.
    TooSmall {
        /// Blocks the geometry offers.
        blocks: u64,
    },
    /// The volume needs more bitmap blocks than a rootblock can name.
    ///
    /// Past 25 pointers the rest live in a `bm_ext` chain — a hard disk above
    /// roughly 50 MB. Refused rather than written wrongly: a volume whose
    /// bitmap is only half described reports free blocks that are not, which
    /// is how a write path destroys data.
    TooLarge {
        /// Blocks the geometry offers.
        blocks: u64,
    },
}

impl core::fmt::Display for FormatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NameTooLong { len } => {
                write!(f, "volume name is {len} bytes; AmigaDOS holds {MAX_NAME}")
            }
            Self::NameInvalid { byte } => {
                write!(
                    f,
                    "volume name contains a byte AmigaDOS forbids: {byte:#04x}"
                )
            }
            Self::TooSmall { blocks } => {
                write!(f, "a volume needs more than {blocks} blocks")
            }
            Self::TooLarge { blocks } => write!(
                f,
                "{blocks} blocks needs a bitmap extension chain, which ADE does not write"
            ),
        }
    }
}

impl core::error::Error for FormatError {}

/// When a volume was created, as AmigaDOS counts time.
///
/// Taken from the caller rather than the clock, so formatting is
/// deterministic: two runs with the same arguments produce the same bytes,
/// which is what makes the result testable at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    /// Days since 1978-01-01.
    pub days: u32,
    /// Minutes past midnight, below 1440.
    pub mins: u32,
    /// Ticks at 1/50 s past the minute, below 3000.
    pub ticks: u32,
}

/// Build a blank volume of `geometry`, returning the whole image.
///
/// # Errors
/// [`FormatError`] if the name will not fit or is not a legal AmigaDOS volume
/// name, or the geometry is too small to hold a rootblock and a bitmap.
pub fn blank(
    geometry: Geometry,
    dostype: Dostype,
    name: &str,
    created: Stamp,
) -> Result<Vec<u8>, FormatError> {
    check_name(name)?;
    let block_size = geometry.block_size() as usize;
    let total = geometry.total_blocks();
    let root = geometry.root_block().0;

    // How many bitmap blocks the volume needs. One 512-byte block carries a
    // checksum and then 127 longs, so it maps `(512/4 - 1) * 32 = 4064`
    // blocks — a floppy fits in one, and an 8 MB hardfile needs five. Writing
    // exactly one regardless is BUG-006, which panicked above about 2 MB.
    let mapped = total.saturating_sub(u64::from(geometry.reserved()));
    let per_block = bits_per_bitmap_block(geometry.block_size());
    let pages = mapped.div_ceil(per_block.max(1));

    // 25 pointers fit in the rootblock; past that they need a `bm_ext` chain,
    // which is a hard disk above roughly 50 MB. Refused rather than written
    // wrongly: a volume whose bitmap is half-described reports free blocks
    // that are not, which is how a write path destroys data.
    if pages > BM_PAGES {
        return Err(FormatError::TooLarge { blocks: total });
    }

    // The bitmap goes immediately after the rootblock, which is where every
    // real disk puts it and where a reader looking for it will find it first.
    let bitmap = root.saturating_add(1);
    if total <= bitmap.saturating_add(pages) {
        return Err(FormatError::TooSmall { blocks: total });
    }
    let blocks: Vec<u64> = (0..pages).map(|i| bitmap.saturating_add(i)).collect();

    let mut image = vec![0u8; usize::try_from(geometry.total_bytes()).unwrap_or(0)];
    write_bootblock(&mut image, dostype, root);
    write_rootblock(&mut image, geometry, name, created, root, &blocks);
    write_bitmap(&mut image, geometry, root, &blocks);
    let _ = block_size;
    Ok(image)
}

/// Bitmap pointers the rootblock itself can hold. Past this a `bm_ext` chain
/// is needed, which is a hard disk above roughly 50 MB.
const BM_PAGES: u64 = 25;

/// How many blocks one bitmap block can map.
///
/// A bitmap block spends its first long on a checksum and maps 32 blocks per
/// long after that.
const fn bits_per_bitmap_block(block_size: u32) -> u64 {
    ((block_size as u64 / 4).saturating_sub(1)).saturating_mul(32)
}

/// A volume name AmigaDOS would accept.
///
/// The forbidden bytes are the two path separators and the control range:
/// AmigaDOS parses `:` as the device terminator and `/` as the directory
/// separator, so a name holding either cannot be typed at a prompt.
fn check_name(name: &str) -> Result<(), FormatError> {
    let bytes = name.as_bytes();
    if bytes.len() > MAX_NAME {
        return Err(FormatError::NameTooLong { len: bytes.len() });
    }
    for &byte in bytes {
        if byte == b':' || byte == b'/' || byte < 0x20 {
            return Err(FormatError::NameInvalid { byte });
        }
    }
    Ok(())
}

/// Blocks 0–1: the dostype and its checksum, and nothing else.
fn write_bootblock(image: &mut [u8], dostype: Dostype, root: u64) {
    let raw = dostype.raw();
    let _ = put_u32(image, 0, raw);
    // The stored rootblock pointer. C-007: readers must not trust it — ADE
    // computes the location instead — but a disk that omits it is unusual, and
    // writing the truth costs nothing.
    let _ = put_u32(image, 8, u32::try_from(root).unwrap_or(880));
    // The checksum covers both boot blocks. Written last, over the zeros that
    // stand in for boot code: this disk is not bootable and does not pretend
    // to be (AV-002).
    if let Some(sum) = image.get(..1024).and_then(checksum::boot) {
        let _ = put_u32(image, 4, sum);
    }
}

/// The rootblock, per SPEC §Rootblock.
fn write_rootblock(
    image: &mut [u8],
    geometry: Geometry,
    name: &str,
    created: Stamp,
    root: u64,
    bitmap: &[u64],
) {
    let bsize = geometry.block_size() as usize;
    let at = usize::try_from(root).unwrap_or(0).saturating_mul(bsize);
    let Some(block) = image.get_mut(at..at.saturating_add(bsize)) else {
        return;
    };

    let _ = put_u32(block, 0x00, 2); // T_HEADER
    // ht_size is BSIZE/4 - 56, which is 72 for a 512-byte block.
    let ht_size = (geometry.block_size() / 4).saturating_sub(56);
    let _ = put_u32(block, 0x0c, ht_size);
    // The hash table is left as zeros: an empty directory has no entries, and
    // zero is "no entry" rather than a block number.

    let end = bsize;
    let _ = put_u32(block, end.saturating_sub(200), u32::MAX); // bm_flag = -1
    // Every bitmap block, not just the first: `bm_pages` is an array of 25,
    // and a volume that names one of its five bitmap blocks describes an
    // eighth of its own free space. `bm_ext` stays zero — a volume needing it
    // was refused before this was called.
    for (index, block_number) in bitmap.iter().enumerate() {
        let offset = end
            .saturating_sub(196)
            .saturating_add(index.saturating_mul(4));
        let _ = put_u32(block, offset, u32::try_from(*block_number).unwrap_or(0));
    }

    // All three datestamps are the format date on a fresh disk: nothing has
    // changed since it was made, because nothing has happened to it.
    for offset in [92usize, 40, 28] {
        let base = end.saturating_sub(offset);
        let _ = put_u32(block, base, created.days);
        let _ = put_u32(block, base.saturating_add(4), created.mins);
        let _ = put_u32(block, base.saturating_add(8), created.ticks);
    }

    let bytes = name.as_bytes();
    // A byte at a time: `ade-endian` writes multi-byte integers, and a name
    // is neither an integer nor byte-order-dependent.
    if let Some(slot) = block.get_mut(end.saturating_sub(80)) {
        *slot = u8::try_from(bytes.len()).unwrap_or(0);
    }
    for (i, byte) in bytes.iter().take(MAX_NAME).enumerate() {
        if let Some(slot) = block.get_mut(end.saturating_sub(79).saturating_add(i)) {
            *slot = *byte;
        }
    }

    let _ = put_u32(block, end.saturating_sub(4), 1); // ST_ROOT
    if let Some(sum) = checksum::normal_at(block, 0x14) {
        let _ = put_u32(block, 0x14, sum);
    }
}

/// The bitmap, per SPEC §Bitmap.
///
/// **A set bit means the block is free.** That is the opposite of the usual
/// convention and the single easiest thing to get backwards here: inverted, a
/// fresh disk would report itself completely full and every write to it would
/// fail for want of space.
fn write_bitmap(image: &mut [u8], geometry: Geometry, root: u64, bitmap: &[u64]) {
    let bsize = geometry.block_size() as usize;
    let reserved = u64::from(geometry.reserved());
    let mapped = geometry.total_blocks().saturating_sub(reserved);
    let per_block = bits_per_bitmap_block(geometry.block_size());

    for (page, block_number) in bitmap.iter().enumerate() {
        let at = usize::try_from(*block_number)
            .unwrap_or(0)
            .saturating_mul(bsize);
        let Some(block) = image.get_mut(at..at.saturating_add(bsize)) else {
            return;
        };

        // The map begins at the first block after the reserved area, so bit 0
        // of page 0 is block `reserved`, not block 0 — and bit 0 of page 1 is
        // 4064 blocks after that.
        let first = (page as u64).saturating_mul(per_block);
        for bit in 0..per_block {
            let index = first.saturating_add(bit);
            if index >= mapped {
                break;
            }
            let block_index = index.saturating_add(reserved);
            // Everything is free except the blocks this filesystem occupies:
            // the rootblock, and every one of its bitmap blocks.
            if block_index == root || bitmap.contains(&block_index) {
                continue;
            }
            let long = usize::try_from(bit / 32).unwrap_or(0);
            let offset = 4usize.saturating_add(long.saturating_mul(4));
            let Ok(current) = ade_endian::u32_at(block, offset) else {
                break;
            };
            let _ = put_u32(block, offset, current | (1u32 << (bit % 32)));
        }

        // The bitmap block is the one exception to block layout: its checksum
        // sits at offset 0, where every other block keeps it at 20 (BUG-004).
        if let Some(sum) = checksum::normal_at(block, 0x00) {
            let _ = put_u32(block, 0x00, sum);
        }
    }
}
