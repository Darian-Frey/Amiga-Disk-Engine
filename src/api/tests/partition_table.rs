//! Editing a partitioned disk's table (F-038, D-024 part 5).
//!
//! Each edit changes only the blocks it is about — a `PART`, the `RDSK`, a
//! chain link — never regenerating the table, so a filesystem stored in the
//! RDB and everything else a real disk carries survive. Each is a step of
//! history, kept only if the table passes D-024 part 3's checks. AmigaDOS
//! mounting what these make is in `amigados.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::manual_assert_eq,
    clippy::cast_possible_truncation,
    reason = "test scaffolding over disks it makes; disks are compared with assert!, since \
              assert_eq! would print megabytes of each"
)]

use std::path::PathBuf;

use ade_core::Image;
use ade_core::create::{self, PartitionSize, PartitionSpec};
use ade_core::edit::{EditError, Session};
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::Stamp;
use ade_core::layers::filesystem::volume::Volume;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

/// A partitioned disk `create` makes, as bytes.
fn disk(megabytes: u32, specs: &[&str]) -> Vec<u8> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ade-ptable-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("disk.hdf");
    let specs: Vec<PartitionSpec> = specs
        .iter()
        .map(|s| PartitionSpec::parse(s).unwrap())
        .collect();
    create::partitioned(&path, megabytes, &specs, "ffs-intl", WHEN).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

fn session(bytes: Vec<u8>) -> Session {
    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    s
}

/// The partitions, as (drive name, first block, blocks, dostype, bootable).
fn table(bytes: &[u8]) -> Vec<(String, u64, u64, u32, bool)> {
    let inspection = ade_core::inspect_bytes(bytes.to_vec());
    assert!(
        inspection.partition_faults.is_empty(),
        "{:?}",
        inspection.partition_faults
    );
    inspection
        .partitions
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.first_block,
                p.blocks,
                p.dostype,
                p.bootable,
            )
        })
        .collect()
}

fn read(bytes: &[u8], partition: &str, path: &str) -> Option<Vec<u8>> {
    let image = Image::from_bytes(bytes.to_vec()).unwrap();
    let (parts, _) = image.partitions().unwrap();
    let p = parts.iter().find(|p| p.name_lossy() == partition)?;
    let window = image.partition_window(p).unwrap();
    let volume = Volume::mount(&window).ok()?;
    let entry = volume.lookup(path).ok()?;
    Some(volume.read_file(&entry).unwrap().bytes)
}

