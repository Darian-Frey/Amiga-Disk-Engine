//! The 512-byte block abstraction: geometry, bounds-checked addressing, and
//! the [`BlockSource`] seam.
//!
//! This crate sits at the centre of the pipeline. Everything below it — the
//! container front-end for ADF/ADZ/HDF/HDZ/DMS, and the MFM track codec —
//! *implements* [`BlockSource`]. Everything above it consumes one. The
//! dependency arrows therefore point downward at an abstraction rather than
//! sideways at a sibling, which is what keeps D-003's "no module spans two
//! layers" true of the dependency graph and not merely of the prose.
//!
//! # Bounds checking is structural
//!
//! AV-004 (out-of-range block pointers causing wild reads) is not defended
//! against by remembering to check. [`BlockSource::read_block`] takes a
//! [`ValidBlock`], whose only constructor is [`Geometry::validate`] and whose
//! field is private to this crate. An implementor of [`BlockSource`] cannot be
//! handed an unchecked index, and a caller cannot manufacture one.

use core::fmt;

/// A logical block address, numbered from zero. Not yet known to be in range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockIndex(pub u64);

impl fmt::Display for BlockIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block {}", self.0)
    }
}

/// A block index proven to lie within a particular [`Geometry`].
///
/// Constructible only by [`Geometry::validate`]. This is the type-level form
/// of the AV-004 defence: possession of a `ValidBlock` *is* the proof that the
/// bounds check happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidBlock {
    index: u64,
}

impl ValidBlock {
    /// The validated address.
    #[must_use]
    pub const fn index(self) -> u64 {
        self.index
    }
}

impl fmt::Display for ValidBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "block {}", self.index)
    }
}

/// Physical shape of a volume.
///
/// Amiga geometries are described in cylinders/heads/sectors; the block layer
/// works in a flat logical address space derived from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    cylinders: u32,
    heads: u32,
    sectors: u32,
    block_size: u32,
    reserved: u32,
}

/// Constructing a geometry that cannot describe a real device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    /// A dimension was zero, so the volume would hold no blocks.
    ZeroDimension,
    /// The dimensions multiply out beyond what an address space can hold.
    Overflow,
    /// More blocks are reserved than the volume contains.
    ReservedExceedsVolume,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension => f.write_str("geometry has a zero dimension"),
            Self::Overflow => f.write_str("geometry dimensions overflow the address space"),
            Self::ReservedExceedsVolume => {
                f.write_str("more blocks are reserved than the volume contains")
            }
        }
    }
}

impl core::error::Error for GeometryError {}

impl Geometry {
    /// Reserved blocks at the start of a standard floppy volume — the two boot
    /// blocks. Hard-disk partitions carry their own value in the DOSEnvVec
    /// `Reserved` field (usually 2, minimum 1).
    pub const FLOPPY_RESERVED: u32 = 2;

    /// Standard 880 KB double-density Amiga floppy: 80 × 2 × 11 × 512.
    pub const DD_FLOPPY: Self = Self {
        cylinders: 80,
        heads: 2,
        sectors: 11,
        block_size: 512,
        reserved: Self::FLOPPY_RESERVED,
    };

    /// 1.76 MB high-density Amiga floppy: 80 × 2 × 22 × 512.
    pub const HD_FLOPPY: Self = Self {
        cylinders: 80,
        heads: 2,
        sectors: 22,
        block_size: 512,
        reserved: Self::FLOPPY_RESERVED,
    };

    /// Build a geometry, rejecting shapes that cannot describe a device.
    ///
    /// Hard-disk images carry configurable block sizes (512/1K/2K/4K per
    /// C-002/C-005), so `block_size` is a parameter rather than a constant.
    /// `reserved` is the number of reserved blocks at the volume start, which
    /// feeds [`Self::root_block`] — see C-007.
    pub const fn new(
        cylinders: u32,
        heads: u32,
        sectors: u32,
        block_size: u32,
        reserved: u32,
    ) -> Result<Self, GeometryError> {
        if cylinders == 0 || heads == 0 || sectors == 0 || block_size == 0 {
            return Err(GeometryError::ZeroDimension);
        }
        let geometry = Self {
            cylinders,
            heads,
            sectors,
            block_size,
            reserved,
        };
        let Some(total) = geometry.checked_total_blocks() else {
            return Err(GeometryError::Overflow);
        };
        if reserved as u64 >= total {
            return Err(GeometryError::ReservedExceedsVolume);
        }
        Ok(geometry)
    }

