//! Building a partitioned hard-disk image.
//!
//! A [`crate::Volume`] is one filesystem; a [`Device`] is a disk that
//! holds several, described by a Rigid Disk Block in its reserved area
//! (SPEC §Hard disks — RDB).
//!
//! Two details the layout depends on, both easy to get backwards:
//!
//! - **List terminators are -1**, not 0. The filesystem's own lists end at
//!   zero; the RDB family ends at `0xFFFFFFFF`.
//! - **`SizeBlock` is in longs.** A 512-byte block is written as 128.

use crate::{BSIZE, RESERVED, Volume, normal_checksum_at, put_u32, write_bcpl};

/// Terminator for an RDB-family list.
pub const END_OF_LIST: u32 = 0xFFFF_FFFF;

/// A partitioned device under construction.
pub struct Device {
    blocks: Vec<u8>,
    total_blocks: u32,
    heads: u32,
    sectors: u32,
    partitions: Vec<Built>,
    filesystem_driver: bool,
}

struct Built {
    name: String,
    low_cylinder: u32,
    high_cylinder: u32,
    dostype: u8,
    bootable: bool,
    image: Vec<u8>,
}

impl Device {
    /// Start a device of `cylinders × heads × sectors` blocks.
    ///
    /// The first two cylinders are the reserved area, as AmigaDOS conventionally
    /// lays them out, so partitions begin at cylinder 2.
    #[must_use]
    pub fn new(cylinders: u32, heads: u32, sectors: u32) -> Self {
        let total_blocks = cylinders.saturating_mul(heads).saturating_mul(sectors);
        Self {
            blocks: vec![0u8; total_blocks as usize * BSIZE],
            total_blocks,
            heads,
            sectors,
            partitions: Vec::new(),
            // A driver list is not needed to reach partitions — the FAQ says
            // so explicitly of LSEG blocks — but ADFlib refuses to mount a
            // device without one, so fixtures carry a minimal FSHD/LSEG pair
            // by default to keep the oracle usable.
            filesystem_driver: true,
        }
    }

    /// Omit the FSHD/LSEG driver list.
    ///
    /// Spec-legal — "it isn't needed to reach partitions" (ADF FAQ §6.5) — and
    /// the shape ADFlib declines to mount, which is worth being able to build.
    #[must_use]
    pub const fn without_filesystem_driver(mut self) -> Self {
        self.filesystem_driver = false;
        self
    }

    /// Blocks in one cylinder.
    #[must_use]
    pub const fn cylinder_blocks(&self) -> u32 {
        self.heads * self.sectors
    }

    /// Add a partition spanning `low_cylinder..=high_cylinder`, inclusive.
    ///
    /// The closure receives an empty volume of the right size to populate.
    ///
    /// # Panics
    /// If the cylinders are out of order or exceed the device.
    pub fn add_partition(
        &mut self,
        name: &str,
        low_cylinder: u32,
        high_cylinder: u32,
        dostype: u8,
        bootable: bool,
        fill: impl FnOnce(&mut Volume),
    ) {
        assert!(low_cylinder <= high_cylinder, "cylinders out of order");
        let cyls = high_cylinder - low_cylinder + 1;
        let blocks = cyls * self.cylinder_blocks();
        assert!(
            (high_cylinder + 1) * self.cylinder_blocks() <= self.total_blocks,
            "partition extends past the device"
        );
        // The partition is a volume in its own right: its rootblock sits at its
        // own midpoint, not the device's.
        let mut v = Volume::new(blocks, 1, 1, dostype).named(name);
        fill(&mut v);
        self.partitions.push(Built {
            name: name.to_owned(),
            low_cylinder,
            high_cylinder,
            dostype,
            bootable,
            image: v.build(),
        });
    }

    /// Write the RDB and every partition, and return the device image.
    #[must_use]
    pub fn build(mut self) -> Vec<u8> {
        // Reserved area: RDB at block 0, partition blocks from block 1, then
        // the filesystem driver's FSHD and LSEG.
        let part_start = 1u32;
        let cyl_blocks = self.cylinder_blocks();
        let fshd_block = part_start + self.partitions.len() as u32;
        let lseg_block = fshd_block + 1;
        let reserved_used = if self.filesystem_driver {
            lseg_block
        } else {
            fshd_block - 1
        };

        self.write_partition_blocks(part_start, cyl_blocks);
        self.write_rdb(part_start, fshd_block, reserved_used, cyl_blocks);
        self.write_filesystem_driver(fshd_block, lseg_block);

        // Each partition's own image, at its cylinder offset.
        for p in &self.partitions {
            let start = p.low_cylinder as usize * cyl_blocks as usize * BSIZE;
            let len = p.image.len();
            self.blocks[start..start + len].copy_from_slice(&p.image);
        }
        self.blocks
    }

