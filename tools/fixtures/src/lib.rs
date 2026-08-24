//! Deterministic Amiga disk-image fixtures, built in code.
//!
//! **D-010.** No disk image is committed to this repository. Every fixture the
//! test suite needs is constructed here, at test time, from the format
//! described in [SPEC.md](../../../Docs/SPEC.md).
//!
//! # Why this crate depends on nothing
//!
//! It would be shorter to write big-endian words with `ade-endian` and take
//! geometry from `ade-block`. It deliberately does neither.
//!
//! A generator sharing code with the parser shares the parser's misreadings: if
//! `ade-endian` swapped its bytes the wrong way, a fixture written through it
//! would be read back correctly and the test would pass. This crate is a
//! *second, independent* statement of the on-disk format, written from the
//! documentation rather than from ADE's interpretation of it. Where the two
//! disagree, that disagreement is the finding.
//!
//! That independence is bounded and worth being honest about. Both statements
//! were written by the same hand from the same sources, so a misreading of
//! *SPEC itself* survives in both. Only the black-box oracle over real images
//! (D-002) catches that class, which is why D-010 depends on both mechanisms
//! being run rather than either alone.
//!
//! # What it builds
//!
//! [`Volume`] produces structurally valid images: correct checksums, a real
//! hash table, an accurate bitmap. [`corrupt`] then breaks specific things in
//! specific ways, which is how AV-001 and AV-004 get fixtures at all — no
//! genuine disk contains a hash-chain loop.

// This crate has no inputs. It constructs data it fully controls, so the
// hostile-input lints that guard the parse paths (D-006, F-001) protect
// nothing here, while forcing every array write through a fallible path would
// obscure the format description this code exists to be. A panic here is a bug
// in a test helper, caught immediately, never a crash on a user's disk image.
#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "no untrusted input: this crate writes buffers it allocated itself"
)]
// Narrowing a shifted word to a byte is what big-endian serialisation *is*;
// flagging it here would mean an allow on every line of the writers.
#![allow(
    clippy::cast_possible_truncation,
    reason = "byte-order writers narrow by design"
)]

pub mod corrupt;
pub mod device;

/// Bytes per block. ADE supports other sizes for hard disks; fixtures are
/// floppies.
pub const BSIZE: usize = 512;

/// Reserved blocks at the start of a floppy volume — the boot blocks.
pub const RESERVED: u32 = 2;

/// Block primary type `T_HEADER`.
pub const T_HEADER: u32 = 2;
/// Block primary type `T_DATA`.
pub const T_DATA: u32 = 8;
/// Block primary type `T_LIST` (file extension blocks).
pub const T_LIST: u32 = 16;
/// Block secondary type `ST_ROOT`.
pub const ST_ROOT: u32 = 1;
/// Block secondary type `ST_USERDIR`.
pub const ST_USERDIR: u32 = 2;
/// Block secondary type `ST_FILE`, as an unsigned word (-3).
pub const ST_FILE: u32 = 0xFFFF_FFFD;
/// Block secondary type `ST_LINKFILE`, as an unsigned word (-4).
pub const ST_LINKFILE: u32 = 0xFFFF_FFFC;
/// Block secondary type `ST_LINKDIR`.
pub const ST_LINKDIR: u32 = 4;
/// Block secondary type `ST_SOFTLINK`.
pub const ST_SOFTLINK: u32 = 3;

// --- big-endian writers -----------------------------------------------------
//
// Written with explicit shifts rather than `to_be_bytes`, so this crate states
// the byte order itself instead of borrowing ADE's statement of it. This also
// keeps it clear of the C-001 lint without needing an exemption.

/// Write a big-endian `u32`.
pub fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off] = (v >> 24) as u8;
    buf[off + 1] = (v >> 16) as u8;
    buf[off + 2] = (v >> 8) as u8;
    buf[off + 3] = v as u8;
}

/// Write a big-endian `u16`.
pub fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off] = (v >> 8) as u8;
    buf[off + 1] = v as u8;
}

