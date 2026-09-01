//! Recovering files nothing points at any more (F-030).
//!
//! # Why this can be done honestly, when LNFS cannot
//!
//! A carver is the classic thing you cannot check. It produces files that no
//! directory entry claims, so there is nothing to compare them against, and a
//! carver checked only by its own author is what D-002 gave up ADFlib's
//! accumulated knowledge to avoid. That is why this sat on the candidate list
//! marked "blocked on verifiability, not effort".
//!
//! **It turns out an OFS file proves itself.** Every OFS data block carries a
//! header: its type (`T_DATA`), the block of the file header that owns it, its
//! sequence number in the file, and its own checksum. So a carved file can be
//! verified from the disk alone — the header names a list of blocks, and each
//! of those blocks independently names the header back. Three agreements per
//! block, none of them ADE's opinion. This is the same property that makes the
//! MFM decode self-evidencing, and it is the only reason this feature exists.
//!
//! Measured over 600 corpus images: **50 hold orphaned headers** — 573 files
//! and 52 directories, all with valid checksums — and of the file headers,
//! **346 are fully self-evidencing**, 21 partly, and 204 recover nothing.
//!
//! # FFS cannot do this, and is not pretended to
//!
//! An FFS data block is raw payload with no header at all. A carved FFS file
//! yields its name, its size and its block list from the header, and **no way
//! to confirm a single byte of the contents**. Those are reported as
//! header-only, and a caller that writes them out is told what it is writing.

use ade_filesystem::entry::{Entry, EntryKind};

use crate::json::Value;

/// How well a carved file is supported by the disk itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Every data block names this header back and checksums correctly.
    ///
    /// The strongest answer available, and it needs nothing external.
    SelfEvident,
    /// Some data blocks agree and some do not — the file is partly overwritten.
    Partial {
        /// Blocks that agreed.
        good: usize,
        /// Blocks that did not.
        bad: usize,
    },
    /// The header is sound; nothing confirms the data.
    ///
    /// Either the file is FFS, whose data blocks carry no header to check, or
    /// its blocks have been reused. **Not the same as recovered**, and the
    /// difference must survive into whatever reports it.
    HeaderOnly,
}

impl Evidence {
    /// The name this is reported by. Part of the JSON surface (F-015).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SelfEvident => "self-evident",
            Self::Partial { .. } => "partial",
            Self::HeaderOnly => "header only",
        }
    }
}

/// A header nothing in the directory tree points at.
#[derive(Debug, Clone)]
pub struct Carved {
    /// The block the header sits in.
    pub block: u32,
    /// Its name, as stored. Latin-1, and not necessarily a legal host name.
    pub name: String,
    /// The size the header claims.
    pub size: u32,
    /// Whether it is a file or a directory.
    pub kind: EntryKind,
    /// How far the disk backs it up.
    pub evidence: Evidence,
    /// The data blocks that agreed, in order.
    pub blocks: Vec<u32>,
}

/// Every orphaned header on the volume, in block order.
///
/// "Orphaned" means the block is in space the directory tree does not reach —
/// [`crate::layout`]'s `unclaimed` — and holds something that parses as a file
/// or directory header. A block the tree *does* reach is a live file and is
/// not carving.
#[must_use]
pub fn carve(image: &crate::Image) -> Vec<Carved> {
    let map = crate::layout::Layout::of(image);
    let block_size = u64::from(image.geometry().block_size());

    let mut out = Vec::new();
    for span in &map.spans {
        if span.region != crate::layout::Region::Unclaimed {
            continue;
        }
        for offset in 0..span.blocks {
            let block = span.block.saturating_add(offset);
            let Ok(number) = u32::try_from(block) else {
                continue;
            };
            let raw = image.read_range(block.saturating_mul(block_size), block_size);
            let Ok(entry) = Entry::parse(&raw, number) else {
                continue;
            };
            if !entry.looks_like_an_entry() {
                continue;
            }
            if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
                continue;
            }
            out.push(examine(image, &entry));
        }
    }
    out
}

