//! Content sniffing — deciding what an image is, and recording why.
//!
//! Dispatch is by content, never by file extension (F-003). But content is not
//! decisive either, so this module produces a [`Detection`] carrying both a
//! [`Kind`] and the [`Evidence`] behind it (C-008).

use core::fmt;

/// Bytes in a block for every format ADE sniffs. Hard-disk block sizes are read
/// from the RDB, which is found by scanning in these units regardless.
const BLOCK: usize = 512;

/// Canonical double-density ADF: 80 × 2 × 11 × 512.
pub const DD_BYTES: u64 = 901_120;
/// Canonical high-density ADF: 80 × 2 × 22 × 512.
pub const HD_BYTES: u64 = 1_802_240;
/// Bytes in one DD cylinder — 2 heads × 11 sectors × 512.
pub const DD_CYLINDER: u64 = 11_264;

/// What an image appears to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A raw floppy image, with the cylinder count derived from its size.
    Adf {
        /// Cylinders the size implies. 80 is canonical; 81–83 occur.
        cylinders: u32,
        /// Sectors per track — 11 for DD, 22 for HD.
        sectors: u32,
    },
    /// Extended ADF carrying raw MFM tracks. Phase 4.
    ExtendedAdf {
        /// Track count from the header.
        tracks: u16,
    },
    /// A hard-disk image with a Rigid Disk Block. Phase 2.
    RigidDisk,
    /// A raw volume that is not a recognised floppy size — most likely an
    /// unpartitioned hardfile. Phase 2.
    Hardfile,
    /// Gzip-wrapped ADF or HDF. Phase 3.
    Gzip,
    /// DiskMasher. Phase 3.
    Dms,
    /// SuperCard Pro flux. Phase 4.
    Scp,
    /// IPF flux, read-only and licence-gated. Phase 4.
    Ipf,
    /// Nothing recognisable.
    Unknown,
}

impl Kind {
    /// A stable identifier for this container kind.
    ///
    /// Distinct from [`Display`](fmt::Display), which produces a human
    /// sentence — `ADF (DD, 80 cylinders)` — carrying a geometry that varies
    /// between images of the same kind. That is the right thing to read and
    /// the wrong thing to match on, and F-015 draws exactly this line for
    /// faults already: the message may be reworded, the code may not.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Adf { .. } => "adf",
            Self::ExtendedAdf { .. } => "extended-adf",
            Self::RigidDisk => "rdb",
            Self::Hardfile => "hardfile",
            Self::Gzip => "gzip",
            Self::Dms => "dms",
            Self::Scp => "scp",
            Self::Ipf => "ipf",
            Self::Unknown => "unknown",
        }
    }

    /// Whether block 0 of this container is an AmigaDOS bootblock.
    ///
    /// Only a raw block image has one. An extended ADF opens with a `UAE-1ADF`
    /// header, a device opens with an `RDSK` block, and a compressed or flux
    /// container is not blocks at all — parsing any of them as a bootblock
    /// yields a confident report about a checksum that was never a checksum.
    ///
    /// `Unknown` counts, because an unrecognised container of the right length
    /// may well be a raw volume whose bootblock is simply not `DOS` — 7% of
    /// real images are exactly that (SPEC §Corpus observations).
    #[must_use]
    pub const fn has_bootblock(self) -> bool {
        matches!(self, Self::Adf { .. } | Self::Hardfile | Self::Unknown)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adf { cylinders, sectors } => {
                let density = if *sectors >= 22 { "HD" } else { "DD" };
                write!(f, "ADF ({density}, {cylinders} cylinders)")
            }
            Self::ExtendedAdf { tracks } => write!(f, "extended ADF ({tracks} tracks)"),
            Self::RigidDisk => f.write_str("hard-disk image (RDB)"),
            Self::Hardfile => f.write_str("hardfile (raw volume)"),
            Self::Gzip => f.write_str("gzip-wrapped image (ADZ/HDZ)"),
            Self::Dms => f.write_str("DMS"),
            Self::Scp => f.write_str("SCP flux"),
            Self::Ipf => f.write_str("IPF flux"),
            Self::Unknown => f.write_str("unrecognised"),
        }
    }
}

