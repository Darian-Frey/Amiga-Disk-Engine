//! The two Amiga block checksums.
//!
//! They are **different algorithms at different offsets**, and confusing them
//! produces a silent wrong answer rather than an error — so they are separate
//! functions with separate names, never one function with a flag.
//!
//! | | offset | algorithm |
//! |---|---|---|
//! | Bootblock | 4 | add with carry, then one's complement |
//! | Everything else | 20 | sum, then two's complement negate |
//!
//! "Everything else" is rootblock, directory, file header, file extension, OFS
//! data, bitmap, dircache, and the RDB family. Bitmap *extension* blocks carry
//! no checksum at all.
//!
//! Source: Clévy's ADF FAQ §4.1 and §4.2.3, tabulated in SPEC §Checksums.

use ade_endian::u32_at;

/// Offset of the checksum field in a bootblock.
pub const BOOT_OFFSET: usize = 4;
/// Offset of the checksum field in every other block type.
pub const NORMAL_OFFSET: usize = 20;

/// Sum every big-endian long with the checksum field taken as zero, then
/// negate. Used by every block type except the bootblock.
///
/// Returns `None` if `block` is not a whole number of longs.
#[must_use]
pub fn normal(block: &[u8]) -> Option<u32> {
    fold(block, NORMAL_OFFSET, u32::wrapping_add).map(u32::wrapping_neg)
}

/// Add every big-endian long with carry, then take the one's complement.
///
/// Used only by the bootblock, whose extent is two blocks on a floppy and
/// `DosEnvVec->Bootblocks` blocks on a hard disk.
///
/// Returns `None` if `boot` is not a whole number of longs.
#[must_use]
pub fn boot(boot: &[u8]) -> Option<u32> {
    fold(boot, BOOT_OFFSET, |sum, v| {
        let (next, carried) = sum.overflowing_add(v);
        if carried { next.wrapping_add(1) } else { next }
    })
    .map(|sum| !sum)
}

/// Whether a block's stored normal checksum matches its contents.
#[must_use]
pub fn normal_valid(block: &[u8]) -> bool {
    match (normal(block), u32_at(block, NORMAL_OFFSET)) {
        (Some(computed), Ok(stored)) => computed == stored,
        _ => false,
    }
}

/// Whether a bootblock's stored checksum matches its contents.
///
/// A false result is **not** grounds for rejecting an image: only bootable
/// disks need a valid bootblock checksum, and 26% of a 4288-image survey lack
/// one (C-008). It is a health-report observation.
#[must_use]
pub fn boot_valid(bootblock: &[u8]) -> bool {
    match (boot(bootblock), u32_at(bootblock, BOOT_OFFSET)) {
        (Some(computed), Ok(stored)) => computed == stored,
        _ => false,
    }
}

fn fold(buf: &[u8], skip_at: usize, step: impl Fn(u32, u32) -> u32) -> Option<u32> {
    if buf.is_empty() || buf.len() % 4 != 0 {
        return None;
    }
    let mut sum: u32 = 0;
    let mut offset = 0;
    while offset < buf.len() {
        let value = if offset == skip_at {
            0
        } else {
            // Bounds already established by the length check above.
            u32_at(buf, offset).ok()?
        };
        sum = step(sum, value);
        offset = offset.checked_add(4)?;
    }
    Some(sum)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn the_algorithms_differ() {
        let block = [0x11u8; 512];
        assert_ne!(
            normal(&block).unwrap(),
            boot(&block).unwrap(),
            "sum-then-negate must not coincide with add-carry-then-complement"
        );
    }

    #[test]
    fn normal_ignores_the_stored_field() {
        let mut a = [0u8; 512];
        let mut b = [0u8; 512];
        a[NORMAL_OFFSET..NORMAL_OFFSET + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        b[NORMAL_OFFSET..NORMAL_OFFSET + 4].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(
            normal(&a),
            normal(&b),
            "the field itself must not be summed"
        );
    }

    #[test]
    fn boot_ignores_the_stored_field() {
        let mut a = [0u8; 1024];
        let mut b = [0u8; 1024];
        a[BOOT_OFFSET..BOOT_OFFSET + 4].copy_from_slice(&[0xFF; 4]);
        b[BOOT_OFFSET..BOOT_OFFSET + 4].copy_from_slice(&[0x01; 4]);
        assert_eq!(boot(&a), boot(&b));
    }

    #[test]
    fn a_written_checksum_validates() {
        let mut block = [0u8; 512];
        block[0] = 0x99;
        block[100] = 0x42;
        let ck = normal(&block).unwrap();
        ade_endian::put_u32(&mut block, NORMAL_OFFSET, ck).unwrap();
        assert!(normal_valid(&block));

        let mut bb = [0u8; 1024];
        bb[..4].copy_from_slice(b"DOS\x00");
        let ck = boot(&bb).unwrap();
        ade_endian::put_u32(&mut bb, BOOT_OFFSET, ck).unwrap();
        assert!(boot_valid(&bb));
        assert!(!normal_valid(&bb), "the wrong algorithm must not validate");
    }

    #[test]
    fn the_boot_carry_actually_carries() {
        // Two longs that overflow: without the carry-in the result differs.
        let mut bb = [0u8; 1024];
        bb[8..12].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        bb[12..16].copy_from_slice(&[0x00, 0x00, 0x00, 0x02]);
        let with_carry = boot(&bb).unwrap();
        let plain_sum = 0xFFFF_FFFFu32.wrapping_add(2);
        assert_ne!(with_carry, !plain_sum, "carry must be folded back in");
    }

    #[test]
    fn rejects_lengths_that_are_not_whole_longs() {
        assert_eq!(normal(&[0u8; 511]), None);
        assert_eq!(normal(&[]), None);
        assert_eq!(boot(&[0u8; 3]), None);
    }
}