/// Read a big-endian `u32`.
#[must_use]
pub fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from(buf[off]) << 24
        | u32::from(buf[off + 1]) << 16
        | u32::from(buf[off + 2]) << 8
        | u32::from(buf[off + 3])
}

// --- checksums --------------------------------------------------------------

/// The normal block checksum: sum all longs with the field zeroed, then negate.
///
/// Used by every block type except the bootblock. Field lives at offset 20.
#[must_use]
pub fn normal_checksum(block: &[u8]) -> u32 {
    normal_checksum_at(block, 20)
}

/// The normal checksum for a block whose field is not at the usual offset.
///
/// Bitmap blocks keep theirs at 0 rather than 20 (ADF FAQ §4.3).
#[must_use]
pub fn normal_checksum_at(block: &[u8], field: usize) -> u32 {
    let mut sum: u32 = 0;
    for i in (0..block.len()).step_by(4) {
        let v = if i == field { 0 } else { get_u32(block, i) };
        sum = sum.wrapping_add(v);
    }
    sum.wrapping_neg()
}

/// The bootblock checksum: add with carry, then one's-complement.
///
/// A different algorithm at a different offset (4) from every other block.
/// Confusing the two is a silent-corruption bug, so they are separate
/// functions with separate names rather than a flag.
#[must_use]
pub fn bootblock_checksum(boot: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    for i in (0..boot.len()).step_by(4) {
        let v = if i == 4 { 0 } else { get_u32(boot, i) };
        let (next, carried) = sum.overflowing_add(v);
        sum = if carried { next.wrapping_add(1) } else { next };
    }
    !sum
}

// --- name hashing -----------------------------------------------------------

/// AmigaDOS `toupper`, in the non-international form.
#[must_use]
pub fn toupper(c: u8) -> u8 {
    if c.is_ascii_lowercase() {
        c - (b'a' - b'A')
    } else {
        c
    }
}

/// AmigaDOS `toupper` in international mode, which also folds Latin-1 accents.
///
/// Codes 224..=254 are the lowercase accented range; 247 is the division sign
/// and is excluded.
#[must_use]
pub fn intl_toupper(c: u8) -> u8 {
    if c.is_ascii_lowercase() || ((224..=254).contains(&c) && c != 247) {
        c - (b'a' - b'A')
    } else {
        c
    }
}

/// The directory hash, over a hash table of `ht_size` entries.
///
/// `international` selects the case-folding variant, and it is the *only*
/// difference between the two — which is why getting it wrong makes lookups
/// miss rather than fail (C-006).
#[must_use]
pub fn hash_name(name: &[u8], ht_size: u32, international: bool) -> u32 {
    let mut hash = name.len() as u32;
    for &c in name {
        hash = hash.wrapping_mul(13);
        hash = hash.wrapping_add(u32::from(if international {
            intl_toupper(c)
        } else {
            toupper(c)
        }));
        hash &= 0x7ff;
    }
    hash % ht_size
}

// --- volume builder ---------------------------------------------------------

/// A structurally valid Amiga volume under construction.
pub struct Volume {
    blocks: Vec<u8>,
    total_blocks: u32,
    dostype: u8,
    root: u32,
    /// Every block the bitmap occupies. A 512-byte bitmap block covers 4064
    /// blocks, so anything past about 2 MB needs more than one (BUG-006).
    bitmap_blocks: Vec<u32>,
    next_free: u32,
    name: String,
    /// Every directory block, root first. Directory caches are built for these
    /// and only these: a hard link to a directory has an `extension` field but
    /// no cache of its own.
    directories: Vec<Block>,
}

/// A logical block number within a fixture.
pub type Block = u32;

impl Volume {
    /// Hash-table entries in a root or directory block.
    pub const HT_SIZE: u32 = (BSIZE as u32) / 4 - 56;

