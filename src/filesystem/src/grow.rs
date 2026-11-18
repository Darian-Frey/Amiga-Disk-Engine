//! Growing a volume into blocks added after it (F-038, D-024 part 5).
//!
//! AmigaDOS keeps no pointer to a volume's rootblock: it computes it, from
//! the partition's size, as the block in the middle (C-007). A partition
//! that grows therefore has its rootblock somewhere else, and everything
//! that names the old one has to follow. That is a short list, and this
//! module is it:
//!
//! - the rootblock, copied to its new place;
//! - each entry directly in the root, whose header names the root as its
//!   parent, and on a directory-cache volume the root's cache blocks, which
//!   name it too;
//! - the bitmap, which grows to map the new blocks — new pages, and a
//!   `bm_ext` chain past 25 — written where nothing is, with every block that
//!   was in use still in use and the old rootblock and pages free.
//!
//! Nothing else on an AmigaDOS volume refers to the root. Where the new
//! rootblock would land on a block something uses, growing is refused rather
//! than moving file data out of the way. The old rootblock is left as it
//! was, free, as a format leaves what it replaces.

use std::collections::HashSet;

use ade_block::{Geometry, checksum};
use ade_endian::{put_u32, u32_at};

use crate::bitmap::Bitmap;
use crate::edit::Writes;

/// Why a volume cannot grow as asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrowError {
    /// The new size is not larger, or not a size this volume can have.
    Size(String),
    /// The block the rootblock must move to is in use.
    RootBlockInUse(u32),
    /// The volume's own structures cannot be read well enough to move.
    Unreadable(String),
    /// No free blocks for the bitmap the larger volume needs.
    Full,
}

impl core::fmt::Display for GrowError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Size(why) | Self::Unreadable(why) => f.write_str(why),
            Self::RootBlockInUse(block) => write!(
                f,
                "grown, the volume's rootblock would be at block {block}, which a file or \
                 directory uses; AmigaDOS puts the rootblock in the middle of a partition, \
                 so free that block first"
            ),
            Self::Full => f.write_str("there are no free blocks for the larger bitmap"),
        }
    }
}

impl core::error::Error for GrowError {}

/// The writes that make the volume in `image` — the partition's bytes at
/// its new size, numbered by `new` — the volume of `new` it grows into, from
/// the volume of `old` it is: the rootblock moved, what names it updated, the
/// bitmap grown. `in_use` is every block the volume's tree reaches, as the
/// health report finds them; blocks the old bitmap marks used and nothing
/// reaches stay used, since a game may keep data there.
///
/// # Errors
/// [`GrowError`] for a size that does not grow, a rootblock that would land
/// on a block in use, a bitmap or root that cannot be read, or no room for
/// the larger bitmap.
pub fn grow_writes<S: std::hash::BuildHasher>(
    image: &[u8],
    old: Geometry,
    new: Geometry,
    in_use: &HashSet<u32, S>,
) -> Result<Writes, GrowError> {
    if new.total_blocks() <= old.total_blocks()
        || old.block_size() != new.block_size()
        || old.reserved() != new.reserved()
    {
        return Err(GrowError::Size(
            "a volume grows only into more blocks of the same size".to_owned(),
        ));
    }
    let bsize = new.block_size() as usize;
    let root = u32::try_from(old.root_block().0).unwrap_or(u32::MAX);
    let moved = u32::try_from(new.root_block().0).unwrap_or(u32::MAX);
    let root_raw = read(image, root, bsize)?;
    let rootblock = crate::rootblock::Rootblock::parse(&root_raw)
        .map_err(|e| GrowError::Unreadable(format!("the rootblock: {e}")))?;
    let old_bytes = image
        .get(..usize::try_from(old.total_bytes()).unwrap_or(0))
        .unwrap_or(&[]);
    let bitmap = Bitmap::read(&crate::edit::Draft::new(old_bytes, old), &old, &rootblock)
        .map_err(|e| GrowError::Unreadable(format!("the bitmap: {e}")))?;
    if bitmap.incomplete || !bitmap.bad_checksums.is_empty() {
        return Err(GrowError::Unreadable(
            "the bitmap is not whole, so which blocks are free is not known; repair the disk \
             first"
                .to_owned(),
        ));
    }
    if moved != root && (in_use.contains(&moved) || bitmap.is_allocated(moved)) {
        return Err(GrowError::RootBlockInUse(moved));
    }

    // What the grown volume uses: everything in use before, but the old
    // rootblock and bitmap, and the rootblock where it now is.
    let mut used: HashSet<u32> = in_use.iter().chain(bitmap.allocated()).copied().collect();
    used.remove(&root);
    for b in bitmap.blocks.iter().chain(&bitmap.extensions) {
        used.remove(b);
    }
    used.insert(moved);
    let (pages, extensions) = place_bitmap(new, moved, &mut used)?;

    let mut writes = bitmap_writes(new, &used, &pages, &extensions);
    writes.insert(moved, moved_root(&root_raw, bsize, &pages, &extensions));
    let first_cache = u32_at(&root_raw, bsize.saturating_sub(8)).unwrap_or(0);
    reparent(image, new, &root_raw, first_cache, moved, &mut writes)?;
    Ok(writes)
}

