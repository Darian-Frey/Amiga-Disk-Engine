//! Mounting a volume and walking it.
//!
//! Consumes an [`ade_block::BlockSource`]; knows nothing of how the blocks were
//! obtained.
//!
//! # Cycles are legal, not merely hostile (AV-001)
//!
//! AmigaDOS permits hard links to directories, "which opens the way to endless
//! recursion" (ADF FAQ §4.6). A traversal cycle is therefore reachable on a
//! structurally valid, uncorrupted disk — so cycle detection is a correctness
//! requirement, not a defence bolted on for malformed input.
//!
//! It has to be a **visited set of block numbers**, not a depth limit: a depth
//! limit cannot tell a legitimately deep tree from a two-block loop, and would
//! either truncate real disks or spin on fake ones. Every chain walked here —
//! hash chains, file extension chains, and the OFS data chain — carries one.
//!
//! # Every pointer is checked before it is followed (AV-004)
//!
//! Block numbers come from the image, which is untrusted. They are validated
//! against the geometry before use, which the type system enforces:
//! [`ade_block::BlockSource::read_block`] takes a `ValidBlock` that only
//! [`ade_block::Geometry::validate`] can mint.

use std::collections::HashSet;

use ade_block::{BlockError, BlockIndex, BlockSource, Geometry, read_at};

use crate::{
    bootblock::Bootblock,
    dostype::{Dostype, FileSystem},
    entry::{Entry, T_LIST},
    rootblock::Rootblock,
};

/// Bytes of metadata at the head of an OFS data block (C-005).
pub const OFS_HEADER: usize = 24;

/// Why a volume operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    /// A block could not be read.
    Block(BlockError),
    /// A block was not the structure it should have been.
    Malformed {
        /// Where.
        block: u32,
        /// What was wrong.
        detail: String,
    },
    /// A chain looped back on itself (AV-001).
    Cycle {
        /// The block reached for the second time.
        block: u32,
        /// Which chain was being walked.
        chain: &'static str,
    },
    /// A path component did not exist.
    NotFound {
        /// The component that was missing.
        name: String,
    },
    /// A path component existed but was not a directory.
    NotADirectory {
        /// The component.
        name: String,
    },
    /// The volume has no rootblock where one should be.
    NoRootblock {
        /// Where it was looked for.
        block: u32,
    },
}

impl core::fmt::Display for FsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Block(e) => write!(f, "{e}"),
            Self::Malformed { block, detail } => write!(f, "block {block}: {detail}"),
            Self::Cycle { block, chain } => {
                write!(f, "{chain} chain loops at block {block}")
            }
            Self::NotFound { name } => write!(f, "no such entry: {name}"),
            Self::NotADirectory { name } => write!(f, "not a directory: {name}"),
            Self::NoRootblock { block } => write!(f, "no rootblock at block {block}"),
        }
    }
}

impl core::error::Error for FsError {}

impl From<BlockError> for FsError {
    fn from(e: BlockError) -> Self {
        Self::Block(e)
    }
}

/// A mounted volume.
pub struct Volume<'a> {
    source: &'a dyn BlockSource,
    geometry: Geometry,
    dostype: Option<Dostype>,
    root: u32,
    rootblock: Rootblock,
}