/// A single observation that informed the conclusion.
///
/// Recorded whether it supports or undermines the verdict — an image that is
/// *probably* an ADF despite a missing `DOS` prefix should say so, because that
/// is exactly the case a user needs to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A known signature matched at an offset.
    Magic {
        /// What matched.
        what: &'static str,
        /// Where it matched.
        offset: usize,
    },
    /// The size corresponds to a whole number of cylinders.
    SizeFitsGeometry {
        /// Bytes in the image.
        bytes: u64,
        /// Cylinders implied.
        cylinders: u32,
    },
    /// The size is close to a known geometry but not exact.
    SizeAnomaly {
        /// Bytes in the image.
        bytes: u64,
        /// Bytes over (positive) or under (negative) the nearest whole cylinder.
        delta: i64,
    },
    /// An AmigaDOS-family bootblock prefix was present.
    DosPrefix {
        /// The flags byte following `DOS`.
        flags: u8,
    },
    /// A non-`DOS` prefix. Not a rejection: 7% of real images have one.
    ForeignPrefix {
        /// The first four bytes, for reporting.
        bytes: [u8; 4],
    },
    /// The image is too short to hold anything meaningful.
    TooShort {
        /// Bytes available.
        bytes: u64,
    },
}

impl fmt::Display for Evidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Magic { what, offset } => write!(f, "{what} signature at offset {offset}"),
            Self::SizeFitsGeometry { bytes, cylinders } => {
                write!(f, "{bytes} bytes = exactly {cylinders} cylinders")
            }
            Self::SizeAnomaly { bytes, delta } => {
                write!(f, "{bytes} bytes, {delta:+} from a whole cylinder")
            }
            Self::DosPrefix { flags } => write!(f, "bootblock begins DOS\\{flags}"),
            Self::ForeignPrefix { bytes } => {
                let printable: String = bytes
                    .iter()
                    .map(|&b| {
                        if (32..127).contains(&b) {
                            char::from(b)
                        } else {
                            '.'
                        }
                    })
                    .collect();
                write!(
                    f,
                    "bootblock begins {printable:?} — not DOS, which is not disqualifying"
                )
            }
            Self::TooShort { bytes } => write!(f, "only {bytes} bytes"),
        }
    }
}

/// What sniffing concluded, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    /// The conclusion.
    pub kind: Kind,
    /// Observations behind it, in the order they were made.
    pub evidence: Vec<Evidence>,
}

/// Identify an image from its leading bytes and total size.
///
/// `head` should be at least the first two blocks; more is not needed. `size`
/// is the total length, which the caller knows without reading the whole file.
///
/// Ordered most-specific first: unambiguous signatures, then the RDB scan, then
/// size-and-prefix reasoning for the formats that have no signature at all.
#[must_use]
pub fn sniff(head: &[u8], size: u64) -> Detection {
    let mut evidence = Vec::new();

    if head.len() < 4 || size < 4 {
        evidence.push(Evidence::TooShort { bytes: size });
        return Detection {
            kind: Kind::Unknown,
            evidence,
        };
    }

    // 1. Unambiguous signatures.
    for (magic, kind, what) in [
        (
            &b"UAE-1ADF"[..],
            Kind::ExtendedAdf { tracks: 0 },
            "UAE-1ADF",
        ),
        (&b"\x1f\x8b"[..], Kind::Gzip, "gzip"),
        (&b"DMS!"[..], Kind::Dms, "DMS!"),
        (&b"CAPS"[..], Kind::Ipf, "CAPS"),
    ] {
        if head.starts_with(magic) {
            evidence.push(Evidence::Magic { what, offset: 0 });
            let kind = if what == "UAE-1ADF" {
                // Track count is a big-endian u16 at offset 10.
                let tracks = match (head.get(10), head.get(11)) {
                    (Some(&hi), Some(&lo)) => u16::from(hi) << 8 | u16::from(lo),
                    _ => 0,
                };
                Kind::ExtendedAdf { tracks }
            } else {
                kind
            };
            return Detection { kind, evidence };
        }
    }
    // SCP is three bytes followed by a version, so it needs a narrower test
    // than `starts_with` on a four-byte literal.
    if head.starts_with(b"SCP") {
        evidence.push(Evidence::Magic {
            what: "SCP",
            offset: 0,
        });
        return Detection {
            kind: Kind::Scp,
            evidence,
        };
    }

    // 2. An RDB lives within the first 16 blocks, not necessarily at zero.
    for block in 0..16usize {
        let at = block.saturating_mul(BLOCK);
        if head.get(at..at.saturating_add(4)) == Some(b"RDSK") {
            evidence.push(Evidence::Magic {
                what: "RDSK",
                offset: at,
            });
            return Detection {
                kind: Kind::RigidDisk,
                evidence,
            };
        }
    }

    // 3. No signature: reason from prefix and size, and commit to neither alone.
    match head.get(..4) {
        Some([b'D', b'O', b'S', flags]) => evidence.push(Evidence::DosPrefix { flags: *flags }),
        Some(&[a, b, c, d]) => evidence.push(Evidence::ForeignPrefix {
            bytes: [a, b, c, d],
        }),
        _ => {}
    }

    let kind = if let Some((cylinders, sectors)) = floppy_geometry(size) {
        evidence.push(Evidence::SizeFitsGeometry {
            bytes: size,
            cylinders,
        });
        Kind::Adf { cylinders, sectors }
    } else {
        let nearest = size.div_euclid(DD_CYLINDER).saturating_mul(DD_CYLINDER);
        let delta = i64::try_from(size.saturating_sub(nearest)).unwrap_or(i64::MAX);
        evidence.push(Evidence::SizeAnomaly { bytes: size, delta });
        // A hardfile is a raw volume — bootblock, rootblock, bitmap — so its
        // first three bytes are `DOS` (SPEC §Hardfiles). Without them this is
        // some other file that merely fails to be a floppy, and calling it a
        // hardfile was BUG-010: every one of the 7 corpus images detected as a
        // hardfile begins `DOS`, while an Amiga executable dragged out of a
        // disk and dropped back on the window was opened as a 5,732-byte hard
        // disk and reported as damaged.
        //
        // `Unknown` rather than a refusal, because C-008 keeps the bootblock
        // and the filesystem separate: a file of the right shape with an
        // unrecognised bootblock is still worth opening and saying so about.
        if matches!(head.get(..3), Some(b"DOS")) {
            Kind::Hardfile
        } else {
            Kind::Unknown
        }
    };
    Detection { kind, evidence }
}