/// A block of `image`.
fn read(image: &[u8], b: u32, bsize: usize) -> Result<Vec<u8>, GrowError> {
    let at = (b as usize).saturating_mul(bsize);
    image
        .get(at..at.saturating_add(bsize))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| GrowError::Unreadable(format!("block {b} is past the volume")))
}

/// Blocks per bitmap page: every long but the checksum, 32 bits each.
fn bits_per_page(bsize: usize) -> u64 {
    u64::try_from((bsize / 4).saturating_sub(1))
        .unwrap_or(1)
        .saturating_mul(32)
        .max(1)
}

/// Where the larger bitmap goes — its pages, then its `bm_ext` blocks — in
/// blocks nothing uses, after the rootblock first as Format puts it; each
/// added to `used`.
fn place_bitmap(
    new: Geometry,
    root: u32,
    used: &mut HashSet<u32>,
) -> Result<(Vec<u32>, Vec<u32>), GrowError> {
    let bsize = new.block_size() as usize;
    let mapped = new.total_blocks().saturating_sub(u64::from(new.reserved()));
    let pages = usize::try_from(mapped.div_ceil(bits_per_page(bsize))).unwrap_or(usize::MAX);
    let per_extension = (bsize / 4).saturating_sub(1).max(1);
    let extensions = pages.saturating_sub(25).div_ceil(per_extension);
    let total = u32::try_from(new.total_blocks()).unwrap_or(u32::MAX);
    let mut free = (root.saturating_add(1)..total)
        .chain(new.reserved()..root)
        .filter(|b| !used.contains(b));
    let mut take = |n: usize| -> Result<Vec<u32>, GrowError> {
        (0..n).map(|_| free.next().ok_or(GrowError::Full)).collect()
    };
    let page_blocks = take(pages)?;
    let extension_blocks = take(extensions)?;
    used.extend(page_blocks.iter().chain(&extension_blocks));
    Ok((page_blocks, extension_blocks))
}