/// The data blocks a file header names, following its extension chain.
///
/// Done here rather than through a mounted `Volume`, because the disks worth
/// carving include the ones that do not mount — 3 of 600 corpus images hold 91
/// orphaned headers with no filesystem left to ask. A volume-only carver would
/// have refused exactly the disks where recovery is the whole point.
///
/// Carries a visited set and a cap, for the same reason every other chain walk
/// in ADE does: a crafted or corrupt header can point at itself (AV-001), and
/// this one is *expected* to be reading damaged structures.
fn data_blocks(image: &crate::Image, entry: &Entry) -> Vec<u32> {
    /// More than a double-density floppy holds, so a chain that does not end
    /// stops here rather than running until something else does.
    const CAP: usize = 4096;

    let block_size = u64::from(image.geometry().block_size());
    let mut out = Vec::new();
    let Ok(size) = usize::try_from(block_size) else {
        return out;
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut header = entry.block;

    while seen.insert(header) && out.len() < CAP {
        let raw = image.read_range(u64::from(header).saturating_mul(block_size), block_size);
        if raw.len() != size {
            break;
        }
        // The block table runs downward from BSIZE-204, `high_seq` entries of
        // it in use.
        let Ok(high) = ade_endian::u32_at(&raw, 8) else {
            break;
        };
        for index in 0..high.min(u32::try_from(CAP).unwrap_or(u32::MAX)) {
            let Some(at) = size
                .checked_sub(204)
                .and_then(|base| base.checked_sub(index as usize * 4))
            else {
                break;
            };
            let Ok(block) = ade_endian::u32_at(&raw, at) else {
                break;
            };
            if block == 0 {
                continue;
            }
            out.push(block);
            if out.len() >= CAP {
                break;
            }
        }
        // The extension block, at BSIZE-8, or zero when there is none.
        let Some(at) = size.checked_sub(8) else { break };
        match ade_endian::u32_at(&raw, at) {
            Ok(next) if next != 0 => header = next,
            _ => break,
        }
    }
    out
}

/// Work out how far the disk supports one orphaned header.
fn examine(image: &crate::Image, entry: &Entry) -> Carved {
    let mut good = Vec::new();
    let mut bad = 0usize;

    if entry.kind == EntryKind::File {
        for block in data_blocks(image, entry) {
            if owns(image, block, entry.block) {
                good.push(block);
            } else {
                bad = bad.saturating_add(1);
            }
        }
    }

    let evidence = match (good.len(), bad, entry.kind) {
        // A directory has no data blocks, and a file whose blocks all
        // disagree has nothing left that confirms it. Same answer, and it is
        // the honest one in both cases.
        (_, _, EntryKind::Directory) | (0, _, _) => Evidence::HeaderOnly,
        (_, 0, _) => Evidence::SelfEvident,
        (g, b, _) => Evidence::Partial { good: g, bad: b },
    };

    Carved {
        block: entry.block,
        name: entry.name_lossy(),
        size: entry.byte_size,
        kind: entry.kind,
        evidence,
        blocks: good,
    }
}

/// Whether `block` is an OFS data block that names `header` as its owner.
///
/// Three agreements: the block says it is data, it says which header owns it,
/// and its checksum holds. An FFS data block says none of this — it is raw
/// payload — so this is `false` there, which is why an FFS carve can never be
/// better than header-only.
fn owns(image: &crate::Image, block: u32, header: u32) -> bool {
    /// `T_DATA`, the OFS data-block type.
    const T_DATA: u32 = 8;

    let size = u64::from(image.geometry().block_size());
    let raw = image.read_range(u64::from(block).saturating_mul(size), size);
    if raw.len() as u64 != size {
        return false;
    }
    let (Ok(kind), Ok(owner)) = (ade_endian::u32_at(&raw, 0), ade_endian::u32_at(&raw, 4)) else {
        return false;
    };
    kind == T_DATA
        && owner == header
        && ade_block::checksum::normal_at(&raw, 20)
            .is_some_and(|sum| ade_endian::u32_at(&raw, 20).is_ok_and(|stored| stored == sum))
}

/// What `carve` found, as a document (F-015).
///
/// Built here rather than in the CLI so the field inventory in
/// `src/api/tests/schema.rs` can reach it: a document assembled in a front end
/// is one D-015's mechanism cannot see change.
#[must_use]
pub fn to_json(found: &[Carved]) -> Value {
    Value::Obj(vec![
        ("found", Value::Num(found.len() as u64)),
        ("carved", Value::Arr(found.iter().map(one).collect())),
    ])
}

fn one(c: &Carved) -> Value {
    Value::Obj(vec![
        ("block", Value::Num(u64::from(c.block))),
        ("name", Value::str(&c.name)),
        ("size", Value::Num(u64::from(c.size))),
        ("kind", Value::str(kind_name(c.kind))),
        ("evidence", Value::str(c.evidence.name())),
        // A count, not the list: a large carve has thousands of blocks and the
        // list is the same information the block chain already holds.
        ("data_blocks", Value::Num(c.blocks.len() as u64)),
    ])
}

/// The name a kind is reported by. Part of the JSON surface (F-015).
#[must_use]
pub const fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "directory",
        _ => "file",
    }
}

