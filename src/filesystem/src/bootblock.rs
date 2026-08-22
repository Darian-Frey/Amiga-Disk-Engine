//! The bootblock: blocks 0–1 of a volume.
//!
//! ADE parses it, checksums it, and **never executes it** (AV-002, D-006). The
//! boot code is data here and nowhere else.
//!
//! # It is a poor witness in both directions (C-008)
//!
//! Measured over 4288 real images: only 74% of `DOS`-prefixed images carry a
//! valid bootblock checksum, because only bootable disks need one; 19% of them
//! have no rootblock at all; and ten images with a foreign prefix mount
//! perfectly. So nothing here is a gate. [`Bootblock::parse`] succeeds on any
//! two blocks and reports what it found.

use ade_block::checksum;
use ade_endian::{OutOfBounds, u32_at};

use crate::dostype::{Dostype, DostypeError};

/// Bytes the bootblock spans on a floppy.
pub const FLOPPY_BYTES: usize = 1024;

/// Offset of the rootblock pointer.
pub const ROOTBLOCK_OFFSET: usize = 8;

/// What the first two blocks contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bootblock {
    /// The dostype, if the prefix was `DOS`.
    pub dostype: Result<Dostype, DostypeError>,
    /// The stored checksum.
    pub stored_checksum: u32,
    /// Whether the stored checksum matches the contents.
    ///
    /// **Not** a validity test for the image. See the module documentation.
    pub checksum_valid: bool,
    /// The rootblock pointer as stored.
    ///
    /// Reads 880 even on HD volumes whose rootblock is at 1760, so it must not
    /// be used to locate anything (C-007). Recorded for reporting only.
    pub stored_rootblock: u32,
    /// Whether any boot code is present — non-zero bytes past the header.
    pub has_boot_code: bool,
    /// The first four bytes, whatever they are.
    pub prefix: [u8; 4],
}

impl Bootblock {
    /// Parse the first two blocks.
    ///
    /// # Errors
    /// [`OutOfBounds`] only if fewer than [`FLOPPY_BYTES`] bytes are supplied.
    /// A foreign prefix, a bad checksum and a nonsense rootblock pointer are
    /// all reported through the returned value rather than as errors.
    pub fn parse(boot: &[u8]) -> Result<Self, OutOfBounds> {
        let oob = OutOfBounds {
            offset: 0,
            needed: FLOPPY_BYTES,
            len: boot.len(),
        };
        let bytes = boot.get(..FLOPPY_BYTES).ok_or(oob)?;
        let prefix = match bytes.get(..4) {
            Some(&[a, b, c, d]) => [a, b, c, d],
            _ => return Err(oob),
        };
        Ok(Self {
            dostype: Dostype::parse(bytes, 0),
            stored_checksum: u32_at(bytes, checksum::BOOT_OFFSET)?,
            checksum_valid: checksum::boot_valid(bytes),
            stored_rootblock: u32_at(bytes, ROOTBLOCK_OFFSET)?,
            has_boot_code: bytes.get(12..).is_some_and(|c| c.iter().any(|&b| b != 0)),
            prefix,
        })
    }

    /// Whether the prefix is `DOS`.
    ///
    /// Its absence is an observation, not a rejection: 7% of real images begin
    /// with something else and some of those mount.
    #[must_use]
    pub fn is_dos(&self) -> bool {
        matches!(&self.prefix, [b'D', b'O', b'S', _])
    }

    /// The prefix rendered for display, with unprintable bytes as dots.
    #[must_use]
    pub fn prefix_display(&self) -> String {
        self.prefix
            .iter()
            .map(|&b| {
                if (32..127).contains(&b) {
                    char::from(b)
                } else {
                    '.'
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests build their own buffers"
)]
mod tests {
    use super::*;
    use crate::dostype::FileSystem;

    fn bootblock(prefix: &[u8], with_code: bool) -> Vec<u8> {
        let mut b = vec![0u8; FLOPPY_BYTES];
        b[..prefix.len()].copy_from_slice(prefix);
        ade_endian::put_u32(&mut b, ROOTBLOCK_OFFSET, 880).unwrap();
        if with_code {
            b[12] = 0x43;
            b[13] = 0xFA;
        }
        let ck = checksum::boot(&b).unwrap();
        ade_endian::put_u32(&mut b, checksum::BOOT_OFFSET, ck).unwrap();
        b
    }

    #[test]
    fn parses_a_bootable_ofs_disk() {
        let bb = Bootblock::parse(&bootblock(b"DOS\x00", true)).unwrap();
        assert!(bb.is_dos());
        assert_eq!(bb.dostype.unwrap().filesystem(), FileSystem::Ofs);
        assert!(bb.checksum_valid);
        assert!(bb.has_boot_code);
        assert_eq!(bb.stored_rootblock, 880);
    }

    #[test]
    fn a_non_bootable_disk_has_no_code() {
        let bb = Bootblock::parse(&bootblock(b"DOS\x01", false)).unwrap();
        assert!(!bb.has_boot_code);
        assert!(
            bb.checksum_valid,
            "a data disk can still checksum correctly"
        );
    }

    #[test]
    fn a_bad_checksum_is_reported_not_refused() {
        let mut b = bootblock(b"DOS\x00", true);
        b[4] ^= 0xFF;
        let bb = Bootblock::parse(&b).unwrap();
        assert!(!bb.checksum_valid);
        assert!(bb.dostype.is_ok(), "the dostype is still readable");
    }

    #[test]
    fn a_foreign_prefix_parses_and_is_reported() {
        // 7% of real images look like this; ten of them mount.
        let bb = Bootblock::parse(&bootblock(b"ATN!", true)).unwrap();
        assert!(!bb.is_dos());
        assert_eq!(bb.prefix_display(), "ATN!");
        assert!(bb.dostype.is_err());
        assert!(bb.checksum_valid, "a custom loader can still checksum");
    }

    #[test]
    fn unprintable_prefixes_render_safely() {
        let bb = Bootblock::parse(&bootblock(&[0x00, 0x01, 0xFF, 0x7F], false)).unwrap();
        assert_eq!(bb.prefix_display(), "....");
    }

    #[test]
    fn short_input_errors_rather_than_panicking() {
        for n in [0usize, 4, 512, 1023] {
            assert!(Bootblock::parse(&vec![0u8; n]).is_err(), "{n} bytes");
        }
        assert!(Bootblock::parse(&vec![0u8; 1024]).is_ok());
    }
}
