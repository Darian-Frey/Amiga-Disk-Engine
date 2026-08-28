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
/// Offset of the checksum field in a **bitmap** block, which is the exception.
///
/// Bitmap blocks put their checksum at 0 and the map from 4 (ADF FAQ §4.3),
/// where every other block type reserves 0 for its primary type and puts the
/// checksum at 20.
pub const BITMAP_OFFSET: usize = 0;

/// Sum every big-endian long with the checksum field taken as zero, then
/// negate. Used by every block type except the bootblock.
///
/// Returns `None` if `block` is not a whole number of longs.
#[must_use]
pub fn normal(block: &[u8]) -> Option<u32> {
    normal_at(block, NORMAL_OFFSET)
}

/// The normal checksum for a block whose field is not at the usual offset.
///
/// Bitmap blocks are the only such case; see [`BITMAP_OFFSET`].
#[must_use]
pub fn normal_at(block: &[u8], field: usize) -> Option<u32> {
    fold(block, field, u32::wrapping_add).map(u32::wrapping_neg)
}

/// Whether a block satisfies the checksum invariant, wherever its field sits.
///
/// The normal checksum is defined so that **the whole block sums to zero**:
/// the stored value is the negation of everything else. Validation is therefore
/// insensitive to *which* long holds it — checking against offset 20 succeeds
/// on a bitmap block whose checksum is at 0, which is why the mismatch went
/// unnoticed on 317 real disks.
///
/// The offset still matters for *writing*, and for reading the map: a bitmap
/// block's offset 20 is map data covering blocks 130–161, and treating it as a
/// checksum field loses them.
#[must_use]
pub fn sums_to_zero(block: &[u8]) -> bool {
    fold(block, usize::MAX, u32::wrapping_add) == Some(0)
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

/// The CRC32 table for the reflected polynomial `0xEDB88320`, built at compile
/// time.
///
/// A byte at a time rather than a bit at a time. The bitwise form is eight
/// times the work, which is invisible in a release build and very visible in a
/// debug one — where the tests run.
#[allow(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "const-evaluated: an out-of-range index or a truncating cast here \
              is a compile error, not a runtime one, and `i` is bounded by 256"
)]
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC32 with the reflected polynomial `0xEDB88320`.
///
/// **Not an Amiga checksum.** It lives here because two unrelated parts of ADE
/// need the same algorithm and `ade-block` is the lowest layer both can reach:
/// gzip uses it to verify a decompressed image (`ade-container::inflate`), and
/// TOSEC datfiles use it to identify one (`ade-catalogue`). Implementing it
/// twice is how two copies of a checksum silently diverge.
///
/// It is a content hash, not a cryptographic one, and 32 bits is not many, so
/// a caller identifying content by it must report every match rather than
/// assume one.
///
/// *(The TOSEC Amiga set was cited here as holding 71 such collisions.
/// Re-measured 2026-08-29: it holds none. The 77 groups sharing a CRC32 also
/// share their SHA-1 and MD5 — duplicate content, not collisions. The advice
/// stands; the evidence given for it was wrong.)*
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        // Taking the low byte is the algorithm, not a lossy conversion.
        let index = usize::try_from((crc ^ u32::from(byte)) & 0xFF).unwrap_or(0);
        let entry = CRC32_TABLE.get(index).copied().unwrap_or(0);
        crc = entry ^ (crc >> 8);
    }
    !crc
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