/// The bytes of a carved file, assembled from the blocks that confirmed it.
///
/// Empty for a header-only carve, which is the point: there is nothing
/// confirmed to hand over. A partial carve returns the confirmed part, which
/// is shorter than [`Carved::size`] and must be labelled as such wherever it
/// is written — see [`recovered_name`].
///
/// Lives here rather than in a front end because reading an OFS data block's
/// 24-byte header and honouring its used-length field is a statement about the
/// format, and two front ends must not be able to disagree with the engine
/// about it.
#[must_use]
pub fn contents(image: &crate::Image, carved: &Carved) -> Vec<u8> {
    let block_size = u64::from(image.geometry().block_size());
    let mut bytes = Vec::with_capacity(carved.blocks.len().saturating_mul(488));

    for block in &carved.blocks {
        let raw = image.read_range(u64::from(*block).saturating_mul(block_size), block_size);
        // An OFS data block is a 24-byte header then payload, and its own
        // header says how much of the payload is real.
        let Ok(used) = ade_endian::u32_at(&raw, 12) else {
            continue;
        };
        let to = 24usize
            .saturating_add(usize::try_from(used).unwrap_or(0))
            .min(raw.len());
        if let Some(slice) = raw.get(24..to) {
            bytes.extend_from_slice(slice);
        }
    }
    bytes
}

/// The filename a carved file should be written under.
///
/// Two things it encodes, both load-bearing:
///
/// The **block number**, because two lost files routinely share a name — a
/// deleted file and the one that replaced it usually do — and the header's
/// block is the only thing that makes each answer unique. It is also the
/// number to go back to in a hex view.
///
/// A **`.partial` suffix** when the recovery is incomplete. One corpus file
/// claims 40,000 bytes and confirms 12,688; handing that over under its own
/// name gives somebody a truncated file that looks whole, which is the one
/// outcome a recovery tool must not produce.
#[must_use]
pub fn recovered_name(carved: &Carved) -> String {
    format!(
        "{:05}-{}{}",
        carved.block,
        crate::unpack::host_name(&carved.name),
        if matches!(carved.evidence, Evidence::Partial { .. }) {
            ".partial"
        } else {
            ""
        }
    )
}

/// Whether there is anything confirmed to write out.
///
/// False for a header-only carve. A file on disk carrying the right name and
/// unconfirmed bytes is worse than no file, because somebody will believe it.
#[must_use]
pub fn is_recoverable(carved: &Carved) -> bool {
    carved.evidence != Evidence::HeaderOnly && !carved.blocks.is_empty()
}
