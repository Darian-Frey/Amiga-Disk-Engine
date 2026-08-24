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

/// A window onto part of another block source.
///
/// A partition is a range of a device's blocks presented as a volume in its
/// own right: block 0 of the view is block `start` of the device, and the
/// rootblock sits at the view's midpoint rather than the device's.
///
/// Deliberately knows nothing about partition tables. It takes a start and a
/// count, so the RDB parsing that produces those numbers stays in the
/// filesystem layer and this stays in the container layer (D-003).
pub struct Window<'a> {
    source: &'a dyn BlockSource,
    start: u64,
    geometry: Geometry,
}

impl<'a> Window<'a> {
    /// Present `blocks` blocks of `source`, beginning at `start`, as a volume.
    ///
    /// `reserved` is the partition's own reserved-block count, which comes from
    /// its DOSEnvVec and is usually 2 — it feeds the rootblock computation, so
    /// taking the floppy default would put the rootblock in the wrong place on
    /// any partition that differs.
    ///
    /// # Errors
    /// [`BlockError::OutOfRange`] if the window falls outside the device, or a
    /// geometry error if the shape is unusable.
    pub fn new(
        source: &'a dyn BlockSource,
        start: u64,
        blocks: u32,
        block_size: u32,
        reserved: u32,
    ) -> Result<Self, BlockError> {
        let device_blocks = source.geometry().total_blocks();
        let end = start.saturating_add(u64::from(blocks));
        if end > device_blocks {
            return Err(BlockError::OutOfRange {
                index: BlockIndex(end),
                total: device_blocks,
            });
        }
        // A window is addressed as a flat run of blocks; heads and sectors are
        // the device's business, not the volume's (SPEC §A raw volume has no
        // geometry).
        let geometry = Geometry::new(blocks, 1, 1, block_size, reserved).map_err(|_| {
            BlockError::BufferSize {
                got: 0,
                want: block_size as usize,
            }
        })?;
        Ok(Self {
            source,
            start,
            geometry,
        })
    }

    /// Where this window begins on the underlying device.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }
}

impl BlockSource for Window<'_> {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        // `block` is valid for the *window*; translating it to the device and
        // validating again is what keeps AV-004 intact across the boundary —
        // a window cannot be used to reach outside its own range.
        let device_block = self.start.saturating_add(block.index());
        let valid = self.source.geometry().validate(BlockIndex(device_block))?;
        self.source.read_block(valid, out)
    }
}
