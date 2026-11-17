//! The Rigid Disk Block: a hard disk's partition table.
//!
//! An RDB sits within the first 16 blocks of a device and describes how the
//! rest is divided (SPEC §Hard disks — RDB). It is 256 bytes regardless of the
//! device's block size, and it anchors three linked lists: partitions, bad
//! blocks, and filesystem drivers.
//!
//! # Two traps worth naming
//!
//! **List terminators are -1, not 0.** Every list in the filesystem proper ends
//! at zero; the RDB family ends at `0xFFFFFFFF`. A reader that checks for zero
//! walks off the end of the table into whatever block 4,294,967,295 decodes as.
//!
//! **`SizeBlock` is in longs.** The DOSEnvVec records a partition's block size
//! as 128 for a 512-byte block, not 512. Taking it at face value gives
//! partitions a quarter of their real size.
//!
//! # The dostype in a partition block is advisory
//!
//! > "The first two blocks of a partition contain a Bootblock. You have to use
//! > it to determine the correct file system... **Don't rely only on the PART
//! > and FSHD 'DosType' field.**" — ADF FAQ §6.3
//!
//! ADE reads both and reports disagreement rather than picking a winner: which
//! one is wrong is a finding about the disk (F-010).

use std::collections::HashSet;

use ade_block::{BlockIndex, BlockSource, Geometry, checksum, read_at};
use ade_endian::{i32_at, u32_at};

use crate::volume::FsError;

/// `RDSK` — the Rigid Disk Block.
pub const RDSK: &[u8; 4] = b"RDSK";
/// `PART` — a partition block.
pub const PART: &[u8; 4] = b"PART";
/// `FSHD` — a filesystem header block.
pub const FSHD: &[u8; 4] = b"FSHD";

/// How far into a device an RDB may sit.
pub const SEARCH_BLOCKS: u32 = 16;

/// The value that ends an RDB-family list.
///
/// **Not zero.** The filesystem's own lists end at zero; these end at -1.
pub const END_OF_LIST: u32 = 0xFFFF_FFFF;

/// The Rigid Disk Block itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RigidDiskBlock {
    /// Where it was found.
    pub block: u32,
    /// Whether its checksum verifies.
    pub checksum_valid: bool,
    /// Device block size in bytes.
    pub block_size: u32,
    /// First partition block, or [`END_OF_LIST`].
    pub partition_list: u32,
    /// First filesystem header block, or [`END_OF_LIST`].
    pub filesystem_list: u32,
    /// First bad-block block, or [`END_OF_LIST`].
    pub bad_block_list: u32,
    /// Physical cylinders.
    pub cylinders: u32,
    /// Sectors per track.
    pub sectors: u32,
    /// Heads.
    pub heads: u32,
    /// The first cylinder partitions may use: those before it hold the RDB
    /// and its lists.
    pub lo_cylinder: u32,
    /// The last cylinder partitions may use.
    pub hi_cylinder: u32,
    /// Blocks per cylinder — should equal `heads × sectors`.
    pub cylinder_blocks: u32,
    /// Highest block used by the reserved area.
    pub high_rdsk_block: u32,
    /// Drive vendor, product and revision, as stored.
    pub vendor: Vec<u8>,
    /// Product string.
    pub product: Vec<u8>,
    /// Revision string.
    pub revision: Vec<u8>,
}

impl RigidDiskBlock {
    /// Find and parse the RDB, if the device has one.
    ///
    /// Scans the first [`SEARCH_BLOCKS`] blocks: an RDB "must exist within the
    /// first 16 blocks" but need not be at zero.
    ///
    /// # Errors
    /// A read error. A device with no RDB is `Ok(None)` — most images have
    /// none, and that is not a fault.
    pub fn find(source: &dyn BlockSource, geometry: &Geometry) -> Result<Option<Self>, FsError> {
        let bsize = geometry.block_size() as usize;
        let mut buf = vec![0u8; bsize];
        for block in
            0..SEARCH_BLOCKS.min(u32::try_from(geometry.total_blocks()).unwrap_or(u32::MAX))
        {
            if read_at(source, BlockIndex(u64::from(block)), &mut buf).is_err() {
                continue;
            }
            if buf.get(..4) != Some(RDSK) {
                continue;
            }
            return Ok(Some(Self::parse(&buf, block)?));
        }
        Ok(None)
    }

