//! The allocation bitmap.
//!
//! One bit per block: **set means free, cleared means allocated** — the
//! opposite of the usual convention, and an easy inversion to make that would
//! mis-report every block on every disk (ADF FAQ §4.3).
//!
//! The map does not start at block 0. Reserved blocks — the boot blocks — are
//! excluded, so the bit for block *n* is at index `n - reserved`.
//!
//! # The valid flag is advisory (AV-003)
//!
//! `bm_flag` in the rootblock is -1 when the bitmap is trustworthy. The Linux
//! AFFS documentation warns it "may not be accurate when the system crashes
//! while an affs partition is mounted", so a cleared flag is routine rather
//! than sinister — 260 of 4652 real images have one. ADE treats it as an
//! observation and can check the bitmap against reality instead of believing
//! either answer.

use std::collections::HashSet;

use ade_block::{BlockIndex, BlockSource, Geometry, checksum, read_at};
use ade_endian::u32_at;

use crate::{rootblock::Rootblock, volume::FsError};

/// Bitmap pointers stored directly in the rootblock.
pub const ROOT_BITMAP_POINTERS: usize = 25;

/// The allocation bitmap, as read from disk.
#[derive(Debug, Clone)]
pub struct Bitmap {
    /// Blocks the bitmap says are in use.
    allocated: HashSet<u32>,
    /// Blocks covered by the map at all.
    covered: u32,
    /// The bitmap blocks themselves, in order.
    pub blocks: Vec<u32>,
    /// Bitmap blocks whose checksum did not verify.
    pub bad_checksums: Vec<u32>,
    /// Whether the rootblock's `bm_flag` said the map is valid.
    pub flagged_valid: bool,
    /// Whether the map ran out before covering the volume.
    pub incomplete: bool,
}

impl Bitmap {
    /// Read the bitmap for a volume.
    ///
    /// Follows the rootblock's 25 direct pointers and then the `bm_ext` chain,
    /// carrying a visited set: an extension chain loops as readily as any other
    /// (AV-001), and bitmap extension blocks carry **no checksum** to catch it.
    ///
    /// # Errors
    /// A read error. A short, looping or corrupt map is reported through the
    /// returned value rather than as an error — the bitmap being wrong is the
    /// most ordinary finding there is.
    pub fn read(
        source: &dyn BlockSource,
        geometry: &Geometry,
        root: &Rootblock,
    ) -> Result<Self, FsError> {
        let bsize = geometry.block_size() as usize;
        let reserved = geometry.reserved();
        let total = geometry.total_blocks();
        let covered = u32::try_from(total.saturating_sub(u64::from(reserved))).unwrap_or(u32::MAX);
        // Each bitmap block holds (BSIZE/4 - 1) longs of 32 bits.
        let bits_per_block = bsize.saturating_div(4).saturating_sub(1).saturating_mul(32);

        let mut pointers = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        for &p in &root.bitmap_pages {
            if p != 0 {
                pointers.push(p);
            }
        }

        // The extension chain, for volumes needing more than 25 bitmap blocks.
        let mut next_ext = root.bitmap_extension;
        let mut buf = vec![0u8; bsize];
        while next_ext != 0 && seen.insert(next_ext) {
            if geometry.validate(BlockIndex(u64::from(next_ext))).is_err() {
                break;
            }
            if read_at(source, BlockIndex(u64::from(next_ext)), &mut buf).is_err() {
                break;
            }
            let slots = bsize.saturating_div(4).saturating_sub(1);
            for i in 0..slots {
                if let Ok(p) = u32_at(&buf, i.saturating_mul(4))
                    && p != 0
                {
                    pointers.push(p);
                }
            }
            // Bitmap extension blocks carry no checksum, so a corrupt `next`
            // is only caught by the visited set and the bounds check.
            next_ext = u32_at(&buf, bsize.saturating_sub(4)).unwrap_or(0);
        }

        let mut allocated = HashSet::new();
        let mut bad_checksums = Vec::new();
        let mut bit_base: u64 = 0;
        for &bm in &pointers {
            if geometry.validate(BlockIndex(u64::from(bm))).is_err() {
                continue;
            }
            if read_at(source, BlockIndex(u64::from(bm)), &mut buf).is_err() {
                continue;
            }
            if !checksum::sums_to_zero(&buf) {
                bad_checksums.push(bm);
            }
            for word in 0..bsize.saturating_div(4).saturating_sub(1) {
                let offset = 4usize.saturating_add(word.saturating_mul(4));
                let Ok(bits) = u32_at(&buf, offset) else {
                    continue;
                };
                for bit in 0..32u32 {
                    // Set means FREE. Only cleared bits are recorded.
                    if bits & (1 << bit) != 0 {
                        continue;
                    }
                    let index = bit_base
                        .saturating_add((word as u64).saturating_mul(32))
                        .saturating_add(u64::from(bit));
                    let block = index.saturating_add(u64::from(reserved));
                    if block < total
                        && let Ok(b) = u32::try_from(block)
                    {
                        allocated.insert(b);
                    }
                }
            }
            bit_base = bit_base.saturating_add(bits_per_block as u64);
        }

        Ok(Self {
            allocated,
            covered,
            blocks: pointers,
            bad_checksums,
            flagged_valid: root.bitmap_flag_valid(),
            incomplete: bit_base < u64::from(covered),
        })
    }