    /// Start a double-density floppy volume with the given dostype flags byte.
    ///
    /// `0` is OFS, `1` FFS, `3` FFS-INTL, `5` FFS with dircache, `7` FFS-LNFS,
    /// and so on — see SPEC §Dostypes.
    #[must_use]
    pub fn dd(dostype: u8) -> Self {
        Self::new(80, 2, 11, dostype)
    }

    /// Start a high-density floppy volume.
    #[must_use]
    pub fn hd(dostype: u8) -> Self {
        Self::new(80, 2, 22, dostype)
    }

    /// Start a volume of arbitrary geometry.
    ///
    /// Cylinder counts above 80 are legitimate — 81 to 83 occur in the wild
    /// (SPEC §Corpus observations) — and are the reason this is parameterised.
    #[must_use]
    pub fn new(cylinders: u32, heads: u32, sectors: u32, dostype: u8) -> Self {
        let total_blocks = cylinders * heads * sectors;
        // rootKey = (numReserved + highKey) / 2, per ADF FAQ §4.2 (C-007).
        let root = (RESERVED + total_blocks - 1) / 2;
        // One bitmap block covers (BSIZE/4 - 1) * 32 bits, and the map starts
        // at the first non-reserved block.
        let bits_per_block = ((BSIZE / 4 - 1) * 32) as u32;
        let covered = total_blocks.saturating_sub(RESERVED);
        let needed = covered.div_ceil(bits_per_block).max(1);
        // They sit immediately after the rootblock, as AmigaDOS lays them out.
        let bitmap_blocks: Vec<u32> = (0..needed).map(|i| root + 1 + i).collect();
        Self {
            blocks: vec![0u8; total_blocks as usize * BSIZE],
            total_blocks,
            dostype,
            root,
            bitmap_blocks,
            next_free: RESERVED,
            name: "Fixture".to_owned(),
            directories: vec![root],
        }
    }

    /// Set the volume name (max 30 characters).
    #[must_use]
    pub fn named(mut self, name: &str) -> Self {
        name.clone_into(&mut self.name);
        self
    }

    /// Whether this volume's dostype implies international hashing (C-006).
    #[must_use]
    pub fn is_international(&self) -> bool {
        matches!(self.dostype, 4..=7) || self.dostype & 0b010 != 0
    }

    /// Whether data blocks carry the 24-byte OFS header (C-005).
    #[must_use]
    pub fn is_ofs(&self) -> bool {
        self.dostype & 1 == 0
    }

    /// Block number of the rootblock.
    #[must_use]
    pub fn root(&self) -> Block {
        self.root
    }

    /// Total blocks in the volume.
    #[must_use]
    pub fn total_blocks(&self) -> u32 {
        self.total_blocks
    }

    fn block_mut(&mut self, n: Block) -> &mut [u8] {
        let o = n as usize * BSIZE;
        &mut self.blocks[o..o + BSIZE]
    }

    fn alloc(&mut self) -> Block {
        loop {
            let b = self.next_free;
            assert!(b < self.total_blocks, "fixture volume is full");
            self.next_free += 1;
            if b != self.root && !self.bitmap_blocks.contains(&b) {
                return b;
            }
        }
    }

