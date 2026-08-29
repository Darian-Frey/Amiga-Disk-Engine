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

use crate::json::Value;

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

impl Conversion {
    /// One conversion as JSON (F-015, BUG-007).
    ///
    /// `kind` and `reason` are separate fields because they answer different
    /// questions, and F-016 turns on the difference: *refused* is a decision
    /// that does not expire, *not implemented* is a gap with a cause, and a
    /// caller deciding whether to wait or to look elsewhere needs to tell them
    /// apart. Collapsing both into one human-readable sentence would leave it
    /// parsing prose to find out.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let reason = match self {
            Self::Lossless => None,
            Self::Lossy { lost } => Some(*lost),
            Self::NotImplemented { why } | Self::Refused { why } => Some(*why),
        };
        Value::Obj(vec![
            ("kind", Value::str(self.label())),
            ("possible", Value::Bool(self.is_possible())),
            ("reason", Value::opt(reason, Value::str)),
        ])
    }
}

/// The whole conversion matrix as JSON (F-015, BUG-007).
///
/// Every ordered pair, including the impossible ones — the matrix is the
/// answer, and a caller asking "can I convert this to that" needs the pairs
/// that say no as much as the ones that say yes. Identity pairs are omitted,
/// as they are in the text form: a format converts to itself by copying.
#[must_use]
pub fn matrix_json() -> Value {
    let kinds = known_formats();
    let mut rows = Vec::new();
    for from in &kinds {
        for to in &kinds {
            if core::mem::discriminant(from) == core::mem::discriminant(to) {
                continue;
            }
            let verdict = conversion(*from, *to);
            rows.push(Value::Obj(vec![
                // Codes, not the display strings: `ADF (DD, 80 cylinders)`
                // carries a geometry that varies between images of one kind,
                // so a caller matching on it would be parsing prose. The
                // labels are alongside for anyone rendering the matrix.
                ("from", Value::str(from.code())),
                ("to", Value::str(to.code())),
                ("from_label", Value::str(from.to_string())),
                ("to_label", Value::str(to.to_string())),
                ("conversion", verdict.to_json()),
            ]));
        }
    }
    Value::Obj(vec![("conversions", Value::Arr(rows))])
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

/// Doing a conversion, as distinct from deciding whether to (F-016, IMP-007).
///
/// The decision half — [`conversion`] and the matrix — has always lived here.
/// The doing half lived in the CLI until 2026-08-29, which was fine while
/// there was one caller and became an F-002 violation the moment `ade batch`
/// wanted to convert a corpus: sixty lines of track encoding in a front end is
/// engine logic in UI code, and the layering check cannot see it because the
/// edge is inside a crate that is allowed to depend on the engine.
///
/// # What this will not do
///
/// Refuse and lossy are both errors here, not warnings. F-016's whole position
/// is that a conversion which quietly discards something is the behaviour ADE
/// exists to replace, so the caller gets a reason and no bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// The pair is possible but would discard something.
    Lossy {
        /// What would not survive.
        lost: &'static str,
    },
    /// ADE cannot do this pair yet.
    NotImplemented {
        /// Why, naming the register entry that tracks it.
        why: &'static str,
    },
    /// ADE will not do this pair, ever.
    Refused {
        /// Why.
        why: &'static str,
    },
    /// The input could not be read or re-encoded.
    Failed {
        /// What went wrong.
        reason: String,
    },
}

impl core::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Lossy { lost } => write!(f, "this would discard {lost}"),
            Self::NotImplemented { why } => write!(f, "not implemented — {why}"),
            Self::Refused { why } => write!(f, "refused — {why}"),
            Self::Failed { reason } => f.write_str(reason),
        }
    }
}

impl core::error::Error for ConvertError {}

