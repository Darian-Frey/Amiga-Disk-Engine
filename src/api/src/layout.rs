//! What occupies each part of a disk (F-022).
//!
//! `ade find` answers this one block at a time, for the block a search hit
//! landed in. The same question asked of the *whole* disk is a different and
//! more useful thing: a map of where the bootblock, the rootblock, the bitmap,
//! the directories and the files are, and — usually the interesting part —
//! where none of them is.
//!
//! # Runs, not blocks
//!
//! A map is emitted as spans of consecutive blocks sharing a region and an
//! owner, because that is what it is. An 880 KB floppy has 1,760 blocks and
//! typically a few hundred spans; a hard disk has hundreds of thousands of
//! blocks and still only as many spans as it has files. Emitting a row per
//! block would make the map bigger than the thing it describes.
//!
//! # Every byte is covered
//!
//! The spans tile the image with no gaps and no overlaps, `Unclaimed` where
//! nothing else applies. A consumer can therefore colour a whole disk without
//! deciding what to do about a byte the map forgot, which is the failure a
//! partial map produces: a hex view with holes in it that look like data.

use std::collections::BTreeMap;

use crate::json::Value;

/// What part of a disk a byte belongs to.
///
/// Measured across all 4,652 corpus images: `Copylock` appears on 103 of them,
/// and the region turns one string into four findings. On 86 it is in the
/// **bootblock** — protection lives there and in the trackloader it starts,
/// which is the part no directory entry points at. On 10 it is in the
/// **rootblock**, because those disks are *named* `Copylock(tm) Amiga`. On 11
/// it is in space nothing reaches, and on 5 it is inside a file.
///
/// So "no owning file" is the normal answer for the searches people most want
/// to run, and reporting it as *unallocated* would be wrong: a bootblock is
/// not unallocated space, it is the most deliberately written block on the
/// disk. A volume name is not either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Region {
    /// The reserved blocks a disk boots from.
    Bootblock,
    /// The volume's rootblock.
    Rootblock,
    /// A block of the allocation bitmap.
    Bitmap,
    /// A directory's header block — where its name lives.
    Directory,
    /// A file's header or data.
    File,
    /// Inside the volume, and nothing reaches it: deleted, hidden, or damage.
    Unclaimed,
}

impl Region {
    /// The name this region is reported by. Part of the JSON surface (F-015).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bootblock => "bootblock",
            Self::Rootblock => "rootblock",
            Self::Bitmap => "bitmap",
            Self::Directory => "directory",
            Self::File => "file",
            Self::Unclaimed => "unclaimed",
        }
    }

    /// A one-line description, for a legend.
    #[must_use]
    pub const fn describes(self) -> &'static str {
        match self {
            Self::Bootblock => "boot code and the dostype — where protection lives",
            Self::Rootblock => "the volume's name, datestamps and hash table",
            Self::Bitmap => "which blocks are free; a set bit means free",
            Self::Directory => "a directory header, holding its name",
            Self::File => "a file's header or its data",
            Self::Unclaimed => "nothing points here: free space, deleted data, or damage",
        }
    }
}

/// A run of consecutive blocks sharing a region and an owner.
#[derive(Debug, Clone)]
pub struct Span {
    /// First byte, inclusive.
    pub start: u64,
    /// Last byte, exclusive.
    pub end: u64,
    /// First block of the run.
    pub block: u64,
    /// How many blocks it covers.
    pub blocks: u64,
    /// What it is.
    pub region: Region,
    /// The path of the entry that owns it, where one does.
    pub owner: Option<String>,
    /// The block of the directory entry that owns it, where one does.
    ///
    /// The path names the owner for a reader; this identifies it for a
    /// program. A front end showing a disk in a tree has the entry's block
    /// already and can match on it exactly, where matching Latin-1 path
    /// strings is a comparison that can go wrong in ways a block cannot.
    pub owner_block: Option<u32>,
}

/// A map of the whole image.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Every span, in offset order, tiling the image with no gaps.
    pub spans: Vec<Span>,
    /// The block size the map is in.
    pub block_size: u32,
    /// How many blocks the image holds.
    pub blocks: u64,
    /// Whether a volume mounted. When it did not, everything outside the
    /// reserved blocks is `Unclaimed` — which is honest rather than useless:
    /// a quarter of real images do not mount, and a hex view of one still
    /// wants to know where its bootblock ends.
    pub mounted: bool,
}

impl Layout {
    /// Map an open image.
    #[must_use]
    pub fn of(image: &crate::Image) -> Self {
        let geometry = image.geometry();
        let block_size = geometry.block_size();
        let blocks = geometry.total_blocks();
        let (owners, mounted) = attribute(image);
        Self {
            spans: coalesce(&owners, blocks, block_size),
            block_size,
            blocks,
            mounted,
        }
    }

