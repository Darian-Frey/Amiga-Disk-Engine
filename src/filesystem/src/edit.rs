//! Writing into an existing volume: making a directory, adding a file,
//! deleting an entry, renaming one.
//!
//! # The terms are D-018's
//!
//! This module changes bytes in memory and nothing else. It never sees a file
//! path, so it cannot write over the image a disk was opened from — saving,
//! and the rule that a save never lands on the original, belong to the layer
//! above. What it owes D-018 is part four: **an edit leaves a volume that ADE
//! and ADFlib both accept**, which means doing every piece of bookkeeping a
//! real filesystem does rather than only placing the new block.
//!
//! For one new entry that is: allocate its blocks, write them, insert it into
//! its parent's hash table with the volume's own hash function (C-006), clear
//! its bits in the bitmap, move the parent's and the volume's datestamps, and,
//! on a `DOS\4`/`DOS\5` volume, add it to the parent's directory cache. Skip
//! the last and the health report calls the new entry missing from the cache;
//! skip the bitmap and it calls the new blocks referenced-but-free, which is
//! the finding that means the next write destroys them.
//!
//! # Planned, then applied
//!
//! Every block an edit touches is built in a `Plan` first, reading the
//! volume as it stood. Only when the whole plan exists is anything copied
//! into the image. A refusal halfway through — a full disk, a damaged
//! directory — therefore leaves the image exactly as it was, rather than with
//! a header written and a bitmap not. The layer above relies on this.
//!
//! # Written from SPEC
//!
//! As [`crate::format`] is, and for the same reason: the fixture generator
//! stays independent (D-010), so a misreading here cannot cancel out against
//! it. The layouts are SPEC §User directory block, §File header block, §File
//! extension block, §Data blocks, §Bitmap and §Directory cache blocks.
//!
//! # What it refuses
//!
//! - A volume whose bootblock carries no dostype. Without one ADE cannot know
//!   how names hash, and a guess that is wrong files the entry under a slot
//!   AmigaDOS will never look in.
//! - LNFS (`DOS\6`/`DOS\7`), per D-013 and D-018.
//! - A bitmap that is flagged invalid, fails its checksum, or does not cover
//!   the volume. Allocating from a map ADE already distrusts is how a write
//!   lands on live data.
//! - A parent directory whose listing or cache is damaged. Inserting into a
//!   broken chain extends the damage to the new entry.
//! - A name that already exists, compared the way the volume compares names.
//! - Deleting the root, a directory that is not empty, or anything a hard link
//!   is part of — a link or a file a link points at. Removing one end of a
//!   link chain means relinking the other, which ADE does not write yet.
//!
//! # Deleting leaves the blocks where they are
//!
//! As AmigaDOS does (SPEC §Deletion): the entry is taken out of its parent's
//! hash chain and cache, and its blocks are marked free, but nothing in them
//! is erased. That is what makes a deleted file recoverable until its blocks
//! are reused, and what ADE's own carver reads (F-030).

use std::collections::{BTreeMap, HashSet};

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, ValidBlock, checksum};
use ade_endian::{put_u16, put_u32, u32_at};

use crate::{
    bitmap::Bitmap,
    dircache::{Record, T_DIRCACHE},
    dostype::{FileSystem, Mode},
    entry::{EntryKind, Protection, T_DATA, T_HEADER, T_LIST},
    format::Stamp,
    hash::hash_name,
    volume::{FsError, Volume},
};

/// The longest name a directory entry holds, per SPEC §User directory block.
pub const MAX_NAME: usize = 30;

/// `ST_USERDIR`.
const ST_USERDIR: u32 = 2;
/// `ST_FILE`, which is -3 stored as an unsigned long.
const ST_FILE: u32 = 0xFFFF_FFFD;

/// Bytes of header in a directory cache block before the first record.
const DIRCACHE_HEADER: usize = 24;

/// Why an edit was refused.
///
/// Every variant leaves the image untouched: see the module documentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The volume is not one ADE will write into, and why.
    Unsupported(String),
    /// The parent is not a directory ADE will write into.
    NotADirectory {
        /// The block that was named as the parent.
        block: u32,
    },
    /// The name is empty.
    NameEmpty,
    /// The name is longer than the 30 bytes an entry holds.
    NameTooLong {
        /// How long it was.
        len: usize,
    },
    /// The name holds a byte AmigaDOS does not allow in one.
    NameInvalid {
        /// The offending byte.
        byte: u8,
    },
    /// The entry cannot be deleted or renamed, and why.
    NotRemovable(String),
    /// Something in the parent already has that name.
    Exists {
        /// The name, as stored on the disk.
        name: String,
    },
    /// There are not enough free blocks.
    DiskFull {
        /// Blocks the edit needs.
        needed: usize,
        /// Blocks the bitmap offers.
        free: usize,
    },
    /// The file is larger than AmigaDOS can describe.
    TooLarge {
        /// Its length in bytes.
        len: usize,
    },
    /// The bitmap cannot be trusted to say which blocks are free.
    BitmapUntrusted(String),
    /// The parent directory is damaged, and writing into it would extend the
    /// damage.
    Damaged(String),
    /// A repair was asked for that the volume does not need, and why.
    NotNeeded(String),
    /// Reading the volume failed.
    Fs(FsError),
}