    /// Reserved blocks at the start of the volume.
    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }

    const fn checked_total_blocks(&self) -> Option<u64> {
        let Some(tracks) = (self.cylinders as u64).checked_mul(self.heads as u64) else {
            return None;
        };
        tracks.checked_mul(self.sectors as u64)
    }

    /// Total addressable blocks.
    #[must_use]
    pub const fn total_blocks(&self) -> u64 {
        // `new` and the associated constants both establish this cannot fail.
        match self.checked_total_blocks() {
            Some(total) => total,
            None => 0,
        }
    }

    /// Bytes in one block.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        self.block_size
    }

    /// Cylinder count.
    #[must_use]
    pub const fn cylinders(&self) -> u32 {
        self.cylinders
    }

    /// Head count.
    #[must_use]
    pub const fn heads(&self) -> u32 {
        self.heads
    }

    /// Sectors per track.
    #[must_use]
    pub const fn sectors(&self) -> u32 {
        self.sectors
    }

    /// Total size in bytes.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_blocks().saturating_mul(self.block_size as u64)
    }

    /// Where this volume's rootblock is located.
    ///
    /// Computed, never read from the bootblock: that field reports 880 even on
    /// HD volumes whose rootblock is at 1760, so it cannot be trusted (C-007).
    ///
    /// Clévy's ADF FAQ §4.2 gives the formula as
    ///
    /// ```text
    /// highKey = numCyls * numSurfaces * numBlocksPerTrack - 1
    /// rootKey = (numReserved + highKey) / 2
    /// ```
    ///
    /// which is *not* half the block count once `reserved` rises above two:
    /// 1000 blocks with four reserved gives 501, not 500. It yields 880 for a
    /// DD floppy and 1760 for HD.
    ///
    /// This is the conventional location, not a guarantee — a rootblock found
    /// here should still be validated (type, secondary type, checksum) rather
    /// than assumed.
    #[must_use]
    pub const fn root_block(&self) -> BlockIndex {
        let high_key = self.total_blocks().saturating_sub(1);
        BlockIndex((self.reserved as u64).saturating_add(high_key) / 2)
    }

    /// Prove that `index` addresses a block inside this geometry.
    ///
    /// The only way to obtain a [`ValidBlock`], and therefore the only way to
    /// reach [`BlockSource::read_block`] (AV-004).
    pub const fn validate(&self, index: BlockIndex) -> Result<ValidBlock, BlockError> {
        let total = self.total_blocks();
        if index.0 < total {
            Ok(ValidBlock { index: index.0 })
        } else {
            Err(BlockError::OutOfRange { index, total })
        }
    }
}

/// A failure reading a block, carrying enough context to report where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// The address lies outside the volume (AV-004).
    OutOfRange {
        /// The rejected address.
        index: BlockIndex,
        /// How many blocks the volume actually holds.
        total: u64,
    },
    /// The caller's buffer is the wrong size for this geometry's blocks.
    BufferSize {
        /// Bytes the buffer holds.
        got: usize,
        /// Bytes a block needs.
        want: usize,
    },
    /// The backing store ended early — a truncated image.
    Truncated {
        /// The block that could not be fully read.
        index: BlockIndex,
    },
    /// The backing store failed for a reason it must describe itself.
    Backing {
        /// The block being read when the failure occurred.
        index: BlockIndex,
        /// What the backing store reported.
        detail: String,
    },
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { index, total } => {
                write!(f, "{index} is outside the volume ({total} blocks)")
            }
            Self::BufferSize { got, want } => {
                write!(f, "buffer is {got} bytes, need {want}")
            }
            Self::Truncated { index } => write!(f, "image ends part-way through {index}"),
            Self::Backing { index, detail } => write!(f, "reading {index}: {detail}"),
        }
    }
}

impl core::error::Error for BlockError {}

/// A source of fixed-size blocks.
///
/// Implemented *below* this crate — by the container front-end and the track
/// codec — and consumed *above* it by the filesystem layer. Neither side is a
/// dependency of the other.
pub trait BlockSource {
    /// The shape of the volume this source presents.
    fn geometry(&self) -> &Geometry;

    /// Read one block into `out`.
    ///
    /// Takes a [`ValidBlock`], so an implementation never has to re-check the
    /// address and can never be handed an unchecked one.
    ///
    /// Implementations must fill `out` completely or return an error; a
    /// partial read is [`BlockError::Truncated`].
    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError>;
}

