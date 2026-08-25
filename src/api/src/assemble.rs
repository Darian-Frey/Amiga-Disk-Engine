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
                sectors_placed = sectors_placed.saturating_add(len / SECTOR_BYTES);
            }
            TrackKind::RawMfm => {
                let mut placed_here = false;
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
                    placed_here = true;
                }
                if placed_here {
                    from_raw_tracks = from_raw_tracks.saturating_add(1);
                }
            }
            TrackKind::Unknown(_) => {}
        }
    }

    Assembly {
        bytes: out,
        sectors_placed,
        sectors_total: DD_TOTAL_SECTORS,
        from_sector_tracks,
        from_raw_tracks,
    }
}