    /// Build the bitmap a volume *should* have, from the blocks its tree
    /// actually reaches.
    ///
    /// Returns the corrected contents for each bitmap block, paired with the
    /// block number it belongs in — **in memory**. Nothing is written.
    ///
    /// # Why this stops short of applying
    ///
    /// D-004 ships every write path only after its read path is proven, and is
    /// marked never-reversible within v1. AV-003 asks for a rebuild to be
    /// *offered*, which is this; putting it back on a disk is a write and waits
    /// for Phase 4. The separation is not bureaucratic — a bitmap rebuilt from
    /// a misread tree, written to the only copy of a disk, destroys exactly
    /// what the tool exists to preserve.
    ///
    /// The result is still checkable without writing anything: feed it back
    /// through [`Self::read`] and it must describe the set it was built from.
    #[must_use]
    pub fn rebuild(
        allocated: &HashSet<u32>,
        geometry: &Geometry,
        bitmap_blocks: &[u32],
    ) -> Vec<(u32, Vec<u8>)> {
        let bsize = geometry.block_size() as usize;
        let reserved = geometry.reserved();
        let words = bsize.saturating_div(4).saturating_sub(1);
        let bits_per_block = words.saturating_mul(32);

        let mut out = Vec::with_capacity(bitmap_blocks.len());
        for (i, &bm) in bitmap_blocks.iter().enumerate() {
            let mut buf = vec![0u8; bsize];
            // A SET bit means FREE, so start with everything free and clear
            // the bits for blocks in use. Getting this inverted would mark
            // every disk entirely full.
            if let Some(map) = buf.get_mut(4..) {
                map.fill(0xFF);
            }
            let base = i.saturating_mul(bits_per_block);
            for word in 0..words {
                let mut bits: u32 = 0xFFFF_FFFF;
                for bit in 0..32usize {
                    let index = base
                        .saturating_add(word.saturating_mul(32))
                        .saturating_add(bit);
                    let Ok(index) = u32::try_from(index) else {
                        continue;
                    };
                    let Some(block) = index.checked_add(reserved) else {
                        continue;
                    };
                    if allocated.contains(&block) {
                        bits &= !(1u32 << bit);
                    }
                }
                let offset = 4usize.saturating_add(word.saturating_mul(4));
                let _ = ade_endian::put_u32(&mut buf, offset, bits);
            }
            // The bitmap block is the one exception to block layout: checksum
            // at 0, map from 4 (BUG-004).
            if let Some(ck) = checksum::normal_at(&buf, checksum::BITMAP_OFFSET) {
                let _ = ade_endian::put_u32(&mut buf, checksum::BITMAP_OFFSET, ck);
            }
            out.push((bm, buf));
        }
        out
    }

    /// Blocks the bitmap marks used that the tree does not reach.
    ///
    /// Lost space, or deleted files whose blocks were never freed.
    #[must_use]
    pub fn orphaned(&self, reachable: &HashSet<u32>) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .allocated
            .iter()
            .filter(|b| !reachable.contains(b))
            .copied()
            .collect();
        v.sort_unstable();
        v
    }

    /// Blocks the tree reaches that the bitmap marks free.
    ///
    /// The dangerous direction: the next write would put something else there.
    #[must_use]
    pub fn referenced_but_free(&self, reachable: &HashSet<u32>) -> Vec<u32> {
        let mut v: Vec<u32> = reachable
            .iter()
            .filter(|b| !self.allocated.contains(b))
            .copied()
            .collect();
        v.sort_unstable();
        v
    }

    /// Whether the bitmap says this block is in use.
    #[must_use]
    pub fn is_allocated(&self, block: u32) -> bool {
        self.allocated.contains(&block)
    }

    /// Every block the bitmap says is in use.
    #[must_use]
    pub fn allocated(&self) -> &HashSet<u32> {
        &self.allocated
    }

    /// How many blocks are marked in use.
    #[must_use]
    pub fn used_count(&self) -> usize {
        self.allocated.len()
    }

    /// How many blocks the map covers.
    #[must_use]
    pub const fn covered(&self) -> u32 {
        self.covered
    }

    /// Proportion of the volume in use, 0.0 to 1.0.
    #[must_use]
    pub fn fill_ratio(&self) -> f64 {
        if self.covered == 0 {
            return 0.0;
        }
        // A block count cannot exceed u32, so the conversion is exact.
        let used = u32::try_from(self.allocated.len()).unwrap_or(u32::MAX);
        f64::from(used) / f64::from(self.covered)
    }
}
