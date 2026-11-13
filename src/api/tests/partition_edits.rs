//! Editing inside the partitions of a partitioned hard disk (F-038, D-024).
//!
//! A session over a device holds the whole disk and edits one partition at a
//! time: block numbers are that partition's, every write lands inside it, and
//! Undo is the device's, wherever the last edit was made. A disk whose
//! partition table is not sound is refused before anything is written
//! (D-024 part 3). The device comes from the fixture generator, which shares
//! no code with ADE's readers (D-010).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding over data it builds; disks are compared with assert!, since \
              assert_eq! would print megabytes of each"
)]

use ade_core::Image;
use ade_core::edit::{EditError, Session};
use ade_core::layers::block::checksum::normal_at;
use ade_core::layers::endian::{put_u32, u32_at};
use ade_core::layers::filesystem::format::Stamp;
use ade_core::layers::filesystem::volume::Volume;
use ade_fixtures::device::Device;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

/// `DH0`, FFS, holding `startup`; `DH1`, OFS, holding `data.bin`.
fn device() -> Vec<u8> {
    let mut d = Device::new(64, 4, 32);
    d.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello from DH0");
    });
    d.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    d.build()
}

/// Where a partition lies on the device, in bytes.
fn extent(bytes: &[u8], name: &str) -> std::ops::Range<usize> {
    let inspection = ade_core::inspect_bytes(bytes.to_vec());
    let p = inspection
        .partitions
        .iter()
        .find(|p| p.name == name)
        .unwrap();
    let start = p.first_block as usize * 512;
    start..start + p.blocks as usize * 512
}

/// A file's contents in a partition, read as `ade` reads any partition.
fn read(bytes: &[u8], partition: &str, path: &str) -> Option<Vec<u8>> {
    let image = Image::from_bytes(bytes.to_vec()).unwrap();
    let (parts, _) = image.partitions().unwrap();
    let p = parts.iter().find(|p| p.name_lossy() == partition).unwrap();
    let window = image.partition_window(p).unwrap();
    let volume = Volume::mount(&window).unwrap();
    let entry = volume.lookup(path).ok()?;
    Some(volume.read_file(&entry).unwrap().bytes)
}

