//! The Rigid Disk Block and its partition list (Phase 2, F-004).
//!
//! A partitioned device holds no volume of its own: block 0 is an `RDSK`
//! structure, not a bootblock, and every filesystem lives inside a partition.
//! These tests cover the two places that discipline is easy to get wrong — the
//! units the RDB counts in, and the terminators its lists use — plus the
//! guarantee that a partition window cannot address outside itself.
//!
//! The fixtures are generated, which D-010 permits now that the oracle
//! validates generated images; `oracle_fixtures.rs` runs ADFlib over the same
//! device to keep that claim honest.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests over data they construct"
)]

use ade_core::layers::{
    block::{BlockIndex, BlockSource, Geometry},
    container::{RawImage, Window},
    endian::{put_u32, u32_at},
    filesystem::{
        dostype::FileSystem,
        rdb::{self, END_OF_LIST, Partition, RigidDiskBlock},
        volume::{FsError, Volume},
    },
};
use ade_fixtures::{Volume as FixtureVolume, device::Device};

/// The device every test starts from: two partitions, FFS then OFS, with a
/// two-cylinder gap at the front for the reserved area.
fn two_partition_device() -> Vec<u8> {
    let mut d = Device::new(64, 4, 32);
    d.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello from DH0");
        v.add_dir("Tools");
    });
    d.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    d.build()
}

fn image(bytes: Vec<u8>) -> RawImage {
    let blocks = (bytes.len() / 512) as u32;
    // A device is addressed as a flat run of blocks (SPEC §A raw volume has no
    // geometry); the drive's own cylinder shape lives in the RDB.
    let geometry = Geometry::new(blocks, 1, 1, 512, 0).unwrap();
    RawImage::new(bytes, geometry).unwrap()
}

fn partitions(img: &RawImage) -> (Vec<Partition>, Vec<FsError>) {
    let geometry = *img.geometry();
    let rdb = RigidDiskBlock::find(img, &geometry)
        .unwrap()
        .expect("device has an RDB");
    rdb::read_partitions(img, &geometry, &rdb)
}

#[test]
fn finds_the_rdb_and_its_geometry() {
    let img = image(two_partition_device());
    let rdb = RigidDiskBlock::find(&img, img.geometry())
        .unwrap()
        .expect("device has an RDB");

    assert_eq!(rdb.block, 0, "RDSK is at block 0 on a normal device");
    assert!(rdb.checksum_valid);
    assert_eq!(rdb.block_size, 512);
    assert_eq!((rdb.cylinders, rdb.heads, rdb.sectors), (64, 4, 32));
    assert_eq!(
        rdb.cylinder_blocks,
        rdb.heads * rdb.sectors,
        "CylBlocks should agree with heads x sectors"
    );
}

