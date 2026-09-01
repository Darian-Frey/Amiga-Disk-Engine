//! Assembling a raw-track container into a filesystem view (F-007).
//!
//! An extended ADF holds tracks, not a volume — but most of a protected disk is
//! usually ordinary, and the ordinary part is a perfectly good AmigaDOS volume
//! that nothing was reading. Six of the corpus's eleven extended ADFs mount
//! this way, one of them yielding 29 files and 560 KB that were previously
//! unreachable.
//!
//! This is F-007's dual view: the same image reported *both* as a track table,
//! which is what it is, and as a filesystem, which is what most of it holds.
//!
//! # The result is a reconstruction, and is labelled as one
//!
//! What comes out is not the file. Sectors that could not be decoded are left
//! as zeros, so a volume assembled from a heavily protected disk is mostly
//! holes. [`Assembly::sectors_placed`] says how much is real, and every report
//! of an assembled volume carries it — a directory listing that silently omits
//! half a disk is worse than no listing.
//!
//! Two disks assemble to nothing useful despite decoding over a thousand
//! sectors each: `Deep Space` and `Terrorpods` label **every** raw track as
//! track 0, so placing sectors by physical position cannot reconstruct them
//! (SPEC §MFM). That is a property of those disks, not a failure here.

use ade_container::extended::{ExtendedAdf, STANDARD_TRACK_BYTES, TrackKind};
use ade_flux::mfm;
use ade_flux::scp::Scp;
use ade_track::{DD_SECTORS, SECTOR_BYTES, decode_track};

/// Tracks in the standard double-density layout an assembly targets.
const DD_TRACKS: usize = 160;

/// Sectors on a whole double-density disk.
const DD_TOTAL_SECTORS: usize = DD_TRACKS * DD_SECTORS;

/// What assembling a raw-track container produced.
#[derive(Debug, Clone)]
pub struct Assembly {
    /// The reconstructed image, standard double-density size.
    pub bytes: Vec<u8>,
    /// Sectors actually written. The rest are zeros.
    pub sectors_placed: usize,
    /// Sectors a full disk would hold — the denominator for the above.
    pub sectors_total: usize,
    /// Tracks contributed by ordinary sector tracks.
    pub from_sector_tracks: usize,
    /// Tracks contributed by decoding raw MFM.
    pub from_raw_tracks: usize,
    /// What happened on each of the disk's 160 tracks, in order (F-029).
    ///
    /// Always the full length, so a track the container never mentioned is
    /// present and empty rather than absent — "nothing was recovered here" and
    /// "nobody looked here" are the same picture on a surface view, and only
    /// one of them is a fact about the disk.
    pub tracks: Vec<TrackState>,
}

/// What was recovered from one track, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackState {
    /// 0–159: cylinder times two, plus the head.
    pub index: usize,
    /// Sectors actually placed from this track.
    pub sectors: u8,
    /// Sectors a whole track holds — the denominator.
    pub expected: u8,
    /// How the track reached the image.
    pub source: TrackSource,
}

impl TrackState {
    /// The cylinder this track is on.
    #[must_use]
    pub const fn cylinder(&self) -> usize {
        self.index / 2
    }

    /// Which side of it.
    #[must_use]
    pub const fn head(&self) -> usize {
        self.index % 2
    }
}

/// Where a track's sectors came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackSource {
    /// Stored already decoded, as ordinary sector data.
    Sectors,
    /// Decoded here, out of raw MFM.
    RawMfm,
    /// The container carried nothing for this track.
    Absent,
}

impl TrackSource {
    /// The name this source is reported by. Part of the JSON surface (F-015).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sectors => "sectors",
            Self::RawMfm => "raw MFM",
            Self::Absent => "absent",
        }
    }
}