fn warnings(bytes: &[u8], partition: &str) -> Vec<String> {
    ade_core::health::examine_partition(bytes.to_vec(), Some(partition))
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

fn ofs() -> Dostype {
    Dostype::from_raw(0x444F_5300).unwrap()
}

#[test]
fn a_partition_is_renamed_and_undo_puts_the_name_back() {
    let original = disk(12, &["DH0:4", "DH1:rest"]);
    let mut s = session(original.clone());
    s.select_partition("DH1").unwrap();
    s.add_file(s.root(), b"note", b"kept").unwrap();
    let before = s.bytes().to_vec();

    s.rename_partition("dh1", "WORK").unwrap();
    let names: Vec<String> = table(s.bytes()).into_iter().map(|p| p.0).collect();
    assert_eq!(names, ["DH0", "WORK"]);
    assert_eq!(s.partition(), Some("WORK"), "the selection follows it");
    assert_eq!(
        read(s.bytes(), "WORK", "note").as_deref(),
        Some(&b"kept"[..])
    );
    s.add_file(s.root(), b"after", b"still editable").unwrap();

    assert!(s.undo() && s.undo());
    assert!(
        s.bytes() == before.as_slice(),
        "the name back, and nothing else"
    );
    assert_eq!(s.partition(), Some("DH1"));
    assert!(s.redo());
    assert_eq!(table(s.bytes())[1].0, "WORK");

    // Refused: a name taken, ignoring case, or one AmigaDOS cannot mount.
    for bad in ["dh0", "A:B", "", "X/Y"] {
        assert!(s.rename_partition("WORK", bad).is_err(), "{bad:?}");
    }
    assert!(s.rename_partition("DH9", "NEW").is_err());
}

#[test]
fn the_boot_flag_and_priority_are_set_and_cleared() {
    let mut s = session(disk(12, &["DH0:4", "DH1:rest"]));
    s.set_partition_boot("DH1", Some(-3)).unwrap();
    s.set_partition_boot("DH0", None).unwrap();
    let parts = table(s.bytes());
    assert!(!parts[0].4 && parts[1].4);
    let image = Image::from_bytes(s.bytes().to_vec()).unwrap();
    let (raw, _) = image.partitions().unwrap();
    assert_eq!(raw[1].boot_priority, -3, "signed, as Kickstart reads it");
    for name in ["DH0", "DH1"] {
        assert_eq!(warnings(s.bytes(), name), Vec::<String>::new());
    }
}

#[test]
fn a_deleted_partition_goes_with_its_volume_and_undo_brings_both_back() {
    let original = disk(16, &["DH0:4", "DH1:4", "DH2:rest"]);
    let mut s = session(original.clone());
    s.select_partition("DH2").unwrap();
    s.add_file(s.root(), b"keep", b"in DH2").unwrap();
    let before = s.bytes().to_vec();

    s.delete_partition("DH1").unwrap();
    let names: Vec<String> = table(s.bytes()).into_iter().map(|p| p.0).collect();
    assert_eq!(names, ["DH0", "DH2"]);
    assert_eq!(
        s.partition(),
        Some("DH2"),
        "the selection stays where it was"
    );
    assert_eq!(
        read(s.bytes(), "DH2", "keep").as_deref(),
        Some(&b"in DH2"[..])
    );

    // The first, then: the RDB's own list head moves.
    s.delete_partition("DH0").unwrap();
    assert_eq!(table(s.bytes()).len(), 1);
    assert!(s.undo() && s.undo());
    assert!(s.bytes() == before.as_slice());
    assert_eq!(table(s.bytes()).len(), 3);
}

#[test]
fn a_partition_is_added_in_unused_cylinders_formatted_and_editable() {
    let mut s = session(disk(20, &["DH0:8"]));
    let before = s.bytes().to_vec();
    let index = s
        .add_partition("DH1", PartitionSize::Rest, ofs(), Some("Data"))
        .unwrap();
    assert_eq!(index, 1);
    let parts = table(s.bytes());
    assert_eq!(parts.len(), 2);
    let (name, first, blocks, dostype, bootable) = parts[1].clone();
    assert_eq!(name, "DH1");
    assert_eq!(first, 64 + 8 * 2048, "straight after DH0");
    assert_eq!(first + blocks, 20 * 2048, "to the end of the disk");
    assert_eq!(dostype, 0x444F_5300);
    assert!(!bootable, "an added partition is not bootable");
    assert!(
        warnings(s.bytes(), "DH1").is_empty(),
        "{:?}",
        warnings(s.bytes(), "DH1")
    );
    let inspection = ade_core::inspect_bytes(s.bytes().to_vec());
    assert_eq!(
        inspection.partitions[1].volume_name.as_deref(),
        Some("Data")
    );

    s.select_partition("DH1").unwrap();
    s.add_file(s.root(), b"into-the-new-one", b"yes").unwrap();
    assert!(s.undo() && s.undo());
    assert!(
        s.bytes() == before.as_slice(),
        "the table and the disk as they were"
    );
}

#[test]
fn an_added_partition_takes_the_first_gap_that_holds_it() {
    let mut s = session(disk(16, &["DH0:4", "DH1:4", "DH2:rest"]));
    let dh1 = table(s.bytes())[1].clone();
    s.delete_partition("DH1").unwrap();
    // Too large for the gap: refused, saying how much there is.
    let refused = s
        .add_partition("NEW", PartitionSize::Megabytes(5), ofs(), None)
        .unwrap_err();
    assert!(refused.to_string().contains("largest is 4 MB"), "{refused}");
    s.add_partition("NEW", PartitionSize::Megabytes(4), ofs(), None)
        .unwrap();
    let new = table(s.bytes()).into_iter().find(|p| p.0 == "NEW").unwrap();
    assert_eq!((new.1, new.2), (dh1.1, dh1.2), "exactly where DH1 was");
    assert!(
        warnings(s.bytes(), "NEW").is_empty(),
        "{:?}",
        warnings(s.bytes(), "NEW")
    );
}

#[test]
fn a_partition_reformatted_as_another_filesystem_changes_its_table_entry_too() {
    let mut s = session(disk(12, &["DH0:4", "DH1:rest:ffs-intl"]));
    s.select_partition("DH1").unwrap();
    s.add_file(s.root(), b"gone", b"after the format").unwrap();
    s.format_as(Some(b"Fresh"), Some(ofs())).unwrap();
    let parts = table(s.bytes());
    assert_eq!(parts[1].3, 0x444F_5300, "the table says OFS");
    let at = usize::try_from(parts[1].1).unwrap() * 512;
    assert_eq!(
        &s.bytes()[at..at + 4],
        b"DOS\0",
        "and so does the bootblock"
    );
    assert_eq!(read(s.bytes(), "DH1", "gone"), None);
    assert!(
        warnings(s.bytes(), "DH1").is_empty(),
        "{:?}",
        warnings(s.bytes(), "DH1")
    );
    assert!(s.undo());
    assert_eq!(table(s.bytes())[1].3, 0x444F_5303);
    assert_eq!(
        read(s.bytes(), "DH1", "gone").as_deref(),
        Some(&b"after the format"[..])
    );
}

#[test]
fn a_full_table_has_no_block_for_another_partition() {
    let specs: Vec<String> = (0..63).map(|i| format!("P{i}:1")).collect();
    let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
    let mut s = session(disk(100, &specs));
    let refused = s
        .add_partition("MORE", PartitionSize::Megabytes(1), ofs(), None)
        .unwrap_err();
    assert!(refused.to_string().contains("no free block"), "{refused}");
    // Deleting one frees its block for the next.
    s.delete_partition("P10").unwrap();
    s.add_partition("MORE", PartitionSize::Megabytes(1), ofs(), None)
        .unwrap();
    assert_eq!(table(s.bytes()).len(), 63);
}

#[test]
fn a_filesystem_stored_in_the_table_survives_an_added_partition() {
    // The fixture generator stores a (minimal) filesystem in its RDB, as real
    // disks set up for PFS or a newer FFS do: an FSHD and its LSEG. A new
    // PART must not land on either.
    let mut device = ade_fixtures::device::Device::new(64, 4, 32);
    device.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello");
    });
    let original = device.build();
    let fshd = original
        .chunks(512)
        .position(|b| b.starts_with(b"FSHD"))
        .unwrap();
    let lseg = original
        .chunks(512)
        .position(|b| b.starts_with(b"LSEG"))
        .unwrap();
    let mut s = session(original.clone());
    s.add_partition("DH1", PartitionSize::Rest, ofs(), None)
        .unwrap();
    for block in [fshd, lseg] {
        let at = block * 512;
        assert!(
            s.bytes()[at..at + 512] == original[at..at + 512],
            "block {block} changed"
        );
    }
    assert_eq!(table(s.bytes()).len(), 2);
    assert_eq!(
        read(s.bytes(), "DH0", "startup").as_deref(),
        Some(&b"hello"[..])
    );
    assert!(
        warnings(s.bytes(), "DH1").is_empty(),
        "{:?}",
        warnings(s.bytes(), "DH1")
    );
}

