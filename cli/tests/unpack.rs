//! `ade extract --all` at the command line (F-024).
//!
//! The names and the skipping are tested in the engine. These check what only
//! the command can get wrong: that `--all` is seen at all, and that a partial
//! recovery is visible to a script without parsing the output.

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
    let dir = std::env::temp_dir().join(format!("ade-unpack-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A disk with a drawer, a file inside it, and one at the top.
fn disk(dir: &std::path::Path) -> PathBuf {
    let mut v = ade_fixtures::Volume::dd(1).named("Unpack");
    v.add_file("readme", b"at the top");
    v.add_dir("Tools");
    v.add_file("Tools/deep", b"inside a drawer");
    let path = dir.join("test.adf");
    fs::write(&path, v.build()).unwrap();
    path
}

#[test]
fn every_file_lands_in_the_folder_with_its_drawers() {
    let dir = scratch("all");
    let image = disk(&dir);
    let out_dir = dir.join("out");
    let (out, err, code) = run(&[
        "extract",
        image.to_str().unwrap(),
        "--all",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "stdout={out} stderr={err}");
    assert!(out.contains("2 files"), "{out}");

    assert_eq!(fs::read(out_dir.join("readme")).unwrap(), b"at the top");
    assert_eq!(
        fs::read(out_dir.join("Tools/deep")).unwrap(),
        b"inside a drawer",
        "the drawer is a drawer, not part of the filename"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_all_the_second_argument_is_still_a_path_on_the_disk() {
    // `("extract", 2)` matches both shapes, and match arms are tried in order.
    // With the `--all` arm second, `--all` was silently ignored and the
    // destination folder was read as a path on the disk: "no such entry: tmp".
    let dir = scratch("shapes");
    let image = disk(&dir);
    let target = dir.join("one.bin");
    let (_, _, code) = run(&[
        "extract",
        image.to_str().unwrap(),
        "readme",
        target.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(fs::read(&target).unwrap(), b"at the top");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_recovery_exits_non_zero_and_names_what_it_missed() {
    // A run that reported success while quietly missing a file would have
    // somebody believe they have the whole disk.
    let dir = scratch("partial");
    let image = disk(&dir);
    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(out_dir.join("readme"), b"already here").unwrap();

    let (out, _, code) = run(&[
        "extract",
        image.to_str().unwrap(),
        "--all",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("skipped readme"), "{out}");
    assert!(out.contains("already exists"), "{out}");
    assert_eq!(
        fs::read(out_dir.join("readme")).unwrap(),
        b"already here",
        "and what was there is still there"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_carries_the_counts_and_every_skip() {
    let dir = scratch("json");
    let image = disk(&dir);
    let out_dir = dir.join("out");
    let (out, _, _) = run(&[
        "--format=json",
        "extract",
        image.to_str().unwrap(),
        "--all",
        out_dir.to_str().unwrap(),
    ]);
    assert!(out.contains("\"files\":2"), "{out}");
    assert!(out.contains("\"directories\":1"), "{out}");
    assert!(out.contains("\"skipped\":[]"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_image_with_no_volume_says_so_rather_than_writing_nothing_quietly() {
    let dir = scratch("novolume");
    let image = dir.join("blank.adf");
    fs::write(&image, vec![0u8; 901_120]).unwrap();
    let (_, err, code) = run(&[
        "extract",
        image.to_str().unwrap(),
        "--all",
        dir.join("out").to_str().unwrap(),
    ]);
    assert_eq!(code, Some(4), "{err}");
    let _ = fs::remove_dir_all(&dir);
}
