//! Directory cache blocks (`DOS\4`, `DOS\5`).
//!
//! A dircache is exactly what its name says: a **cache**. Every fact it holds
//! — name, size, protection, datestamp, comment — is duplicated from the entry
//! block it points at. AmigaDOS maintains it so that listing a directory costs
//! one sequential read instead of one seek per entry.
//!
//! That redundancy is what makes it forensically interesting. Two independent
//! descriptions of the same directory either agree, or the disk is damaged or
//! the cache is stale. SPEC §Directory cache blocks says the disagreement is a
//! health finding and neither side is to be silently preferred; [`compare`]
//! computes it and declines to choose.
//!
//! # Why this exists beyond completeness
//!
//! Without it, a dircache block is a block nothing reaches. The bitmap marks it
//! in use, the tree walk never visits it, and the health report calls it
//! orphaned — 19 such false positives on the Workbench 3.1 install disk alone.
//! A reader that does not understand dircache does not merely miss a feature;
//! it reports lost space that is not lost.
//!
//! # Reading discipline
//!
//! Records are variable-length and their lengths come off the disk, so every
//! one is bounded before it is trusted: the name is capped at 30 bytes and the
//! comment at 22, per SPEC, and a record that would run past the block ends the
//! block rather than wrapping into the next. The chain carries a visited set
//! (AV-001) and every block number is validated (AV-004).

use std::collections::HashSet;

use ade_block::{BlockIndex, BlockSource, Geometry, checksum, read_at};
use ade_endian::{i16_at, u8_at, u16_at, u32_at};

use crate::{datestamp::Datestamp, entry::Protection, volume::FsError};

/// Primary type of a dircache block, `T_DIRCACHE`.
pub const T_DIRCACHE: u32 = 33;

/// Bytes of header before the first record.
const HEADER_BYTES: usize = 24;

/// Longest name a record may declare (SPEC §Directory cache blocks).
const MAX_NAME: usize = 30;

/// Longest comment a record may declare.
const MAX_COMMENT: usize = 22;

/// Shortest a record can be: the fixed fields, a one-byte name, no comment.
const MIN_RECORD: usize = 26;

/// A cap on the chain, so a corrupt `next_dirc` cannot walk forever even if
/// the visited set were somehow defeated.
const MAX_BLOCKS: usize = 4096;

/// One cached directory entry.
///
/// Every field duplicates the entry block at [`Self::header`]. Where they
/// disagree, see [`compare`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The entry block this record describes.
    pub header: u32,
    /// File size in bytes. Zero for a directory or a link.
    pub size: u32,
    /// Protection flags.
    pub protection: Protection,
    /// Owner user id.
    pub uid: u16,
    /// Owner group id.
    pub gid: u16,
    /// Last modification.
    ///
    /// Stored as three **16-bit** fields here, against the entry block's three
    /// 32-bit ones. Widened on read, so the two are comparable.
    pub altered: Datestamp,
    /// Secondary type, as stored — signed, so `-3` is a file.
    pub secondary_type: i8,
    /// Name, as stored. Latin-1, not UTF-8.
    pub name: Vec<u8>,
    /// Comment, as stored.
    pub comment: Vec<u8>,
}

impl Record {
    /// The name as a lossy string, for reporting.
    #[must_use]
    pub fn name_lossy(&self) -> String {
        self.name.iter().map(|&b| char::from(b)).collect()
    }

    /// The secondary type widened the way an entry block stores it.
    ///
    /// The dircache holds one signed byte where the entry block holds a 32-bit
    /// word, so `-3` here is `0xFFFF_FFFD` there. Comparing them without this
    /// is the obvious way to report every file on the disk as a mismatch.
    #[must_use]
    #[allow(
        clippy::cast_sign_loss,
        reason = "sign extension is the conversion: -3 must become 0xFFFFFFFD"
    )]
    pub const fn secondary_type_widened(&self) -> u32 {
        self.secondary_type as u32
    }
}

/// One dircache block, with the records it held.
#[derive(Debug, Clone)]
pub struct DirCacheBlock {
    /// Where it sits.
    pub block: u32,
    /// Whether its checksum verifies.
    pub checksum_valid: bool,
    /// Self pointer, which should equal [`Self::block`].
    pub own_key: u32,
    /// The directory this caches.
    pub parent: u32,
    /// How many records the header claims.
    pub declared_records: u32,
    /// The next block in the chain, or 0 for the last.
    pub next: u32,
    /// The records actually parsed, which may be fewer than declared.
    pub records: Vec<Record>,
}

/// Every dircache block reachable from a directory, and what went wrong.
#[derive(Debug, Clone)]
pub struct Chain {
    /// The blocks, in chain order.
    pub blocks: Vec<DirCacheBlock>,
    /// Faults found walking it. A fault stops the walk; what was read before
    /// it is kept.
    pub faults: Vec<FsError>,
}