impl<'a> Volume<'a> {
    /// Mount a volume from a block source.
    ///
    /// The dostype comes from the bootblock, but its absence is not fatal: 7%
    /// of real images have a foreign bootblock and some of those mount. When
    /// there is no dostype, the filesystem is inferred from the rootblock and
    /// international hashing is assumed, which is the safer default — INTL
    /// folding differs from plain only for accented characters.
    ///
    /// # Errors
    /// [`FsError::NoRootblock`] if the computed rootblock location does not
    /// hold one, or a read error.
    pub fn mount(source: &'a dyn BlockSource) -> Result<Self, FsError> {
        let geometry = *source.geometry();
        let root = geometry.root_block();

        // The bootblock spans two blocks, so it is read as two.
        let bsize = geometry.block_size() as usize;
        let mut boot = vec![0u8; bsize.saturating_mul(2)];
        for i in 0..2usize {
            let start = i.saturating_mul(bsize);
            let Some(slice) = boot.get_mut(start..start.saturating_add(bsize)) else {
                break;
            };
            read_at(source, BlockIndex(i as u64), slice)?;
        }
        let dostype = Bootblock::parse(&boot).ok().and_then(|b| b.dostype.ok());

        // Block numbers are u32 throughout the on-disk format; a geometry
        // whose rootblock exceeds that is not an Amiga volume.
        let root_u32 =
            u32::try_from(root.0).map_err(|_| FsError::NoRootblock { block: u32::MAX })?;

        let mut block = vec![0u8; bsize];
        read_at(source, root, &mut block)?;
        let rootblock = Rootblock::parse(&block).map_err(|e| FsError::Malformed {
            block: root_u32,
            detail: e.to_string(),
        })?;
        if !rootblock.looks_like_a_rootblock() {
            return Err(FsError::NoRootblock { block: root_u32 });
        }

        Ok(Self {
            source,
            geometry,
            dostype,
            root: root_u32,
            rootblock,
        })
    }

    /// The rootblock, already parsed.
    #[must_use]
    pub fn rootblock(&self) -> &Rootblock {
        &self.rootblock
    }

    /// The dostype, if the bootblock carried a readable one.
    #[must_use]
    pub fn dostype(&self) -> Option<Dostype> {
        self.dostype
    }

    /// The rootblock's block number.
    #[must_use]
    pub const fn root(&self) -> u32 {
        self.root
    }

    /// Which filesystem this volume uses.
    ///
    /// Falls back to FFS when there is no dostype: an FFS reader treats the
    /// whole data block as payload, so mistaking OFS for FFS yields visible
    /// garbage, while the reverse silently discards 24 bytes per block. The
    /// louder failure is the right default.
    #[must_use]
    pub fn filesystem(&self) -> FileSystem {
        self.dostype.map_or(FileSystem::Ffs, Dostype::filesystem)
    }

    /// Whether directory hashing folds international characters (C-006).
    #[must_use]
    pub fn is_international(&self) -> bool {
        self.dostype.is_none_or(Dostype::is_international)
    }

    /// Hash-table entries per directory block.
    #[must_use]
    pub fn hash_table_size(&self) -> u32 {
        // The rootblock states it; fall back to the value the block size
        // implies if it claims something impossible.
        let implied = self
            .geometry
            .block_size()
            .saturating_div(4)
            .saturating_sub(56);
        let stated = self.rootblock.hash_table_size;
        if stated > 0 && stated <= implied {
            stated
        } else {
            implied
        }
    }

    fn read_block(&self, block: u32) -> Result<Vec<u8>, FsError> {
        let mut buf = vec![0u8; self.geometry.block_size() as usize];
        read_at(self.source, BlockIndex(u64::from(block)), &mut buf)?;
        Ok(buf)
    }

    /// Read the entry that occupies a given block.
    ///
    /// Useful for walking *up* a tree: a directory does not appear in its own
    /// listing, so recovering its name and parent means reading its block
    /// directly.
    ///
    /// # Errors
    /// A read error, or a block that is not shaped like a directory entry.
    pub fn entry_at(&self, block: u32) -> Result<Entry, FsError> {
        let raw = self.read_block(block)?;
        let entry = Entry::parse(&raw, block).map_err(|e| FsError::Malformed {
            block,
            detail: e.to_string(),
        })?;
        if entry.looks_like_an_entry() {
            Ok(entry)
        } else {
            Err(FsError::Malformed {
                block,
                detail: format!(
                    "not a directory entry: type {:#x}, secondary {:#x}",
                    entry.block_type, entry.secondary_type
                ),
            })
        }
    }

