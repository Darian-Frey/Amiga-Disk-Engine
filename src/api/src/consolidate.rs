//! Comparing and consolidating several dumps of the same disk (F-008, F-009).
//!
//! Marginal disks get read more than once, and the reads disagree. Merging them
//! is the obvious response and open tooling does it poorly — usually by picking
//! one file and hoping.
//!
//! # What this can and cannot claim
//!
//! F-008's wording is "merge N reads of the same disk into a best-estimate
//! image". That wording assumes the N came from repeated reads of one physical
//! disk, where a disagreement means one read was wrong. **The material actually
//! available is not that.** The corpus holds 46 titles with differing dumps,
//! but they are independent dumps of possibly *different physical copies*, so a
//! disagreement can mean the disks genuinely differ rather than that either
//! read failed.
//!
//! One corpus pair differs in exactly one track: track 80, sector 0 — block
//! 880, the rootblock — by seventeen bytes. That is a volume datestamp. One
//! copy was mounted at some point and neither dump is wrong.
//!
//! So this reports **agreement**, not correctness. Where dumps disagree it says
//! which tracks, which sectors, and how the votes fell; it does not pronounce a
//! winner correct. Calling a plurality across three dumps of possibly-different
//! disks a "best estimate" would be a stronger claim than the evidence carries.
//!
//! # Merging is per sector, reporting is per track
//!
//! Differences are frequently confined to one sector — the rootblock case
//! above is a single sector of 1760 — so merging a whole track on a plurality
//! would discard good sectors along with the disputed one. F-008 asks for a
//! per-track report, and that is what is reported; the merge underneath is
//! finer.

use std::collections::BTreeMap;

use crate::json::Value;

/// Bytes in one sector.
const SECTOR: usize = 512;

/// Sectors in one standard double-density track.
const SECTORS_PER_TRACK: usize = 11;

/// How the dumps voted on one sector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agreement {
    /// Every dump gave the same bytes.
    Unanimous,
    /// One version was held by more dumps than any other.
    Plurality {
        /// How many dumps held the winning version.
        agreeing: usize,
        /// How many distinct versions were seen.
        distinct: usize,
    },
    /// No version had more support than another. Nothing here is a merge so
    /// much as a coin toss, and it is reported rather than hidden.
    Tied {
        /// How many distinct versions were seen.
        distinct: usize,
    },
}

impl Agreement {
    /// Whether the dumps settled this sector between them.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        matches!(self, Self::Unanimous | Self::Plurality { .. })
    }
}

/// What the dumps said about one track.
#[derive(Debug, Clone)]
pub struct TrackReport {
    /// Track number.
    pub track: usize,
    /// Sectors on which the dumps disagreed at all.
    pub disputed: Vec<usize>,
    /// Sectors where no version had a plurality.
    pub unresolved: Vec<usize>,
}

impl TrackReport {
    /// Whether every dump agreed on every sector of this track.
    #[must_use]
    pub fn is_unanimous(&self) -> bool {
        self.disputed.is_empty()
    }

    /// One track's disagreements as JSON (F-015, BUG-007).
    ///
    /// **The sector numbers are within this track, not absolute.** Sector 2 of
    /// track 1 is absolute sector 13, and this reports 2. `diff` reports
    /// absolute indices because it has no track to hang them from; here they
    /// sit inside the track that owns them, and repeating the absolute number
    /// would be reporting the track twice. Worth stating because both fields
    /// are called `sectors` in prose, and a caller comparing the two outputs
    /// without knowing this would find them disagreeing about the same disk.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("track", Value::Num(self.track as u64)),
            (
                "disputed",
                Value::Arr(
                    self.disputed
                        .iter()
                        .map(|s| Value::Num(*s as u64))
                        .collect(),
                ),
            ),
            (
                "unresolved",
                Value::Arr(
                    self.unresolved
                        .iter()
                        .map(|s| Value::Num(*s as u64))
                        .collect(),
                ),
            ),
        ])
    }
}

/// The result of consolidating several dumps.
#[derive(Debug, Clone)]
pub struct Consolidation {
    /// How many dumps went in.
    pub sources: usize,
    /// The merged image.
    ///
    /// Built from the winning version of each sector. **Not** a best estimate:
    /// see the module documentation.
    pub bytes: Vec<u8>,
    /// One entry per track that had any disagreement. Tracks the dumps agreed
    /// on entirely are omitted — a report listing 160 unanimous tracks buries
    /// the four that matter.
    pub tracks: Vec<TrackReport>,
    /// Sectors on which every dump agreed.
    pub unanimous_sectors: usize,
    /// Sectors that were disputed but had a clear plurality.
    pub resolved_sectors: usize,
    /// Sectors where no version had a plurality.
    pub unresolved_sectors: usize,
}