impl Chain {
    /// The block numbers the chain occupies.
    ///
    /// This is what the health report adds to its reachable set, and the whole
    /// reason the orphan count on a `DOS\5` disk was wrong.
    #[must_use]
    pub fn block_numbers(&self) -> Vec<u32> {
        self.blocks.iter().map(|b| b.block).collect()
    }

    /// Every record across every block, in order.
    #[must_use]
    pub fn records(&self) -> Vec<&Record> {
        self.blocks.iter().flat_map(|b| b.records.iter()).collect()
    }
}

/// Parse one dircache block.
///
/// A record whose declared lengths would run past the end of the block ends
/// the block: the remaining declared records are not invented, and the
/// shortfall shows up as `records.len() < declared_records`.
///
/// # Errors
/// [`FsError::Malformed`] if the block is too short to hold a header, or its
/// primary type is not [`T_DIRCACHE`].
pub fn parse(buf: &[u8], block: u32) -> Result<DirCacheBlock, FsError> {
    let bad = |detail: String| FsError::Malformed { block, detail };
    let at = |o: usize| -> Result<u32, FsError> {
        u32_at(buf, o).map_err(|e| FsError::Malformed {
            block,
            detail: e.to_string(),
        })
    };

    let block_type = at(0)?;
    if block_type != T_DIRCACHE {
        return Err(bad(format!(
            "expected a dircache block (type {T_DIRCACHE}), found type {block_type}"
        )));
    }

    let declared_records = at(0x0c)?;
    let mut records = Vec::new();
    let mut offset = HEADER_BYTES;

    // Bounded by the declared count *and* by the block, so neither a wild
    // count nor a record that lies about its length can run away.
    for _ in 0..declared_records {
        match parse_record(buf, offset) {
            Some((record, next)) => {
                records.push(record);
                offset = next;
            }
            None => break,
        }
    }

    Ok(DirCacheBlock {
        block,
        checksum_valid: checksum::sums_to_zero(buf),
        own_key: at(0x04)?,
        parent: at(0x08)?,
        declared_records,
        next: at(0x10)?,
        records,
    })
}

/// Parse one record, returning it and the offset the next one starts at.
///
/// `None` when the record does not fit, which ends the block. Every length is
/// checked against both its documented maximum and the buffer, because both
/// come off the disk.
fn parse_record(buf: &[u8], offset: usize) -> Option<(Record, usize)> {
    if offset.checked_add(MIN_RECORD)? > buf.len() {
        return None;
    }

    let name_len = usize::from(u8_at(buf, offset.checked_add(23)?).ok()?);
    if name_len == 0 || name_len > MAX_NAME {
        return None;
    }
    let name_at = offset.checked_add(24)?;
    let comment_len_at = name_at.checked_add(name_len)?;
    let comment_len = usize::from(u8_at(buf, comment_len_at).ok()?);
    if comment_len > MAX_COMMENT {
        return None;
    }
    let comment_at = comment_len_at.checked_add(1)?;
    let end = comment_at.checked_add(comment_len)?;
    if end > buf.len() {
        return None;
    }

    let record = Record {
        header: u32_at(buf, offset).ok()?,
        size: u32_at(buf, offset.checked_add(4)?).ok()?,
        protection: Protection(u32_at(buf, offset.checked_add(8)?).ok()?),
        uid: u16_at(buf, offset.checked_add(12)?).ok()?,
        gid: u16_at(buf, offset.checked_add(14)?).ok()?,
        // Three 16-bit fields where an entry block has three 32-bit ones.
        // Negative is nonsense for a datestamp; clamping to zero makes it
        // read as unset rather than as a date in the far future.
        altered: Datestamp {
            days: u32::try_from(i16_at(buf, offset.checked_add(16)?).ok()?).unwrap_or(0),
            mins: u32::try_from(i16_at(buf, offset.checked_add(18)?).ok()?).unwrap_or(0),
            ticks: u32::try_from(i16_at(buf, offset.checked_add(20)?).ok()?).unwrap_or(0),
        },
        secondary_type: i8::from_ne_bytes([*buf.get(offset.checked_add(22)?)?]),
        name: buf.get(name_at..comment_len_at)?.to_vec(),
        comment: buf.get(comment_at..end)?.to_vec(),
    };

    // Records are word-aligned: an odd length carries a trailing pad byte.
    let next = if end % 2 == 0 {
        end
    } else {
        end.checked_add(1)?
    };
    Some((record, next))
}