impl Assembly {
    /// Whether anything was recovered at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.sectors_placed == 0
    }

    /// How complete the reconstruction is, as a percentage.
    ///
    /// Integer arithmetic: a completeness figure derived from sector counts has
    /// no business going through a float.
    #[must_use]
    pub const fn percent_complete(&self) -> usize {
        if self.sectors_total == 0 {
            return 0;
        }
        match self
            .sectors_placed
            .saturating_mul(100)
            .checked_div(self.sectors_total)
        {
            Some(percent) => percent,
            None => 0,
        }
    }
}

/// Reconstruct whatever raw-track container `bytes` is, if it is one.
///
/// `None` for a plain ADF, an ADZ, a hardfile — anything already stored as
/// sectors. That is not a failure: those carry no record of what came off the
/// medium, so there is nothing to say about their tracks, and inventing 160
/// full ones would claim a measurement nobody made.
#[must_use]
pub fn of_bytes(bytes: &[u8]) -> Option<Assembly> {
    use ade_container::sniff::{Kind, sniff};

    let head = bytes.get(..bytes.len().min(4096)).unwrap_or(&[]);
    match sniff(head, bytes.len() as u64).kind {
        Kind::ExtendedAdf { .. } => {
            let parsed = ExtendedAdf::parse(bytes).ok()?;
            Some(assemble(&parsed, bytes))
        }
        Kind::Scp => {
            let parsed = Scp::parse(bytes).ok()?;
            Some(assemble_scp(&parsed, bytes))
        }
        _ => None,
    }
}

/// One state per track, all absent, so a track nothing mentioned is still a row.
fn blank_tracks() -> Vec<TrackState> {
    (0..DD_TRACKS)
        .map(|index| TrackState {
            index,
            sectors: 0,
            expected: u8::try_from(DD_SECTORS).unwrap_or(11),
            source: TrackSource::Absent,
        })
        .collect()
}

/// Assemble a plain double-density image from whatever a raw-track container
/// can yield.
///
/// Sectors are placed by their **physical** track position and their own sector
/// number. The track number a sector claims is deliberately not used: two
/// corpus disks label every track 0, and trusting that would pile the whole
/// disk onto track 0.
#[must_use]
pub fn assemble(parsed: &ExtendedAdf, bytes: &[u8]) -> Assembly {
    let mut out = vec![0u8; DD_TRACKS.saturating_mul(STANDARD_TRACK_BYTES)];
    let mut sectors_placed = 0usize;
    let mut from_sector_tracks = 0usize;
    let mut from_raw_tracks = 0usize;
    let mut states = blank_tracks();

    for track in &parsed.tracks {
        if track.index >= DD_TRACKS {
            continue;
        }
        let Some(data) = parsed.track_data(bytes, track.index) else {
            continue;
        };
        let base = track.index.saturating_mul(STANDARD_TRACK_BYTES);

        match track.kind {
            TrackKind::Sectors => {
                let len = data.len().min(STANDARD_TRACK_BYTES);
                let Some(slot) = out.get_mut(base..base.saturating_add(len)) else {
                    continue;
                };
                let Some(source) = data.get(..len) else {
                    continue;
                };
                slot.copy_from_slice(source);
                from_sector_tracks = from_sector_tracks.saturating_add(1);
                let here = len / SECTOR_BYTES;
                sectors_placed = sectors_placed.saturating_add(here);
                if let Some(state) = states.get_mut(track.index) {
                    state.sectors = u8::try_from(here).unwrap_or(u8::MAX);
                    state.source = TrackSource::Sectors;
                }
            }
            TrackKind::RawMfm => {
                let mut placed_here = false;
                let mut here = 0usize;
                for sector in decode_track(data).sectors.iter().filter(|s| s.is_sound()) {
                    let index = usize::from(sector.sector);
                    if index >= DD_SECTORS {
                        continue;
                    }
                    let at = base.saturating_add(index.saturating_mul(SECTOR_BYTES));
                    let Some(slot) = out.get_mut(at..at.saturating_add(SECTOR_BYTES)) else {
                        continue;
                    };
                    if sector.data.len() != SECTOR_BYTES {
                        continue;
                    }
                    slot.copy_from_slice(&sector.data);
                    sectors_placed = sectors_placed.saturating_add(1);
                    here = here.saturating_add(1);
                    placed_here = true;
                }
                if placed_here {
                    from_raw_tracks = from_raw_tracks.saturating_add(1);
                }
                // Recorded even when nothing decoded: a raw track that yielded
                // no sectors is the single most interesting cell on a surface
                // view, and treating it as absent would hide it.
                if let Some(state) = states.get_mut(track.index) {
                    state.sectors = u8::try_from(here).unwrap_or(u8::MAX);
                    state.source = TrackSource::RawMfm;
                }
            }
            TrackKind::Unknown(_) => {}
        }
    }

    Assembly {
        bytes: out,
        sectors_placed,
        sectors_total: DD_TOTAL_SECTORS,
        tracks: states,
        from_sector_tracks,
        from_raw_tracks,
    }
}