impl core::fmt::Display for EditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported(why)
            | Self::BitmapUntrusted(why)
            | Self::Damaged(why)
            | Self::NotNeeded(why)
            | Self::NotRemovable(why) => f.write_str(why),
            Self::NotADirectory { block } => {
                write!(f, "block {block} is not a directory ADE can write into")
            }
            Self::NameEmpty => f.write_str("a name cannot be empty"),
            Self::NameTooLong { len } => {
                write!(f, "the name is {len} bytes; AmigaDOS holds {MAX_NAME}")
            }
            Self::NameInvalid { byte } => {
                write!(f, "the name contains a byte AmigaDOS forbids: {byte:#04x}")
            }
            Self::Exists { name } => write!(f, "{name:?} already exists there"),
            Self::DiskFull { needed, free } => {
                write!(f, "not enough space: needs {needed} blocks, {free} free")
            }
            Self::TooLarge { len } => {
                write!(f, "{len} bytes is larger than an AmigaDOS file can be")
            }
            Self::Fs(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for EditError {}

impl From<FsError> for EditError {
    fn from(e: FsError) -> Self {
        Self::Fs(e)
    }
}

/// Check a name the way AmigaDOS would.
///
/// `:` ends a device name and `/` separates directories, so a name holding
/// either cannot be typed at a prompt; control characters are refused for the
/// same reason [`crate::format`] refuses them in a volume name.
///
/// # Errors
/// The first rule the name breaks.
pub fn check_name(name: &[u8]) -> Result<(), EditError> {
    if name.is_empty() {
        return Err(EditError::NameEmpty);
    }
    if name.len() > MAX_NAME {
        return Err(EditError::NameTooLong { len: name.len() });
    }
    if let Some(&byte) = name
        .iter()
        .find(|&&b| b == b':' || b == b'/' || b < 0x20 || b == 0x7F)
    {
        return Err(EditError::NameInvalid { byte });
    }
    Ok(())
}

/// Make an empty directory called `name` inside the directory at `parent`.
///
/// `image` is the volume's bytes, block 0 first; it may be longer than the
/// volume (BUG-009). Returns the new directory's block.
///
/// # Errors
/// [`EditError`]; the image is unchanged whenever one is returned.
pub fn make_dir(
    image: &mut [u8],
    geometry: Geometry,
    parent: u32,
    name: &[u8],
    when: Stamp,
) -> Result<u32, EditError> {
    let plan = {
        let source = Slice { image, geometry };
        let mut plan = Plan::new(&source, parent, name)?;
        plan.reserve(1)?;
        let block = plan.allocate()?;
        let header = plan.entry_block(block, name, ST_USERDIR, 0, when);
        plan.put(block, header);
        plan.link(block, name, 0, ST_USERDIR, when)?;
        (plan.finish(), block)
    };
    apply(image, geometry, &plan.0);
    Ok(plan.1)
}

/// Add a file called `name`, holding `data`, to the directory at `parent`.
///
/// Returns the new file header's block.
///
/// # Errors
/// [`EditError`]; the image is unchanged whenever one is returned.
pub fn add_file(
    image: &mut [u8],
    geometry: Geometry,
    parent: u32,
    name: &[u8],
    data: &[u8],
    when: Stamp,
) -> Result<u32, EditError> {
    let size = u32::try_from(data.len()).map_err(|_| EditError::TooLarge { len: data.len() })?;
    let plan = {
        let source = Slice { image, geometry };
        let mut plan = Plan::new(&source, parent, name)?;

        let filesystem = plan.volume.filesystem();
        let bsize = geometry.block_size();
        let payload = filesystem
            .payload_bytes(bsize)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p > 0)
            .ok_or_else(|| EditError::Unsupported("the block size cannot hold data".to_owned()))?;
        let ht = plan.ht;
        let data_blocks = data.len().div_ceil(payload);
        // The header holds the first `ht` pointers and each extension block
        // the next `ht`, so a file of `n` blocks needs `ceil(n / ht) - 1`
        // extension blocks — none for an empty file.
        let extensions = data_blocks.div_ceil(ht).saturating_sub(1);
        plan.reserve(
            1usize
                .saturating_add(data_blocks)
                .saturating_add(extensions),
        )?;

        let header = plan.allocate()?;
        // Each table — the header's, then each extension block's — and the
        // data blocks it lists, allocated in that order so a file reads
        // forwards across the disk.
        let mut tables: Vec<(u32, Vec<u32>)> = Vec::new();
        let mut data_at: Vec<u32> = Vec::with_capacity(data_blocks);
        let mut remaining = data_blocks;
        let mut table_block = header;
        loop {
            let take = remaining.min(ht);
            let mut table = Vec::with_capacity(take);
            for _ in 0..take {
                let b = plan.allocate()?;
                table.push(b);
                data_at.push(b);
            }
            tables.push((table_block, table));
            remaining = remaining.saturating_sub(take);
            if remaining == 0 {
                break;
            }
            table_block = plan.allocate()?;
        }

        // The data blocks.
        let chunks: Vec<&[u8]> = data.chunks(payload).collect();
        for (i, (&block, chunk)) in data_at.iter().zip(&chunks).enumerate() {
            let raw = match filesystem {
                FileSystem::Ffs => {
                    let mut raw = vec![0u8; bsize as usize];
                    if let Some(dst) = raw.get_mut(..chunk.len()) {
                        dst.copy_from_slice(chunk);
                    }
                    raw
                }
                FileSystem::Ofs => {
                    let next = data_at.get(i.saturating_add(1)).copied().unwrap_or(0);
                    ofs_data_block(bsize, header, i, next, chunk)
                }
            };
            plan.put(block, raw);
        }

        // The extension blocks, each naming the next.
        for (index, (block, table)) in tables.iter().enumerate().skip(1) {
            let next = tables.get(index.saturating_add(1)).map_or(0, |t| t.0);
            let raw = plan.extension_block(*block, header, table, next);
            plan.put(*block, raw);
        }

        // The header last, because it names everything above.
        let mut raw = plan.entry_block(header, name, ST_FILE, size, when);
        if let Some((_, table)) = tables.first() {
            plan.write_table(&mut raw, table);
            let _ = put_u32(&mut raw, 16, data_at.first().copied().unwrap_or(0));
        }
        let first_extension = tables.get(1).map_or(0, |t| t.0);
        let _ = put_back(&mut raw, 8, first_extension);
        seal(&mut raw);
        plan.put(header, raw);

        plan.link(header, name, size, ST_FILE, when)?;
        (plan.finish(), header)
    };
    apply(image, geometry, &plan.0);
    Ok(plan.1)
}