    /// The path of an entry relative to the volume root.
    ///
    /// Walks up the `parent` chain with a visited set: a corrupt or looping
    /// parent pointer must terminate the walk, not the process (AV-001).
    /// Returns the raw name components, since Amiga names are Latin-1 and
    /// re-encoding them would break comparison against anything else that
    /// wrote them to a filesystem verbatim.
    #[must_use]
    pub fn path_components(&self, entry: &Entry) -> Vec<Vec<u8>> {
        let mut parts = vec![entry.name.clone()];
        let mut seen: HashSet<u32> = HashSet::from([entry.block]);
        let mut cur = entry.parent;
        while cur != 0 && cur != self.root && seen.insert(cur) {
            let Ok(dir) = self.entry_at(cur) else { break };
            parts.push(dir.name.clone());
            cur = dir.parent;
        }
        parts.reverse();
        parts
    }

    /// Every entry in a directory, in hash-table order.
    ///
    /// Walks each hash slot and its same-hash chain, carrying a visited set so
    /// a cycle terminates the walk rather than the process (AV-001). A cycle is
    /// reported through [`Listing::cycles`] rather than as an error, because
    /// the entries found before it are still good data.
    ///
    /// # Errors
    /// A read error, or a block that is not a directory.
    pub fn list(&self, dir: u32) -> Result<Listing, FsError> {
        let block = self.read_block(dir)?;
        let header = Entry::parse(&block, dir).map_err(|e| FsError::Malformed {
            block: dir,
            detail: e.to_string(),
        })?;
        if dir != self.root && !header.kind.is_directory() {
            return Err(FsError::NotADirectory {
                name: header.name_lossy(),
            });
        }

        let mut listing = Listing::default();
        let ht = self.hash_table_size() as usize;
        let mut seen: HashSet<u32> = HashSet::new();

        for slot in 0..ht {
            let offset = 24usize.saturating_add(slot.saturating_mul(4));
            let Ok(first) = ade_endian::u32_at(&block, offset) else {
                continue;
            };
            let mut next = first;
            while next != 0 {
                if !seen.insert(next) {
                    listing.cycles.push(FsError::Cycle {
                        block: next,
                        chain: "hash",
                    });
                    break;
                }
                // AV-004: validate before dereferencing.
                if self.geometry.validate(BlockIndex(u64::from(next))).is_err() {
                    listing.faults.push(FsError::Malformed {
                        block: dir,
                        detail: format!("hash slot {slot} points outside the volume: {next}"),
                    });
                    break;
                }
                let raw = match self.read_block(next) {
                    Ok(r) => r,
                    Err(e) => {
                        listing.faults.push(e);
                        break;
                    }
                };
                match Entry::parse(&raw, next) {
                    Ok(entry) => {
                        let chain = entry.hash_chain;
                        if entry.looks_like_an_entry() {
                            listing.entries.push(entry);
                        } else {
                            listing.faults.push(FsError::Malformed {
                                block: next,
                                detail: format!(
                                    "not a directory entry: type {:#x}, secondary {:#x}",
                                    entry.block_type, entry.secondary_type
                                ),
                            });
                        }
                        next = chain;
                    }
                    Err(e) => {
                        listing.faults.push(FsError::Malformed {
                            block: next,
                            detail: e.to_string(),
                        });
                        break;
                    }
                }
            }
        }
        Ok(listing)
    }

