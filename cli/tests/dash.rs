//! `-` is standard output, never a file called `-` (IMP-021).
//!
//! `ade extract <image> <path> -` once wrote a file named `-` into the working
//! directory, exit 0, and a stray one reached a commit. Every command here runs
//! with a scratch folder as its working directory, so a file called `-` would
//! land there and be seen.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::path::{Path, PathBuf};
use std::{fs, process::Command};

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

/// Run `ade` with `dir` as its working directory.
fn run_in(dir: &Path, args: &[&str]) -> (Vec<u8>, String, Option<i32>) {
    let out = Command::new(ade())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("ade runs");
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-dash-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A disk holding `note.txt`, in `dir`.
fn disk(dir: &Path) {
    let (_, err, code) = run_in(dir, &["create", "blank.adf", "--name=Dash"]);
    assert_eq!(code, Some(0), "{err}");
    fs::write(dir.join("note.txt"), b"to standard output\n").unwrap();
    let (_, err, code) = run_in(dir, &["put", "blank.adf", "note.txt", "--output=disk.adf"]);
    assert_eq!(code, Some(0), "{err}");
}

#[test]
fn extract_to_dash_writes_standard_output_and_no_file() {
    let dir = scratch("extract");
    disk(&dir);
    let (out, err, code) = run_in(&dir, &["extract", "disk.adf", "note.txt", "-"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(out, b"to standard output\n");
    assert!(!dir.join("-").exists(), "no file called -");

    // As with no destination, there is nowhere for a JSON report to go.
    let (_, err, code) = run_in(
        &dir,
        &["extract", "disk.adf", "note.txt", "-", "--format=json"],
    );
    assert_eq!(code, Some(2), "{err}");

    // A file really called `-` is still reachable, as `./-`.
    let (_, err, code) = run_in(&dir, &["extract", "disk.adf", "note.txt", "./-"]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(dir.join("-")).unwrap(), b"to standard output\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dash_where_an_image_or_a_folder_is_written_is_refused() {
    let dir = scratch("refused");
    disk(&dir);
    for args in [
        vec!["extract", "disk.adf", "--all", "-"],
        vec!["carve", "disk.adf", "--all", "-"],
        vec!["convert", "disk.adf", "-"],
        vec!["create", "-"],
        vec!["mkdir", "disk.adf", "Box", "--output=-"],
        vec!["rm", "disk.adf", "note.txt", "--output=-"],
        vec!["consolidate", "disk.adf", "blank.adf", "--output=-"],
        vec!["batch", "disk.adf", "--convert=adz", "--output=-"],
    ] {
        let (_, err, code) = run_in(&dir, &args);
        assert_eq!(code, Some(2), "{args:?}: {err}");
        assert!(err.contains("standard output"), "{args:?}: {err}");
        assert!(!dir.join("-").exists(), "{args:?} wrote a file called -");
    }
    let _ = fs::remove_dir_all(&dir);
}