/// Delete the entry at `block`: a file, a soft link, or an empty directory.
///
/// The entry leaves its parent's hash chain and cache, and every block it
/// occupied is marked free — but none is erased (SPEC §Deletion).
///
/// # Errors
/// [`EditError::NotRemovable`] for the root, a directory with anything in it,
/// or an entry a hard link is part of; otherwise as [`make_dir`]. The image
/// is unchanged whenever one is returned.
pub fn delete(
    image: &mut [u8],
    geometry: Geometry,
    block: u32,
    when: Stamp,
) -> Result<(), EditError> {
    let writes = {
        let source = Slice { image, geometry };
        let (entry, parent) = removable(&source, block)?;
        let mut plan = Plan::open(&source, parent)?;
        let blocks = match entry.kind {
            EntryKind::File => plan.volume.file_blocks(&entry)?,
            EntryKind::Directory => {
                let listing = plan.volume.list(block)?;
                if !listing.entries.is_empty() {
                    return Err(EditError::NotRemovable(format!(
                        "{} is a directory with {} entries in it; delete them first",
                        entry.name_lossy(),
                        listing.entries.len()
                    )));
                }
                let mut blocks = vec![block];
                // An empty directory can still own a cache chain.
                if plan.volume.has_dircache() && entry.extension != 0 {
                    blocks.extend(plan.volume.dircache(block)?.block_numbers());
                }
                blocks
            }
            _ => vec![block],
        };
        // A block another entry also uses stays allocated. AmigaDOS would
        // free it, and the next write would land in the other file — on a
        // cross-linked corpus disk (Championship Golf, block 1687) that is
        // exactly what ADE's own health check refused.
        let shared = plan.blocks_used_elsewhere(block)?;
        let blocks: Vec<u32> = blocks.into_iter().filter(|b| !shared.contains(b)).collect();
        plan.unlink(block)?;
        if plan.volume.has_dircache() {
            plan.rewrite_cache(|records| records.retain(|r| r.header != block))?;
        }
        plan.touch(when)?;
        plan.release(&blocks);
        plan.finish()
    };
    apply(image, geometry, &writes);
    Ok(())
}

/// Rename the entry at `block` to `name`, in the same directory.
///
/// The entry moves to the hash slot its new name falls in, and its cache
/// record, if any, takes the new name. Its own datestamp is kept: the file
/// did not change, its directory did.
///
/// # Errors
/// [`EditError::NotRemovable`] for the root; [`EditError::Exists`] when
/// another entry has the name; otherwise as [`make_dir`]. The image is
/// unchanged whenever one is returned.
pub fn rename(
    image: &mut [u8],
    geometry: Geometry,
    block: u32,
    name: &[u8],
    when: Stamp,
) -> Result<(), EditError> {
    check_name(name)?;
    let writes = {
        let source = Slice { image, geometry };
        let (_, parent) = removable_for_rename(&source, block)?;
        let mut plan = Plan::open(&source, parent)?;
        // Taken by anything but this entry: changing only a name's case is a
        // rename of the entry to itself, and allowed.
        if let Some(existing) = plan
            .volume
            .list(parent)?
            .entries
            .iter()
            .find(|e| e.block != block && plan.volume.names_match(&e.name, name))
        {
            return Err(EditError::Exists {
                name: existing.name_lossy(),
            });
        }
        plan.unlink(block)?;
        let mut raw = plan.read(block)?;
        // The old name's bytes past the new length are cleared, so a shorter
        // name leaves no tail of the longer one behind it in the block.
        let at = from_end(&raw, 80);
        if let Some(field) = raw.get_mut(at..at.saturating_add(MAX_NAME.saturating_add(1))) {
            field.fill(0);
        }
        put_name(&mut raw, name);
        seal(&mut raw);
        plan.put(block, raw);
        plan.insert(block, name)?;
        if plan.volume.has_dircache() {
            plan.rewrite_cache(|records| {
                for r in records.iter_mut().filter(|r| r.header == block) {
                    r.name = name.to_vec();
                }
            })?;
        }
        plan.touch(when)?;
        plan.finish()
    };
    apply(image, geometry, &writes);
    Ok(())
}

/// Move the entry at `block` from a hash slot its name does not hash to
/// (IMP-012) into the one it does, so AmigaDOS can open it by name.
///
/// Only the hash chains change. The entry keeps its name and its datestamp,
/// the directory keeps its own, and a directory cache is left alone: it
/// records entries, not slots, and the entry was in it already.
///
/// # Errors
/// [`EditError::NotNeeded`] when the entry is where its name hashes to;
/// [`EditError::Damaged`] when another entry in the directory has the same
/// name — refiling would change which of the two AmigaDOS opens, which is a
/// choice for whoever owns the disk — or when a block the move would reseal
/// already fails its checksum, since resealing it would hide that. Otherwise
/// as [`rename`]. The image is unchanged whenever one is returned.
pub fn refile(image: &mut [u8], geometry: Geometry, block: u32) -> Result<(), EditError> {
    let writes = {
        let source = Slice { image, geometry };
        let (entry, parent) = removable_for_rename(&source, block)?;
        // No block is taken or freed, so the bitmap is not consulted — and a
        // disk whose bitmap needs repair can still have its entries refiled.
        let mut plan = Plan::mount(&source, parent)?;
        if !plan
            .volume
            .misplaced(parent)?
            .iter()
            .any(|m| m.block == block)
        {
            return Err(EditError::NotNeeded(format!(
                "{} is already where its name hashes to",
                entry.name_lossy()
            )));
        }
        if plan
            .volume
            .list(parent)?
            .entries
            .iter()
            .any(|e| e.block != block && plan.volume.names_match(&e.name, &entry.name))
        {
            return Err(EditError::Damaged(format!(
                "another entry is also called {:?}; refiling this one would change which of \
                 them AmigaDOS opens, so rename or delete one first",
                entry.name_lossy()
            )));
        }
        plan.unlink(block)?;
        plan.insert(block, &entry.name)?;
        let writes = plan.finish();
        for &b in writes.keys() {
            if !checksum::normal_valid(&plan_read(&source, b)?) {
                return Err(EditError::Damaged(format!(
                    "block {b} already fails its checksum; refiling would reseal it and hide that"
                )));
            }
        }
        writes
    };
    apply(image, geometry, &writes);
    Ok(())
}

