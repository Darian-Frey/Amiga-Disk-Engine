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
    let mut sum: u32 = 0;
    for i in (0..block.len()).step_by(4) {
        let v = if i == 20 { 0 } else { get_u32(block, i) };
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
    bitmap_block: u32,
    next_free: u32,
    name: String,
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
        Self {
            blocks: vec![0u8; total_blocks as usize * BSIZE],
            total_blocks,
            dostype,
            root,
            bitmap_block: root + 1,
            next_free: RESERVED,
            name: "Fixture".to_owned(),
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
            if b != self.root && b != self.bitmap_block {
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
        assert!(
            chunks.len() <= Self::HT_SIZE as usize,
            "fixture files must fit one header block; extension blocks are not built yet"
        );

        let data_blocks: Vec<Block> = (0..chunks.len()).map(|_| self.alloc()).collect();
        let is_ofs = self.is_ofs();
        for (i, (&blk, chunk)) in data_blocks.iter().zip(&chunks).enumerate() {
            let next = data_blocks.get(i + 1).copied().unwrap_or(0);
            let b = self.block_mut(blk);
            if is_ofs {
                put_u32(b, 0, T_DATA);
                put_u32(b, 4, header);
                put_u32(b, 8, i as u32 + 1); // seq_num counts from 1
                put_u32(b, 12, chunk.len() as u32);
                put_u32(b, 16, next);
                b[24..24 + chunk.len()].copy_from_slice(chunk);
                let ck = normal_checksum(b);
                put_u32(b, 20, ck);
            } else {
                b[..chunk.len()].copy_from_slice(chunk);
            }
        }

        let ht = Self::HT_SIZE as usize;
        let root = self.root;
        {
            let b = self.block_mut(header);
            put_u32(b, 0, T_HEADER);
            put_u32(b, 4, header);
            put_u32(b, 8, data_blocks.len() as u32);
            put_u32(b, 16, data_blocks.first().copied().unwrap_or(0));
            // data_blocks[] runs BACKWARDS: the first is at index ht-1.
            for (i, &blk) in data_blocks.iter().enumerate() {
                put_u32(b, 24 + (ht - 1 - i) * 4, blk);
            }
            put_u32(b, BSIZE - 188, data.len() as u32);
            put_u32(b, BSIZE - 92, 1); // days: 0 is treated as illegal
            write_bcpl(b, BSIZE - 80, name.as_bytes(), 30);
            put_u32(b, BSIZE - 12, root);
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
        self.write_rootblock();
        self.write_bitmap();
        self.write_bootblock();
        self.blocks
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
        let bm = self.bitmap_block;
        let name = self.name.clone();
        let b = self.block_mut(root);
        put_u32(b, 0, T_HEADER);
        put_u32(b, 12, Self::HT_SIZE);
        put_u32(b, BSIZE - 200, 0xFFFF_FFFF); // bm_flag: -1 means valid
        put_u32(b, BSIZE - 196, bm);
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
        let used: Vec<u32> = (RESERVED..total)
            .filter(|&b| b == self.root || b == self.bitmap_block || b < self.next_free)
            .collect();
        let bm = self.bitmap_block;
        let b = self.block_mut(bm);
        // A SET bit means FREE, and the map starts at block RESERVED, not 0.
        b[4..].fill(0xFF);
        for blk in used {
            let idx = blk - RESERVED;
            let long = 4 + (idx / 32) as usize * 4;
            let bit = idx % 32;
            let v = get_u32(b, long) & !(1 << bit);
            put_u32(b, long, v);
        }
        let ck = normal_checksum(b);
        put_u32(b, 20, ck);
    }
}

/// Write a BCPL-style string: a length byte followed by the characters, padded.
fn write_bcpl(buf: &mut [u8], off: usize, s: &[u8], max: usize) {
    let n = s.len().min(max);
    buf[off] = n as u8;
    buf[off + 1..off + 1 + n].copy_from_slice(&s[..n]);
}