    /// Parse an RDB from a block already read.
    ///
    /// # Errors
    /// [`FsError::Malformed`] if the block is too short.
    pub fn parse(buf: &[u8], block: u32) -> Result<Self, FsError> {
        let at = |o: usize| -> Result<u32, FsError> {
            u32_at(buf, o).map_err(|e| FsError::Malformed {
                block,
                detail: e.to_string(),
            })
        };
        let text = |o: usize, n: usize| -> Vec<u8> {
            buf.get(o..o.saturating_add(n))
                .unwrap_or(&[])
                .iter()
                .copied()
                .take_while(|&b| b != 0)
                .collect()
        };
        Ok(Self {
            block,
            // The RDB is 256 bytes and its checksum covers `size` longs, but
            // every real one sets size to 64, so the first 256 bytes are what
            // is summed.
            checksum_valid: buf
                .get(..256)
                .is_some_and(|b| checksum::normal_at(b, 8) == u32_at(b, 8).ok()),
            block_size: at(0x10)?,
            bad_block_list: at(0x18)?,
            partition_list: at(0x1c)?,
            filesystem_list: at(0x20)?,
            cylinders: at(0x40)?,
            sectors: at(0x44)?,
            heads: at(0x48)?,
            lo_cylinder: at(0x88)?,
            hi_cylinder: at(0x8c)?,
            cylinder_blocks: at(0x90)?,
            high_rdsk_block: at(0x98)?,
            vendor: text(0xa0, 8),
            product: text(0xa8, 16),
            revision: text(0xb8, 4),
        })
    }

    /// Whether `cylinder_blocks` agrees with `heads × sectors`.
    ///
    /// A disagreement means the geometry is internally inconsistent, which
    /// matters because partition extents are computed from it.
    #[must_use]
    pub fn geometry_consistent(&self) -> bool {
        self.heads
            .checked_mul(self.sectors)
            .is_some_and(|n| n == self.cylinder_blocks)
    }
}

/// One partition on a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The block this partition's `PART` structure occupies.
    pub block: u32,
    /// Whether its checksum verifies.
    pub checksum_valid: bool,
    /// Drive name, as stored — `DH0` and the like.
    pub name: Vec<u8>,
    /// Whether the partition is marked bootable.
    pub bootable: bool,
    /// Whether it is marked no-automount.
    pub no_automount: bool,
    /// Block size in **bytes**, converted from the DOSEnvVec's long count.
    pub block_size: u32,
    /// Heads.
    pub surfaces: u32,
    /// Blocks per track.
    pub blocks_per_track: u32,
    /// Reserved blocks at the partition's start — usually 2.
    pub reserved: u32,
    /// First cylinder, inclusive.
    pub low_cylinder: u32,
    /// Last cylinder, inclusive.
    pub high_cylinder: u32,
    /// Boot priority.
    pub boot_priority: i32,
    /// The dostype the partition table claims. **Advisory** — the partition's
    /// own bootblock is authoritative (ADF FAQ §6.3).
    pub dostype: u32,
}

impl Partition {
    /// Parse a `PART` block.
    ///
    /// # Errors
    /// [`FsError::Malformed`] if the block is too short.
    pub fn parse(buf: &[u8], block: u32) -> Result<Self, FsError> {
        let at = |o: usize| -> Result<u32, FsError> {
            u32_at(buf, o).map_err(|e| FsError::Malformed {
                block,
                detail: e.to_string(),
            })
        };
        let flags = at(0x14)?;
        let name_len = usize::from(*buf.get(0x24).unwrap_or(&0)).min(31);
        Ok(Self {
            block,
            checksum_valid: buf
                .get(..256)
                .is_some_and(|b| checksum::normal_at(b, 8) == u32_at(b, 8).ok()),
            name: buf
                .get(0x25..0x25usize.saturating_add(name_len))
                .unwrap_or(&[])
                .to_vec(),
            bootable: flags & 1 != 0,
            no_automount: flags & 2 != 0,
            // SizeBlock is in LONGS: 128 means a 512-byte block.
            block_size: at(0x84)?.saturating_mul(4),
            surfaces: at(0x8c)?,
            blocks_per_track: at(0x94)?,
            reserved: at(0x98)?,
            low_cylinder: at(0xa4)?,
            high_cylinder: at(0xa8)?,
            // BootPri is a signed field stored in 32 bits; reinterpreting the
            // bits is the conversion, not a lossy cast.
            // BootPri is signed: a partition can be ranked below the default.
            boot_priority: i32_at(buf, 0xbc).map_err(|e| FsError::Malformed {
                block,
                detail: e.to_string(),
            })?,
            dostype: at(0xc0)?,
        })
    }