/// Read a block by raw index, validating it first.
///
/// The ordinary entry point for upper layers: it performs the AV-004 check and
/// then dispatches. Callers that already hold a [`ValidBlock`] can call
/// [`BlockSource::read_block`] directly.
pub fn read_at<S: BlockSource + ?Sized>(
    source: &S,
    index: BlockIndex,
    out: &mut [u8],
) -> Result<(), BlockError> {
    let geometry = source.geometry();
    let want = geometry.block_size() as usize;
    if out.len() != want {
        return Err(BlockError::BufferSize {
            got: out.len(),
            want,
        });
    }
    let valid = geometry.validate(index)?;
    source.read_block(valid, out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn dd_floppy_is_880k() {
        let g = Geometry::DD_FLOPPY;
        assert_eq!(g.total_blocks(), 1760);
        assert_eq!(g.total_bytes(), 901_120, "the canonical 880 KB ADF");
        assert_eq!(g.root_block(), BlockIndex(880));
    }

    #[test]
    fn hd_floppy_is_1760k() {
        let g = Geometry::HD_FLOPPY;
        assert_eq!(g.total_bytes(), 1_802_240);
        // The bootblock claims 880 here; the real rootblock is at 1760 (C-007).
        assert_eq!(g.root_block(), BlockIndex(1760));
    }

    #[test]
    fn root_block_follows_the_documented_formula_not_the_midpoint() {
        // BUG-002. (reserved + total - 1) / 2 diverges from total / 2 as soon
        // as `reserved` rises above two — the case RDB partitions will hit.
        let g = Geometry::new(25, 1, 40, 512, 4).unwrap();
        assert_eq!(g.total_blocks(), 1000);
        assert_eq!(g.root_block(), BlockIndex(501), "not 500");

        // ...while staying correct for the floppy geometries, where the two
        // formulas happen to coincide.
        for g in [Geometry::DD_FLOPPY, Geometry::HD_FLOPPY] {
            assert_eq!(g.root_block().0, g.total_blocks() / 2);
        }
    }

    #[test]
    fn rejects_degenerate_geometry() {
        assert_eq!(
            Geometry::new(0, 2, 11, 512, 2),
            Err(GeometryError::ZeroDimension)
        );
        assert_eq!(
            Geometry::new(80, 2, 11, 0, 2),
            Err(GeometryError::ZeroDimension)
        );
        assert_eq!(
            Geometry::new(u32::MAX, u32::MAX, u32::MAX, 512, 2),
            Err(GeometryError::Overflow)
        );
        assert_eq!(
            Geometry::new(1, 1, 4, 512, 4),
            Err(GeometryError::ReservedExceedsVolume),
            "reserving the whole volume leaves nothing to address"
        );
    }

    #[test]
    fn validate_admits_only_in_range_blocks() {
        let g = Geometry::DD_FLOPPY;
        assert_eq!(g.validate(BlockIndex(0)).unwrap().index(), 0);
        assert_eq!(g.validate(BlockIndex(1759)).unwrap().index(), 1759);
        // AV-004: one past the end, and a wildly hostile pointer.
        assert!(matches!(
            g.validate(BlockIndex(1760)),
            Err(BlockError::OutOfRange { total: 1760, .. })
        ));
        assert!(g.validate(BlockIndex(u64::MAX)).is_err());
    }

    /// A source that would happily read anything — the point being that it
    /// never gets the chance, because it cannot be reached without a
    /// `ValidBlock`.
    struct Fake {
        geometry: Geometry,
    }

    impl BlockSource for Fake {
        fn geometry(&self) -> &Geometry {
            &self.geometry
        }
        fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
            out.fill(u8::try_from(block.index() % 256).unwrap_or(0));
            Ok(())
        }
    }

    #[test]
    fn read_at_checks_bounds_before_dispatching() {
        let src = Fake {
            geometry: Geometry::DD_FLOPPY,
        };
        let mut buf = [0u8; 512];

        read_at(&src, BlockIndex(3), &mut buf).unwrap();
        assert_eq!(buf[0], 3);

        assert!(matches!(
            read_at(&src, BlockIndex(1760), &mut buf),
            Err(BlockError::OutOfRange { .. })
        ));
    }

    #[test]
    fn read_at_rejects_a_mis_sized_buffer() {
        let src = Fake {
            geometry: Geometry::DD_FLOPPY,
        };
        let mut buf = [0u8; 256];
        assert_eq!(
            read_at(&src, BlockIndex(0), &mut buf),
            Err(BlockError::BufferSize {
                got: 256,
                want: 512
            })
        );
    }
}