fn faults(bytes: &[u8], partition: &str) -> Vec<String> {
    ade_core::health::examine_partition(bytes.to_vec(), Some(partition))
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

fn session(bytes: Vec<u8>) -> Session {
    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    s
}

#[test]
fn a_device_opens_with_its_partitions_and_edits_the_one_selected() {
    let original = device();
    let mut s = session(original.clone());
    assert_eq!(
        s.partitions(),
        vec![("DH0".to_owned(), true), ("DH1".to_owned(), true)]
    );
    assert_eq!(s.partition(), Some("DH0"), "the first that mounts");

    s.select_partition("dh1").unwrap();
    assert_eq!(s.partition(), Some("DH1"));
    let root = s.root();
    let dir = s.make_dir(root, b"Docs").unwrap();
    s.add_file(dir, b"note.txt", b"written into DH1").unwrap();
    s.rename(s.entry("data.bin").unwrap(), b"renamed.bin")
        .unwrap();

    let after = s.bytes();
    assert_eq!(
        read(after, "DH1", "Docs/note.txt").as_deref(),
        Some(&b"written into DH1"[..])
    );
    assert_eq!(read(after, "DH1", "renamed.bin"), Some(vec![0xAA; 3000]));
    assert!(
        faults(after, "DH1").is_empty(),
        "{:?}",
        faults(after, "DH1")
    );
    // Nothing outside DH1 moved: not the table, not DH0.
    let dh1 = extent(&original, "DH1");
    assert!(after[..dh1.start] == original[..dh1.start], "before DH1");
    assert!(after[dh1.end..] == original[dh1.end..], "after DH1");
    assert_eq!(
        read(after, "DH0", "startup").as_deref(),
        Some(&b"hello from DH0"[..])
    );
}

#[test]
fn undo_is_the_devices_whichever_partition_is_selected() {
    let original = device();
    let mut s = session(original.clone());
    let root = s.root();
    s.add_file(root, b"zero.txt", b"in DH0").unwrap();
    let after_dh0 = s.bytes().to_vec();
    s.select_partition("DH1").unwrap();
    let root = s.root();
    s.add_file(root, b"one.txt", b"in DH1").unwrap();

    // DH1 is selected, so the last edit — DH1's — comes off first, then
    // DH0's, though DH0 is not selected.
    assert!(s.undo());
    assert!(s.bytes() == after_dh0.as_slice());
    assert!(s.undo());
    assert!(s.bytes() == original.as_slice(), "the disk as it was");
    assert!(s.redo() && s.redo());
    assert_eq!(
        read(s.bytes(), "DH0", "zero.txt").as_deref(),
        Some(&b"in DH0"[..])
    );
    assert_eq!(
        read(s.bytes(), "DH1", "one.txt").as_deref(),
        Some(&b"in DH1"[..])
    );

    // And DH0's state followed its undo: selecting it again, it edits.
    s.select_partition("0").unwrap();
    s.delete(s.entry("zero.txt").unwrap()).unwrap();
    assert_eq!(read(s.bytes(), "DH0", "zero.txt"), None);
}

#[test]
fn a_format_wipe_and_repair_stay_inside_their_partition() {
    let original = device();
    let mut s = session(original.clone());
    s.select_partition("DH1").unwrap();
    s.wipe(true, true).unwrap();
    s.format(Some(b"Empty")).unwrap();
    let after = s.bytes();
    let dh1 = extent(&original, "DH1");
    assert!(after[..dh1.start] == original[..dh1.start]);
    assert!(after[dh1.end..] == original[dh1.end..]);
    assert_eq!(read(after, "DH1", "data.bin"), None, "formatted");
    assert!(
        faults(after, "DH1").is_empty(),
        "{:?}",
        faults(after, "DH1")
    );
    let repaired = s.repair(false);
    assert!(
        repaired.is_ok() || format!("{repaired:?}").contains("NotNeeded"),
        "{repaired:?}"
    );
}

#[test]
fn a_partition_the_device_lacks_is_named_in_the_refusal() {
    let mut s = session(device());
    let refused = s.select_partition("DH7").unwrap_err();
    assert!(refused.to_string().contains("DH0, DH1"), "{refused}");
    assert_eq!(s.partition(), Some("DH0"), "the selection is unchanged");
}

#[test]
fn an_image_without_partitions_has_none_to_select() {
    let blank = ade_core::layers::filesystem::format::blank(
        ade_core::create::Shape::Dd.geometry().unwrap(),
        ade_core::layers::filesystem::dostype::Dostype::from_raw(0x444F_5300).unwrap(),
        "Floppy",
        WHEN,
    )
    .unwrap();
    let mut s = session(blank);
    assert_eq!(s.partitions(), Vec::new());
    assert_eq!(s.partition(), None);
    assert!(s.select_partition("DH0").is_err());
}

#[test]
fn overlapping_partitions_are_refused_before_anything_is_written() {
    let mut d = Device::new(64, 4, 32);
    d.add_partition("DH0", 2, 40, 1, true, |_| {});
    d.add_partition("DH1", 31, 63, 0, false, |_| {});
    let refused = Session::from_bytes(d.build()).err().unwrap();
    assert!(
        matches!(&refused, EditError::NotEditable(why) if why.contains("overlap")),
        "{refused}"
    );
}

#[test]
fn a_part_block_whose_checksum_fails_is_refused() {
    let mut bytes = device();
    let inspection = ade_core::inspect_bytes(bytes.clone());
    let rdb = inspection.rdb.unwrap();
    // The first PART block, named by the RDSK block's partition list.
    let rdsk = rdb.block as usize * 512;
    let part = u32_at(&bytes, rdsk + 28).unwrap() as usize;
    bytes[part * 512 + 40] ^= 0xFF;
    let refused = Session::from_bytes(bytes.clone()).err().unwrap();
    assert!(refused.to_string().contains("checksum"), "{refused}");

    // Made good again, the same disk opens: it was the checksum.
    let at = part * 512;
    put_u32(&mut bytes, at + 8, 0).unwrap();
    let sum = normal_at(&bytes[at..at + 512], 8).unwrap();
    put_u32(&mut bytes, at + 8, sum).unwrap();
    assert!(Session::from_bytes(bytes).is_ok());
}

/// Set a long in the RDSK block at block 0 and make its checksum good again,
/// so the only thing wrong with the disk is the value set.
fn set_rdsk(bytes: &mut [u8], offset: usize, value: u32) {
    put_u32(bytes, offset, value).unwrap();
    put_u32(bytes, 8, 0).unwrap();
    // The RDSK checksum covers its first 64 longs.
    let sum = normal_at(&bytes[..256], 8).unwrap();
    put_u32(bytes, 8, sum).unwrap();
}

#[test]
fn a_partition_outside_the_cylinders_the_rdb_reserves_is_refused() {
    // IMP-026, D-024 part 3: partitions lie between the RDSK block's
    // LoCylinder and HiCylinder. The device is 64 cylinders, DH0 on 2–30 and
    // DH1 on 31–63; the fixture reserves 2–63.
    const LO_CYLINDER: usize = 0x88;
    const HI_CYLINDER: usize = 0x8c;
    let original = device();
    let rdb = ade_core::inspect_bytes(original.clone()).rdb.unwrap();
    assert_eq!((rdb.lo_cylinder, rdb.hi_cylinder), (2, 63));
    assert!(Session::from_bytes(original.clone()).is_ok());

    // HiCylinder short of DH1's end.
    let mut bytes = original.clone();
    set_rdsk(&mut bytes, HI_CYLINDER, 50);
    let refused = Session::from_bytes(bytes).err().unwrap();
    let why = refused.to_string();
    assert!(
        why.contains("DH1") && why.contains("outside the cylinders") && !why.contains("DH0 "),
        "{why}"
    );

    // LoCylinder past DH0's start.
    let mut bytes = original.clone();
    set_rdsk(&mut bytes, LO_CYLINDER, 3);
    let why = Session::from_bytes(bytes).err().unwrap().to_string();
    assert!(
        why.contains("DH0") && why.contains("outside the cylinders"),
        "{why}"
    );

    // A range that is no range.
    let mut bytes = original.clone();
    set_rdsk(&mut bytes, HI_CYLINDER, 1);
    let why = Session::from_bytes(bytes).err().unwrap().to_string();
    assert!(why.contains("no range"), "{why}");

    // Exactly the partitions' own cylinders is enough.
    let mut bytes = original;
    set_rdsk(&mut bytes, LO_CYLINDER, 2);
    set_rdsk(&mut bytes, HI_CYLINDER, 63);
    assert!(Session::from_bytes(bytes).is_ok());
}

#[test]
fn amiga_forevers_partitioned_workbench_disk_opens_for_editing() {
    // The one real partitioned disk on hand: its table must pass every check
    // D-024 part 3 makes, or the checks are stricter than real disks.
    let dir = std::env::var_os("ADE_HARDFILES").map_or_else(
        || std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks/hardfiles"),
        std::path::PathBuf::from,
    );
    let path = dir.join("workbench-135.hdf");
    if !path.is_file() {
        eprintln!("skipping: no workbench-135.hdf (set ADE_HARDFILES)");
        return;
    }
    let s = Session::open(&path).unwrap();
    assert_eq!(s.partitions(), vec![("DH0".to_owned(), true)]);
}
