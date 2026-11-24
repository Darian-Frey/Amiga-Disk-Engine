//! Reading a large image where it is gives the answers holding it gives
//! (IMP-029).
//!
//! `Input` holds a small file and leaves a large one in place. Everything it
//! answers is asked here both ways of the same file, and at the places the
//! two could differ: content across the boundary between two pieces, a volume
//! smaller than its file, and a file that gets shorter while it is open.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::fs;
use std::path::{Path, PathBuf};

use ade_core::Input;
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::layers::object::find::Pattern;

const MB: usize = 1024 * 1024;

fn file(tag: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ade-in-place-{}-{tag}", std::process::id()));
    fs::write(&path, bytes).unwrap();
    path
}

fn held(path: &Path) -> Input {
    Input::open_over(path, u64::MAX).unwrap()
}

fn in_place(path: &Path) -> Input {
    Input::open_over(path, 0).unwrap()
}

/// Everything the two ways of reading must agree on.
fn answers(input: Input, partition: Option<&str>) -> Vec<String> {
    let pattern = Pattern::parse("library", true, true).unwrap();
    let mut out = vec![
        input.size().to_string(),
        input.inspect().to_json().to_json(),
        input.examine(partition).to_json().to_json(),
        input.scan().unwrap().to_json().to_json(),
        input.search(&pattern).unwrap().to_json().to_json(),
    ];
    out.push(
        input
            .specs()
            .map_or_else(|e| e.to_string(), |s| s.to_json().to_json()),
    );
    out
}

fn hardfile(megabytes: u32) -> Vec<u8> {
    let geometry = ade_core::create::Shape::Hard(megabytes).geometry().unwrap();
    let dostype = Dostype::from_raw(0x444F_5301).unwrap();
    let when = Stamp {
        days: 17_000,
        mins: 1,
        ticks: 0,
    };
    format::blank(geometry, dostype, "Large", when).unwrap()
}

#[test]
fn a_hard_disk_reads_the_same_held_or_in_place() {
    let mut s = ade_core::edit::Session::from_bytes(hardfile(20)).unwrap();
    let root = s.root();
    s.add_file(root, b"asl.library", &vec![0x4Eu8; 2_000_000])
        .unwrap();
    let path = file("plain.hdf", s.bytes());
    assert_eq!(answers(held(&path), None), answers(in_place(&path), None));
    // The size alone decides, by default: 20 MB is left in its file, so
    // truncating that file is seen.
    let opened = Input::open(&path).unwrap();
    fs::write(&path, b"gone").unwrap();
    assert!(opened.examine(None).examined.is_none());
    let _ = fs::remove_file(&path);
}

