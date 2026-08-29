//! Searching an image, and saying where a hit landed (F-021).
//!
//! The byte search is [`ade_object::find`]. What this adds is the answer a hex
//! editor cannot give: **which file owns the block a match fell in**, or that
//! nothing does. "Found at offset 322,205" sends someone to a hex view;
//! "found in `s/startup-sequence`" ends the question, and "found in a block no
//! directory entry points at" is frequently the more interesting of the two.

use std::collections::BTreeMap;

use ade_object::find::{Match, Pattern, search};

use crate::json::Value;

/// What part of the disk a match landed in.
///
/// Measured across all 4,652 corpus images: `Copylock` appears on 103 of them,
/// and the region tells four different stories about the same string. On 86 it
/// is in the **bootblock** — protection lives there and in the trackloader it
/// starts, which is the part no directory entry points at. On 10 it is in the
/// **rootblock**, because those disks are *named* `Copylock(tm) Amiga`. On 11
/// it is in space nothing reaches, and on 5 it is inside a file.
///
/// So "no owning file" is the normal answer for the searches people most want
/// to run, and reporting it as *unallocated* would be wrong: a bootblock is
/// not unallocated space, it is the most deliberately written block on the
/// disk. A volume name is not either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// One match, with the file it belongs to where there is one.
#[derive(Debug, Clone)]
pub struct Found {
    /// Where it is.
    pub at: Match,
    /// The path of the entry whose blocks cover this one, if any.
    pub owner: Option<String>,
    /// What part of the disk it landed in.
    pub region: Region,
}

/// What a search found.
#[derive(Debug, Clone)]
pub struct Search {
    /// Every match, in offset order.
    pub matches: Vec<Found>,
    /// Bytes examined.
    pub scanned: u64,
    /// Whether the pattern was read as hex.
    pub was_hex: bool,
}

impl Search {
    /// Search an image, attributing each hit to a file where possible.
    #[must_use]
    pub fn run(bytes: &[u8], pattern: &Pattern) -> Self {
        // Mounted once. Both the block size and the owner map come from the
        // same image, and each `from_bytes` copies the whole disk — twice over
        // a 100 MB hardfile is the kind of quiet cost IMP-006 was about.
        let image = crate::Image::from_bytes(bytes.to_vec()).ok();
        let block_size = image.as_ref().map_or(512, |i| i.geometry().block_size());
        let map = image.as_ref().map(attribute).unwrap_or_default();
        let matches = search(bytes, pattern, block_size)
            .into_iter()
            .map(|at| {
                let (region, owner) = map
                    .get(&at.block)
                    .map_or((Region::Unclaimed, None), |(r, o)| (*r, o.clone()));
                Found { at, owner, region }
            })
            .collect();
        Self {
            matches,
            scanned: bytes.len() as u64,
            was_hex: pattern.is_hex,
        }
    }

    /// The search as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("scanned", Value::Num(self.scanned)),
            ("hex", Value::Bool(self.was_hex)),
            ("found", Value::Num(self.matches.len() as u64)),
            (
                "matches",
                Value::Arr(
                    self.matches
                        .iter()
                        .map(|m| {
                            Value::Obj(vec![
                                ("offset", Value::Num(m.at.offset)),
                                ("block", Value::Num(m.at.block)),
                                // Null means no directory entry points at this
                                // block — deleted, unallocated, or outside the
                                // filesystem entirely.
                                ("file", Value::opt(m.owner.as_ref(), Value::str)),
                                ("region", Value::str(m.region.name())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// What each block is, and what owns it, where the image holds a volume.
///
/// Empty when it does not, which is a quarter of real images — a search still
/// works there, it simply cannot say where a hit landed.
fn attribute(image: &crate::Image) -> BTreeMap<u64, (Region, Option<String>)> {
    let mut out: BTreeMap<u64, (Region, Option<String>)> = BTreeMap::new();
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
            out.insert(block, (Region::Bootblock, None));
        }
    }

    let Ok(volume) = image.volume() else {
        return out;
    };

    // Then the rest of the structure, so a file can never be shadowed by one:
    // if a file's data really does sit in a reserved block the volume is
    // damaged, and the file is the more informative answer.
    out.insert(geometry.root_block().0, (Region::Rootblock, None));
    if let Ok(bitmap) =
        ade_filesystem::bitmap::Bitmap::read(image.source(), volume.geometry(), volume.rootblock())
    {
        for &block in &bitmap.blocks {
            out.insert(u64::from(block), (Region::Bitmap, None));
        }
    }

    let Ok(walk) = volume.walk(volume.root()) else {
        return out;
    };
    for (path, entry) in &walk.entries {
        // A directory has no data blocks, but its header is where its name
        // lives — a search for a directory name should say which one.
        if entry.kind.is_directory() {
            out.insert(
                u64::from(entry.block),
                (Region::Directory, Some(path.clone())),
            );
            continue;
        }
        if !entry.kind.is_file() {
            continue;
        }
        // The header block belongs to the file too: a search matching a
        // filename in its own header should say so rather than report an
        // unowned block.
        out.insert(u64::from(entry.block), (Region::File, Some(path.clone())));
        let Ok(blocks) = volume.file_blocks(entry) else {
            continue;
        };
        for block in blocks {
            out.insert(u64::from(block), (Region::File, Some(path.clone())));
        }
    }
    out
}