    /// The map as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("block_size", Value::Num(u64::from(self.block_size))),
            ("blocks", Value::Num(self.blocks)),
            ("mounted", Value::Bool(self.mounted)),
            (
                "spans",
                Value::Arr(
                    self.spans
                        .iter()
                        .map(|s| {
                            Value::Obj(vec![
                                ("offset", Value::Num(s.start)),
                                ("block", Value::Num(s.block)),
                                ("blocks", Value::Num(s.blocks)),
                                ("region", Value::str(s.region.name())),
                                ("file", Value::opt(s.owner.as_ref(), Value::str)),
                                (
                                    "file_block",
                                    Value::opt(s.owner_block.as_ref(), |b| {
                                        Value::Num(u64::from(*b))
                                    }),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// How many blocks each region accounts for, in region order.
    #[must_use]
    pub fn totals(&self) -> Vec<(Region, u64)> {
        let mut totals: BTreeMap<Region, u64> = BTreeMap::new();
        for span in &self.spans {
            let total = totals.entry(span.region).or_default();
            *total = total.saturating_add(span.blocks);
        }
        totals.into_iter().collect()
    }
}

/// What each block is, and what owns it. `false` when no volume mounted.
pub(crate) type Owned = (Region, Option<String>, Option<u32>);

pub(crate) fn attribute(image: &crate::Image) -> (BTreeMap<u64, Owned>, bool) {
    let mut out: BTreeMap<u64, Owned> = BTreeMap::new();
    let geometry = image.geometry();

    // The bootblock does not depend on the volume mounting. C-008 keeps the
    // two facts separate, and this is where that matters most: a protected
    // disk is exactly the one that does not mount *and* the one whose hits are
    // all in block 0. Answering "unclaimed" there would be the tool at its
    // least useful on the search it is best at.
    //
    // Unless it is a device, whose block 0 is an `RDSK` and not a bootblock at
    // all — parsing one as the other is a mistake this project has already
    // made twice.
    if matches!(image.rdb(), Ok(None)) {
        for block in 0..u64::from(geometry.reserved()) {
            out.insert(block, (Region::Bootblock, None, None));
        }
    }

    let Ok(volume) = image.volume() else {
        return (out, false);
    };

    // Then the rest of the structure, so a file can never be shadowed by one:
    // if a file's data really does sit in a reserved block the volume is
    // damaged, and the file is the more informative answer.
    out.insert(geometry.root_block().0, (Region::Rootblock, None, None));
    if let Ok(bitmap) =
        ade_filesystem::bitmap::Bitmap::read(image.source(), volume.geometry(), volume.rootblock())
    {
        for &block in &bitmap.blocks {
            out.insert(u64::from(block), (Region::Bitmap, None, None));
        }
    }

    let Ok(walk) = volume.walk(volume.root()) else {
        return (out, true);
    };
    for (path, entry) in &walk.entries {
        // A directory has no data blocks, but its header is where its name
        // lives — a search for a directory name should say which one.
        if entry.kind.is_directory() {
            out.insert(
                u64::from(entry.block),
                (Region::Directory, Some(path.clone()), Some(entry.block)),
            );
            continue;
        }
        if !entry.kind.is_file() {
            continue;
        }
        // The header block belongs to the file too: a search matching a
        // filename in its own header should say so rather than report an
        // unowned block.
        out.insert(
            u64::from(entry.block),
            (Region::File, Some(path.clone()), Some(entry.block)),
        );
        let Ok(blocks) = volume.file_blocks(entry) else {
            continue;
        };
        for block in blocks {
            out.insert(
                u64::from(block),
                (Region::File, Some(path.clone()), Some(entry.block)),
            );
        }
    }
    (out, true)
}

/// Turn a per-block map into runs covering every block.
fn coalesce(owners: &BTreeMap<u64, Owned>, blocks: u64, block_size: u32) -> Vec<Span> {
    let size = u64::from(block_size);
    let mut spans: Vec<Span> = Vec::new();

    for block in 0..blocks {
        let (region, owner, owner_block) = owners
            .get(&block)
            .map_or((Region::Unclaimed, None, None), |(r, o, b)| {
                (*r, o.clone(), *b)
            });

        // Extend the run when it is the same thing continuing. Two files with
        // adjacent blocks are two spans, because the owner is what a reader
        // wants named — merging them would produce one span attributed to
        // whichever file happened to come first.
        if let Some(last) = spans.last_mut() {
            if last.region == region && last.owner == owner {
                last.end = last.end.saturating_add(size);
                last.blocks = last.blocks.saturating_add(1);
                continue;
            }
        }
        spans.push(Span {
            start: block.saturating_mul(size),
            end: block.saturating_add(1).saturating_mul(size),
            block,
            blocks: 1,
            region,
            owner,
            owner_block,
        });
    }
    spans
}