#[test]
fn deleting_every_partition_leaves_a_table_to_add_to() {
    let mut s = session(disk(8, &["DH0:rest"]));
    s.delete_partition("DH0").unwrap();
    assert!(!s.has_volume());
    assert!(matches!(
        s.make_dir(s.root(), b"Nowhere"),
        Err(EditError::NotEditable(_))
    ));
    // Saved like this, it opens again — to add a partition to.
    let mut again = session(s.bytes().to_vec());
    assert!(!again.has_volume() && again.partitions().is_empty());
    again
        .add_partition("DH0", PartitionSize::Rest, ofs(), Some("Back"))
        .unwrap();
    assert!(again.has_volume());
    again.make_dir(again.root(), b"Somewhere").unwrap();
    assert_eq!(warnings(again.bytes(), "DH0"), Vec::<String>::new());
}

#[test]
fn renaming_a_partition_on_a_real_disk_changes_one_block() {
    // Amiga Forever's workbench-135.hdf, set up by somebody else's tools:
    // only its PART block may differ after a rename.
    let dir = std::env::var_os("ADE_HARDFILES").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks/hardfiles"),
        PathBuf::from,
    );
    let Ok(original) = std::fs::read(dir.join("workbench-135.hdf")) else {
        eprintln!("skipping: no workbench-135.hdf (set ADE_HARDFILES)");
        return;
    };
    let mut s = session(original.clone());
    s.rename_partition("DH0", "SYS").unwrap();
    let changed: Vec<usize> = original
        .chunks(512)
        .zip(s.bytes().chunks(512))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(table(s.bytes())[0].0, "SYS");
    assert_eq!(warnings(s.bytes(), "SYS"), Vec::<String>::new());
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