/// One block of `source`, as it stands.
fn plan_read(source: &dyn BlockSource, block: u32) -> Result<Vec<u8>, EditError> {
    let mut raw = vec![0u8; source.geometry().block_size() as usize];
    ade_block::read_at(source, BlockIndex(u64::from(block)), &mut raw).map_err(FsError::Block)?;
    Ok(raw)
}

/// The entry at `block` and its parent, if it is one ADE will delete.
fn removable(
    source: &dyn BlockSource,
    block: u32,
) -> Result<(crate::entry::Entry, u32), EditError> {
    let (entry, parent) = removable_for_rename(source, block)?;
    match entry.kind {
        EntryKind::HardLinkFile | EntryKind::HardLinkDir => Err(EditError::NotRemovable(format!(
            "{} is a hard link; ADE does not yet rewrite link chains, so it will not remove one",
            entry.name_lossy()
        ))),
        _ if entry.next_link != 0 => Err(EditError::NotRemovable(format!(
            "a hard link points at {}; ADE does not yet rewrite link chains, so it will not \
             delete it",
            entry.name_lossy()
        ))),
        EntryKind::File | EntryKind::Directory | EntryKind::SoftLink => Ok((entry, parent)),
        _ => Err(EditError::NotRemovable(format!(
            "{} is not an entry ADE knows how to delete",
            entry.name_lossy()
        ))),
    }
}

/// The entry at `block` and its parent, if it is an entry at all and not the
/// root.
fn removable_for_rename(
    source: &dyn BlockSource,
    block: u32,
) -> Result<(crate::entry::Entry, u32), EditError> {
    let volume = Volume::mount(source)?;
    if block == volume.root() {
        return Err(EditError::NotRemovable(
            "that is the volume itself, not an entry in it".to_owned(),
        ));
    }
    let entry = volume
        .entry_at(block)
        .map_err(|_| EditError::NotRemovable(format!("block {block} is not a directory entry")))?;
    if entry.kind == EntryKind::Root {
        return Err(EditError::NotRemovable(
            "that is the volume itself, not an entry in it".to_owned(),
        ));
    }
    let parent = entry.parent;
    Ok((entry, parent))
}

/// A volume's bytes, read as blocks.
pub(crate) struct Slice<'a> {
    pub(crate) image: &'a [u8],
    pub(crate) geometry: Geometry,
}

impl BlockSource for Slice<'_> {
    fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    fn read_block(&self, block: ValidBlock, out: &mut [u8]) -> Result<(), BlockError> {
        let size = self.geometry.block_size() as usize;
        let truncated = BlockError::Truncated {
            index: BlockIndex(block.index()),
        };
        let start = usize::try_from(block.index())
            .ok()
            .and_then(|b| b.checked_mul(size))
            .ok_or(truncated.clone())?;
        let end = start.checked_add(size).ok_or(truncated.clone())?;
        let src = self.image.get(start..end).ok_or(truncated)?;
        out.copy_from_slice(src);
        Ok(())
    }
}

/// Every block an edit will write, built before any of it is.
struct Plan<'a> {
    volume: Volume<'a>,
    source: &'a dyn BlockSource,
    geometry: Geometry,
    /// Hash-table slots per directory block.
    ht: usize,
    /// The directory being written into.
    parent: u32,
    /// Blocks to write, by number.
    writes: BTreeMap<u32, Vec<u8>>,
    /// The bitmap as read.
    bitmap: Bitmap,
    /// Blocks this plan has taken, in order.
    taken: Vec<u32>,
    /// The same, for lookup: a large file takes tens of thousands.
    taken_set: HashSet<u32>,
    /// Blocks this plan has given back: dircache blocks no longer needed.
    released: Vec<u32>,
    /// Where the next allocation search starts.
    cursor: u64,
}

impl<'a> Plan<'a> {
    /// Mount, and check everything an addition depends on: the volume, the
    /// parent, the bitmap, and that the name is valid and free.
    fn new(source: &'a dyn BlockSource, parent: u32, name: &[u8]) -> Result<Self, EditError> {
        check_name(name)?;
        let plan = Self::open(source, parent)?;
        if let Some(existing) = plan
            .volume
            .list(parent)?
            .entries
            .iter()
            .find(|e| plan.volume.names_match(&e.name, name))
        {
            return Err(EditError::Exists {
                name: existing.name_lossy(),
            });
        }
        Ok(plan)
    }

    /// Mount, and check what every edit of `parent` depends on: the volume,
    /// the parent, its listing and the bitmap.
    fn open(source: &'a dyn BlockSource, parent: u32) -> Result<Self, EditError> {
        let plan = Self::mount(source, parent)?;
        let bitmap = &plan.bitmap;
        if !bitmap.flagged_valid {
            return Err(EditError::BitmapUntrusted(
                "the rootblock flags its bitmap as invalid, so it cannot say which blocks are \
                 free"
                    .to_owned(),
            ));
        }
        if let Some(bad) = bitmap.bad_checksums.first() {
            return Err(EditError::BitmapUntrusted(format!(
                "bitmap block {bad} fails its checksum"
            )));
        }
        if bitmap.incomplete || bitmap.blocks.is_empty() {
            return Err(EditError::BitmapUntrusted(
                "the bitmap does not cover the whole volume".to_owned(),
            ));
        }
        Ok(plan)
    }

