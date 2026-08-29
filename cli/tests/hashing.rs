//! `--hash`: the content hashes a cataloguer needs (F-013, VOCABULARY.md).
//!
//! # Why it is a flag rather than always on
//!
//! SHA-1 runs at about 349 MB/s here, so hashing a 4.2 GB corpus costs roughly
//! twelve seconds against the five a health pass takes. A cataloguer wants the
//! hash — it is the key duplicates are found with — and a health run has no
//! use for it. ADE does not hash unless asked, and these check both halves of
//! that: the field appears when asked and is absent when not.
//!
//! The hashes themselves are checked against `sha1sum` where it exists, which
//! is the same oracle rule the SHA-1 implementation itself is held to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_fixtures::Volume;

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

fn run(args: &[&str]) -> String {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `sha1sum` of a file, or `None` when it is not installed.
fn oracle(path: &std::path::Path) -> Option<String> {
    let out = Command::new("sha1sum").arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

/// Pull one field out of a JSON Lines document without a parser.
fn field(line: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\":\"");
    let at = line.find(&key)? + key.len();
    let rest = line.get(at..)?;
    let end = rest.find('"')?;
    rest.get(..end).map(str::to_owned)
}

fn fixture(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ade-hash-{}-{tag}.adf", std::process::id()));
    let mut volume = Volume::dd(1).named("HASHED");
    volume.add_file("startup", b"hello from a hashed fixture");
    volume.add_file("data.bin", &[0x5Au8; 2048]);
    volume.add_dir("Tools");
    fs::write(&path, volume.build()).unwrap();
    path
}

#[test]
fn an_image_hash_appears_only_when_asked_for() {
    let image = fixture("image");
    let plain = run(&["--format=json", "batch", image.to_str().unwrap()]);
    let hashed = run(&["--format=json", "--hash", "batch", image.to_str().unwrap()]);

    assert!(plain.contains(r#""sha1":null"#), "absent by default");
    assert!(field(&hashed, "sha1").is_some(), "present when asked");
    let _ = fs::remove_file(&image);
}

#[test]
fn the_image_hash_is_the_hash_of_the_file() {
    let image = fixture("oracle");
    let Some(expected) = oracle(&image) else {
        eprintln!("skipping: sha1sum not installed");
        let _ = fs::remove_file(&image);
        return;
    };
    let hashed = run(&["--format=json", "--hash", "batch", image.to_str().unwrap()]);
    assert_eq!(field(&hashed, "sha1").as_deref(), Some(expected.as_str()));
    let _ = fs::remove_file(&image);
}

#[test]
fn every_file_is_hashed_and_directories_are_not() {
    let image = fixture("files");
    let listing = run(&["--format=json", "--hash", "ls", image.to_str().unwrap()]);

    let mut files = 0;
    let mut dirs = 0;
    for line in listing.lines() {
        let is_file = line.contains(r#""kind":"file""#);
        let has_hash = field(line, "sha1").is_some();
        if is_file {
            files += 1;
            assert!(has_hash, "a file should be hashed: {line:.90}");
        } else {
            dirs += 1;
            // A directory has no contents to hash, and a hash of nothing would
            // be a real-looking value for a question nobody asked.
            assert!(!has_hash, "a directory should not be: {line:.90}");
        }
    }
    assert_eq!(files, 2);
    assert_eq!(dirs, 1);
    let _ = fs::remove_file(&image);
}

#[test]
fn a_files_hash_is_the_hash_of_what_extracting_it_produces() {
    // The two paths must agree: hashing during a listing and hashing the file
    // ADE writes out are the same bytes or one of them is wrong.
    let image = fixture("extract");
    let out = std::env::temp_dir().join(format!("ade-hash-{}-out.bin", std::process::id()));
    let _ = fs::remove_file(&out);

    let listing = run(&["--format=json", "--hash", "ls", image.to_str().unwrap()]);
    let line = listing
        .lines()
        .find(|l| l.contains(r#""name":"startup""#))
        .expect("the fixture has a startup file");
    let listed = field(line, "sha1").expect("it should be hashed");

    let status = Command::new(ade())
        .args(["extract", image.to_str().unwrap(), "startup"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());

    if let Some(expected) = oracle(&out) {
        assert_eq!(listed, expected, "listing and extraction must agree");
    } else {
        eprintln!("skipping the oracle half: sha1sum not installed");
    }
    for p in [&image, &out] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn the_container_code_is_always_there_and_is_a_code() {
    // Unlike the hash, this costs nothing, so it is not behind the flag. It is
    // what a cataloguer keys on: `container` is a sentence and may be
    // reworded, `container_code` may not (F-015).
    let image = fixture("code");
    let line = run(&["--format=json", "batch", image.to_str().unwrap()]);
    assert_eq!(field(&line, "container_code").as_deref(), Some("adf"));
    assert!(
        field(&line, "container").unwrap().contains("cylinders"),
        "and the prose is still there for a person"
    );
    let _ = fs::remove_file(&image);
}