/// Assemble a plain double-density image from an SCP flux capture.
///
/// The same reconstruction as [`assemble`], one layer further down: the flux
/// has to become bits before it can become sectors. Sectors are placed by
/// physical track position for the same reason and with the same caveat.
///
/// # Every revolution is tried, and the sectors are merged
///
/// An SCP normally stores two or more revolutions of each track. They are not
/// duplicates — a marginal or weak-bit region reads differently each time,
/// which is *why* the format stores several. So each revolution is decoded and
/// any sound sector it yields that is still missing is taken.
///
/// That is F-008's merge applied within one file rather than across several
/// dumps, and it is the reason reading flux beats reading a sector image of
/// the same disk: the sector image already threw away the second opinion.
#[must_use]
pub fn assemble_scp(parsed: &Scp, bytes: &[u8]) -> Assembly {
    let mut out = vec![0u8; DD_TRACKS.saturating_mul(STANDARD_TRACK_BYTES)];
    let mut sectors_placed = 0usize;
    let mut from_raw_tracks = 0usize;
    let mut states = blank_tracks();

    for track in &parsed.tracks {
        if track.index >= DD_TRACKS {
            continue;
        }
        let base = track.index.saturating_mul(STANDARD_TRACK_BYTES);
        let mut placed_here = false;
        // Which sectors of this track are already recovered, so a later
        // revolution only fills what an earlier one missed.
        let mut have = [false; DD_SECTORS];

        for revolution in 0..track.revolutions.len() {
            if have.iter().all(|h| *h) {
                break;
            }
            let Some(intervals) = parsed.intervals(bytes, track.index, revolution) else {
                continue;
            };
            let stream = mfm::to_bits(&intervals, mfm::NOMINAL_CELL_TICKS);
            for sector in decode_track(&stream.bits)
                .sectors
                .iter()
                .filter(|s| s.is_sound())
            {
                let index = usize::from(sector.sector);
                if index >= DD_SECTORS || have.get(index).copied().unwrap_or(true) {
                    continue;
                }
                let at = base.saturating_add(index.saturating_mul(SECTOR_BYTES));
                let Some(slot) = out.get_mut(at..at.saturating_add(SECTOR_BYTES)) else {
                    continue;
                };
                if sector.data.len() != SECTOR_BYTES {
                    continue;
                }
                slot.copy_from_slice(&sector.data);
                if let Some(flag) = have.get_mut(index) {
                    *flag = true;
                }
                sectors_placed = sectors_placed.saturating_add(1);
                placed_here = true;
            }
        }
        if placed_here {
            from_raw_tracks = from_raw_tracks.saturating_add(1);
        }
        // `have` is the per-sector record this loop already keeps to stop
        // later revolutions overwriting earlier ones, so the count is free.
        if let Some(state) = states.get_mut(track.index) {
            state.sectors = u8::try_from(have.iter().filter(|h| **h).count()).unwrap_or(u8::MAX);
            state.source = TrackSource::RawMfm;
        }
    }

    Assembly {
        bytes: out,
        sectors_placed,
        sectors_total: DD_TOTAL_SECTORS,
        tracks: states,
        from_sector_tracks: 0,
        from_raw_tracks,
    }
}
