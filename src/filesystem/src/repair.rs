//! Repairing a volume's allocation bitmap.
//!
//! The one repair here is the one AmigaDOS makes itself: its validator, run
//! when a disk's bitmap is flagged invalid, walks the tree and writes the map
//! the tree implies. ADE does the same with two differences, both towards
//! keeping what is on the disk.
//!
//! - **What is in use comes from the caller**, as a set of blocks. The layer
//!   above passes the set its health report is built on, so the repaired map
//!   and the report that judges it cannot disagree about what a file owns.
//! - **Blocks marked used stay used** unless the caller asks otherwise. A
//!   block nothing in the tree reaches is not necessarily free: a trackloaded
//!   game keeps its data outside any file and marks it used so AmigaDOS never
//!   writes there. AmigaDOS's validator frees such blocks; ADE frees them only
//!   when told to.
//!
//! Nothing here touches a datestamp. A repair puts back what the volume's own
//! structures already say, rather than making a change to its contents.

use std::collections::HashSet;
use std::hash::BuildHasher;

use ade_block::{BlockIndex, Geometry, checksum};
use ade_endian::{put_u32, u32_at};

use crate::{
    bitmap::Bitmap,
    dostype::Mode,
    edit::{EditError, Slice, apply, put_back, seal},
    rootblock::BITMAP_VALID,
    volume::{FsError, Volume},
};

/// What a bitmap repair changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitmapRepair {
    /// Blocks in use that the map had marked free, now marked used.
    pub now_used: Vec<u32>,
    /// Blocks nothing reaches that the map had marked used, now marked free.
    /// Always empty unless orphans were asked to be freed.
    pub now_free: Vec<u32>,
    /// Bitmap blocks rewritten.
    pub rewritten: Vec<u32>,
    /// Whether the rootblock's bitmap-valid flag was set.
    pub flag_set: bool,
}

impl BitmapRepair {
    /// Whether anything was changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rewritten.is_empty() && !self.flag_set
    }
}

/// Make the bitmap mark every block in `in_use` as used, seal every bitmap
/// block, and set the rootblock's flag saying the map can be trusted.
///
/// `in_use` is every block the volume's tree reaches; the rootblock and the
/// bitmap's own blocks are added here. With `free_orphans`, any other block
/// the map marks used is freed, as AmigaDOS's validator would; without it,
/// those stay as they are.
///
/// A volume that needs nothing is left alone and an empty repair returned.
///
/// # Errors
/// [`EditError::Unsupported`] for a long-name volume (D-013) or one whose
/// bitmap needs an extension chain, which ADE does not write;
/// [`EditError::Damaged`] when a bitmap pointer is outside the volume or
/// names a block something else uses, or when the flag needs setting on a
/// rootblock that already fails its checksum — setting it would reseal the
/// block and hide that; [`EditError::BitmapUntrusted`] when the bitmap does
/// not cover the volume. The image is unchanged whenever one is returned.
pub fn rebuild_bitmap<S: BuildHasher>(
    image: &mut [u8],
    geometry: Geometry,
    in_use: &HashSet<u32, S>,
    free_orphans: bool,
) -> Result<BitmapRepair, EditError> {
    let (repair, writes) = {
        let source = Slice { image, geometry };
        let volume = Volume::mount(&source)?;
        if volume
            .dostype()
            .is_some_and(|d| d.mode() == Mode::LongNames)
        {
            return Err(EditError::Unsupported(
                "this is a long-name volume, which ADE does not write (D-013)".to_owned(),
            ));
        }
        let root = volume.root();
        let bitmap = Bitmap::read(&source, &geometry, volume.rootblock())?;
        let pages = bitmap_layout(&bitmap, &volume, &geometry, in_use)?;
        let reserved = geometry.reserved();
        let total = geometry.total_blocks();
        let inside = |b: u32| b >= reserved && u64::from(b) < total;

        let mut target: HashSet<u32> = in_use.iter().copied().filter(|&b| inside(b)).collect();
        target.insert(root);
        target.extend(pages.iter().copied());
        if !free_orphans {
            target.extend(bitmap.allocated().iter().copied());
        }
        let old = bitmap.allocated();
        let mut now_used: Vec<u32> = target.difference(old).copied().collect();
        let mut now_free: Vec<u32> = old.difference(&target).copied().collect();
        now_used.sort_unstable();
        now_free.sort_unstable();

        let mut repair = BitmapRepair {
            now_used,
            now_free,
            ..BitmapRepair::default()
        };
        let mut writes = std::collections::BTreeMap::new();
        // Rewritten when the map is wrong or a block of it fails its checksum;
        // a sound map is left byte for byte as it was.
        if !repair.now_used.is_empty()
            || !repair.now_free.is_empty()
            || !bitmap.bad_checksums.is_empty()
        {
            let per_page = (geometry.block_size() as usize / 4)
                .saturating_sub(1)
                .saturating_mul(32);
            for (i, &page) in bitmap.blocks.iter().enumerate() {
                let old_raw = read(&source, page)?;
                let raw = remap(&old_raw, i.saturating_mul(per_page), &geometry, &target);
                if raw != old_raw {
                    repair.rewritten.push(page);
                    writes.insert(page, raw);
                }
            }
        }
        if !bitmap.flagged_valid {
            let mut raw = read(&source, root)?;
            if !checksum::normal_valid(&raw) {
                return Err(EditError::Damaged(
                    "the rootblock fails its checksum; setting its bitmap flag would reseal it \
                     and hide that"
                        .to_owned(),
                ));
            }
            let _ = put_back(&mut raw, 200, BITMAP_VALID);
            seal(&mut raw);
            writes.insert(root, raw);
            repair.flag_set = true;
        }
        (repair, writes)
    };
    apply(image, geometry, &writes);
    Ok(repair)
}