    /// Add a file to the root directory.
    ///
    /// Writes a file header, the data blocks (OFS or FFS as the dostype
    /// dictates), and links the header into the root hash table, following the
    /// same-hash chain to its tail if the slot is occupied.
    ///
    /// # Panics
    /// If the volume runs out of blocks, or the name exceeds 30 characters.
    pub fn add_file(&mut self, name: &str, data: &[u8]) -> Block {
        assert!(
            name.len() <= 30,
            "classic filenames are 30 characters at most"
        );
        let header = self.alloc();
        let payload = if self.is_ofs() { BSIZE - 24 } else { BSIZE };
        let chunks: Vec<&[u8]> = if data.is_empty() {
            Vec::new()
        } else {
            data.chunks(payload).collect()
        };
        let ht = Self::HT_SIZE as usize;

        // Every data block first, so `next_data` and `seq_num` can be numbered
        // across the whole file rather than per header block.
        let data_blocks: Vec<Block> = (0..chunks.len()).map(|_| self.alloc()).collect();

        // Pointers past the header's table live in file extension blocks,
        // chained from its `extension` field (SPEC §Files). One extension
        // block per group of `ht` beyond the first (IMP-004).
        let extra_groups = data_blocks.len().saturating_sub(ht).div_ceil(ht.max(1));
        let ext_blocks: Vec<Block> = (0..extra_groups).map(|_| self.alloc()).collect();

        let is_ofs = self.is_ofs();
        for (i, (&blk, chunk)) in data_blocks.iter().zip(&chunks).enumerate() {
            let next = data_blocks.get(i + 1).copied().unwrap_or(0);
            let b = self.block_mut(blk);
            if is_ofs {
                put_u32(b, 0, T_DATA);
                put_u32(b, 4, header);
                put_u32(b, 8, i as u32 + 1); // seq_num counts from 1, file-wide
                put_u32(b, 12, chunk.len() as u32);
                put_u32(b, 16, next);
                b[24..24 + chunk.len()].copy_from_slice(chunk);
                let ck = normal_checksum(b);
                put_u32(b, 20, ck);
            } else {
                b[..chunk.len()].copy_from_slice(chunk);
            }
        }

        let root = self.root;
        let first_group: Vec<Block> = data_blocks.iter().take(ht).copied().collect();
        {
            let b = self.block_mut(header);
            put_u32(b, 0, T_HEADER);
            put_u32(b, 4, header);
            put_u32(b, 8, first_group.len() as u32);
            put_u32(b, 16, data_blocks.first().copied().unwrap_or(0));
            // data_blocks[] runs BACKWARDS: the first is at index ht-1.
            for (i, &blk) in first_group.iter().enumerate() {
                put_u32(b, 24 + (ht - 1 - i) * 4, blk);
            }
            put_u32(b, BSIZE - 188, data.len() as u32);
            put_u32(b, BSIZE - 92, 1); // days: 0 is treated as illegal
            write_bcpl(b, BSIZE - 80, name.as_bytes(), 30);
            put_u32(b, BSIZE - 12, root);
            put_u32(b, BSIZE - 8, ext_blocks.first().copied().unwrap_or(0));
            put_u32(b, BSIZE - 4, ST_FILE);
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
        }

        for (g, &ext) in ext_blocks.iter().enumerate() {
            let from = ht.saturating_mul(g + 1);
            let group: Vec<Block> = data_blocks.iter().skip(from).take(ht).copied().collect();
            let next_ext = ext_blocks.get(g + 1).copied().unwrap_or(0);
            let b = self.block_mut(ext);
            // An extension block is T_LIST, not T_HEADER, but keeps the file's
            // secondary type and the same reversed pointer table.
            put_u32(b, 0, T_LIST);
            put_u32(b, 4, ext);
            put_u32(b, 8, group.len() as u32);
            for (i, &blk) in group.iter().enumerate() {
                put_u32(b, 24 + (ht - 1 - i) * 4, blk);
            }
            put_u32(b, BSIZE - 12, header); // parent: the file header
            put_u32(b, BSIZE - 8, next_ext);
            put_u32(b, BSIZE - 4, ST_FILE);
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
        }

        self.link_into(self.root, header, name.as_bytes());
        header
    }

    /// Add a subdirectory to the root directory.
    ///
    /// # Panics
    /// If the volume runs out of blocks, or the name exceeds 30 characters.
    pub fn add_dir(&mut self, name: &str) -> Block {
        assert!(name.len() <= 30, "classic names are 30 characters at most");
        let blk = self.alloc();
        let root = self.root;
        {
            let b = self.block_mut(blk);
            put_u32(b, 0, T_HEADER);
            put_u32(b, 4, blk);
            put_u32(b, BSIZE - 92, 1);
            write_bcpl(b, BSIZE - 80, name.as_bytes(), 30);
            put_u32(b, BSIZE - 12, root);
            put_u32(b, BSIZE - 4, ST_USERDIR);
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
        }
        self.link_into(self.root, blk, name.as_bytes());
        self.directories.push(blk);
        blk
    }

