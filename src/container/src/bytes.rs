//! Bytes read by position, wherever they are kept (IMP-029).
//!
//! Inspecting or examining an image reads a few blocks here and there and
//! never needs to own it. Written against this, the same code serves an image
//! in memory and one too large to want there.

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock};

/// Bytes that can be read from any offset.
pub trait ByteSource {
    /// How many there are.
    fn size(&self) -> u64;

    /// Fill `out` from `offset`; false, with `out` unspecified, if every byte
    /// of it could not be read.
    #[must_use]
    fn read_at(&self, offset: u64, out: &mut [u8]) -> bool;

    /// Up to `want` bytes from `offset`: fewer only at the end, and none if
    /// they cannot be read.
    #[must_use]
    fn range(&self, offset: u64, want: usize) -> Vec<u8> {
        let left = self.size().saturating_sub(offset);
        let len = usize::try_from(left).unwrap_or(usize::MAX).min(want);
        let mut out = vec![0u8; len];
        if self.read_at(offset, &mut out) {
            out
        } else {
            Vec::new()
        }
    }
}

impl ByteSource for [u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> bool {
        let Some(src) = usize::try_from(offset)
            .ok()
            .and_then(|at| self.get(at..at.checked_add(out.len())?))
        else {
            return false;
        };
        out.copy_from_slice(src);
        true
    }
}

impl ByteSource for &[u8] {
    fn size(&self) -> u64 {
        (**self).size()
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> bool {
        (**self).read_at(offset, out)
    }
}

/// Bytes presented as blocks of a geometry.
pub struct Blocks<'a> {
    bytes: &'a dyn ByteSource,
    geometry: Geometry,
}

impl<'a> Blocks<'a> {
    /// Present `bytes` as blocks.
    ///
    /// # Errors
    /// [`BlockError::Truncated`] if the bytes do not cover the geometry.
    pub fn new(bytes: &'a dyn ByteSource, geometry: Geometry) -> Result<Self, BlockError> {
        if bytes.size() < geometry.total_bytes() {
            return Err(BlockError::Truncated {
                index: BlockIndex(
                    bytes
                        .size()
                        .checked_div(u64::from(geometry.block_size()))
                        .unwrap_or(0),
                ),
            });
        }
        Ok(Self { bytes, geometry })
    }
}

impl BlockSource for Blocks<'_> {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        let size = self.geometry.block_size();
        if out.len() != size as usize {
            return Err(BlockError::BufferSize {
                got: out.len(),
                want: size as usize,
            });
        }
        let offset = block.index().saturating_mul(u64::from(size));
        if self.bytes.read_at(offset, out) {
            Ok(())
        } else {
            Err(BlockError::Truncated {
                index: BlockIndex(block.index()),
            })
        }
    }
}

/// A block source's blocks, presented as the run of bytes they make.
pub struct Flat<'a>(pub &'a dyn BlockSource);

impl ByteSource for Flat<'_> {
    fn size(&self) -> u64 {
        self.0.geometry().total_bytes()
    }

    fn read_at(&self, offset: u64, out: &mut [u8]) -> bool {
        let size = u64::from(self.0.geometry().block_size());
        let mut block = vec![0u8; self.0.geometry().block_size() as usize];
        let mut done = 0usize;
        while done < out.len() {
            let at = offset.saturating_add(done as u64);
            let (Some(index), Some(within)) = (at.checked_div(size), at.checked_rem(size)) else {
                return false;
            };
            if ade_block::read_at(self.0, BlockIndex(index), &mut block).is_err() {
                return false;
            }
            let from = usize::try_from(within).unwrap_or(usize::MAX);
            let Some(src) = block.get(from..) else {
                return false;
            };
            let Some(dst) = out.get_mut(done..) else {
                return false;
            };
            let n = src.len().min(dst.len());
            if let (Some(d), Some(s)) = (dst.get_mut(..n), src.get(..n)) {
                d.copy_from_slice(s);
            }
            if n == 0 {
                return false;
            }
            done = done.saturating_add(n);
        }
        true
    }
}
