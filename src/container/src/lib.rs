//! Container front-end — normalises ADF, ADZ, HDF, HDZ and DMS into the block
//! layer.
//!
//! Sits beside the pipeline rather than in it: the upper layers never see
//! compression or wrapping. Implements [`ade_block::BlockSource`].
//!
//! # Identification is evidence, not a verdict (C-008)
//!
//! A plain ADF has no magic number, is not a fixed size, need not carry a valid
//! bootblock checksum, and its `DOS` prefix neither implies a mountable volume
//! nor is required for one. Measured against 4288 real images: 7% do not begin
//! with `DOS`, only 74% of those that do have a valid bootblock checksum, and
//! 19% of them have no rootblock at all — while ten non-`DOS` images mount
//! perfectly.
//!
//! So [`sniff()`] returns what it concluded **and the evidence it used**, and
//! never rejects an image for failing a test that real disks routinely fail.
//! The caller reports both.

pub mod sniff;

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock};

pub use sniff::{Detection, Evidence, Kind, sniff};

/// An image held in memory, presenting its bytes as blocks.
///
/// Adequate for floppies, which are under two megabytes. Whole-disk HDF images
/// reach gigabytes and will want a positional-read source instead; the
/// [`BlockSource`] seam takes `&self` and fills a caller buffer specifically so
/// that can be added in Phase 2 without disturbing anything above it.
pub struct RawImage {
    bytes: Vec<u8>,
    geometry: Geometry,
}

impl RawImage {
    /// Wrap bytes with an explicit geometry.
    ///
    /// # Errors
    /// [`BlockError::Truncated`] if the bytes do not cover the geometry.
    pub fn new(bytes: Vec<u8>, geometry: Geometry) -> Result<Self, BlockError> {
        let needed = geometry.total_bytes();
        if (bytes.len() as u64) < needed {
            return Err(BlockError::Truncated {
                index: BlockIndex(
                    (bytes.len() as u64)
                        .checked_div(u64::from(geometry.block_size()))
                        .unwrap_or(0),
                ),
            });
        }
        Ok(Self { bytes, geometry })
    }

    /// The raw bytes, for parsers that need to look outside the block grid —
    /// the bootblock spans two blocks, and its checksum covers both.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl BlockSource for RawImage {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        let size = self.geometry.block_size() as usize;
        if out.len() != size {
            return Err(BlockError::BufferSize {
                got: out.len(),
                want: size,
            });
        }
        // A 64-bit block index cannot address memory on a 32-bit host; a
        // failed conversion means the block is beyond anything we could hold,
        // which is the same outcome as a short image.
        let start = usize::try_from(block.index())
            .ok()
            .and_then(|i| i.checked_mul(size))
            .ok_or(BlockError::Truncated {
                index: BlockIndex(block.index()),
            })?;
        let end = start.saturating_add(size);
        let src = self.bytes.get(start..end).ok_or(BlockError::Truncated {
            index: BlockIndex(block.index()),
        })?;
        out.copy_from_slice(src);
        Ok(())
    }
}