    /// As [`Self::open`], without trusting the bitmap: for an edit that
    /// neither takes nor frees a block, and so never consults it.
    fn mount(source: &'a dyn BlockSource, parent: u32) -> Result<Self, EditError> {
        let volume = Volume::mount(source)?;
        let Some(dostype) = volume.dostype() else {
            return Err(EditError::Unsupported(
                "the bootblock carries no dostype, so ADE cannot tell how this volume \
                 hashes names"
                    .to_owned(),
            ));
        };
        if dostype.mode() == Mode::LongNames {
            return Err(EditError::Unsupported(format!(
                "{dostype} is a long-name volume, which ADE does not write (D-013)"
            )));
        }
        let geometry = *volume.geometry();

        if parent != volume.root() {
            let entry = volume
                .entry_at(parent)
                .map_err(|_| EditError::NotADirectory { block: parent })?;
            // A hard link to a directory is refused rather than followed: the
            // entry belongs in the directory it names, and inserting through
            // the link would leave the link's own datestamp stale.
            if entry.kind != EntryKind::Directory {
                return Err(EditError::NotADirectory { block: parent });
            }
        }

        let listing = volume.list(parent)?;
        if let Some(fault) = listing.cycles.first().or_else(|| listing.faults.first()) {
            return Err(EditError::Damaged(format!(
                "the directory is damaged ({fault}); ADE will not write into it"
            )));
        }
        if let Some(block) = listing.merges.first() {
            return Err(EditError::Damaged(format!(
                "two of the directory's hash chains merge at block {block}; ADE will not write \
                 into it"
            )));
        }

        let bitmap = Bitmap::read(source, &geometry, volume.rootblock())?;
        let ht = volume.hash_table_size() as usize;
        let cursor = u64::from(volume.root()).saturating_add(1);
        Ok(Self {
            volume,
            source,
            geometry,
            ht,
            parent,
            writes: BTreeMap::new(),
            bitmap,
            taken: Vec::new(),
            taken_set: HashSet::new(),
            released: Vec::new(),
            cursor,
        })
    }

    /// Whether a block is free: in the map, unclaimed, and not one this plan
    /// has already taken.
    fn is_free(&self, block: u32) -> bool {
        block != self.volume.root()
            && !self.bitmap.is_allocated(block)
            && !self.bitmap.blocks.contains(&block)
            && !self.taken_set.contains(&block)
    }

    /// Free blocks in the volume.
    fn free_count(&self) -> usize {
        let reserved = u64::from(self.geometry.reserved());
        (reserved..self.geometry.total_blocks())
            .filter_map(|b| u32::try_from(b).ok())
            .filter(|&b| self.is_free(b))
            .count()
    }

    /// Refuse now, before anything is built, if `needed` blocks will not fit.
    ///
    /// A directory cache may need one more on top; that is caught when it is
    /// allocated, which is still before anything is applied.
    fn reserve(&self, needed: usize) -> Result<(), EditError> {
        let free = self.free_count();
        if free < needed {
            return Err(EditError::DiskFull { needed, free });
        }
        Ok(())
    }

    /// Take the next free block.
    ///
    /// Searched from the rootblock outward, which is where AmigaDOS itself
    /// looks (SPEC §Deletion): up to the end of the volume, then from the
    /// first block after the reserved area back up to the rootblock.
    fn allocate(&mut self) -> Result<u32, EditError> {
        let total = self.geometry.total_blocks();
        let reserved = u64::from(self.geometry.reserved());
        let found = (self.cursor..total)
            .chain(reserved..self.cursor.min(total))
            .filter_map(|b| u32::try_from(b).ok())
            .find(|&b| self.is_free(b));
        let Some(block) = found else {
            return Err(EditError::DiskFull {
                needed: self.taken.len().saturating_add(1),
                free: self.taken.len(),
            });
        };
        self.taken.push(block);
        self.taken_set.insert(block);
        self.cursor = u64::from(block).saturating_add(1);
        Ok(block)
    }

    /// A block as this plan currently has it: a pending write, or the disk.
    fn read(&self, block: u32) -> Result<Vec<u8>, EditError> {
        if let Some(raw) = self.writes.get(&block) {
            return Ok(raw.clone());
        }
        let mut raw = vec![0u8; self.geometry.block_size() as usize];
        ade_block::read_at(self.source, BlockIndex(u64::from(block)), &mut raw)
            .map_err(FsError::Block)?;
        Ok(raw)
    }

    fn put(&mut self, block: u32, raw: Vec<u8>) {
        self.writes.insert(block, raw);
    }

    /// The header block every entry shares, unsealed.
    ///
    /// Everything not set here is zero: no comment, no owner, no protection
    /// bits — which on the owner bits means *permitted* — and no hash chain
    /// yet, since [`Self::link`] decides that.
    fn entry_block(
        &self,
        block: u32,
        name: &[u8],
        sec_type: u32,
        size: u32,
        when: Stamp,
    ) -> Vec<u8> {
        let mut raw = vec![0u8; self.geometry.block_size() as usize];
        let _ = put_u32(&mut raw, 0x00, T_HEADER);
        let _ = put_u32(&mut raw, 0x04, block);
        let _ = put_back(&mut raw, 188, size);
        put_stamp(&mut raw, 92, when);
        put_name(&mut raw, name);
        let _ = put_back(&mut raw, 12, self.parent);
        let _ = put_back(&mut raw, 4, sec_type);
        seal(&mut raw);
        raw
    }

    /// Write a table of data-block pointers into a header or extension block.
    ///
    /// **Backwards**: the first block goes in the last slot (SPEC §File
    /// header block). `high_seq` says how many are filled.
    fn write_table(&self, raw: &mut [u8], table: &[u32]) {
        let _ = put_u32(raw, 0x08, u32::try_from(table.len()).unwrap_or(0));
        for (i, &block) in table.iter().enumerate() {
            let slot = self.ht.saturating_sub(1).saturating_sub(i);
            let _ = put_u32(raw, 24usize.saturating_add(slot.saturating_mul(4)), block);
        }
    }