impl Consolidation {
    /// Whether the dumps agreed on the whole disk.
    #[must_use]
    pub const fn is_unanimous(&self) -> bool {
        self.resolved_sectors == 0 && self.unresolved_sectors == 0
    }

    /// Total sectors compared.
    #[must_use]
    pub const fn total_sectors(&self) -> usize {
        self.unanimous_sectors
            .saturating_add(self.resolved_sectors)
            .saturating_add(self.unresolved_sectors)
    }

    /// The consolidation as JSON (F-015, BUG-007).
    ///
    /// `can_vote` is false with two dumps, and it is not a convenience: every
    /// disagreement between two dumps ties by definition, so `unresolved` is
    /// arithmetic rather than damage. A caller that sorted by `unresolved`
    /// without it would rank two-dump runs as the most broken thing it had.
    ///
    /// The merged bytes are deliberately absent. They can be megabytes, JSON
    /// is the wrong container for them, and `--output` already writes them to
    /// a file.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("sources", Value::Num(self.sources as u64)),
            ("sectors_total", Value::Num(self.total_sectors() as u64)),
            ("unanimous", Value::Bool(self.is_unanimous())),
            ("agreed_sectors", Value::Num(self.unanimous_sectors as u64)),
            ("resolved_sectors", Value::Num(self.resolved_sectors as u64)),
            (
                "unresolved_sectors",
                Value::Num(self.unresolved_sectors as u64),
            ),
            ("can_vote", Value::Bool(self.sources > 2)),
            (
                "tracks",
                Value::Arr(self.tracks.iter().map(TrackReport::to_json).collect()),
            ),
        ])
    }
}

/// Why dumps could not be consolidated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsolidateError {
    /// Fewer than two dumps were given.
    TooFewSources,
    /// The dumps are not the same length, so they are not dumps of one disk.
    SizeMismatch {
        /// The first dump's length.
        expected: usize,
        /// The length that differed.
        found: usize,
    },
    /// The dumps are not a whole number of sectors.
    NotWholeSectors {
        /// The length that was not a multiple of 512.
        length: usize,
    },
}

impl core::fmt::Display for ConsolidateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooFewSources => f.write_str("consolidating needs at least two dumps"),
            Self::SizeMismatch { expected, found } => write!(
                f,
                "dumps are different sizes ({expected} and {found}) — not dumps of one disk"
            ),
            Self::NotWholeSectors { length } => {
                write!(
                    f,
                    "{length} bytes is not a whole number of 512-byte sectors"
                )
            }
        }
    }
}

impl core::error::Error for ConsolidateError {}

/// Merge several dumps of the same disk, sector by sector.
///
/// The winning version of a sector is the one the most dumps hold. Ties are
/// broken by taking the first dump's version — arbitrary, and reported as
/// unresolved so nobody mistakes the choice for a judgement.
///
/// # Errors
/// [`ConsolidateError`] when the inputs cannot be dumps of one disk.
pub fn consolidate(dumps: &[Vec<u8>]) -> Result<Consolidation, ConsolidateError> {
    let first = dumps.first().ok_or(ConsolidateError::TooFewSources)?;
    if dumps.len() < 2 {
        return Err(ConsolidateError::TooFewSources);
    }
    let length = first.len();
    if length % SECTOR != 0 {
        return Err(ConsolidateError::NotWholeSectors { length });
    }
    for dump in dumps {
        if dump.len() != length {
            return Err(ConsolidateError::SizeMismatch {
                expected: length,
                found: dump.len(),
            });
        }
    }

    let sector_count = length / SECTOR;
    let mut bytes = Vec::with_capacity(length);
    let mut per_track: BTreeMap<usize, TrackReport> = BTreeMap::new();
    let mut unanimous_sectors = 0usize;
    let mut resolved_sectors = 0usize;
    let mut unresolved_sectors = 0usize;

    for index in 0..sector_count {
        let at = index.saturating_mul(SECTOR);
        let end = at.saturating_add(SECTOR);

        // Count votes by content. A BTreeMap keyed on the bytes keeps this
        // deterministic, which matters for a tool whose output people compare.
        let mut votes: BTreeMap<&[u8], usize> = BTreeMap::new();
        for dump in dumps {
            let Some(sector) = dump.get(at..end) else {
                continue;
            };
            let count = votes.entry(sector).or_insert(0usize);
            *count = count.saturating_add(1);
        }

        let distinct = votes.len();
        let best = votes.values().copied().max().unwrap_or(0);
        let leaders = votes.values().filter(|count| **count == best).count();

        let agreement = if distinct <= 1 {
            unanimous_sectors = unanimous_sectors.saturating_add(1);
            Agreement::Unanimous
        } else if leaders == 1 {
            resolved_sectors = resolved_sectors.saturating_add(1);
            Agreement::Plurality {
                agreeing: best,
                distinct,
            }
        } else {
            unresolved_sectors = unresolved_sectors.saturating_add(1);
            Agreement::Tied { distinct }
        };

        // The winning bytes, or the first dump's on a tie.
        let winner: &[u8] = match &agreement {
            Agreement::Tied { .. } => first.get(at..end).unwrap_or(&[0u8; SECTOR]),
            _ => votes
                .iter()
                .find(|(_, count)| **count == best)
                .map_or(first.get(at..end).unwrap_or(&[0u8; SECTOR]), |(data, _)| {
                    data
                }),
        };
        bytes.extend_from_slice(winner);

        if agreement != Agreement::Unanimous {
            let track = index / SECTORS_PER_TRACK;
            let sector = index % SECTORS_PER_TRACK;
            let entry = per_track.entry(track).or_insert_with(|| TrackReport {
                track,
                disputed: Vec::new(),
                unresolved: Vec::new(),
            });
            entry.disputed.push(sector);
            if !agreement.is_resolved() {
                entry.unresolved.push(sector);
            }
        }
    }

    Ok(Consolidation {
        sources: dumps.len(),
        bytes,
        tracks: per_track.into_values().collect(),
        unanimous_sectors,
        resolved_sectors,
        unresolved_sectors,
    })
}