    /// Add a hard link to an existing entry.
    ///
    /// A link block carries no data of its own: `real_entry` names the block it
    /// stands for, and the target's `next_link` chains the links pointing at
    /// it. The secondary type distinguishes a link to a file (`ST_LINKFILE`,
    /// -4) from one to a directory (`ST_LINKDIR`, 4).
    ///
    /// Links to directories are the reason traversal needs a visited set: they
    /// are legal, and they make cycles reachable on an uncorrupted disk
    /// (AV-001, ADF FAQ §4.6).
    ///
    /// # Panics
    /// If the volume runs out of blocks, or the name exceeds 30 characters.
    pub fn add_hardlink(&mut self, name: &str, target: Block, to_dir: bool) -> Block {
        assert!(name.len() <= 30, "classic names are 30 characters at most");
        let blk = self.alloc();
        let root = self.root;
        {
            let b = self.block_mut(blk);
            put_u32(b, 0, T_HEADER);
            put_u32(b, 4, blk);
            put_u32(b, BSIZE - 92, 1);
            write_bcpl(b, BSIZE - 80, name.as_bytes(), 30);
            // real_entry: the block this link stands for.
            put_u32(b, BSIZE - 44, target);
            put_u32(b, BSIZE - 12, root);
            put_u32(b, BSIZE - 4, if to_dir { ST_LINKDIR } else { ST_LINKFILE });
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
        }
        // Chain this link onto the target's next_link list.
        {
            let b = self.block_mut(target);
            put_u32(b, BSIZE - 40, blk);
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
        }
        self.link_into(self.root, blk, name.as_bytes());
        blk
    }

    /// Insert `entry` into `dir`'s hash table, appending to the chain tail.
    fn link_into(&mut self, dir: Block, entry: Block, name: &[u8]) {
        let intl = self.is_international();
        let slot = hash_name(name, Self::HT_SIZE, intl) as usize;
        let off = 24 + slot * 4;
        let head = get_u32(self.block_mut(dir), off);
        if head == 0 {
            let b = self.block_mut(dir);
            put_u32(b, off, entry);
            let ck = normal_checksum(b);
            put_u32(b, 20, ck);
            return;
        }
        // Walk the same-hash chain to its tail and append there.
        let mut cur = head;
        loop {
            let next = get_u32(self.block_mut(cur), BSIZE - 16);
            if next == 0 {
                break;
            }
            cur = next;
        }
        let b = self.block_mut(cur);
        put_u32(b, BSIZE - 16, entry);
        let ck = normal_checksum(b);
        put_u32(b, 20, ck);
    }

    /// Finish the volume: write bootblock, rootblock and bitmap, and return the
    /// image bytes.
    #[must_use]
    pub fn build(mut self) -> Vec<u8> {
        // Before the bitmap, which must account for the blocks the caches take.
        self.write_dircaches();
        self.write_rootblock();
        self.write_bitmap();
        self.write_bootblock();
        self.blocks
    }

    /// Build a directory cache for every directory, on a `DOS\4`/`DOS\5`
    /// volume.
    ///
    /// The records are read back out of the entry blocks that were actually
    /// written, rather than accumulated as the volume was built. That is
    /// deliberate: a cache assembled from the generator's own bookkeeping could
    /// agree with the generator while both disagreed with the disk. Reading the
    /// blocks back means the cache describes what is really there.
    ///
    /// To build a *stale* cache on purpose — the case the health report exists
    /// to catch — see `corrupt::stale_dircache`.
    fn write_dircaches(&mut self) {
        if !matches!(self.dostype, 4 | 5) {
            return;
        }
        for dir in self.directories.clone() {
            let entries = self.entries_of(dir);
            if entries.is_empty() {
                continue;
            }
            let blocks = self.write_dircache_chain(dir, &entries);
            if let Some(&first) = blocks.first() {
                // On a directory the `extension` field points at the cache.
                let b = self.block_mut(dir);
                put_u32(b, BSIZE - 8, first);
                let ck = normal_checksum(b);
                put_u32(b, 20, ck);
            }
        }
    }