impl ConvertError {
    /// A stable code for the machine surface (F-015).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Lossy { .. } => "lossy",
            Self::NotImplemented { .. } => "not-implemented",
            Self::Refused { .. } => "refused",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Convert an image's bytes into another container.
///
/// `to` is the target kind; for an extended ADF the caller asks for
/// [`Kind::ExtendedAdf`] rather than passing a flag, since the extension alone
/// cannot say which of the two `.adf` formats was meant.
///
/// # Errors
/// [`ConvertError`] carrying the matrix's own reason where the pair is refused,
/// lossy or unimplemented, and the failure where the bytes would not encode.
pub fn convert_bytes(bytes: Vec<u8>, to: Kind) -> Result<Vec<u8>, ConvertError> {
    let head = bytes.get(..bytes.len().min(512 * 16)).unwrap_or(&[]);
    let from = ade_container::sniff(head, bytes.len() as u64).kind;

    match conversion(from, to) {
        Conversion::Lossless => {}
        Conversion::Lossy { lost } => return Err(ConvertError::Lossy { lost }),
        Conversion::NotImplemented { why } => return Err(ConvertError::NotImplemented { why }),
        Conversion::Refused { why } => return Err(ConvertError::Refused { why }),
    }

    // Decompression first, so a raw-MFM target works from an ADZ as readily as
    // from an ADF.
    let sectors = if matches!(from, Kind::Gzip) {
        ade_container::inflate::gunzip(&bytes, crate::MAX_DECOMPRESSED).map_err(|e| {
            ConvertError::Failed {
                reason: e.to_string(),
            }
        })?
    } else {
        bytes
    };

    if matches!(to, Kind::ExtendedAdf { .. }) {
        return encode_raw_mfm(&sectors).map_err(|reason| ConvertError::Failed { reason });
    }
    Ok(sectors)
}

/// Encode a sector image as an extended ADF of raw MFM tracks.
///
/// # Errors
/// A description of why the image could not be encoded — no usable geometry,
/// or a length that is not a whole number of tracks.
///
/// Lossless, and provably so: reading the result back reassembles the input
/// byte for byte. What it cannot do is invent protection the source never had
/// — the output is an extended ADF holding an ordinary disk, which is a
/// container change rather than a preservation upgrade.
pub fn encode_raw_mfm(sectors: &[u8]) -> Result<Vec<u8>, String> {
    use ade_container::extended::{self, TrackSource};
    use ade_track::{SECTOR_BYTES, encode_track};

    // The sectors-per-track comes from the image's own geometry rather than
    // being assumed: an HD image has 22 where a DD one has 11.
    let inspection = crate::inspect_bytes(sectors.to_vec());
    let geometry = inspection
        .geometry
        .ok_or("cannot establish a geometry for this image")?;
    let per_track = usize::try_from(geometry.sectors()).unwrap_or(11).max(1);
    let track_bytes = per_track.saturating_mul(SECTOR_BYTES);
    let remainder = sectors
        .len()
        .checked_rem(track_bytes)
        .ok_or("a track cannot be zero bytes")?;
    if remainder != 0 {
        return Err(format!(
            "not a whole number of {per_track}-sector tracks — {} bytes",
            sectors.len()
        ));
    }
    let track_count = sectors
        .len()
        .checked_div(track_bytes)
        .ok_or("a track cannot be zero bytes")?;

    let mut encoded: Vec<Vec<u8>> = Vec::with_capacity(track_count);
    for index in 0..track_count {
        let base = index.saturating_mul(track_bytes);
        let mut slices: Vec<&[u8]> = Vec::with_capacity(per_track);
        for s in 0usize..per_track {
            let at = base.saturating_add(s.saturating_mul(SECTOR_BYTES));
            let end = at.saturating_add(SECTOR_BYTES);
            slices.push(sectors.get(at..end).ok_or("image ends mid-track")?);
        }
        let number = u8::try_from(index).unwrap_or(u8::MAX);
        encoded.push(encode_track(number, &slices).ok_or("a sector was the wrong size")?);
    }

    let sources: Vec<TrackSource<'_>> = encoded
        .iter()
        .map(|data| TrackSource::RawMfm {
            data,
            length_bits: u32::try_from(data.len().saturating_mul(8)).unwrap_or(u32::MAX),
        })
        .collect();
    extended::write(&sources).map_err(|e| e.to_string())
}
