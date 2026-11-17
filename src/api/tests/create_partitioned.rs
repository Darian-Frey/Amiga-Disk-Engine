//! Making a partitioned hard disk (F-038, D-024).
//!
//! `create::partitioned` writes an RDB — no filesystem in it (D-024 part 4) —
//! with a partition per spec, each formatted. These hold what it lays out to
//! ADE's own RDB reader and health report, and to the table checks an edit
//! session makes; AmigaDOS in FS-UAE mounting and filling one is in
//! `amigados.rs`, since ADFlib refuses an RDB with no filesystem in it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test scaffolding over disks it makes"
)]

use std::path::PathBuf;

use ade_core::create::{self, CreateError, PartitionSize, PartitionSpec};
use ade_core::edit::Session;
use ade_core::layers::filesystem::format::Stamp;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-mkrdb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec(text: &str) -> PartitionSpec {
    PartitionSpec::parse(text).unwrap()
}

fn make(name: &str, megabytes: u32, specs: &[&str]) -> Result<Vec<u8>, CreateError> {
    let dir = scratch(name);
    let path = dir.join("disk.hdf");
    let specs: Vec<PartitionSpec> = specs.iter().map(|s| spec(s)).collect();
    let made = create::partitioned(&path, megabytes, &specs, "ffs-intl", WHEN);
    let bytes = made.map(|_| std::fs::read(&path).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

fn warnings(bytes: &[u8], partition: &str) -> Vec<String> {
    ade_core::health::examine_partition(bytes.to_vec(), Some(partition))
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

#[test]
fn a_spec_is_name_size_type_and_volume() {
    assert_eq!(
        spec("DH1:rest:ofs:Data"),
        PartitionSpec {
            name: "DH1".to_owned(),
            size: PartitionSize::Rest,
            type_name: Some("ofs".to_owned()),
            volume: Some("Data".to_owned()),
        }
    );
    let short = spec("DH0:60");
    assert_eq!(short.size, PartitionSize::Megabytes(60));
    assert_eq!((short.type_name, short.volume), (None, None));
    // An empty type keeps the default while naming the volume.
    assert_eq!(spec("DH0:5::Work").type_name, None);
    for bad in ["DH0", "DH0:0", "DH0:lots", "DH0:-3"] {
        assert!(PartitionSpec::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_table_and_every_partition_read_back_as_written() {
    let bytes = make("layout", 20, &["DH0:8", "DH1:rest:ofs:Data"]).unwrap();
    assert_eq!(bytes.len(), 20 * 1024 * 1024);
    let inspection = ade_core::inspect_bytes(bytes.clone());
    assert!(
        inspection.partition_faults.is_empty(),
        "{:?}",
        inspection.partition_faults
    );
    let rdb = inspection.rdb.as_ref().unwrap();
    assert!(rdb.checksum_valid);
    // One head, 32 sectors: 64 cylinders a megabyte, two kept for the table.
    assert_eq!((rdb.cylinders, rdb.heads, rdb.sectors), (1280, 1, 32));
    assert_eq!(
        (rdb.lo_cylinder, rdb.hi_cylinder, rdb.cylinder_blocks),
        (2, 1279, 32)
    );
    assert_eq!(
        rdb.high_rdsk_block, 2,
        "RDSK, then a PART for each partition"
    );

    let parts = &inspection.partitions;
    assert_eq!(parts.len(), 2);
    let (dh0, dh1) = (&parts[0], &parts[1]);
    assert_eq!(
        (dh0.name.as_str(), dh0.first_block, dh0.blocks),
        ("DH0", 64, 16_384)
    );
    assert_eq!((dh1.name.as_str(), dh1.first_block), ("DH1", 64 + 16_384));
    assert_eq!(
        dh1.first_block + dh1.blocks,
        40_960,
        "the rest, to the last block"
    );
    assert_eq!((dh0.dostype, dh1.dostype), (0x444F_5303, 0x444F_5300));
    assert!(
        dh0.bootable && !dh1.bootable,
        "the first bootable, as HDToolBox makes it"
    );
    assert!(dh0.checksum_valid && dh1.checksum_valid);
    assert_eq!(dh0.reserved, 2);
    assert_eq!(
        dh0.volume_name.as_deref(),
        Some("DH0"),
        "named after the drive"
    );
    assert_eq!(dh1.volume_name.as_deref(), Some("Data"));
    for name in ["DH0", "DH1"] {
        assert!(
            warnings(&bytes, name).is_empty(),
            "{name}: {:?}",
            warnings(&bytes, name)
        );
    }
}

#[test]
fn a_disk_create_partitions_is_one_an_edit_session_opens_and_edits() {
    // Every table check D-024 part 3 makes passes on what `create` writes.
    let bytes = make("edit", 12, &["DH0:4:ffs-dc", "DH1:rest"]).unwrap();
    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    for partition in ["DH0", "DH1"] {
        s.select_partition(partition).unwrap();
        let root = s.root();
        let dir = s.make_dir(root, b"Box").unwrap();
        s.add_file(dir, b"note", b"written by ADE").unwrap();
    }
    for name in ["DH0", "DH1"] {
        assert!(
            warnings(s.bytes(), name).is_empty(),
            "{name}: {:?}",
            warnings(s.bytes(), name)
        );
    }
}

#[test]
fn space_the_partitions_leave_is_left_unpartitioned() {
    let bytes = make("gap", 10, &["DH0:3"]).unwrap();
    let inspection = ade_core::inspect_bytes(bytes);
    assert_eq!(inspection.partitions.len(), 1);
    assert_eq!(inspection.partitions[0].blocks, 3 * 2048);
}

#[test]
fn a_plan_that_is_not_a_disk_is_refused_with_the_reason() {
    let refused = |megabytes, specs: &[&str], says: &str| {
        let e = make("refused", megabytes, specs).unwrap_err();
        assert!(e.to_string().contains(says), "{specs:?}: {e}");
    };
    refused(10, &["DH0:rest", "DH1:2"], "only the last partition");
    refused(10, &["DH0:4", "dh0:4"], "two partitions are named");
    refused(10, &["DH0:8", "DH1:4"], "do not fit");
    refused(10, &["DH0:10"], "do not fit"); // the table takes 32 KB of it
    refused(10, &["DH0:4:lnfs"], "expected one of");
    refused(10, &["D/H0:4"], "drive name");
    refused(10, &["DH 0:4"], "drive name");
    refused(
        10,
        &["ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789:4"],
        "drive name",
    );
    refused(0, &["DH0:rest"], "1 to 4096 MB");
    refused(4097, &["DH0:rest"], "1 to 4096 MB");
    refused(10, &[], "at least one partition");
    let many: Vec<String> = (0..64).map(|i| format!("P{i}:1")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    refused(100, &many, "at most 63 partitions");
}

#[test]
fn an_existing_file_is_never_overwritten() {
    let dir = scratch("exists");
    let path = dir.join("disk.hdf");
    std::fs::write(&path, b"somebody's").unwrap();
    let e = create::partitioned(&path, 4, &[spec("DH0:rest")], "ffs", WHEN).unwrap_err();
    assert!(matches!(e, CreateError::Exists), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), b"somebody's");
    let _ = std::fs::remove_dir_all(&dir);
}