    /// Resolve a slash-separated path from the volume root.
    ///
    /// Comparison is case-insensitive and uses the volume's hashing mode, so
    /// it matches what AmigaDOS would find.
    ///
    /// # Errors
    /// [`FsError::NotFound`] or [`FsError::NotADirectory`] for a bad path.
    pub fn lookup(&self, path: &str) -> Result<Entry, FsError> {
        let mut current = self.root;
        let mut last: Option<Entry> = None;
        for component in path.split('/').filter(|c| !c.is_empty() && *c != ".") {
            let listing = self.list(current)?;
            let found = listing
                .entries
                .into_iter()
                .find(|e| self.names_match(&e.name, component.as_bytes()))
                .ok_or_else(|| FsError::NotFound {
                    name: component.to_owned(),
                })?;
            current = if found.kind.is_directory() {
                found.block
            } else {
                // A non-directory can only be the final component; if it is
                // not, the next iteration's `list` will report it.
                found.block
            };
            last = Some(found);
        }
        last.map_or_else(
            || {
                self.read_block(self.root).and_then(|b| {
                    Entry::parse(&b, self.root).map_err(|e| FsError::Malformed {
                        block: self.root,
                        detail: e.to_string(),
                    })
                })
            },
            Ok,
        )
    }

    /// Whether two names are equal under this volume's case folding.
    #[must_use]
    pub fn names_match(&self, a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let fold = |c: u8| {
            if self.is_international() {
                crate::hash::intl_toupper(c)
            } else {
                crate::hash::toupper(c)
            }
        };
        a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
    }

    /// Read a file's contents.
    ///
    /// Uses the `data_blocks[]` tables in the header and any extension blocks —
    /// the only route FFS offers. The table runs **backwards**: the first data
    /// block sits at the highest index (ADF FAQ §4.4), and iterating forwards
    /// would return the file reversed.
    ///
    /// Returns [`FileContents`] rather than a bare `Vec` so a short read cannot
    /// pass unnoticed. Real disks do disagree with themselves: in a 400-image
    /// sample, 8 files of 11,087 yielded fewer bytes than their header
    /// declared, because the OFS data blocks' own `data_size` fields summed to
    /// less. ADE returns what it found and says so, rather than padding to the
    /// declared length — inventing bytes would be worse than reporting a gap
    /// (D-006, F-010).
    ///
    /// # Errors
    /// A read error, a cycle in the extension chain, or an out-of-range
    /// pointer.
    pub fn read_file(&self, entry: &Entry) -> Result<FileContents, FsError> {
        if !entry.kind.is_file() {
            return Err(FsError::NotADirectory {
                name: entry.name_lossy(),
            });
        }
        let ofs = self.filesystem() == FileSystem::Ofs;
        let payload = if ofs {
            (self.geometry.block_size() as usize).saturating_sub(OFS_HEADER)
        } else {
            self.geometry.block_size() as usize
        };

        let mut out: Vec<u8> = Vec::with_capacity(entry.byte_size as usize);
        let mut header_block = entry.block;
        let mut header_raw = self.read_block(header_block)?;
        let mut seen_headers: HashSet<u32> = HashSet::from([header_block]);
        let ht = self.hash_table_size() as usize;

        loop {
            let high_seq = ade_endian::u32_at(&header_raw, 8).unwrap_or(0) as usize;
            // A corrupt high_seq must not make us read the whole block as
            // pointers: clamp to the table the block actually has.
            for i in 0..high_seq.min(ht) {
                let index = ht.saturating_sub(1).saturating_sub(i);
                let offset = 24usize.saturating_add(index.saturating_mul(4));
                let Ok(ptr) = ade_endian::u32_at(&header_raw, offset) else {
                    continue;
                };
                if ptr == 0 {
                    continue;
                }
                if self.geometry.validate(BlockIndex(u64::from(ptr))).is_err() {
                    return Err(FsError::Malformed {
                        block: header_block,
                        detail: format!("data pointer outside the volume: {ptr}"),
                    });
                }
                let data = self.read_block(ptr)?;
                if ofs {
                    let size = ade_endian::u32_at(&data, 12).unwrap_or(0) as usize;
                    let take = size.min(payload);
                    if let Some(slice) = data.get(OFS_HEADER..OFS_HEADER.saturating_add(take)) {
                        out.extend_from_slice(slice);
                    }
                } else {
                    out.extend_from_slice(&data);
                }
                if out.len() >= entry.byte_size as usize {
                    break;
                }
            }

            let next =
                ade_endian::u32_at(&header_raw, header_raw.len().saturating_sub(8)).unwrap_or(0);
            if next == 0 {
                break;
            }
            if !seen_headers.insert(next) {
                return Err(FsError::Cycle {
                    block: next,
                    chain: "file extension",
                });
            }
            if self.geometry.validate(BlockIndex(u64::from(next))).is_err() {
                return Err(FsError::Malformed {
                    block: header_block,
                    detail: format!("extension pointer outside the volume: {next}"),
                });
            }
            header_raw = self.read_block(next)?;
            let block_type = ade_endian::u32_at(&header_raw, 0).unwrap_or(0);
            if block_type != T_LIST {
                return Err(FsError::Malformed {
                    block: next,
                    detail: format!("expected T_LIST ({T_LIST}), found {block_type}"),
                });
            }
            header_block = next;
        }

        // `byte_size` caps the tail: the last data block is padded, and FFS
        // carries no length of its own. It is a cap, never a target — the
        // buffer is never extended to reach it.
        out.truncate(entry.byte_size as usize);
        Ok(FileContents {
            declared_size: entry.byte_size,
            short_by: entry
                .byte_size
                .saturating_sub(u32::try_from(out.len()).unwrap_or(u32::MAX)),
            bytes: out,
        })
    }