    /// Write one `PART` block per partition, chained and terminated with -1.
    fn write_partition_blocks(&mut self, part_start: u32, cyl_blocks: u32) {
        for i in 0..self.partitions.len() {
            let p = &self.partitions[i];
            let (low, high, name, bootable, dostype) = (
                p.low_cylinder,
                p.high_cylinder,
                p.name.clone(),
                p.bootable,
                p.dostype,
            );
            let block = part_start + i as u32;
            let next = if i + 1 < self.partitions.len() {
                block + 1
            } else {
                END_OF_LIST
            };
            let o = block as usize * BSIZE;
            let b = &mut self.blocks[o..o + BSIZE];
            b[..4].copy_from_slice(b"PART");
            put_u32(b, 0x04, 64); // size in longs
            put_u32(b, 0x0c, 7); // hostID
            put_u32(b, 0x10, next);
            put_u32(b, 0x14, u32::from(bootable));
            write_bcpl(b, 0x24, name.as_bytes(), 31);
            // DOSEnvVec
            put_u32(b, 0x80, 16); // size of vector, in longs
            put_u32(b, 0x84, (BSIZE / 4) as u32); // SizeBlock: LONGS, not bytes
            put_u32(b, 0x8c, 1); // Surfaces
            put_u32(b, 0x90, 1); // sectors per block
            put_u32(b, 0x94, cyl_blocks); // blocks per track
            put_u32(b, 0x98, RESERVED);
            put_u32(b, 0xa4, low);
            put_u32(b, 0xa8, high);
            put_u32(b, 0xb4, 0xFFFF_FFFE); // Mask
            put_u32(b, 0xbc, 0); // BootPri
            put_u32(b, 0xc0, 0x444F_5300 | u32::from(dostype));
            let ck = normal_checksum_at(&b[..256], 8);
            put_u32(b, 8, ck);
        }
    }

    /// Write the `RDSK` block itself, at block 0.
    fn write_rdb(&mut self, part_start: u32, fshd_block: u32, reserved_used: u32, cyl_blocks: u32) {
        {
            let b = &mut self.blocks[..BSIZE];
            b[..4].copy_from_slice(b"RDSK");
            put_u32(b, 0x04, 64);
            put_u32(b, 0x0c, 7); // hostID
            put_u32(b, 0x10, BSIZE as u32); // block size, in BYTES here
            put_u32(b, 0x14, 0x17); // flags
            put_u32(b, 0x18, END_OF_LIST); // BadBlockList
            put_u32(
                b,
                0x1c,
                if self.partitions.is_empty() {
                    END_OF_LIST
                } else {
                    part_start
                },
            );
            put_u32(
                b,
                0x20,
                if self.filesystem_driver {
                    fshd_block
                } else {
                    END_OF_LIST
                },
            );
            put_u32(b, 0x24, END_OF_LIST); // DriveInit
            put_u32(b, 0x40, self.total_blocks / cyl_blocks.max(1)); // cylinders
            put_u32(b, 0x44, self.sectors);
            put_u32(b, 0x48, self.heads);
            put_u32(b, 0x80, 0); // RDB_BlockLo
            put_u32(b, 0x84, cyl_blocks.saturating_mul(2).saturating_sub(1)); // RDB_BlockHi
            put_u32(b, 0x88, 2); // LoCylinder: partitions begin after reserved
            put_u32(
                b,
                0x8c,
                (self.total_blocks / cyl_blocks.max(1)).saturating_sub(1),
            );
            put_u32(b, 0x90, cyl_blocks); // CylBlocks
            put_u32(b, 0x98, reserved_used); // HighRSDKBlock
            b[0xa0..0xa0 + 3].copy_from_slice(b"ADE");
            b[0xa8..0xa8 + 7].copy_from_slice(b"FIXTURE");
            b[0xb8..0xb8 + 3].copy_from_slice(b"1.0");
            let ck = normal_checksum_at(&b[..256], 8);
            put_u32(b, 8, ck);
        }
    }

    /// A minimal filesystem driver: one `FSHD` naming one `LSEG`.
    ///
    /// The LSEG would hold the driver's executable code; nothing here needs it,
    /// and the FAQ says it "isn't needed to reach partitions" — but ADFlib will
    /// not mount a device whose driver list is absent, so a fixture without one
    /// cannot be checked against the oracle.
    fn write_filesystem_driver(&mut self, fshd_block: u32, lseg_block: u32) {
        if self.filesystem_driver {
            let dostype = self
                .partitions
                .first()
                .map_or(0x444F_5301, |p| 0x444F_5300 | u32::from(p.dostype));
            {
                let o = fshd_block as usize * BSIZE;
                let b = &mut self.blocks[o..o + BSIZE];
                b[..4].copy_from_slice(b"FSHD");
                put_u32(b, 0x04, 64);
                put_u32(b, 0x0c, 7);
                put_u32(b, 0x10, END_OF_LIST); // next FSHD
                put_u32(b, 0x20, dostype);
                put_u32(b, 0x24, 0x0027_001B); // version 39.27
                put_u32(b, 0x28, 0x180); // PatchFlags: SegList and GlobalVec
                put_u32(b, 0x48, lseg_block);
                put_u32(b, 0x4c, END_OF_LIST); // GlobalVec = -1
                let ck = normal_checksum_at(&b[..256], 8);
                put_u32(b, 8, ck);
            }
            {
                let o = lseg_block as usize * BSIZE;
                let b = &mut self.blocks[o..o + BSIZE];
                b[..4].copy_from_slice(b"LSEG");
                put_u32(b, 0x04, (BSIZE / 4) as u32);
                put_u32(b, 0x0c, 7);
                put_u32(b, 0x10, END_OF_LIST); // last in the chain
                let ck = normal_checksum_at(b, 8);
                put_u32(b, 8, ck);
            }
        }
    }
}
