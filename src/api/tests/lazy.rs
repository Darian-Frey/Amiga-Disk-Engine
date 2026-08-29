//! Reading an image from its file rather than a copy of it (IMP-005).
//!
//! # The trade these pin
//!
//! `Image::open` takes a snapshot: the bytes are read once and the file is
//! irrelevant afterwards. `Image::open_lazy` takes a window: what stays
//! resident is a file handle, which is what lets a front end hold 400 images
//! for 13 MB instead of 364 — and it means the file is **live**. Deleting or
//! truncating it under an open image changes what that image reads.
//!
//! Both halves are tested, because both are the point: the same answers while
//! the file is there, and a reported failure rather than a crash or a wrong
//! answer when it is not.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::fs;
use std::path::PathBuf;

use ade_core::Image;

fn fixture(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ade-lazy-{}-{tag}.adf", std::process::id()));
    let mut volume = ade_fixtures::Volume::dd(1).named("LAZY");
    volume.add_file("startup", b"read from the file");
    volume.add_file("data.bin", &[0x5Au8; 3000]);
    volume.add_dir("Tools");
    fs::write(&path, volume.build()).unwrap();
    path
}

/// Every name on a volume, however it was opened.
fn names(image: &Image) -> Vec<String> {
    let volume = image.volume().expect("the fixture mounts");
    let root = volume.root();
    let mut out: Vec<String> = volume
        .list(root)
        .expect("the root lists")
        .entries
        .iter()
        .map(|e| e.name_lossy().clone())
        .collect();
    out.sort();
    out
}

#[test]
fn a_lazily_opened_image_reads_the_same_as_an_eager_one() {
    let path = fixture("same");
    let eager = Image::open(&path).expect("opens eagerly");
    let lazy = Image::open_lazy(&path).expect("opens lazily");

    assert_eq!(names(&eager), names(&lazy));
    assert_eq!(
        names(&lazy),
        vec![
            "Tools".to_owned(),
            "data.bin".to_owned(),
            "startup".to_owned()
        ]
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn a_file_read_lazily_has_the_same_contents() {
    let path = fixture("contents");
    let lazy = Image::open_lazy(&path).expect("opens lazily");
    let volume = lazy.volume().unwrap();
    let root = volume.root();
    let entry = volume
        .list(root)
        .unwrap()
        .entries
        .into_iter()
        .find(|e| e.name_lossy() == "startup")
        .expect("the fixture has it");
    let contents = volume.read_file(&entry).expect("reads");
    assert_eq!(contents.into_bytes(), b"read from the file");
    let _ = fs::remove_file(&path);
}

#[test]
fn a_container_whose_blocks_are_not_its_file_falls_back_to_reading_whole() {
    // A gzip wrapper has to be decompressed before it is blocks at all, so
    // there is nothing to read positionally. Asking for a lazy open must
    // silently do the eager thing rather than fail or, worse, read the
    // compressed bytes as though they were sectors.
    let plain = fixture("gzip-source");
    let bytes = fs::read(&plain).unwrap();
    let path = std::env::temp_dir().join(format!("ade-lazy-{}-wrapped.adz", std::process::id()));
    let mut child = std::process::Command::new("gzip")
        .arg("-c")
        .stdin(std::process::Stdio::piped())
        .stdout(std::fs::File::create(&path).unwrap())
        .spawn()
        .expect("gzip runs");
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().unwrap();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    assert!(child.wait().unwrap().success());

    let lazy = Image::open_lazy(&path).expect("an ADZ still opens");
    assert_eq!(names(&lazy).len(), 3, "and lists what is inside it");

    // The file being gone proves it was read whole: a positional source would
    // now fail.
    fs::remove_file(&path).unwrap();
    assert_eq!(names(&lazy).len(), 3, "a decompressed image is a snapshot");
    let _ = fs::remove_file(&plain);
}

#[test]
fn deleting_the_file_under_a_lazy_image_is_reported_not_hidden() {
    // The cost of the window. On Unix an unlinked file stays readable through
    // an open descriptor, so this truncates instead — the case a network share
    // going away or a card being pulled actually produces.
    let path = fixture("truncated");
    let lazy = Image::open_lazy(&path).expect("opens lazily");
    assert_eq!(names(&lazy).len(), 3, "fine while the file is whole");

    fs::write(&path, b"gone").unwrap();

    // Whatever comes back, it must be an error or an empty answer — never a
    // crash, and never content invented from a file that no longer holds it.
    // Refusing to mount at all is the other acceptable answer.
    if let Ok(volume) = lazy.volume() {
        let root = volume.root();
        let listed = volume.list(root);
        assert!(
            listed.is_err() || listed.map(|l| l.entries.len()) == Ok(0),
            "a truncated file must not yield the old listing"
        );
    }
    let _ = fs::remove_file(&path);
}

#[test]
fn an_eager_image_is_a_snapshot_and_does_not_care() {
    // The other half of the trade, stated as a test so a future change cannot
    // quietly take it away: `open` reads once and the file is irrelevant.
    let path = fixture("snapshot");
    let eager = Image::open(&path).expect("opens eagerly");
    fs::remove_file(&path).unwrap();
    assert_eq!(names(&eager).len(), 3, "the bytes are ours");
}