    /// Every entry block a directory's hash table reaches, in slot order.
    fn entries_of(&mut self, dir: Block) -> Vec<Block> {
        let mut out = Vec::new();
        for slot in 0..Self::HT_SIZE as usize {
            let mut cur = get_u32(self.block_mut(dir), 24 + slot * 4);
            while cur != 0 {
                out.push(cur);
                cur = get_u32(self.block_mut(cur), BSIZE - 16);
            }
        }
        out
    }

    /// Write the cache blocks for one directory, returning them in chain order.
    fn write_dircache_chain(&mut self, dir: Block, entries: &[Block]) -> Vec<Block> {
        let mut chain: Vec<Block> = Vec::new();
        let mut pending: Vec<Vec<u8>> = entries
            .iter()
            .map(|&e| self.dircache_record(e))
            .collect::<Vec<_>>();
        pending.reverse();

        while !pending.is_empty() {
            let blk = self.alloc();
            chain.push(blk);
            let mut body: Vec<u8> = Vec::new();
            let mut count = 0u32;
            // Fill until the next record will not fit. A record never spans
            // two blocks: SPEC has no continuation, so a block simply ends.
            while let Some(record) = pending.last() {
                if 24 + body.len() + record.len() > BSIZE {
                    break;
                }
                body.extend_from_slice(record);
                count += 1;
                pending.pop();
            }
            assert!(count > 0, "a single dircache record must fit in a block");

            let b = self.block_mut(blk);
            put_u32(b, 0, 33); // T_DIRCACHE
            put_u32(b, 4, blk);
            put_u32(b, 8, dir);
            put_u32(b, 12, count);
            b[24..24 + body.len()].copy_from_slice(&body);
        }

        // Chain them, then checksum: the checksum covers the `next` field.
        for i in 0..chain.len() {
            let next = chain.get(i + 1).copied().unwrap_or(0);
            let blk = chain[i];
            let b = self.block_mut(blk);
            put_u32(b, 16, next);
            let ck = normal_checksum_at(b, 20);
            put_u32(b, 20, ck);
        }
        chain
    }

    /// One cache record, read back from the entry block it describes.
    fn dircache_record(&mut self, entry: Block) -> Vec<u8> {
        let b = self.block_mut(entry);
        let size = get_u32(b, BSIZE - 188);
        let protect = get_u32(b, BSIZE - 192);
        let days = get_u32(b, BSIZE - 92) as u16;
        let mins = get_u32(b, BSIZE - 88) as u16;
        let ticks = get_u32(b, BSIZE - 84) as u16;
        let sec_type = get_u32(b, BSIZE - 4);
        let name_len = usize::from(b[BSIZE - 80]).min(30);
        let name = b[BSIZE - 79..BSIZE - 79 + name_len].to_vec();
        let comment_len = usize::from(b[BSIZE - 184]).min(22);
        let comment = b[BSIZE - 183..BSIZE - 183 + comment_len].to_vec();

        // The fixed part is 24 bytes; the name and comment follow it. Written
        // through the C-001 seam like everything else, rather than by
        // to_be_bytes at the call site.
        let mut r = vec![0u8; 24];
        put_u32(&mut r, 0, entry);
        put_u32(&mut r, 4, size);
        put_u32(&mut r, 8, protect);
        put_u16(&mut r, 12, 0); // UID
        put_u16(&mut r, 14, 0); // GID
        put_u16(&mut r, 16, days);
        put_u16(&mut r, 18, mins);
        put_u16(&mut r, 20, ticks);
        // The secondary type is one signed byte here against a word there.
        r[22] = (sec_type & 0xFF) as u8;
        r[23] = name_len as u8;
        r.extend_from_slice(&name);
        r.push(comment_len as u8);
        r.extend_from_slice(&comment);
        // Records are word-aligned.
        if r.len() % 2 != 0 {
            r.push(0);
        }
        r
    }

