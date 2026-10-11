//! Writing into an existing volume: making a directory, adding a file.
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
    /// Reading the volume failed.
    Fs(FsError),
}

impl core::fmt::Display for EditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported(why) | Self::BitmapUntrusted(why) | Self::Damaged(why) => {
                f.write_str(why)
            }
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

/// A volume's bytes, read as blocks.
struct Slice<'a> {
    image: &'a [u8],
    geometry: Geometry,
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
    /// Mount, and check everything that does not depend on what is being
    /// added: the volume, the parent, the bitmap and the name.
    fn new(source: &'a dyn BlockSource, parent: u32, name: &[u8]) -> Result<Self, EditError> {
        check_name(name)?;
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
        if let Some(existing) = listing
            .entries
            .iter()
            .find(|e| volume.names_match(&e.name, name))
        {
            return Err(EditError::Exists {
                name: existing.name_lossy(),
            });
        }

        let bitmap = Bitmap::read(source, &geometry, volume.rootblock())?;
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

        // The parent changed, and so did the volume. On the root those are
        // two fields of one block; anywhere else, two blocks.
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
            self.cache(record)?;
        }
        Ok(())
    }

    /// Add a record to the parent's directory cache.
    ///
    /// The cache is laid out again from its existing records plus the new
    /// one, reusing its blocks in order and taking one more only when they
    /// are full. Existing records are kept exactly as they were: the cache is
    /// AmigaDOS's description of the directory, and rewriting it from ADE's
    /// reading of the entries would replace that witness with ADE's opinion.
    fn cache(&mut self, record: Record) -> Result<(), EditError> {
        let chain = self.volume.dircache(self.parent)?;
        if let Some(fault) = chain.faults.first() {
            return Err(EditError::Damaged(format!(
                "the directory cache is damaged ({fault}); ADE will not write into it"
            )));
        }
        let mut records: Vec<Record> = chain.records().into_iter().cloned().collect();
        records.push(record);
        let old_blocks = chain.block_numbers();

        // Pack records into blocks. A record never spans two (SPEC §Directory
        // cache blocks), so a block ends early when the next will not fit.
        let room = (self.geometry.block_size() as usize).saturating_sub(DIRCACHE_HEADER);
        let mut packed: Vec<Vec<Vec<u8>>> = vec![Vec::new()];
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
        // A chain can only shrink if records were lost, which this never
        // does, but the arithmetic does not know that.
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
fn apply(image: &mut [u8], geometry: Geometry, writes: &BTreeMap<u32, Vec<u8>>) {
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
fn put_back(raw: &mut [u8], back: usize, value: u32) -> Result<(), ade_endian::OutOfBounds> {
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
fn seal(raw: &mut [u8]) {
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
