//! The format-conversion matrix (F-016).
//!
//! Converting between Amiga image formats is today done by a scatter of
//! single-purpose tools, and the thing they rarely tell you is **what the
//! conversion cost**. Writing an extended ADF out as a plain ADF discards the
//! copy protection that was the reason to capture it; nothing warns you. That
//! silence is what this module exists to break.
//!
//! So the matrix is the feature, and the conversions are what it authorises.
//! Every pair of formats has an answer — lossless, lossy and why, not
//! implemented and why, or refused and why — and a conversion that would lose
//! something says so before it runs.
//!
//! # What can actually be converted today
//!
//! Two directions, and both are proven byte-identically, which is what
//! D-004 asks for before a write path ships.
//!
//! **Decompression** — ADZ to ADF, HDZ to HDF. gunzip round-trips real corpus
//! images byte for byte.
//!
//! **Encoding to raw MFM** — a sector image to an extended ADF. Every track is
//! MFM-encoded, and reading the result back reassembles the original exactly:
//! 1760 of 1760 sectors, byte-identical, mounting under its own name.
//!
//! Nothing else qualifies yet: DMS has no reader (D-009), flux cannot be
//! reconstructed from a sector image at all, and IPF may never be written
//! (C-003).

use ade_container::Kind;

/// What a conversion between two formats would cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversion {
    /// The output holds everything the input did.
    Lossless,
    /// The conversion is possible but discards something. The caller is told
    /// what, and should say so before doing it.
    Lossy {
        /// What would not survive.
        lost: &'static str,
    },
    /// ADE cannot do this yet, and why.
    NotImplemented {
        /// The reason, naming the register entry that tracks it.
        why: &'static str,
    },
    /// ADE will not do this, and why. Distinct from not-implemented: this one
    /// is a decision, not a gap.
    Refused {
        /// The reason.
        why: &'static str,
    },
}

impl Conversion {
    /// Whether a conversion may proceed at all.
    #[must_use]
    pub const fn is_possible(&self) -> bool {
        matches!(self, Self::Lossless | Self::Lossy { .. })
    }

    /// A short label for reporting.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::Lossy { .. } => "lossy",
            Self::NotImplemented { .. } => "not implemented",
            Self::Refused { .. } => "refused",
        }
    }
}

impl core::fmt::Display for Conversion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Lossless => f.write_str("lossless"),
            Self::Lossy { lost } => write!(f, "lossy — {lost}"),
            Self::NotImplemented { why } => write!(f, "not implemented — {why}"),
            Self::Refused { why } => write!(f, "refused — {why}"),
        }
    }
}

/// What converting `from` into `to` would cost.
///
/// The table is deliberately explicit rather than derived from capability
/// flags: each answer carries its own reason, and a reason is what the caller
/// actually needs.
#[must_use]
pub fn conversion(from: Kind, to: Kind) -> Conversion {
    use Kind::{Adf, Dms, ExtendedAdf, Gzip, Hardfile, Ipf, RigidDisk, Scp, Unknown};

    // C-003 is a licence constraint, not a gap, and it does not expire.
    if matches!(to, Ipf) {
        return Conversion::Refused {
            why: "IPF authoring is SPS-only and ADE will never emit it (C-003)",
        };
    }
    if matches!(to, Scp) {
        return Conversion::NotImplemented {
            why: "flux writing is not implemented; flux cannot be reconstructed from a \
                  sector image in any case",
        };
    }
    // Writing raw MFM works, and loses nothing: the sectors are encoded, not
    // discarded, and decode back byte-identically. What it cannot do is invent
    // protection that the source never had.
    if matches!(to, ExtendedAdf { .. }) {
        return match from {
            // A sector image encodes; an extended ADF is already one.
            Adf { .. } | Hardfile | Gzip | ExtendedAdf { .. } => Conversion::Lossless,
            _ => Conversion::NotImplemented {
                why: "only a sector image can be encoded as raw MFM",
            },
        };
    }
    if matches!(to, Gzip) {
        return Conversion::NotImplemented {
            why: "ADE inflates but does not deflate; writing compressed images is not needed to read a corpus",
        };
    }
    if matches!(to, Dms) {
        return Conversion::Refused {
            why: "DMS creation is not a preservation format and ADE will not emit it",
        };
    }
    if matches!(to, RigidDisk) {
        return Conversion::NotImplemented {
            why: "building a partition table is a write path, deferred to Phase 4 (D-004)",
        };
    }

    match from {
        // Two cases, one answer, for different reasons.
        //
        // `Gzip` is the one path whose reader is proven byte-identically
        // against real images, which is what D-004 requires before a write
        // path ships.
        //
        // The sector containers are a byte copy: ADF, HDF and a whole-device
        // image are the same thing — a flat run of sectors — distinguished by
        // naming convention rather than by structure (SPEC §A raw volume has
        // no geometry), so there is nothing to lose.
        Gzip | Adf { .. } | Hardfile | RigidDisk => Conversion::Lossless,
        ExtendedAdf { .. } => Conversion::Lossy {
            lost: "raw MFM tracks and any copy protection they carry — the reason \
                   the disk was captured this way",
        },
        Scp => Conversion::Lossy {
            lost: "flux timings, weak bits and copy protection; only the sectors \
                   that decode cleanly would survive",
        },
        Ipf => Conversion::NotImplemented {
            why: "IPF reading needs the CAPS library and is Phase 4 (C-003)",
        },
        Dms => Conversion::NotImplemented {
            why: "no DMS reader — blocked on test material, not effort (D-009)",
        },
        Unknown => Conversion::NotImplemented {
            why: "the input container was not recognised",
        },
    }
}

/// Every format ADE can name, for reporting the matrix.
#[must_use]
pub fn known_formats() -> Vec<Kind> {
    vec![
        Kind::Adf {
            cylinders: 80,
            sectors: 11,
        },
        Kind::Hardfile,
        Kind::RigidDisk,
        Kind::Gzip,
        Kind::ExtendedAdf { tracks: 160 },
        Kind::Dms,
        Kind::Scp,
        Kind::Ipf,
    ]
}