    /// The partition's first block on the device.
    #[must_use]
    pub fn first_block(&self) -> u64 {
        u64::from(self.low_cylinder)
            .saturating_mul(u64::from(self.surfaces))
            .saturating_mul(u64::from(self.blocks_per_track))
    }

    /// How many blocks the partition spans.
    ///
    /// `LowCyl` and `HighCyl` are both **inclusive**, so the cylinder count is
    /// `high - low + 1`.
    #[must_use]
    pub fn block_count(&self) -> u64 {
        let cylinders = u64::from(self.high_cylinder)
            .saturating_sub(u64::from(self.low_cylinder))
            .saturating_add(1);
        cylinders
            .saturating_mul(u64::from(self.surfaces))
            .saturating_mul(u64::from(self.blocks_per_track))
    }

    /// The name as a lossy string.
    #[must_use]
    pub fn name_lossy(&self) -> String {
        self.name.iter().map(|&b| char::from(b)).collect()
    }

    /// Whether this partition claims an AmigaDOS filesystem.
    ///
    /// `UNI\0`, `UNI\1`, `UNI\2` and `resv` also occur and are not ours to
    /// mount (ADF FAQ §6.3).
    #[must_use]
    pub fn claims_amigados(&self) -> bool {
        self.dostype & 0xFFFF_FF00 == 0x444F_5300
    }
}

/// Every partition on a device, in list order.
///
/// # Errors
/// A read error on the RDB itself. A broken partition chain stops the walk and
/// is reported through the returned faults rather than losing the partitions
/// found before it.
pub fn read_partitions(
    source: &dyn BlockSource,
    geometry: &Geometry,
    rdb: &RigidDiskBlock,
) -> (Vec<Partition>, Vec<FsError>) {
    let mut out = Vec::new();
    let mut faults = Vec::new();
    let bsize = geometry.block_size() as usize;
    let mut buf = vec![0u8; bsize];
    let mut seen: HashSet<u32> = HashSet::new();
    let mut next = rdb.partition_list;

    // Terminator is -1, not 0 — but treat 0 as an end too, since a zeroed
    // field is a likelier corruption than a partition at block 0.
    while next != END_OF_LIST && next != 0 {
        if !seen.insert(next) {
            faults.push(FsError::Cycle {
                block: next,
                chain: "partition",
            });
            break;
        }
        if geometry.validate(BlockIndex(u64::from(next))).is_err() {
            faults.push(FsError::Malformed {
                block: next,
                detail: "partition pointer is outside the device".to_owned(),
            });
            break;
        }
        if read_at(source, BlockIndex(u64::from(next)), &mut buf).is_err() {
            faults.push(FsError::Malformed {
                block: next,
                detail: "partition block could not be read".to_owned(),
            });
            break;
        }
        if buf.get(..4) != Some(PART) {
            faults.push(FsError::Malformed {
                block: next,
                detail: "expected a PART block".to_owned(),
            });
            break;
        }
        match Partition::parse(&buf, next) {
            Ok(p) => {
                out.push(p);
                next = u32_at(&buf, 0x10).unwrap_or(END_OF_LIST);
            }
            Err(e) => {
                faults.push(e);
                break;
            }
        }
    }
    (out, faults)
}

/// One partition of a table to write: where it lies, in the table's
/// cylinders, and what the `PART` block says of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPartition {
    /// Drive name — `DH0` and the like — as the bytes to store, at most 31.
    pub name: Vec<u8>,
    /// First cylinder, inclusive.
    pub low_cylinder: u32,
    /// Last cylinder, inclusive.
    pub high_cylinder: u32,
    /// The dostype the table claims; the partition's own bootblock should
    /// agree (ADF FAQ §6.3).
    pub dostype: u32,
    /// Whether Kickstart may boot from it.
    pub bootable: bool,
    /// Its rank among bootable partitions.
    pub boot_priority: i32,
}