/// Cylinders and sectors-per-track, if `size` is an exact floppy geometry.
///
/// 80 cylinders is the norm, not the limit: 81–83 occur in the wild, so the
/// test is divisibility with a plausible cylinder count rather than equality
/// against 901,120.
#[must_use]
pub fn floppy_geometry(size: u64) -> Option<(u32, u32)> {
    for sectors in [11u64, 22] {
        let cylinder = sectors.saturating_mul(2).saturating_mul(BLOCK as u64);
        // checked_rem / checked_div rather than the operators: a zero divisor
        // would panic, and D-006 admits no panicking path even where the
        // divisor is a literal today.
        if cylinder != 0 && size.checked_rem(cylinder) == Some(0) {
            let Some(cylinders) = size.checked_div(cylinder) else {
                continue;
            };
            // Below 40 cylinders it is a fragment, not a disk; above 84 the
            // geometry is more likely a hardfile that happens to divide.
            if (40..=84).contains(&cylinders) {
                return Some((u32::try_from(cylinders).ok()?, u32::try_from(sectors).ok()?));
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests build their own buffers")]
mod tests {
    #[test]
    fn a_file_that_is_not_a_disk_image_is_not_called_a_hardfile() {
        // BUG-010. The fallback used to call anything that was not an exact
        // floppy geometry a hardfile, so an Amiga executable dragged out of a
        // disk and dropped back on the window opened as a 5,732-byte hard disk
        // and was reported as damaged.
        //
        // A hardfile is a raw volume — bootblock, rootblock, bitmap — so its
        // first three bytes are `DOS` (SPEC §Hardfiles).
        let executable = {
            let mut bytes = vec![0u8; 5_732];
            // The Amiga hunk magic, which is what one of those files began with.
            bytes[..4].copy_from_slice(&[0x00, 0x00, 0x03, 0xF3]);
            bytes
        };
        assert_eq!(
            sniff(&executable, executable.len() as u64).kind,
            Kind::Unknown
        );

        // Size alone does not make one either: a large file of anything.
        let big = vec![0x42u8; 4_000_000];
        assert_eq!(sniff(&big, big.len() as u64).kind, Kind::Unknown);
    }

    #[test]
    fn a_raw_volume_is_still_a_hardfile() {
        // Every one of the 7 corpus images detected as a hardfile begins
        // `DOS`, and requiring it left the whole corpus classified identically.
        for prefix in [b"DOS\x00", b"DOS\x01", b"DOS\x03"] {
            let mut bytes = vec![0u8; 4_000_000];
            bytes[..4].copy_from_slice(prefix);
            assert_eq!(
                sniff(&bytes, bytes.len() as u64).kind,
                Kind::Hardfile,
                "{prefix:?}"
            );
        }
    }

    use super::*;

    fn head_of(prefix: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; BLOCK * 2];
        v[..prefix.len()].copy_from_slice(prefix);
        v
    }

    #[test]
    fn recognises_unambiguous_signatures() {
        assert_eq!(sniff(&head_of(b"\x1f\x8b\x08"), 1000).kind, Kind::Gzip);
        assert_eq!(sniff(&head_of(b"DMS!"), 1000).kind, Kind::Dms);
        assert_eq!(sniff(&head_of(b"SCP\x18"), 1000).kind, Kind::Scp);
        assert_eq!(sniff(&head_of(b"CAPS"), 1000).kind, Kind::Ipf);
    }

    #[test]
    fn reads_the_extended_adf_track_count() {
        let mut h = head_of(b"UAE-1ADF");
        h[10] = 0x00;
        h[11] = 0xA6;
        assert_eq!(sniff(&h, 1_413_190).kind, Kind::ExtendedAdf { tracks: 166 });
    }

    #[test]
    fn finds_an_rdb_anywhere_in_the_first_sixteen_blocks() {
        for block in [0usize, 1, 2, 15] {
            let mut h = vec![0u8; BLOCK * 16];
            h[block * BLOCK..block * BLOCK + 4].copy_from_slice(b"RDSK");
            let d = sniff(&h, 40 * 1024 * 1024);
            assert_eq!(d.kind, Kind::RigidDisk, "RDSK at block {block}");
            assert!(d.evidence.contains(&Evidence::Magic {
                what: "RDSK",
                offset: block * BLOCK
            }));
        }
    }

    #[test]
    fn canonical_floppies_identify_by_size() {
        assert_eq!(
            sniff(&head_of(b"DOS\x00"), DD_BYTES).kind,
            Kind::Adf {
                cylinders: 80,
                sectors: 11
            }
        );
        assert_eq!(
            sniff(&head_of(b"DOS\x01"), HD_BYTES).kind,
            Kind::Adf {
                cylinders: 80,
                sectors: 22
            }
        );
    }

    #[test]
    fn extra_cylinder_images_are_still_adfs() {
        // 81-83 cylinders occur in the wild; equality against 901120 would
        // misclassify them (SPEC §Corpus observations).
        for (cyl, size) in [(81u32, 912_384u64), (82, 923_648), (83, 934_912)] {
            assert_eq!(
                sniff(&head_of(b"DOS\x00"), size).kind,
                Kind::Adf {
                    cylinders: cyl,
                    sectors: 11
                },
                "{cyl} cylinders"
            );
        }
    }

    #[test]
    fn a_missing_dos_prefix_does_not_disqualify_an_adf() {
        // 7% of real images begin with something else, and ten of them mount.
        let d = sniff(&head_of(b"ATN!"), DD_BYTES);
        assert_eq!(
            d.kind,
            Kind::Adf {
                cylinders: 80,
                sectors: 11
            }
        );
        assert!(
            d.evidence
                .iter()
                .any(|e| matches!(e, Evidence::ForeignPrefix { .. })),
            "the foreign prefix must still be reported"
        );
    }

    #[test]
    fn size_anomalies_are_reported_not_swallowed() {
        let d = sniff(&head_of(b"DOS\x00"), DD_BYTES + 1);
        assert!(matches!(
            d.evidence.last(),
            Some(Evidence::SizeAnomaly { delta: 1, .. })
        ));
    }

    #[test]
    fn evidence_is_recorded_even_when_the_verdict_is_confident() {
        let d = sniff(&head_of(b"DOS\x03"), DD_BYTES);
        assert!(d.evidence.contains(&Evidence::DosPrefix { flags: 3 }));
        assert!(d.evidence.contains(&Evidence::SizeFitsGeometry {
            bytes: DD_BYTES,
            cylinders: 80
        }));
    }

    #[test]
    fn tiny_inputs_do_not_panic() {
        for n in 0..8usize {
            let _ = sniff(&vec![0u8; n], n as u64);
        }
    }

    #[test]
    fn a_fragment_is_not_a_floppy() {
        // One survey image is 90,112 bytes: eight cylinders of an eighty-
        // cylinder disk. Divisible, but not plausibly a disk.
        assert_eq!(floppy_geometry(90_112), None);
        assert_eq!(floppy_geometry(0), None);
    }
}
