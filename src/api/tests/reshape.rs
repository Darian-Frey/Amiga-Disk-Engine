//! A partition to a standalone hardfile and back (F-038), byte for byte.
//! AmigaDOS mounting what is made is in `amigados.rs`; the commands are in
//! `cli/tests/partition_table.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding; contents are compared with assert!, since assert_eq! would \
              print megabytes"
)]

use std::fs;
use std::path::PathBuf;

use ade_core::create::{self, PartitionSize, PartitionSpec};
use ade_core::edit::{EditError, Session};
use ade_core::health::Severity;
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::reshape::{self, ReshapeError};
use ade_core::{Image, inspect_bytes};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-reshape-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A volume of exactly `blocks`, with a file in a folder on it.
fn volume(blocks: u32, flags: u32, name: &str) -> Vec<u8> {
    let geometry = Geometry::new(blocks, 1, 1, 512, 2).unwrap();
    let dostype = Dostype::from_raw(0x444F_5300 | flags).unwrap();
    let blank = format::blank(geometry, dostype, name, WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let folder = s.make_dir(root, b"Box").unwrap();
    s.add_file(folder, b"kept.bin", &[0x5A; 9000]).unwrap();
    s.bytes().to_vec()
}

fn disk(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("disk.hdf");
    let spec = |name: &str, size, kind: &str| PartitionSpec {
        name: name.to_owned(),
        size: PartitionSize::Megabytes(size),
        type_name: Some(kind.to_owned()),
        volume: Some(format!("Vol{name}")),
    };
    create::partitioned(
        &path,
        20,
        &[spec("DH0", 4, "ffs-intl"), spec("DH1", 5, "ofs-dc")],
        "ffs-intl",
        WHEN,
    )
    .unwrap();
    let mut s = Session::open(&path).unwrap();
    s.set_clock(Some(WHEN));
    for name in ["DH0", "DH1"] {
        s.select_partition(name).unwrap();
        let root = s.root();
        s.add_file(root, name.as_bytes(), name.as_bytes()).unwrap();
    }
    let out = dir.join("filled.hdf");
    fs::write(&out, s.bytes()).unwrap();
    out
}

/// One partition's bytes, as the table places it.
fn slice(bytes: &[u8], name: &str) -> Vec<u8> {
    let inspection = inspect_bytes(bytes.to_vec());
    let p = inspection
        .partitions
        .iter()
        .find(|p| p.name == name)
        .unwrap();
    let at = p.first_block as usize * 512;
    bytes[at..at + p.blocks as usize * 512].to_vec()
}

fn faults(bytes: &[u8], partition: Option<&str>) -> Vec<String> {
    ade_core::examine_partition(bytes.to_vec(), partition)
        .findings
        .iter()
        .filter(|f| f.severity >= Severity::Warning)
        .map(|f| f.code.to_owned())
        .collect()
}

fn refusal(result: Result<reshape::Reshaped, ReshapeError>) -> String {
    match result {
        Err(ReshapeError::Refused(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_partition_comes_out_as_the_hardfile_it_holds() {
    let dir = scratch("out");
    let image = disk(&dir);
    let device = fs::read(&image).unwrap();
    for name in ["DH0", "DH1"] {
        let out = dir.join(format!("{name}.hdf"));
        let made = reshape::convert(&image, &name.to_lowercase(), &out, false).unwrap();
        assert_eq!(made.volume.as_deref(), Some(format!("Vol{name}").as_str()));
        let got = fs::read(&out).unwrap();
        assert_eq!(made.bytes, got.len() as u64);
        assert!(got == slice(&device, name), "{name}");
        assert_eq!(faults(&got, None), Vec::<String>::new(), "{name}");
        let lone = Image::from_bytes(got).unwrap();
        assert!(lone.volume().unwrap().lookup(name).is_ok());
    }
    // Through HDZ: the same bytes, compressed.
    let out = dir.join("DH1.hdz");
    reshape::convert(&image, "1", &out, true).unwrap();
    let packed = fs::read(&out).unwrap();
    let inside =
        ade_core::layers::container::inflate::gunzip(&packed, ade_core::MAX_DECOMPRESSED).unwrap();
    assert!(inside == slice(&device, "DH1"));
    // And from a disk that is itself an HDZ.
    let hdz = dir.join("disk.hdz");
    fs::write(
        &hdz,
        ade_core::layers::container::deflate::gzip(&device).unwrap(),
    )
    .unwrap();
    let out = dir.join("again.hdf");
    reshape::convert(&hdz, "DH0", &out, false).unwrap();
    assert!(fs::read(&out).unwrap() == slice(&device, "DH0"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_volume_goes_into_a_new_disk_byte_for_byte_whatever_its_size() {
    let dir = scratch("new");
    // Whole cylinders of 32; not; an odd number, whose only divisor is one.
    for blocks in [8192u32, 8200, 8195] {
        for flags in [0u32, 1, 5] {
            let bytes = volume(blocks, flags, "Moved");
            let source = dir.join(format!("v{blocks}-{flags}.hdf"));
            fs::write(&source, &bytes).unwrap();
            let out = dir.join(format!("d{blocks}-{flags}.hdf"));
            let made = reshape::convert(&source, "DH7", &out, false).unwrap();
            assert_eq!(made.volume.as_deref(), Some("Moved"));
            assert_eq!(
                made.notes.iter().any(|n| n.contains("has its own")),
                blocks % 32 != 0,
                "{:?}",
                made.notes
            );
            let device = fs::read(&out).unwrap();
            assert!(slice(&device, "DH7") == bytes, "{blocks} {flags}");
            assert_eq!(faults(&device, Some("DH7")), Vec::<String>::new());
            let table = inspect_bytes(device.clone());
            assert!(table.partitions[0].bootable);
            assert_eq!(table.partitions[0].dostype, 0x444F_5300 | flags);

            // It is a disk ADE edits, and the way back gives the volume.
            let mut s = Session::from_bytes(device).unwrap();
            s.set_clock(Some(WHEN));
            s.select_partition("DH7").unwrap();
            let root = s.root();
            s.make_dir(root, b"More").unwrap();
            let back = dir.join(format!("b{blocks}-{flags}.hdf"));
            reshape::convert(&out, "DH7", &back, false).unwrap();
            assert!(fs::read(&back).unwrap() == bytes);
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_volume_goes_into_an_existing_disk_and_nothing_else_moves() {
    let dir = scratch("import");
    let image = disk(&dir);
    let before = fs::read(&image).unwrap();
    for (blocks, as_hdz) in [(8192u32, false), (8195, false), (1760, true)] {
        let bytes = volume(blocks, 1, "Coming");
        let source = dir.join(format!("v{blocks}"));
        if as_hdz {
            let z = ade_core::layers::container::deflate::gzip(&bytes).unwrap();
            fs::write(&source, z).unwrap();
        } else {
            fs::write(&source, &bytes).unwrap();
        }
        let mut s = Session::from_bytes(before.clone()).unwrap();
        s.set_clock(Some(WHEN));
        let made = s.import_partition("DH2", &source).unwrap();
        assert_eq!(made.index, 2);
        assert_eq!(made.volume, "Coming");
        assert_eq!(made.note.is_some(), blocks % 32 != 0);
        let after = s.bytes().to_vec();
        assert!(slice(&after, "DH2") == bytes, "{blocks}");
        for name in ["DH0", "DH1"] {
            assert!(slice(&after, name) == slice(&before, name), "{name}");
        }
        for name in ["DH0", "DH1", "DH2"] {
            assert_eq!(faults(&after, Some(name)), Vec::<String>::new(), "{name}");
        }
        // Another partition fits in what is left, and beside it.
        s.add_partition(
            "DH3",
            PartitionSize::Megabytes(1),
            Dostype::from_raw(0x444F_5301).unwrap(),
            None,
        )
        .unwrap();
        assert!(slice(s.bytes(), "DH2") == bytes);
        // One step each.
        assert!(s.undo());
        assert!(s.undo());
        assert!(s.bytes() == before.as_slice(), "{blocks}");
        assert!(s.redo());
        assert!(s.bytes() == after.as_slice());
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_imported_partition_overwrites_what_a_deleted_one_left() {
    // Only the blocks that differ are written; every one must still end up
    // the volume's, over a disk that was not zeros.
    let dir = scratch("over");
    let image = disk(&dir);
    let mut device = fs::read(&image).unwrap();
    let at = inspect_bytes(device.clone()).partitions[1].clone();
    let end = (at.first_block + at.blocks) as usize * 512;
    device[end..].fill(0xA5);
    let bytes = volume(6000, 3, "Clean");
    let source = dir.join("v.hdf");
    fs::write(&source, &bytes).unwrap();
    let mut s = Session::from_bytes(device).unwrap();
    s.import_partition("DH2", &source).unwrap();
    assert!(slice(s.bytes(), "DH2") == bytes);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn what_would_not_be_the_same_volume_is_refused_and_writes_nothing() {
    let dir = scratch("refused");
    let image = disk(&dir);
    let out = dir.join("out.hdf");
    let why = refusal(reshape::convert(&image, "DH9", &out, false));
    assert!(why.contains("DH0, DH1"), "{why}");

    // Reserved blocks and block size a hardfile cannot say.
    for (field, value, says) in [(0x98usize, 4u32, "4 reserved"), (0x84, 256, "1024-byte")] {
        let mut device = fs::read(&image).unwrap();
        ade_core::layers::endian::put_u32(&mut device[512..], field, value).unwrap();
        let path = dir.join("odd.hdf");
        fs::write(&path, &device).unwrap();
        let why = refusal(reshape::convert(&path, "DH0", &out, false));
        assert!(why.contains(says), "{why}");
    }

    // Not a volume; a volume with something after it; a partitioned disk as
    // a volume; a bad drive name; a container that is not sectors.
    let path = dir.join("in.hdf");
    let mut rootless = volume(8192, 1, "Gone");
    rootless[4096 * 512..4097 * 512].fill(0);
    fs::write(&path, &rootless).unwrap();
    let why = refusal(reshape::convert(&path, "DH0", &out, false));
    assert!(why.contains("no volume ADE can mount"), "{why}");
    let mut longer = volume(8192, 1, "Short");
    longer.extend_from_slice(b"tail");
    fs::write(&path, &longer).unwrap();
    let why = refusal(reshape::convert(&path, "DH0", &out, false));
    assert!(why.contains("would be lost"), "{why}");
    fs::write(&path, volume(8192, 1, "Fine")).unwrap();
    let why = refusal(reshape::convert(&path, "D:0", &out, false));
    assert!(why.contains("drive name"), "{why}");
    assert!(!out.exists(), "a refusal writes nothing");

    fs::write(&out, b"mine").unwrap();
    assert!(matches!(
        reshape::convert(&path, "DH0", &out, false),
        Err(ReshapeError::Exists)
    ));
    assert_eq!(fs::read(&out).unwrap(), b"mine");

    // Into an existing disk: the same, and a volume too large, a name taken,
    // a volume that is not sound.
    let before = fs::read(&image).unwrap();
    let mut s = Session::from_bytes(before.clone()).unwrap();
    let mut refused = |name: &str, bytes: &[u8], says: &str| {
        fs::write(&path, bytes).unwrap();
        match s.import_partition(name, &path) {
            Err(EditError::NotEditable(why)) => assert!(why.contains(says), "{why}"),
            other => panic!("{says}: {other:?}"),
        }
        assert!(s.bytes() == before.as_slice(), "{says}");
        assert!(!s.undo(), "{says}: nothing to undo");
    };
    refused("DH1", &volume(8192, 1, "Fine"), "already named");
    refused("DH2", &volume(60_000, 1, "Big"), "no unused run");
    refused("DH2", &before, "partitioned disk");
    let mut damaged = volume(8192, 1, "Hurt");
    let root = 4096 * 512;
    damaged[root + 30] ^= 1;
    refused("DH2", &damaged, "repair it first");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partition_ade_cannot_mount_is_copied_as_it_is_and_said_so() {
    let dir = scratch("foreign");
    let image = disk(&dir);
    let mut device = fs::read(&image).unwrap();
    let p = inspect_bytes(device.clone()).partitions[0].clone();
    let at = p.first_block as usize * 512;
    let end = at + p.blocks as usize * 512;
    for (i, b) in device[at..end].iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    device[at..at + 4].copy_from_slice(b"PFS\x01");
    let path = dir.join("pfs.hdf");
    fs::write(&path, &device).unwrap();
    let out = dir.join("out.hdf");
    let made = reshape::convert(&path, "DH0", &out, false).unwrap();
    assert_eq!(made.volume, None);
    assert!(
        made.notes
            .iter()
            .any(|n| n.contains("no volume ADE can mount"))
    );
    assert!(fs::read(&out).unwrap() == device[at..end]);
    let _ = fs::remove_dir_all(&dir);
}