    /// A file extension block, sealed.
    fn extension_block(&self, block: u32, header: u32, table: &[u32], next: u32) -> Vec<u8> {
        let mut raw = vec![0u8; self.geometry.block_size() as usize];
        let _ = put_u32(&mut raw, 0x00, T_LIST);
        let _ = put_u32(&mut raw, 0x04, block);
        self.write_table(&mut raw, table);
        let _ = put_back(&mut raw, 12, header);
        let _ = put_back(&mut raw, 8, next);
        let _ = put_back(&mut raw, 4, ST_FILE);
        seal(&mut raw);
        raw
    }

    /// Make the new entry reachable: hash chain, parent datestamp, volume
    /// datestamp, and directory cache.
    fn link(
        &mut self,
        block: u32,
        name: &[u8],
        size: u32,
        sec_type: u32,
        when: Stamp,
    ) -> Result<(), EditError> {
        self.insert(block, name)?;
        self.touch(when)?;
        if self.volume.has_dircache() {
            let record = Record {
                header: block,
                size,
                protection: Protection(0),
                uid: 0,
                gid: 0,
                altered: crate::datestamp::Datestamp::new(when.days, when.mins, when.ticks),
                // The record holds the secondary type as one signed byte.
                secondary_type: if sec_type == ST_FILE { -3 } else { 2 },
                name: name.to_vec(),
                comment: Vec::new(),
            };
            self.rewrite_cache(|records| records.push(record))?;
        }
        Ok(())
    }

    /// Put `block` into the parent's hash chain for `name`, in block order.
    fn insert(&mut self, block: u32, name: &[u8]) -> Result<(), EditError> {
        let international = self.volume.is_international();
        let ht = u32::try_from(self.ht).unwrap_or(0);
        let slot = hash_name(name, ht, international) as usize;
        let slot_at = 24usize.saturating_add(slot.saturating_mul(4));

        // The chain is kept sorted by block number, as FFS keeps it (SPEC
        // §Collision chains). Readers do not depend on the order, but a chain
        // ADE wrote should look like one AmigaDOS wrote.
        let parent_raw = self.read(self.parent)?;
        let mut previous: Option<u32> = None;
        let mut next = u32_at(&parent_raw, slot_at).unwrap_or(0);
        let mut seen: HashSet<u32> = HashSet::new();
        while next != 0 && next < block {
            if !seen.insert(next) {
                return Err(EditError::Damaged(format!(
                    "hash chain loops at block {next}; ADE will not write into it"
                )));
            }
            let raw = self.read(next)?;
            previous = Some(next);
            next = u32_at(&raw, from_end(&raw, 16)).unwrap_or(0);
        }

        let mut entry = self.read(block)?;
        let _ = put_back(&mut entry, 16, next);
        seal(&mut entry);
        self.put(block, entry);

        // Either the entry before it in the chain points at it, or the slot
        // itself does.
        let (owner, at) = match previous {
            Some(prev) => (prev, None),
            None => (self.parent, Some(slot_at)),
        };
        let mut raw = self.read(owner)?;
        let _ = match at {
            Some(at) => put_u32(&mut raw, at, block),
            None => put_back(&mut raw, 16, block),
        };
        seal(&mut raw);
        self.put(owner, raw);
        Ok(())
    }

    /// Take `block` out of the parent's hash table: whatever points at it —
    /// a slot, or the entry before it in a chain — points past it instead.
    ///
    /// Every slot is searched, not only the one its name hashes to, so an
    /// entry filed under a wrong slot (IMP-012) can still be removed. One that
    /// two chains reach, or none, is refused: the table is not what ADE
    /// would be editing.
    fn unlink(&mut self, block: u32) -> Result<(), EditError> {
        let parent_raw = self.read(self.parent)?;
        let mut pointers: Vec<(u32, Option<usize>)> = Vec::new();
        for slot in 0..self.ht {
            let slot_at = 24usize.saturating_add(slot.saturating_mul(4));
            let mut owner: (u32, Option<usize>) = (self.parent, Some(slot_at));
            let mut next = u32_at(&parent_raw, slot_at).unwrap_or(0);
            let mut seen: HashSet<u32> = HashSet::new();
            while next != 0 && seen.insert(next) {
                if next == block {
                    pointers.push(owner);
                    break;
                }
                let raw = self.read(next)?;
                owner = (next, None);
                next = u32_at(&raw, from_end(&raw, 16)).unwrap_or(0);
            }
        }
        let [(owner, at)] = pointers.as_slice() else {
            return Err(EditError::Damaged(format!(
                "block {block} is reached from {} places in its directory's hash table, not \
                 one; ADE will not edit it",
                pointers.len()
            )));
        };
        let entry_raw = self.read(block)?;
        let after = u32_at(&entry_raw, from_end(&entry_raw, 16)).unwrap_or(0);
        let mut raw = self.read(*owner)?;
        let _ = match at {
            Some(at) => put_u32(&mut raw, *at, after),
            None => put_back(&mut raw, 16, after),
        };
        seal(&mut raw);
        self.put(*owner, raw);
        Ok(())
    }

    /// Every block some entry other than `block` uses: headers, file blocks,
    /// and directory caches, across the whole volume.
    fn blocks_used_elsewhere(&self, block: u32) -> Result<HashSet<u32>, EditError> {
        let root = self.volume.root();
        let mut used = HashSet::from([root]);
        if self.volume.has_dircache() {
            used.extend(self.volume.dircache(root)?.block_numbers());
        }
        for (_, entry) in self.volume.walk(root)?.entries {
            if entry.block == block {
                continue;
            }
            used.insert(entry.block);
            if entry.kind == EntryKind::File {
                used.extend(self.volume.file_blocks(&entry)?);
            }
            if entry.kind == EntryKind::Directory && self.volume.has_dircache() {
                used.extend(self.volume.dircache(entry.block)?.block_numbers());
            }
        }
        Ok(used)
    }