/// The bitmap's own blocks, if the map is laid out where a repair can write
/// it: every block inside the volume, none named twice or used by the tree,
/// no extension chain, and the whole volume covered.
fn bitmap_layout<S: BuildHasher>(
    bitmap: &Bitmap,
    volume: &Volume<'_>,
    geometry: &Geometry,
    in_use: &HashSet<u32, S>,
) -> Result<HashSet<u32>, EditError> {
    // Before the extension check, whose reason would be wrong for a pointer
    // that is noise rather than a chain (BUG-015).
    if let Some(bad) = bitmap.out_of_range.first() {
        return Err(EditError::Damaged(format!(
            "the rootblock names block {bad} for the bitmap, which is outside the volume; \
             ADE does not choose where a bitmap goes"
        )));
    }
    if volume.rootblock().bitmap_extension != 0 {
        return Err(EditError::Unsupported(
            "this volume's bitmap continues in an extension chain, which ADE does not write"
                .to_owned(),
        ));
    }
    let root = volume.root();
    let reserved = geometry.reserved();
    let mut pages: HashSet<u32> = HashSet::new();
    for &page in &bitmap.blocks {
        if page < reserved || page == root {
            return Err(EditError::Damaged(format!(
                "the rootblock names block {page} as part of its bitmap, which is a boot \
                 block or the rootblock itself"
            )));
        }
        if in_use.contains(&page) {
            return Err(EditError::Damaged(format!(
                "bitmap block {page} is also used by the directory tree"
            )));
        }
        if !pages.insert(page) {
            return Err(EditError::Damaged(format!(
                "the rootblock names block {page} twice as part of its bitmap"
            )));
        }
    }
    if bitmap.incomplete || bitmap.blocks.is_empty() {
        return Err(EditError::BitmapUntrusted(
            "the bitmap does not cover the whole volume, so there is nowhere to record some \
             blocks"
                .to_owned(),
        ));
    }
    Ok(pages)
}

/// One bitmap block with the bit for every block of the volume it maps set
/// from `used` — cleared for used, set for free (SPEC §Bitmap) — and
/// resealed. `first` is the index of the first block it maps, counted from
/// the end of the reserved area.
///
/// Bits past the end of the volume are left as they were. They map nothing,
/// and writers disagree about them: ADE's formatter clears them, and
/// [`Bitmap::rebuild`] sets them. A repair has no business choosing.
fn remap(raw: &[u8], first: usize, geometry: &Geometry, used: &HashSet<u32>) -> Vec<u8> {
    let mut out = raw.to_vec();
    let reserved = u64::from(geometry.reserved());
    let total = geometry.total_blocks();
    let words = out.len().saturating_div(4).saturating_sub(1);
    for word in 0..words {
        let at = 4usize.saturating_add(word.saturating_mul(4));
        let Ok(mut bits) = u32_at(&out, at) else {
            continue;
        };
        for bit in 0..32u32 {
            let index = first
                .saturating_add(word.saturating_mul(32))
                .saturating_add(bit as usize) as u64;
            let block = index.saturating_add(reserved);
            if block >= total {
                continue;
            }
            let in_use = u32::try_from(block).is_ok_and(|b| used.contains(&b));
            if in_use {
                bits &= !(1 << bit);
            } else {
                bits |= 1 << bit;
            }
        }
        let _ = put_u32(&mut out, at, bits);
    }
    let _ = put_u32(&mut out, checksum::BITMAP_OFFSET, 0);
    if let Some(sum) = checksum::normal_at(&out, checksum::BITMAP_OFFSET) {
        let _ = put_u32(&mut out, checksum::BITMAP_OFFSET, sum);
    }
    out
}

fn read(source: &Slice<'_>, block: u32) -> Result<Vec<u8>, EditError> {
    let mut raw = vec![0u8; source.geometry.block_size() as usize];
    ade_block::read_at(source, BlockIndex(u64::from(block)), &mut raw).map_err(FsError::Block)?;
    Ok(raw)
}