/// A partition table to write: the device's shape, the cylinders kept for
/// the table itself, and the partitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTable {
    /// Cylinders on the device.
    pub cylinders: u32,
    /// Heads.
    pub heads: u32,
    /// Sectors (blocks) per track.
    pub sectors: u32,
    /// The first cylinder partitions may use; those before it hold the RDB.
    pub lo_cylinder: u32,
    /// The partitions, in list order.
    pub partitions: Vec<NewPartition>,
    /// What the RDB calls the drive — vendor, product, revision — as stored
    /// (at most 8, 16 and 4 bytes).
    pub drive: (Vec<u8>, Vec<u8>, Vec<u8>),
}

/// The `RDSK` and `PART` blocks of `table`, by block number: the RDB at block
/// 0 and each partition in the blocks after it, chained in order. Writes no
/// filesystem into the table (D-024 part 4) — a partition relies on the FFS
/// in Kickstart's ROM — and no bad-block list.
///
/// Every field a reader or AmigaDOS consults is set as HDToolBox sets it:
/// 512-byte blocks, 30 buffers, `MaxTransfer` 0x1FE00 and `Mask` 0x7FFFFFFE,
/// the safe values for any controller; unused list pointers -1.
///
/// Checks nothing about the plan — overlaps, names, sizes are the caller's
/// to have refused — beyond what it needs to write at all.
///
/// # Errors
/// A description, for a table whose blocks would not fit in the cylinders
/// kept for it, or a name or drive string too long for its field.
pub fn table_blocks(table: &NewTable) -> Result<Vec<(u32, [u8; 512])>, String> {
    let cylinder_blocks = table.heads.saturating_mul(table.sectors);
    let reserved = table.lo_cylinder.saturating_mul(cylinder_blocks);
    let count = u32::try_from(table.partitions.len()).map_err(|e| e.to_string())?;
    if count.saturating_add(1) > reserved {
        return Err(format!(
            "{count} partitions need {} blocks for the table, and {} cylinders keep {reserved}",
            count.saturating_add(1),
            table.lo_cylinder
        ));
    }
    let (vendor, product, revision) = &table.drive;
    if vendor.len() > 8 || product.len() > 16 || revision.len() > 4 {
        return Err("the drive's vendor, product or revision is too long".to_owned());
    }
    let mut out = Vec::with_capacity(table.partitions.len().saturating_add(1));
    out.push((0, rdsk_block(table, count, reserved)));
    for (i, p) in table.partitions.iter().enumerate() {
        let block = u32::try_from(i)
            .map_err(|e| e.to_string())?
            .saturating_add(1);
        if p.name.is_empty() || p.name.len() > 31 {
            return Err(format!(
                "a drive name is 1 to 31 bytes, and {:?} is {}",
                String::from_utf8_lossy(&p.name),
                p.name.len()
            ));
        }
        let next = if block == count {
            END_OF_LIST
        } else {
            block.saturating_add(1)
        };
        out.push((block, part_block(table, p, next)));
    }
    Ok(out)
}

/// A long at a fixed offset of a block being built. The offsets are
/// constants inside 512 bytes, so a write cannot fail.
fn put(b: &mut [u8; 512], at: usize, value: u32) {
    let _ = ade_endian::put_u32(b, at, value);
}

/// Bytes at a fixed offset; lengths are checked before, so a copy that would
/// not fit cannot occur and is skipped rather than panicking.
fn place(b: &mut [u8; 512], at: usize, bytes: &[u8]) {
    if let Some(field) = b.get_mut(at..at.saturating_add(bytes.len())) {
        field.copy_from_slice(bytes);
    }
}

/// Set the checksum: `SummedLongs` is 64, so it covers the first 256 bytes.
fn seal(b: &mut [u8; 512]) {
    let sum = checksum::normal_at(b.get(..256).unwrap_or(&[]), 8).unwrap_or(0);
    put(b, 8, sum);
}

