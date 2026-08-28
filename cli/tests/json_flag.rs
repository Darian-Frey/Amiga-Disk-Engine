//! `--format=json` is honoured by every command that accepts it (BUG-007).
//!
//! The bug was not missing output: it was **silence**. Four commands took the
//! flag, printed prose, and exited 0, so a script could not tell an
//! unsupported command from a successful one — and the failure surfaced
//! downstream as a parse error against text that was never meant to be parsed.
//!
//! So these tests check the flag *changes what comes out*, which is the thing
//! that was wrong. A test asserting only "exit code 0" would have passed
//! throughout the bug's life.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_fixtures::Volume;

/// Every document opens with the schema version, first field (D-015). A
/// consumer must be able to read it without parsing the rest.
const VERSIONED: &str = r#"{"schema":"#;

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ade-json-{}-{name}", std::process::id()))
}

fn run(args: &[&str]) -> String {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Two images of the "same" disk, differing in one sector.
///
/// `tag` keeps each test's files to itself: these run concurrently in one
/// process, so a shared name means one test deleting another's image
/// mid-run — which shows up as a confident, wrong assertion about exit codes.
fn two_dumps(tag: &str) -> (PathBuf, PathBuf) {
    let mut volume = Volume::dd(1).named("JSONTEST");
    volume.add_file("a", b"contents");
    let bytes = volume.build();

    let a = temp(&format!("{tag}-a.adf"));
    let b = temp(&format!("{tag}-b.adf"));
    fs::write(&a, &bytes).unwrap();
    let mut other = bytes;
    other[512 * 13] ^= 0xFF;
    fs::write(&b, &other).unwrap();
    (a, b)
}

#[test]
fn formats_emits_json_rather_than_the_table() {
    let json = run(&["--format=json", "formats"]);
    assert!(json.starts_with(VERSIONED), "got: {json:.80}");
    assert!(json.contains(r#""conversions":["#), "got: {json:.80}");
    assert!(!json.contains("What ADE can convert"));
    // One line, so a caller can read it without counting braces.
    assert_eq!(json.lines().count(), 1);
}

#[test]
fn diff_emits_json_rather_than_the_summary() {
    let (a, b) = two_dumps("diff");
    let json = run(&[
        "--format=json",
        "diff",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
    ]);
    assert!(json.starts_with(VERSIONED), "got: {json:.80}");
    assert!(json.contains(r#""identical":false"#), "got: {json:.80}");
    assert!(json.contains(r#""sectors":[13]"#));
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
}

#[test]
fn consolidate_emits_json_rather_than_the_report() {
    let (a, b) = two_dumps("consolidate");
    let json = run(&[
        "--format=json",
        "consolidate",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
    ]);
    assert!(json.starts_with(VERSIONED), "got: {json:.80}");
    assert!(json.contains(r#""sources":2"#), "got: {json:.80}");
    assert!(!json.contains("This reports agreement"));
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
}

#[test]
fn the_merged_image_still_goes_to_a_file_and_never_into_the_json() {
    // `--output` writes the disk; stdout carries the report. Announcing the
    // write on stdout would put a line of prose in the middle of a JSON
    // document, which is no longer a JSON document.
    let (a, b) = two_dumps("output");
    let merged = temp("output-merged.adf");
    let _ = fs::remove_file(&merged);
    let json = run(&[
        "--format=json",
        &format!("--output={}", merged.display()),
        "consolidate",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
    ]);
    assert_eq!(json.lines().count(), 1, "stdout must be one JSON document");
    assert!(!json.contains("wrote "));
    assert!(merged.exists(), "the merged image should still be written");
    for path in [&a, &b, &merged] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn the_exit_code_is_unchanged_by_the_format() {
    // The exit code is the other half of the scriptable surface (F-015), and
    // it says what was found, not how it was rendered.
    let (a, b) = two_dumps("exit");
    for format in ["--format=text", "--format=json"] {
        let out = Command::new(ade())
            .args([format, "diff", a.to_str().unwrap(), b.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "differing dumps exit 1 ({format})"
        );
    }
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
}