    /// Walk the whole tree from `start`, depth first.
    ///
    /// Carries one visited set across the entire walk, so a hard link that
    /// points back up the tree terminates rather than recursing (AV-001).
    ///
    /// # Errors
    /// A read error on the starting directory.
    pub fn walk(&self, start: u32) -> Result<Vec<(String, Entry)>, FsError> {
        let mut out = Vec::new();
        let mut visited: HashSet<u32> = HashSet::from([start]);
        let mut stack = vec![(String::new(), start)];
        while let Some((prefix, dir)) = stack.pop() {
            let Ok(listing) = self.list(dir) else {
                continue;
            };
            for entry in listing.entries {
                let path = if prefix.is_empty() {
                    entry.name_lossy()
                } else {
                    format!("{prefix}/{}", entry.name_lossy())
                };
                if entry.kind.is_directory() && visited.insert(entry.block) {
                    stack.push((path.clone(), entry.block));
                }
                out.push((path, entry));
            }
        }
        Ok(out)
    }
}

/// The result of listing a directory: what was found, and what went wrong.
///
/// Faults do not abort the listing. A directory with one broken hash chain
/// still has usable entries in its other slots, and a forensic tool should
/// return them rather than nothing.
#[derive(Debug, Default)]
pub struct Listing {
    /// Entries found, in hash-table order.
    pub entries: Vec<Entry>,
    /// Cycles encountered (AV-001).
    pub cycles: Vec<FsError>,
    /// Other problems encountered while walking.
    pub faults: Vec<FsError>,
}

impl Listing {
    /// Whether anything went wrong.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.cycles.is_empty() && self.faults.is_empty()
    }

    /// Entries that are directories.
    pub fn directories(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind.is_directory())
    }

    /// Entries that are files.
    pub fn files(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind.is_file())
    }
}

/// A file's contents, with whatever the disk failed to deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContents {
    /// The bytes actually recovered.
    pub bytes: Vec<u8>,
    /// The size the file header claimed.
    pub declared_size: u32,
    /// How many bytes short of the declared size the read came up.
    ///
    /// Non-zero means the data blocks did not supply everything the header
    /// promised. On real disks this is usually genuine damage rather than a
    /// reader fault, but either way it is the caller's to report, not ADE's to
    /// hide.
    pub short_by: u32,
}

impl FileContents {
    /// Whether the file read complete.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.short_by == 0
    }

    /// The recovered bytes, discarding the completeness information.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