#[test]
fn a_partition_grows_into_the_cylinders_after_it_and_its_files_with_it() {
    // The rootblock moves to the middle of the grown partition; the entries
    // in the root, its cache on a dircache volume, and the bitmap follow.
    for kind in ["ffs-intl", "ofs", "ffs-dc", "ofs-dc"] {
        let original = disk(24, &[&format!("DH0:6:{kind}")]);
        let mut s = session(original);
        let root = s.root();
        let dir = s.make_dir(root, b"Box").unwrap();
        let inner = pattern(70_000, 3);
        s.add_file(dir, b"inner", &inner).unwrap();
        s.add_file(root, b"top", b"at the top").unwrap();
        let before = s.bytes().to_vec();
        let old_root = s.root();

        s.grow_partition("DH0", PartitionSize::Rest).unwrap();
        let parts = table(s.bytes());
        assert_eq!(
            parts[0].1 + parts[0].2,
            24 * 2048,
            "{kind}: to the end of the disk"
        );
        assert_eq!(
            read(s.bytes(), "DH0", "top").as_deref(),
            Some(&b"at the top"[..]),
            "{kind}"
        );
        assert!(
            read(s.bytes(), "DH0", "Box/inner") == Some(inner.clone()),
            "{kind}"
        );
        assert_eq!(warnings(s.bytes(), "DH0"), Vec::<String>::new(), "{kind}");
        assert!(s.root() > old_root, "{kind}: the rootblock moved");

        // The grown volume takes files into its new blocks.
        let big = pattern(9_000_000, 9);
        s.add_file(s.root(), b"big", &big).unwrap();
        assert!(read(s.bytes(), "DH0", "big") == Some(big), "{kind}");
        assert_eq!(warnings(s.bytes(), "DH0"), Vec::<String>::new(), "{kind}");
        assert!(s.undo() && s.undo());
        assert!(
            s.bytes() == before.as_slice(),
            "{kind}: Undo takes it all back"
        );
    }
}

#[test]
fn a_partition_grown_past_25_bitmap_pages_gets_an_extension_chain() {
    let mut s = session(disk(160, &["DH0:40"]));
    let data = pattern(300_000, 5);
    s.add_file(s.root(), b"kept", &data).unwrap();
    s.grow_partition("DH0", PartitionSize::Megabytes(120))
        .unwrap();
    let parts = table(s.bytes());
    assert_eq!(parts[0].2, 120 * 2048);
    // 245,760 blocks need 61 bitmap pages: 25 from the rootblock, 36 listed
    // by one extension block.
    let at = (usize::try_from(parts[0].1).unwrap() + s.root() as usize) * 512;
    let bm_ext = ade_core::layers::endian::u32_at(s.bytes(), at + 512 - 96).unwrap();
    assert_ne!(bm_ext, 0);
    assert!(read(s.bytes(), "DH0", "kept") == Some(data));
    assert_eq!(warnings(s.bytes(), "DH0"), Vec::<String>::new());
}

#[test]
fn growing_is_refused_where_it_cannot_be_and_nothing_changes() {
    let mut s = session(disk(16, &["DH0:4", "DH1:rest"]));
    let before = s.bytes().to_vec();
    let refused = |r: Result<(), EditError>, says: &str| {
        let e = r.unwrap_err();
        assert!(e.to_string().contains(says), "{e}");
    };
    refused(
        s.grow_partition("DH0", PartitionSize::Rest),
        "no unused cylinders",
    );
    refused(
        s.grow_partition("DH1", PartitionSize::Megabytes(2)),
        "no unused cylinders",
    );
    refused(s.grow_partition("DH9", PartitionSize::Rest), "no partition");
    assert!(s.bytes() == before.as_slice());

    // A file across the block the rootblock would move to: ADE allocates
    // from the rootblock up, so 1.3 MB from the middle of 4 MB reaches the
    // middle of 6 MB.
    let mut s = session(disk(12, &["DH0:4"]));
    s.select_partition("DH0").unwrap();
    s.add_file(s.root(), b"wide", &pattern(1_300_000, 1))
        .unwrap();
    let before = s.bytes().to_vec();
    refused(
        s.grow_partition("DH0", PartitionSize::Megabytes(3)),
        "never shrinks",
    );
    refused(
        s.grow_partition("DH0", PartitionSize::Megabytes(13)),
        "can grow by 7 MB",
    );
    refused(
        s.grow_partition("DH0", PartitionSize::Megabytes(6)),
        "rootblock would be at block",
    );
    assert!(s.bytes() == before.as_slice());
    assert_eq!(s.partition(), Some("DH0"));
}
