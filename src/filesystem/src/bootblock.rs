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

use std::collections::BTreeSet;

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

/// Bytes of header before the boot code: magic, checksum, rootblock pointer.
const HEADER_BYTES: usize = 12;

/// Shortest run of printable characters worth reporting as text.
///
/// Ten, measured against the corpus: shorter runs are dominated by fragments
/// of 68k opcodes that land in the printable range — `NqNqNq` is three `NOP`s
/// (0x4E71) — and longer ones start dropping real banners like `MINI-NUKE!`.
const MIN_RUN: usize = 10;

/// The percentage of characters that must look like prose rather than code.
///
/// 85, measured: with the other filters, 895 raw runs across a 665-disk sample
/// reduce to 356, of which 70% contain a space.
const MIN_TEXTY: usize = 85;

/// Longest repeating unit that marks a run as padding rather than text.
///
/// `NqNqNq…` has period 2 and `UUUU` period 1. `(W)XCOPY(W)XCOPY…` has period
/// 9 and is real, which is why this is small.
const MAX_PERIOD: usize = 4;

/// Distinct characters a run must contain to be text rather than a separator.
const MIN_DISTINCT: usize = 4;

/// The most text runs to report from one bootblock.
const MAX_RUNS: usize = 12;

/// The most characters to keep from any one run.
const MAX_RUN_LEN: usize = 200;

/// A run of printable text found in the boot code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootText {
    /// Byte offset within the bootblock.
    pub offset: usize,
    /// The text, Latin-1 decoded and trimmed.
    pub text: String,
}

impl Bootblock {
    /// Printable text embedded in the boot code.
    ///
    /// Seventy per cent of the corpus has some, and it is the most
    /// human-legible thing on a great many disks: publisher banners, the
    /// Copylock notice, cracker signatures, virus-protector menus. Extracting
    /// it is the "banner extraction" half of F-011.
    ///
    /// # This reports text, not verdicts
    ///
    /// It is tempting to match virus names here and call the result a scan.
    /// **The corpus says that gets the answer backwards.** Every disk mentioning
    /// a virus by name turned out to be carrying an *anti-virus* bootblock —
    /// "CANNOT BE INFECTED BY THE SCA-VIRUS", "PROTECT AGAINST: BYTEWARRIOR,
    /// LAMER Extermin.", and one virus killer whose menu lists nine strains. A
    /// name-matching scanner would flag precisely the protected disks. So the
    /// text is surfaced for a person to read, and identification is deferred
    /// (D-014).
    ///
    /// Library and device names are dropped: every bootblock opens
    /// `dos.library`, so reporting it is noise rather than information.
    #[must_use]
    pub fn text(boot: &[u8]) -> Vec<BootText> {
        let mut out: Vec<BootText> = Vec::new();
        let mut run_start: Option<usize> = None;

        let end = boot.len().min(FLOPPY_BYTES);
        for offset in HEADER_BYTES..=end {
            let printable = boot
                .get(offset)
                .is_some_and(|&b| (0x20..0x7F).contains(&b) || b == b'\t');
            match (printable, run_start) {
                (true, None) => run_start = Some(offset),
                (false, Some(start)) => {
                    if out.len() < MAX_RUNS {
                        if let Some(found) = run_at(boot, start, offset) {
                            out.push(found);
                        }
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
        out
    }
}

/// Turn one printable range into a reportable run, or `None` if it is noise.
fn run_at(boot: &[u8], start: usize, end: usize) -> Option<BootText> {
    if end.checked_sub(start)? < MIN_RUN {
        return None;
    }
    let bytes = boot.get(start..end)?;
    let text: String = bytes
        .iter()
        .take(MAX_RUN_LEN)
        .map(|&b| char::from(b))
        .collect();
    // Several bootblocks store BCPL-style length-prefixed strings, and a length
    // in the 32..126 range is itself printable, so it joins the run. Dropping
    // it is exact rather than a guess: the byte goes only when its value is
    // precisely the length of what follows.
    let text = match text.chars().next() {
        Some(first) if usize::from(first as u8) == text.len().saturating_sub(1) => {
            text.get(1..).unwrap_or(&text).to_owned()
        }
        _ => text,
    };
    let trimmed = text.trim();
    if trimmed.len() < MIN_RUN {
        return None;
    }

    // Every bootblock opens dos.library and most touch a resource; naming
    // what it opens says nothing about the disk.
    if [".library", ".device", ".resource"]
        .iter()
        .any(|suffix| trimmed.ends_with(suffix))
    {
        return None;
    }
    if repeats_within(trimmed, MAX_PERIOD) {
        return None;
    }
    if trimmed.chars().collect::<BTreeSet<_>>().len() < MIN_DISTINCT {
        return None;
    }
    let texty = trimmed.chars().filter(|c| is_texty(*c)).count();
    if texty.saturating_mul(100) < trimmed.len().saturating_mul(MIN_TEXTY) {
        return None;
    }

    Some(BootText {
        offset: start,
        text: trimmed.to_owned(),
    })
}

/// Whether a string is one short unit repeated — padding, not text.
fn repeats_within(text: &str, max_period: usize) -> bool {
    let bytes = text.as_bytes();
    (1..=max_period).any(|period| {
        bytes.len() >= period.saturating_mul(3)
            && bytes
                .iter()
                .enumerate()
                .all(|(i, b)| bytes.get(i.checked_rem(period).unwrap_or(0)) == Some(b))
    })
}

/// Whether a character is the kind that appears in prose rather than in code.
const fn is_texty(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            ' ' | '.'
                | ','
                | '!'
                | '?'
                | '\''
                | '"'
                | '('
                | ')'
                | '-'
                | ':'
                | ';'
                | '/'
                | '&'
                | '*'
                | '+'
                | '='
                | '@'
                | '#'
                | '%'
                | '$'
                | '_'
                | '['
                | ']'
        )
}