    /// Give blocks back to the free map. Their contents stay as they are.
    fn release(&mut self, blocks: &[u32]) {
        self.released.extend(blocks.iter().copied());
    }

    /// The parent changed, and so did the volume: move both datestamps.
    fn touch(&mut self, when: Stamp) -> Result<(), EditError> {
        // On the root those are two fields of one block; anywhere else, two
        // blocks.
        let root = self.volume.root();
        let mut raw = self.read(self.parent)?;
        put_stamp(&mut raw, 92, when);
        if self.parent == root {
            put_stamp(&mut raw, 40, when);
        }
        seal(&mut raw);
        self.put(self.parent, raw);
        if self.parent != root {
            let mut raw = self.read(root)?;
            put_stamp(&mut raw, 40, when);
            seal(&mut raw);
            self.put(root, raw);
        }
        Ok(())
    }

    /// Change the parent's directory cache by `change`, and lay it out again.
    ///
    /// The cache is rebuilt from its existing records as changed, reusing its
    /// blocks in order, taking one more only when they are full and giving
    /// back any no longer needed. Records `change` does not touch are kept
    /// exactly as they were: the cache is AmigaDOS's description of the
    /// directory, and rewriting it from ADE's reading of the entries would
    /// replace that witness with ADE's opinion.
    fn rewrite_cache(&mut self, change: impl FnOnce(&mut Vec<Record>)) -> Result<(), EditError> {
        let chain = self.volume.dircache(self.parent)?;
        if let Some(fault) = chain.faults.first() {
            return Err(EditError::Damaged(format!(
                "the directory cache is damaged ({fault}); ADE will not write into it"
            )));
        }
        let mut records: Vec<Record> = chain.records().into_iter().cloned().collect();
        change(&mut records);
        let old_blocks = chain.block_numbers();

        // Pack records into blocks. A record never spans two (SPEC §Directory
        // cache blocks), so a block ends early when the next will not fit. No
        // records means no cache at all, as a new directory has.
        let room = (self.geometry.block_size() as usize).saturating_sub(DIRCACHE_HEADER);
        let mut packed: Vec<Vec<Vec<u8>>> = if records.is_empty() {
            Vec::new()
        } else {
            vec![Vec::new()]
        };
        let mut used = 0usize;
        for r in &records {
            let bytes = encode_record(r);
            if used.saturating_add(bytes.len()) > room
                && packed.last().is_some_and(|b| !b.is_empty())
            {
                packed.push(Vec::new());
                used = 0;
            }
            used = used.saturating_add(bytes.len());
            if let Some(last) = packed.last_mut() {
                last.push(bytes);
            }
        }

        let mut blocks: Vec<u32> = old_blocks.iter().copied().take(packed.len()).collect();
        while blocks.len() < packed.len() {
            blocks.push(self.allocate()?);
        }
        // A deletion can leave fewer blocks needed than there were.
        self.released
            .extend(old_blocks.iter().copied().skip(packed.len()));

        for (i, (block, contents)) in blocks.iter().zip(&packed).enumerate() {
            let next = blocks.get(i.saturating_add(1)).copied().unwrap_or(0);
            let mut raw = vec![0u8; self.geometry.block_size() as usize];
            let _ = put_u32(&mut raw, 0x00, T_DIRCACHE);
            let _ = put_u32(&mut raw, 0x04, *block);
            let _ = put_u32(&mut raw, 0x08, self.parent);
            let _ = put_u32(&mut raw, 0x0c, u32::try_from(contents.len()).unwrap_or(0));
            let _ = put_u32(&mut raw, 0x10, next);
            let mut at = DIRCACHE_HEADER;
            for bytes in contents {
                let end = at.saturating_add(bytes.len());
                if let Some(dst) = raw.get_mut(at..end) {
                    dst.copy_from_slice(bytes);
                }
                at = end;
            }
            seal(&mut raw);
            self.put(*block, raw);
        }

        // Point the parent at the chain if it had none. The root's cache
        // pointer and a directory's are the same field (`extension`).
        if old_blocks.first() != blocks.first() {
            let mut raw = self.read(self.parent)?;
            let _ = put_back(&mut raw, 8, blocks.first().copied().unwrap_or(0));
            seal(&mut raw);
            self.put(self.parent, raw);
        }
        Ok(())
    }

    /// The finished plan: every block to write, the bitmap's changes included.
    fn finish(mut self) -> BTreeMap<u32, Vec<u8>> {
        let reserved = self.geometry.reserved();
        let per_page = (self.geometry.block_size() as usize / 4)
            .saturating_sub(1)
            .saturating_mul(32);
        let mut changes: Vec<(u32, bool)> = self.taken.iter().map(|&b| (b, false)).collect();
        changes.extend(self.released.iter().map(|&b| (b, true)));

        for (block, free) in changes {
            let Some(index) = block.checked_sub(reserved).map(|i| i as usize) else {
                continue;
            };
            let Some(page) = index.checked_div(per_page) else {
                continue;
            };
            let Some(&bitmap_block) = self.bitmap.blocks.get(page) else {
                continue;
            };
            let bit = index.checked_rem(per_page).unwrap_or(0);
            let Ok(mut raw) = self.read(bitmap_block) else {
                continue;
            };
            let at = 4usize.saturating_add((bit / 32).saturating_mul(4));
            let mask = 1u32 << (bit % 32);
            let word = u32_at(&raw, at).unwrap_or(0);
            // Set means free (SPEC §Bitmap).
            let word = if free { word | mask } else { word & !mask };
            let _ = put_u32(&mut raw, at, word);
            // The bitmap keeps its checksum at offset 0, not 20 (BUG-004).
            if let Some(sum) = checksum::normal_at(&raw, 0x00) {
                let _ = put_u32(&mut raw, 0x00, sum);
            }
            self.writes.insert(bitmap_block, raw);
        }
        self.writes
    }
}