/// Walk the dircache chain hanging off a directory's `extension` field.
///
/// `start` is that field. Zero means the directory has no cache, which is
/// normal even on a dircache volume — an empty directory needs none.
///
/// The walk stops at the first fault and keeps what it read, on the same
/// reasoning as the partition list: half a cache still tells you something.
#[must_use]
pub fn read_chain(source: &dyn BlockSource, geometry: &Geometry, start: u32) -> Chain {
    let mut blocks = Vec::new();
    let mut faults = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut buf = vec![0u8; geometry.block_size() as usize];
    let mut next = start;

    while next != 0 {
        if blocks.len() >= MAX_BLOCKS {
            faults.push(FsError::Malformed {
                block: next,
                detail: "dircache chain exceeded its structural cap".to_owned(),
            });
            break;
        }
        if !seen.insert(next) {
            faults.push(FsError::Cycle {
                block: next,
                chain: "dircache",
            });
            break;
        }
        let Ok(valid) = geometry.validate(BlockIndex(u64::from(next))) else {
            faults.push(FsError::Malformed {
                block: next,
                detail: "dircache pointer is outside the volume".to_owned(),
            });
            break;
        };
        let _ = valid;
        if let Err(e) = read_at(source, BlockIndex(u64::from(next)), &mut buf) {
            faults.push(FsError::Block(e));
            break;
        }
        match parse(&buf, next) {
            Ok(b) => {
                let following = b.next;
                blocks.push(b);
                next = following;
            }
            Err(e) => {
                faults.push(e);
                break;
            }
        }
    }

    Chain { blocks, faults }
}

/// How a cached record differs from the entry block it describes.
///
/// Named rather than boolean, because which field disagrees says what kind of
/// damage happened: a stale size is an interrupted write, a stale name is
/// something else entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disagreement {
    /// The cache names an entry the directory's hash chains do not reach.
    NotInDirectory {
        /// The block the record points at.
        block: u32,
        /// The name the cache gives it.
        name: String,
    },
    /// The directory holds an entry the cache does not list.
    NotInCache {
        /// The entry block.
        block: u32,
        /// Its name.
        name: String,
    },
    /// Both describe the entry, but a field differs.
    FieldDiffers {
        /// The entry block.
        block: u32,
        /// Which field.
        field: &'static str,
        /// What the cache says.
        cached: String,
        /// What the entry block says.
        actual: String,
    },
}

impl core::fmt::Display for Disagreement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotInDirectory { block, name } => write!(
                f,
                "the cache lists {name:?} (block {block}), which the directory does not contain"
            ),
            Self::NotInCache { block, name } => write!(
                f,
                "the directory contains {name:?} (block {block}), which the cache omits"
            ),
            Self::FieldDiffers {
                block,
                field,
                cached,
                actual,
            } => write!(
                f,
                "block {block}: the cache says {field} is {cached}, the entry says {actual}"
            ),
        }
    }
}

/// Compare a directory's cache against the entries its hash chains reach.
///
/// Neither side is preferred: the result is the difference, for the caller to
/// report. SPEC is explicit that a reader must not silently choose.
///
/// `entries` is what the hash walk found. Comparison is by block number, since
/// that is the one field both sides agree is an identity.
#[must_use]
pub fn compare(cached: &[&Record], entries: &[crate::entry::Entry]) -> Vec<Disagreement> {
    let mut out = Vec::new();

    for record in cached {
        let Some(entry) = entries.iter().find(|e| e.block == record.header) else {
            out.push(Disagreement::NotInDirectory {
                block: record.header,
                name: record.name_lossy(),
            });
            continue;
        };
        if record.name != entry.name {
            out.push(Disagreement::FieldDiffers {
                block: entry.block,
                field: "the name",
                cached: format!("{:?}", record.name_lossy()),
                actual: format!("{:?}", entry.name_lossy()),
            });
        }
        if record.size != entry.byte_size {
            out.push(Disagreement::FieldDiffers {
                block: entry.block,
                field: "the size",
                cached: record.size.to_string(),
                actual: entry.byte_size.to_string(),
            });
        }
        if record.secondary_type_widened() != entry.secondary_type {
            out.push(Disagreement::FieldDiffers {
                block: entry.block,
                field: "the secondary type",
                cached: format!("{:#x}", record.secondary_type_widened()),
                actual: format!("{:#x}", entry.secondary_type),
            });
        }
        if record.protection.0 != entry.protection.0 {
            out.push(Disagreement::FieldDiffers {
                block: entry.block,
                field: "the protection bits",
                cached: format!("{:#x}", record.protection.0),
                actual: format!("{:#x}", entry.protection.0),
            });
        }
        if record.comment != entry.comment {
            out.push(Disagreement::FieldDiffers {
                block: entry.block,
                field: "the comment",
                cached: format!("{:?}", record.comment_lossy()),
                actual: format!("{:?}", entry.comment_lossy()),
            });
        }
    }

    for entry in entries {
        if !cached.iter().any(|r| r.header == entry.block) {
            out.push(Disagreement::NotInCache {
                block: entry.block,
                name: entry.name_lossy(),
            });
        }
    }

    out
}

impl Record {
    /// The comment as a lossy string, for reporting.
    #[must_use]
    pub fn comment_lossy(&self) -> String {
        self.comment.iter().map(|&b| char::from(b)).collect()
    }
}