#[test]
fn a_partitioned_disk_reads_the_same_held_or_in_place() {
    let mut device = ade_fixtures::device::Device::new(200, 4, 32);
    device.add_partition("DH0", 2, 99, 1, true, |v| {
        v.add_file("gadtools.library", b"x dos.library");
    });
    device.add_partition("DH1", 100, 199, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    let path = file("rdb.hdf", &device.build());
    for partition in [None, Some("DH1"), Some("1"), Some("nope")] {
        assert_eq!(
            answers(held(&path), partition),
            answers(in_place(&path), partition)
        );
    }
    let _ = fs::remove_file(&path);
}

#[test]
fn what_crosses_the_boundary_between_pieces_is_found_once() {
    let mut bytes = hardfile(20);
    // At every distance across the first boundary, one per boundary's room:
    // a text, a library's name, and a magic at the block after it.
    let name = b" asl.library";
    for shift in 0..name.len() {
        let mut b = bytes.clone();
        let at = 8 * MB - shift;
        b[at..at + name.len()].copy_from_slice(name);
        let path = file("boundary.hdf", &b);
        assert_eq!(answers(held(&path), None), answers(in_place(&path), None));
        let pattern = Pattern::parse("asl.library", true, false).unwrap();
        let found = in_place(&path).search(&pattern).unwrap();
        assert_eq!(found.matches.len(), 1, "shift {shift}");
        assert_eq!(found.matches[0].at.offset, (at + 1) as u64);
        assert_eq!(in_place(&path).specs().unwrap().libraries, ["asl.library"]);
        let _ = fs::remove_file(&path);
    }
    // A name inside another, cut exactly where the preceding byte is in the
    // piece before: not a name.
    let at = 8 * MB - 1;
    bytes[at..at + name.len()].copy_from_slice(b"xasl.library");
    let path = file("inside.hdf", &bytes);
    assert_eq!(
        in_place(&path).specs().unwrap().libraries,
        Vec::<String>::new()
    );
    let _ = fs::remove_file(&path);

    // Every magic the scan knows, at and across a boundary, repeating either
    // side of it as filler does.
    let mut b = hardfile(20);
    for at in (0..6).map(|i| 8 * MB - 1024 + i * 512) {
        b[at..at + 4].copy_from_slice(b"DMS!");
    }
    for (i, magic) in [&b"Copylock"[..], b"Rob Northen", b"FORM", b"PP20"]
        .iter()
        .enumerate()
    {
        let at = (9 + i) * MB - 3;
        b[at..at + magic.len()].copy_from_slice(magic);
    }
    let path = file("magic.hdf", &b);
    let whole = held(&path).scan().unwrap().to_json().to_json();
    assert_eq!(whole, in_place(&path).scan().unwrap().to_json().to_json());
    assert!(whole.contains("\"hits\""));
    let _ = fs::remove_file(&path);
}

#[test]
fn a_volume_smaller_than_its_file_is_found_in_place() {
    // BUG-009's shape: extra tracks after an 80-cylinder volume.
    let mut bytes = ade_fixtures::Volume::dd(1).named("Extra").build();
    bytes.resize(bytes.len() + 2 * 22 * 512, 0);
    let path = file("extra.adf", &bytes);
    let opened = in_place(&path);
    assert_eq!(opened.inspect().geometry, Some(Geometry::DD_FLOPPY));
    assert_eq!(answers(held(&path), None), answers(opened, None));
    let image = ade_core::Image::open_lazy(&path).unwrap();
    assert_eq!(*image.geometry(), Geometry::DD_FLOPPY);
    let _ = fs::remove_file(&path);
}

#[test]
fn a_file_that_gets_shorter_is_an_error_not_a_shorter_answer() {
    let path = file("shrinks.hdf", &hardfile(20));
    let opened = in_place(&path);
    let whole = opened.scan().unwrap().scanned;
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(whole - 5 * MB as u64)
        .unwrap();
    assert!(opened.scan().is_err());
    let pattern = Pattern::parse("anything", true, false).unwrap();
    assert!(opened.search(&pattern).is_err());
    let copy = file("whole.hdf", &hardfile(20));
    let other = in_place(&copy);
    assert!(ade_core::consolidate::diff_in_place(&*other.bytes(), &*opened.bytes()).is_err());
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&copy);
}

#[test]
fn a_dump_compared_in_place_differs_where_it_differs() {
    let a = hardfile(20);
    let mut b = a.clone();
    for at in [0, MB - 1, MB, 20 * MB - 1] {
        b[at] ^= 0xFF;
    }
    let (pa, pb) = (file("a.hdf", &a), file("b.hdf", &b));
    let (ia, ib) = (in_place(&pa), in_place(&pb));
    let found = ade_core::consolidate::diff_in_place(&*ia.bytes(), &*ib.bytes()).unwrap();
    assert_eq!(found.sectors, [0, 2047, 2048, 20 * 2048 - 1]);
    assert_eq!(found.bytes_differing, 4);
    assert_eq!(found.sectors_total, 20 * 2048);
    let _ = fs::remove_file(&pa);
    let _ = fs::remove_file(&pb);
}

#[test]
fn every_tenth_corpus_image_reads_the_same_held_or_in_place() {
    let disks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(entries) = fs::read_dir(&disks) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    if let Ok(more) = fs::read_dir(disks.join("hardfiles")) {
        paths.splice(0..0, more.flatten().map(|e| e.path()));
    }
    paths.sort();
    let mut compared = 0;
    for path in paths
        .iter()
        .enumerate()
        .filter(|(i, p)| i % 10 == 0 || p.starts_with(disks.join("hardfiles")))
        .map(|(_, p)| p)
    {
        let whole = answers(held(path), None);
        assert_eq!(whole, answers(in_place(path), None), "{}", path.display());
        compared += 1;
    }
    eprintln!("{compared} images compared");
}