/// The bitmap pages for `used`, and the `bm_ext` chain listing those past the
/// rootblock's 25. Bits past the volume's end are cleared, as a format
/// leaves them.
fn bitmap_writes(new: Geometry, used: &HashSet<u32>, pages: &[u32], extensions: &[u32]) -> Writes {
    let bsize = new.block_size() as usize;
    let per_page = bits_per_page(bsize);
    let mapped = new.total_blocks().saturating_sub(u64::from(new.reserved()));
    let mut writes = Writes::new();
    for (i, (b, mut raw)) in Bitmap::rebuild(used, &new, pages).into_iter().enumerate() {
        let first = (i as u64).saturating_mul(per_page);
        for bit in mapped.saturating_sub(first).min(per_page)..per_page {
            let word = usize::try_from(bit / 32).unwrap_or(0);
            let offset = word.saturating_mul(4).saturating_add(4);
            if let Ok(value) = u32_at(&raw, offset) {
                let _ = put_u32(&mut raw, offset, value & !(1u32 << (bit % 32)));
            }
        }
        let _ = put_u32(&mut raw, 0, 0);
        if let Some(sum) = checksum::normal_at(&raw, 0) {
            let _ = put_u32(&mut raw, 0, sum);
        }
        writes.insert(b, raw);
    }
    let per_extension = (bsize / 4).saturating_sub(1).max(1);
    for (i, &ext) in extensions.iter().enumerate() {
        let mut raw = vec![0u8; bsize];
        let listed = pages
            .iter()
            .skip(25usize.saturating_add(i.saturating_mul(per_extension)))
            .take(per_extension);
        for (slot, &page) in listed.enumerate() {
            let _ = put_u32(&mut raw, slot.saturating_mul(4), page);
        }
        let next = extensions.get(i.saturating_add(1)).copied().unwrap_or(0);
        let _ = put_u32(&mut raw, bsize.saturating_sub(4), next);
        writes.insert(ext, raw);
    }
    writes
}

/// The rootblock as it is, naming the new bitmap and marked valid.
fn moved_root(root: &[u8], bsize: usize, pages: &[u32], extensions: &[u32]) -> Vec<u8> {
    let mut raw = root.to_vec();
    let first_page = bsize.saturating_sub(196);
    for slot in 0..25usize {
        let page = pages.get(slot).copied().unwrap_or(0);
        let _ = put_u32(
            &mut raw,
            first_page.saturating_add(slot.saturating_mul(4)),
            page,
        );
    }
    let first_extension = extensions.first().copied().unwrap_or(0);
    let _ = put_u32(&mut raw, bsize.saturating_sub(96), first_extension);
    let _ = put_u32(&mut raw, bsize.saturating_sub(200), u32::MAX); // bm_flag: valid
    reseal(&mut raw);
    raw
}

/// Point whatever names the root as its parent at `moved`: each entry in its
/// hash chains, and each block of its directory cache from `first_cache`.
fn reparent(
    image: &[u8],
    new: Geometry,
    root: &[u8],
    first_cache: u32,
    moved: u32,
    writes: &mut Writes,
) -> Result<(), GrowError> {
    let bsize = new.block_size() as usize;
    let total = u32::try_from(new.total_blocks()).unwrap_or(u32::MAX);
    let slots = usize::try_from(u32_at(root, 0x0c).unwrap_or(0)).unwrap_or(0);
    let mut seen: HashSet<u32> = HashSet::new();
    for slot in 0..slots.min(bsize / 4) {
        let mut next = u32_at(root, slot.saturating_mul(4).saturating_add(24)).unwrap_or(0);
        while next != 0 && next < total && seen.insert(next) {
            let mut raw = match writes.get(&next) {
                Some(raw) => raw.clone(),
                None => read(image, next, bsize)?,
            };
            let _ = put_u32(&mut raw, bsize.saturating_sub(12), moved);
            reseal(&mut raw);
            let chain = u32_at(&raw, bsize.saturating_sub(16)).unwrap_or(0);
            writes.insert(next, raw);
            next = chain;
        }
    }
    let mut cache = first_cache;
    while cache != 0 && cache < total && seen.insert(cache) {
        let mut raw = read(image, cache, bsize)?;
        if u32_at(&raw, 0).ok() != Some(crate::dircache::T_DIRCACHE) {
            break;
        }
        let _ = put_u32(&mut raw, 0x08, moved);
        let _ = put_u32(&mut raw, 0x14, 0);
        if let Some(sum) = checksum::normal_at(&raw, 0x14) {
            let _ = put_u32(&mut raw, 0x14, sum);
        }
        let next = u32_at(&raw, 0x10).unwrap_or(0);
        writes.insert(cache, raw);
        cache = next;
    }
    Ok(())
}

/// Set a header block's checksum, at 20.
fn reseal(raw: &mut [u8]) {
    let _ = put_u32(raw, 20, 0);
    if let Some(sum) = checksum::normal_at(raw, 20) {
        let _ = put_u32(raw, 20, sum);
    }
}
