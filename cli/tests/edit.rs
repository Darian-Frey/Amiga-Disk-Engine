//! `ade mkdir` and `ade put` — writing into a disk, at the command line.
//!
//! The engine's side is checked in `src/api/tests/edit.rs`, against ADE's
//! health report and ADFlib. These check what only the command can get wrong:
//! that it has no in-place form (D-018), that the image it read is untouched
//! afterwards, and that where a file lands follows from the arguments.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-edit-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A blank disk to edit.
fn disk(dir: &std::path::Path) -> String {
    let path = dir.join("blank.adf");
    let (_, err, code) = run(&["create", path.to_str().unwrap(), "--name=Work"]);
    assert_eq!(code, Some(0), "{err}");
    path.to_str().unwrap().to_owned()
}

#[test]
fn there_is_no_in_place_form() {
    let dir = scratch("inplace");
    let image = disk(&dir);
    let before = fs::read(&image).unwrap();

    let (_, err, code) = run(&["mkdir", &image, "New"]);
    assert_eq!(code, Some(2), "without --output it must refuse");
    assert!(err.contains("--output"), "{err}");

    let (_, err, code) = run(&["mkdir", &image, "New", &format!("--output={image}")]);
    assert_eq!(code, Some(2));
    assert!(err.contains("opened from"), "{err}");

    assert_eq!(
        fs::read(&image).unwrap(),
        before,
        "the original is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mkdir_then_put_builds_a_tree_every_command_can_read() {
    let dir = scratch("tree");
    let image = disk(&dir);
    let original = fs::read(&image).unwrap();
    let one = dir.join("one.adf");
    let two = dir.join("two.adf");
    let three = dir.join("three.adf");
    let host = dir.join("notes.txt");
    fs::write(&host, b"written on the host\n").unwrap();

    let (out, err, code) = run(&[
        "mkdir",
        &image,
        "Docs",
        &format!("--output={}", one.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("Docs"), "{out}");

    // Into an existing directory, under its own name.
    let (_, err, code) = run(&[
        "put",
        one.to_str().unwrap(),
        host.to_str().unwrap(),
        "docs",
        &format!("--output={}", two.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    // Under a name the destination path gives it.
    let (_, err, code) = run(&[
        "put",
        two.to_str().unwrap(),
        host.to_str().unwrap(),
        "Docs/ReadMe",
        &format!("--output={}", three.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let (listing, _, code) = run(&["ls", three.to_str().unwrap(), "Docs"]);
    assert_eq!(code, Some(0));
    assert!(listing.contains("notes.txt"), "{listing}");
    assert!(listing.contains("ReadMe"), "{listing}");

    let (report, _, code) = run(&["check", three.to_str().unwrap()]);
    assert_eq!(code, Some(0), "the result passes `ade check`: {report}");

    let out = dir.join("back.txt");
    let (_, err, code) = run(&[
        "extract",
        three.to_str().unwrap(),
        "Docs/ReadMe",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(&out).unwrap(), b"written on the host\n");

    assert_eq!(
        fs::read(&image).unwrap(),
        original,
        "the first image is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_is_refused_rather_than_ignored() {
    // BUG-007's lesson: a flag accepted and ignored lets a script mistake an
    // unsupported command for a successful one.
    let dir = scratch("json");
    let image = disk(&dir);
    let out = dir.join("out.adf");
    let (_, err, code) = run(&[
        "--format=json",
        "mkdir",
        &image,
        "New",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("JSON"), "{err}");
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_refusal_writes_nothing() {
    let dir = scratch("refuse");
    let image = disk(&dir);
    let out = dir.join("out.adf");

    // No parent to put it in.
    let (_, err, code) = run(&[
        "mkdir",
        &image,
        "Missing/Child",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("Missing"), "{err}");
    assert!(!out.exists(), "a refused edit saves nothing");

    // A name AmigaDOS cannot hold.
    let (_, _, code) = run(&[
        "mkdir",
        &image,
        &"x".repeat(31),
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(!out.exists());

    // Not Latin-1.
    let (_, err, code) = run(&[
        "mkdir",
        &image,
        "日本",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("Latin-1"), "{err}");
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_file_put_under_an_accented_name_extracts_by_that_name() {
    // BUG-011, from the command line: `ls` listed `für.txt` and `extract`
    // said "no such entry".
    let dir = scratch("accent");
    let image = disk(&dir);
    let host = dir.join("für.txt");
    fs::write(&host, b"umlaut").unwrap();
    let edited = dir.join("edited.adf");
    let (_, err, code) = run(&[
        "put",
        &image,
        host.to_str().unwrap(),
        &format!("--output={}", edited.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let out = dir.join("back.txt");
    let (_, err, code) = run(&[
        "extract",
        edited.to_str().unwrap(),
        "für.txt",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(&out).unwrap(), b"umlaut");
    let _ = fs::remove_dir_all(&dir);
}