    fn write_bootblock(&mut self) {
        let o = 0;
        self.blocks[o] = b'D';
        self.blocks[o + 1] = b'O';
        self.blocks[o + 2] = b'S';
        self.blocks[o + 3] = self.dostype;
        // The rootblock field: 880 even on HD, which is why C-007 says compute
        // it rather than read it. Fixtures reproduce the quirk faithfully.
        put_u32(&mut self.blocks, 8, 880);
        let ck = bootblock_checksum(&self.blocks[..BSIZE * 2]);
        put_u32(&mut self.blocks, 4, ck);
    }

    fn write_rootblock(&mut self) {
        let root = self.root;
        let bitmap_blocks = self.bitmap_blocks.clone();
        let name = self.name.clone();
        let b = self.block_mut(root);
        put_u32(b, 0, T_HEADER);
        put_u32(b, 12, Self::HT_SIZE);
        put_u32(b, BSIZE - 200, 0xFFFF_FFFF); // bm_flag: -1 means valid
        // The rootblock holds 25 pointers directly; beyond that they go in a
        // bm_ext chain, which this generator does not yet emit.
        assert!(
            bitmap_blocks.len() <= 25,
            "volumes needing more than 25 bitmap blocks require a bm_ext chain, \
             which the fixture generator does not build yet"
        );
        for (i, &bm) in bitmap_blocks.iter().enumerate() {
            put_u32(b, BSIZE - 196 + i * 4, bm);
        }
        for (field, value) in [(92, 1u32), (88, 0), (84, 0)] {
            put_u32(b, BSIZE - field, value); // r_days / r_mins / r_ticks
        }
        write_bcpl(b, BSIZE - 80, name.as_bytes(), 30);
        put_u32(b, BSIZE - 40, 1); // v_days
        put_u32(b, BSIZE - 28, 1); // c_days
        put_u32(b, BSIZE - 4, ST_ROOT);
        let ck = normal_checksum(b);
        put_u32(b, 20, ck);
    }

    fn write_bitmap(&mut self) {
        let total = self.total_blocks;
        let bitmap_blocks = self.bitmap_blocks.clone();
        let used: Vec<u32> = (RESERVED..total)
            .filter(|&b| b == self.root || bitmap_blocks.contains(&b) || b < self.next_free)
            .collect();
        let bits_per_block = ((BSIZE / 4 - 1) * 32) as u32;

        for (i, &bm) in bitmap_blocks.iter().enumerate() {
            let base = (i as u32).saturating_mul(bits_per_block);
            let b = self.block_mut(bm);
            // A SET bit means FREE, and the map starts at block RESERVED, not 0.
            b[4..].fill(0xFF);
            for &blk in &used {
                let idx = blk - RESERVED;
                // Each block covers its own window of the map.
                if idx < base || idx >= base + bits_per_block {
                    continue;
                }
                let local = idx - base;
                let long = 4 + (local / 32) as usize * 4;
                let bit = local % 32;
                let v = get_u32(b, long) & !(1 << bit);
                put_u32(b, long, v);
            }
            // The bitmap block is the one exception to the usual layout: its
            // checksum sits at offset 0 and the map runs from 4. Writing it at
            // 20 — where every other block type keeps it — silently overwrites
            // the map words covering blocks 130..161 (BUG-004).
            let ck = normal_checksum_at(b, 0);
            put_u32(b, 0, ck);
        }
    }
}

/// Write a BCPL-style string: a length byte followed by the characters, padded.
pub(crate) fn write_bcpl(buf: &mut [u8], off: usize, s: &[u8], max: usize) {
    let n = s.len().min(max);
    buf[off] = n as u8;
    buf[off + 1..off + 1 + n].copy_from_slice(&s[..n]);
}