/// The `RDSK` block: the device's shape, the cylinders kept for the table,
/// and the head of the partition list at block 1.
fn rdsk_block(table: &NewTable, count: u32, reserved: u32) -> [u8; 512] {
    let mut b = [0u8; 512];
    place(&mut b, 0, RDSK);
    put(&mut b, 0x04, 64); // SummedLongs
    put(&mut b, 0x0c, 7); // HostID
    put(&mut b, 0x10, 512); // BlockBytes
    // RDBFF_LAST | LASTLUN | LASTTID | DISKID: the only drive, its strings
    // valid.
    put(&mut b, 0x14, 0x17);
    put(&mut b, 0x18, END_OF_LIST); // BadBlockList
    put(&mut b, 0x1c, if count == 0 { END_OF_LIST } else { 1 }); // PartitionList
    put(&mut b, 0x20, END_OF_LIST); // FileSysHeaderList: none (D-024 part 4)
    put(&mut b, 0x24, END_OF_LIST); // DriveInit
    for at in (0x28..0x40).step_by(4) {
        put(&mut b, at, END_OF_LIST); // Reserved1, "set to -1"
    }
    put(&mut b, 0x40, table.cylinders);
    put(&mut b, 0x44, table.sectors);
    put(&mut b, 0x48, table.heads);
    put(&mut b, 0x4c, 1); // Interleave
    put(&mut b, 0x50, table.cylinders); // Park
    put(&mut b, 0x60, table.cylinders); // WritePreComp
    put(&mut b, 0x64, table.cylinders); // ReducedWrite
    put(&mut b, 0x68, 3); // StepRate
    put(&mut b, 0x80, 0); // RDBBlocksLo
    put(&mut b, 0x84, reserved.saturating_sub(1)); // RDBBlocksHi
    put(&mut b, 0x88, table.lo_cylinder);
    put(&mut b, 0x8c, table.cylinders.saturating_sub(1)); // HiCylinder
    put(&mut b, 0x90, table.heads.saturating_mul(table.sectors)); // CylBlocks
    put(&mut b, 0x98, count); // HighRDSKBlock: the last PART
    let (vendor, product, revision) = &table.drive;
    place(&mut b, 0xa0, vendor);
    place(&mut b, 0xa8, product);
    place(&mut b, 0xb8, revision);
    seal(&mut b);
    b
}

/// A `PART` block: the drive name and the DOSEnvVec AmigaDOS mounts from.
fn part_block(table: &NewTable, p: &NewPartition, next: u32) -> [u8; 512] {
    let mut b = [0u8; 512];
    place(&mut b, 0, PART);
    put(&mut b, 0x04, 64); // SummedLongs
    put(&mut b, 0x0c, 7); // HostID
    put(&mut b, 0x10, next);
    put(&mut b, 0x14, u32::from(p.bootable)); // Flags: PBFF_BOOTABLE
    put(&mut b, 0x18, END_OF_LIST); // Reserved1
    put(&mut b, 0x1c, END_OF_LIST);
    // DriveName, a BSTR: its length, then its bytes.
    place(&mut b, 0x24, &[u8::try_from(p.name.len()).unwrap_or(31)]);
    place(&mut b, 0x25, &p.name);
    // The DOSEnvVec, from 0x80.
    put(&mut b, 0x80, 16); // TableSize: through DosType
    put(&mut b, 0x84, 128); // SizeBlock, in LONGS: 512 bytes
    put(&mut b, 0x88, 0); // SecOrg
    put(&mut b, 0x8c, table.heads); // Surfaces
    put(&mut b, 0x90, 1); // SectorPerBlock
    put(&mut b, 0x94, table.sectors); // BlocksPerTrack
    put(&mut b, 0x98, 2); // Reserved: the bootblock's two
    put(&mut b, 0x9c, 0); // PreAlloc
    put(&mut b, 0xa0, 0); // Interleave
    put(&mut b, 0xa4, p.low_cylinder);
    put(&mut b, 0xa8, p.high_cylinder);
    put(&mut b, 0xac, 30); // NumBuffers
    put(&mut b, 0xb0, 0); // BufMemType: any
    put(&mut b, 0xb4, 0x0001_FE00); // MaxTransfer
    put(&mut b, 0xb8, 0x7FFF_FFFE); // Mask
    let _ = ade_endian::put_i32(&mut b, 0xbc, p.boot_priority);
    put(&mut b, 0xc0, p.dostype);
    seal(&mut b);
    b
}