/// Where two dumps of a disk differ (F-009).
#[derive(Debug, Clone)]
pub struct Diff {
    /// Sectors that differ, by absolute index.
    pub sectors: Vec<usize>,
    /// Tracks that contain at least one differing sector.
    pub tracks: Vec<usize>,
    /// Bytes that differ across the whole image.
    pub bytes_differing: usize,
    /// Sectors compared.
    pub sectors_total: usize,
}

impl Diff {
    /// Whether the two dumps are byte-identical.
    #[must_use]
    pub const fn is_identical(&self) -> bool {
        self.bytes_differing == 0
    }

    /// The comparison as JSON (F-015, BUG-007).
    ///
    /// The differing sectors are listed in full rather than summarised. This
    /// is the machine surface, and the whole reason to compare two dumps is to
    /// act on *which* sectors moved — a count would make the caller run the
    /// comparison again itself. A disk holds 1760 of them, so the list is
    /// bounded by the format.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("identical", Value::Bool(self.is_identical())),
            ("sectors_total", Value::Num(self.sectors_total as u64)),
            ("sectors_differing", Value::Num(self.sectors.len() as u64)),
            ("bytes_differing", Value::Num(self.bytes_differing as u64)),
            (
                "sectors",
                Value::Arr(self.sectors.iter().map(|s| Value::Num(*s as u64)).collect()),
            ),
            (
                "tracks",
                Value::Arr(self.tracks.iter().map(|t| Value::Num(*t as u64)).collect()),
            ),
        ])
    }
}

/// Compare two dumps sector by sector.
///
/// # Errors
/// [`ConsolidateError::SizeMismatch`] if they are different lengths.
pub fn diff(a: &[u8], b: &[u8]) -> Result<Diff, ConsolidateError> {
    if a.len() != b.len() {
        return Err(ConsolidateError::SizeMismatch {
            expected: a.len(),
            found: b.len(),
        });
    }
    if a.len() % SECTOR != 0 {
        return Err(ConsolidateError::NotWholeSectors { length: a.len() });
    }

    let sectors_total = a.len() / SECTOR;
    let mut sectors = Vec::new();
    let mut tracks = Vec::new();
    let mut bytes_differing = 0usize;

    for index in 0..sectors_total {
        let at = index.saturating_mul(SECTOR);
        let end = at.saturating_add(SECTOR);
        let (Some(left), Some(right)) = (a.get(at..end), b.get(at..end)) else {
            continue;
        };
        if left == right {
            continue;
        }
        sectors.push(index);
        bytes_differing = bytes_differing.saturating_add(
            left.iter()
                .zip(right.iter())
                .filter(|(x, y)| x != y)
                .count(),
        );
        let track = index / SECTORS_PER_TRACK;
        if tracks.last() != Some(&track) {
            tracks.push(track);
        }
    }

    Ok(Diff {
        sectors,
        tracks,
        bytes_differing,
        sectors_total,
    })
}