/// An OFS data block, sealed. `index` counts from zero; the stored sequence
/// number counts from one.
fn ofs_data_block(bsize: u32, header: u32, index: usize, next: u32, chunk: &[u8]) -> Vec<u8> {
    let mut raw = vec![0u8; bsize as usize];
    let _ = put_u32(&mut raw, 0x00, T_DATA);
    let _ = put_u32(&mut raw, 0x04, header);
    let seq = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
    let _ = put_u32(&mut raw, 0x08, seq);
    let _ = put_u32(&mut raw, 0x0c, u32::try_from(chunk.len()).unwrap_or(0));
    let _ = put_u32(&mut raw, 0x10, next);
    let end = 24usize.saturating_add(chunk.len());
    if let Some(dst) = raw.get_mut(24..end) {
        dst.copy_from_slice(chunk);
    }
    seal(&mut raw);
    raw
}

/// One directory cache record, padded to a word (SPEC §Directory cache
/// blocks). The datestamp is three 16-bit fields here, against the entry
/// block's 32-bit ones.
fn encode_record(r: &Record) -> Vec<u8> {
    let name = r.name.get(..r.name.len().min(MAX_NAME)).unwrap_or(&[]);
    let comment = r.comment.get(..r.comment.len().min(22)).unwrap_or(&[]);
    let mut out = vec![0u8; 24];
    let _ = put_u32(&mut out, 0, r.header);
    let _ = put_u32(&mut out, 4, r.size);
    let _ = put_u32(&mut out, 8, r.protection.0);
    let _ = put_u16(&mut out, 12, r.uid);
    let _ = put_u16(&mut out, 14, r.gid);
    let short = |v: u32| u16::try_from(v.min(0x7FFF)).unwrap_or(0);
    let _ = put_u16(&mut out, 16, short(r.altered.days));
    let _ = put_u16(&mut out, 18, short(r.altered.mins));
    let _ = put_u16(&mut out, 20, short(r.altered.ticks));
    if let Some(slot) = out.get_mut(22) {
        *slot = r.secondary_type.to_ne_bytes()[0];
    }
    if let Some(slot) = out.get_mut(23) {
        *slot = u8::try_from(name.len()).unwrap_or(0);
    }
    out.extend_from_slice(name);
    out.push(u8::try_from(comment.len()).unwrap_or(0));
    out.extend_from_slice(comment);
    if out.len() % 2 != 0 {
        out.push(0);
    }
    out
}

/// Copy a finished plan into the image.
pub(crate) fn apply(image: &mut [u8], geometry: Geometry, writes: &BTreeMap<u32, Vec<u8>>) {
    let bsize = geometry.block_size() as usize;
    for (&block, raw) in writes {
        let Some(at) = (block as usize).checked_mul(bsize) else {
            continue;
        };
        if let Some(dst) = image.get_mut(at..at.saturating_add(bsize)) {
            dst.copy_from_slice(raw);
        }
    }
}

/// An offset counted back from the end of the block, as SPEC writes them.
const fn from_end(raw: &[u8], back: usize) -> usize {
    raw.len().saturating_sub(back)
}

/// Write a long `back` bytes from the end of the block.
pub(crate) fn put_back(
    raw: &mut [u8],
    back: usize,
    value: u32,
) -> Result<(), ade_endian::OutOfBounds> {
    let at = from_end(raw, back);
    put_u32(raw, at, value)
}

/// Write a datestamp whose first field sits `back` bytes from the end.
fn put_stamp(raw: &mut [u8], back: usize, when: Stamp) {
    let at = from_end(raw, back);
    let _ = put_u32(raw, at, when.days);
    let _ = put_u32(raw, at.saturating_add(4), when.mins);
    let _ = put_u32(raw, at.saturating_add(8), when.ticks);
}

/// Write an entry's name: a length byte, then the bytes, at `BSIZE-80`.
fn put_name(raw: &mut [u8], name: &[u8]) {
    let at = from_end(raw, 80);
    let name = name.get(..name.len().min(MAX_NAME)).unwrap_or(&[]);
    if let Some(slot) = raw.get_mut(at) {
        *slot = u8::try_from(name.len()).unwrap_or(0);
    }
    let start = at.saturating_add(1);
    if let Some(dst) = raw.get_mut(start..start.saturating_add(name.len())) {
        dst.copy_from_slice(name);
    }
}

/// Recompute the normal checksum at offset 20.
pub(crate) fn seal(raw: &mut [u8]) {
    let _ = put_u32(raw, 0x14, 0);
    if let Some(sum) = checksum::normal_at(raw, 0x14) {
        let _ = put_u32(raw, 0x14, sum);
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests may unwrap"
)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_as_bytes() {
        assert_eq!(check_name(b""), Err(EditError::NameEmpty));
        assert_eq!(
            check_name(&[b'x'; 31]),
            Err(EditError::NameTooLong { len: 31 })
        );
        assert!(check_name(&[b'x'; 30]).is_ok());
        for bad in [&b"a/b"[..], b"a:b", b"a\nb"] {
            assert!(matches!(
                check_name(bad),
                Err(EditError::NameInvalid { .. })
            ));
        }
        // Latin-1 above 0x7F is an ordinary Amiga name.
        assert!(check_name(b"f\xFCr").is_ok());
    }

    #[test]
    fn a_record_is_word_aligned() {
        let record = Record {
            header: 900,
            size: 0,
            protection: Protection(0),
            uid: 0,
            gid: 0,
            altered: crate::datestamp::Datestamp::new(1, 2, 3),
            secondary_type: 2,
            name: b"abc".to_vec(),
            comment: Vec::new(),
        };
        // 24 fixed + 3 name + 1 comment length = 28: even, no pad.
        assert_eq!(encode_record(&record).len(), 28);
        let record = Record {
            name: b"ab".to_vec(),
            ..record
        };
        // 24 + 2 + 1 = 27, padded to 28.
        assert_eq!(encode_record(&record).len(), 28);
        assert_eq!(encode_record(&record)[22], 2);
    }
}