#[test]
fn a_floppy_has_no_rdb() {
    // Not a fault: most images are floppies and have no partition table at all.
    let bytes = FixtureVolume::dd(1).named("Workbench").build();
    let geometry = Geometry::new(80, 2, 11, 512, 2).unwrap();
    let img = RawImage::new(bytes, geometry).unwrap();

    assert!(
        RigidDiskBlock::find(&img, img.geometry())
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_rdb_is_searched_for_not_assumed() {
    // The RDB is allowed anywhere in the first RDB_LOCATION_LIMIT blocks; a
    // drive that reserves block 0 for a PC-style label is the usual reason.
    let mut bytes = two_partition_device();
    let block: Vec<u8> = bytes[..512].to_vec();
    bytes[..512].fill(0);
    bytes[512..1024].copy_from_slice(&block);

    let img = image(bytes);
    let rdb = RigidDiskBlock::find(&img, img.geometry())
        .unwrap()
        .expect("RDB found away from block 0");
    assert_eq!(rdb.block, 1);
}

#[test]
fn the_search_stops_at_the_documented_limit() {
    // Past the limit the RDB is not ours to find, however valid it looks: a
    // signature that far in is more likely to be file content than a label.
    let mut bytes = two_partition_device();
    let block: Vec<u8> = bytes[..512].to_vec();
    bytes[..512].fill(0);
    let at = (rdb::SEARCH_BLOCKS as usize) * 512;
    bytes[at..at + 512].copy_from_slice(&block);

    let img = image(bytes);
    assert!(
        RigidDiskBlock::find(&img, img.geometry())
            .unwrap()
            .is_none()
    );
}

#[test]
fn partitions_parse_with_their_extents() {
    let img = image(two_partition_device());
    let (parts, faults) = partitions(&img);

    assert!(faults.is_empty(), "clean device: {faults:?}");
    assert_eq!(parts.len(), 2);

    let names: Vec<String> = parts.iter().map(Partition::name_lossy).collect();
    assert_eq!(names, ["DH0", "DH1"]);

    // LowCyl and HighCyl are both inclusive, so cylinder 2..=30 is 29
    // cylinders of 4 x 32 blocks, starting at cylinder 2.
    assert_eq!(parts[0].low_cylinder, 2);
    assert_eq!(parts[0].high_cylinder, 30);
    assert_eq!(parts[0].first_block(), 2 * 4 * 32);
    assert_eq!(parts[0].block_count(), 29 * 4 * 32);

    // The partitions abut: the second starts where the first ends.
    assert_eq!(
        parts[1].first_block(),
        parts[0].first_block() + parts[0].block_count()
    );
    assert_eq!(parts[1].high_cylinder, 63);

    assert!(parts[0].bootable);
    assert!(!parts[1].bootable);
    assert!(parts.iter().all(|p| p.checksum_valid));
}

#[test]
fn size_block_is_counted_in_longs() {
    // SizeBlock is the one field in the DOSEnvVec most likely to be read as
    // bytes. 128 longs is a 512-byte block; reading it raw would give a
    // 128-byte block and put every subsequent computation out by four.
    let img = image(two_partition_device());
    let (parts, _) = partitions(&img);

    let raw = {
        let mut buf = vec![0u8; 512];
        img.read_block(
            img.geometry()
                .validate(BlockIndex(u64::from(parts[0].block)))
                .unwrap(),
            &mut buf,
        )
        .unwrap();
        u32_at(&buf, 0x84).unwrap()
    };

    assert_eq!(raw, 128, "the stored field is a long count");
    assert_eq!(parts[0].block_size, 512, "the parsed field is in bytes");
}

#[test]
fn the_list_terminator_is_minus_one() {
    // Zero is a legitimate block number, so an RDB list ends at -1. Reading 0
    // as the terminator happens to work here only because block 0 is the RDB.
    let img = image(two_partition_device());
    let (parts, _) = partitions(&img);

    let mut buf = vec![0u8; 512];
    img.read_block(
        img.geometry()
            .validate(BlockIndex(u64::from(parts[1].block)))
            .unwrap(),
        &mut buf,
    )
    .unwrap();
    let next = u32_at(&buf, 0x10).unwrap();

    assert_eq!(next, END_OF_LIST, "the last PART points at -1");
}

#[test]
fn a_cyclic_partition_list_is_reported_not_followed() {
    // AV-004: a chain that points back at itself must terminate the walk with a
    // fault, and keep the partitions found before it.
    let mut bytes = two_partition_device();
    let img = image(bytes.clone());
    let (parts, _) = partitions(&img);
    let second = parts[1].block as usize * 512;

    // Point the second partition back at the first.
    put_u32(&mut bytes, second + 0x10, parts[0].block).unwrap();

    let (parts, faults) = partitions(&image(bytes));
    assert_eq!(parts.len(), 2, "both partitions survive the fault");
    assert!(
        faults.iter().any(|f| matches!(
            f,
            FsError::Cycle {
                chain: "partition",
                ..
            }
        )),
        "expected a cycle fault, got {faults:?}"
    );
}

#[test]
fn a_partition_pointer_outside_the_device_is_reported() {
    let mut bytes = two_partition_device();
    let img = image(bytes.clone());
    let (parts, _) = partitions(&img);
    let first = parts[0].block as usize * 512;

    put_u32(&mut bytes, first + 0x10, 0x00FF_FFFFu32).unwrap();

    let (parts, faults) = partitions(&image(bytes));
    assert_eq!(parts.len(), 1, "the partition before the break is kept");
    assert_eq!(faults.len(), 1, "and the break is reported: {faults:?}");
}

#[test]
fn a_block_that_is_not_a_part_is_reported() {
    let mut bytes = two_partition_device();
    let img = image(bytes.clone());
    let (parts, _) = partitions(&img);
    let first = parts[0].block as usize * 512;

    // Point at a block inside DH0's data, which is not a PART.
    let elsewhere = parts[0].first_block() as u32 + 10;
    put_u32(&mut bytes, first + 0x10, elsewhere).unwrap();

    let (parts, faults) = partitions(&image(bytes));
    assert_eq!(parts.len(), 1);
    assert_eq!(faults.len(), 1, "{faults:?}");
}

#[test]
fn each_partition_mounts_as_its_own_volume() {
    let bytes = two_partition_device();
    let img = image(bytes);
    let (parts, _) = partitions(&img);

    let mut found = Vec::new();
    for p in &parts {
        let window = Window::new(
            &img,
            p.first_block(),
            p.block_count() as u32,
            p.block_size,
            p.reserved,
        )
        .unwrap();
        let volume = Volume::mount(&window).unwrap();
        let listing = volume
            .list(volume.geometry().root_block().0 as u32)
            .unwrap();
        let mut names: Vec<String> = listing
            .entries
            .iter()
            .map(|e| e.name.iter().map(|&b| char::from(b)).collect())
            .collect();
        names.sort();
        found.push((volume.rootblock().name_lossy(), names));
    }

    assert_eq!(found[0].0, "DH0");
    assert_eq!(found[0].1, ["Tools", "startup"]);
    assert_eq!(found[1].0, "DH1");
    assert_eq!(found[1].1, ["data.bin"]);
}

#[test]
fn a_window_translates_to_the_device() {
    let bytes = two_partition_device();
    let img = image(bytes);
    let (parts, _) = partitions(&img);
    let p = &parts[1];

    let window = Window::new(
        &img,
        p.first_block(),
        p.block_count() as u32,
        p.block_size,
        p.reserved,
    )
    .unwrap();
    assert_eq!(window.start(), p.first_block());

    // Block 0 of the window is the partition's first block on the device.
    let mut from_window = vec![0u8; 512];
    window
        .read_block(
            window.geometry().validate(BlockIndex(0)).unwrap(),
            &mut from_window,
        )
        .unwrap();
    let mut from_device = vec![0u8; 512];
    img.read_block(
        img.geometry()
            .validate(BlockIndex(p.first_block()))
            .unwrap(),
        &mut from_device,
    )
    .unwrap();

    assert_eq!(from_window, from_device);
}

#[test]
fn a_window_cannot_read_past_its_end() {
    // AV-004 across the partition boundary: the window validates against its
    // own geometry, so a block just past the partition is out of range even
    // though the device has data there.
    let bytes = two_partition_device();
    let img = image(bytes);
    let (parts, _) = partitions(&img);
    let p = &parts[0];

    let window = Window::new(
        &img,
        p.first_block(),
        p.block_count() as u32,
        p.block_size,
        p.reserved,
    )
    .unwrap();

    let last = p.block_count() - 1;
    assert!(window.geometry().validate(BlockIndex(last)).is_ok());
    assert!(
        window.geometry().validate(BlockIndex(last + 1)).is_err(),
        "the block after the partition is not addressable through it"
    );
}

#[test]
fn a_window_past_the_device_is_refused() {
    let img = image(two_partition_device());
    let total = img.geometry().total_blocks();

    assert!(Window::new(&img, total - 4, 8, 512, 2).is_err());
}

#[test]
fn the_partition_dostype_is_advisory() {
    // ADF FAQ §6.3: the partition table's dostype is a mount hint. The
    // partition's own bootblock is authoritative, so where they disagree, what
    // mounts must follow the bootblock.
    let mut bytes = two_partition_device();
    let img = image(bytes.clone());
    let (parts, _) = partitions(&img);
    let first = parts[0].block as usize * 512;

    // Claim OFS in the table for a partition whose bootblock says FFS.
    put_u32(&mut bytes, first + 0xc0, 0x444F_5300u32).unwrap();
    // The PART checksum now fails, which is itself worth noticing.
    let img = image(bytes);
    let (parts, _) = partitions(&img);

    assert_eq!(parts[0].dostype, 0x444F_5300, "the table claims OFS");
    assert!(!parts[0].checksum_valid, "and no longer checksums");

    let window = Window::new(
        &img,
        parts[0].first_block(),
        parts[0].block_count() as u32,
        parts[0].block_size,
        parts[0].reserved,
    )
    .unwrap();
    let volume = Volume::mount(&window).unwrap();

    assert_eq!(
        volume.filesystem(),
        FileSystem::Ffs,
        "the bootblock says FFS, so the volume is FFS"
    );
}

#[test]
fn a_partition_with_a_foreign_dostype_is_not_claimed() {
    // UNI\0 and friends are other people's filesystems; recognising them and
    // declining is the point, so that a catalogue run does not mangle them.
    let mut d = Device::new(16, 2, 32);
    d.add_partition("DH0", 2, 15, 1, true, |v| {
        v.add_file("keep", b"x");
    });
    let mut bytes = d.build();

    let img = image(bytes.clone());
    let (parts, _) = partitions(&img);
    let at = parts[0].block as usize * 512;
    put_u32(&mut bytes, at + 0xc0, 0x554E_4900).unwrap();

    let (parts, _) = partitions(&image(bytes));
    assert!(!parts[0].claims_amigados());
    assert!(
        Partition {
            dostype: 0x444F_5303,
            ..parts[0].clone()
        }
        .claims_amigados(),
        "DOS\\3 is ours"
    );
}
