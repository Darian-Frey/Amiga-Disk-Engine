//! `ade find` at the command line (F-021).
//!
//! The search and the owner attribution are tested in the crates that do
//! them. These check what only the command can get wrong: the exit code a
//! script branches on, the cap on a screen of near-identical lines, and the
//! flags reaching the pattern parser.

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

/// A disk holding `contents` in a file named `name`, written to a scratch path.
fn disk(tag: &str, name: &str, contents: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-find-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let mut v = ade_fixtures::Volume::dd(1).named("FindTest");
    v.add_file(name, contents);
    let path = dir.join("test.adf");
    fs::write(&path, v.build()).unwrap();
    path
}

#[test]
fn a_hit_names_the_file_it_is_in() {
    let path = disk("hit", "s/startup-sequence", b"C:SetPatch QUIET\nLoadWB\n");
    let (out, _, code) = run(&["find", path.to_str().unwrap(), "LoadWB"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("1 match"), "{out}");
    assert!(out.contains("s/startup-sequence"), "{out}");
}

#[test]
fn nothing_found_exits_non_zero_like_grep() {
    // Not an error — the search worked. But a script wants to branch on it,
    // and `grep`'s convention is the one every script already knows.
    let path = disk("miss", "readme", b"hello");
    let (out, err, code) = run(&["find", path.to_str().unwrap(), "notpresentanywhere"]);
    assert_eq!(code, Some(1), "stdout={out} stderr={err}");
    assert!(out.contains("0 matches"), "{out}");
}

#[test]
fn a_bad_pattern_is_a_usage_error_not_a_search_that_finds_nothing() {
    // The distinction matters: exit 1 means "searched, found nothing" and
    // would have a script conclude the disk is clean.
    let path = disk("badpat", "readme", b"hello");
    let (_, err, code) = run(&["find", path.to_str().unwrap(), "0x601"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("hex digits"), "{err}");
}

#[test]
fn a_long_result_is_capped_and_says_how_many_it_kept_back() {
    // The xDMS filler matches 704 times on one real disk. A screen of
    // near-identical lines obscures the answer; silently truncating would be
    // worse still.
    let path = disk("many", "filler", &b"MARK".repeat(64));
    let (out, _, _) = run(&["find", path.to_str().unwrap(), "MARK", "--text"]);
    assert!(out.contains("64 matches"), "{out}");
    assert!(out.contains("... and 44 more"), "{out}");
    assert!(
        out.contains("--format=json"),
        "it must say where the rest are"
    );
}

#[test]
fn json_carries_every_match_and_how_the_pattern_was_read() {
    let path = disk("json", "filler", &b"MARK".repeat(64));
    let (out, _, _) = run(&[
        "--format=json",
        "find",
        path.to_str().unwrap(),
        "MARK",
        "--text",
    ]);
    assert!(out.contains("\"found\":64"), "{out}");
    assert!(out.contains("\"hex\":false"), "{out}");
    assert_eq!(out.matches("\"offset\"").count(), 64, "no cap in JSON");
    assert!(out.contains("\"schema\""), "{out}");
}

#[test]
fn text_and_ignore_case_reach_the_parser() {
    let path = disk("flags", "readme", b"Workbench release 1.3");

    // `dead` would be hex; `--text` makes it a word. Nothing here contains
    // either, so the check is on how it was read, not what it found.
    let (out, _, _) = run(&["--format=json", "find", path.to_str().unwrap(), "dead"]);
    assert!(out.contains("\"hex\":true"), "{out}");
    let (out, _, _) = run(&[
        "--format=json",
        "find",
        path.to_str().unwrap(),
        "dead",
        "--text",
    ]);
    assert!(out.contains("\"hex\":false"), "{out}");

    let (_, _, code) = run(&["find", path.to_str().unwrap(), "WORKBENCH"]);
    assert_eq!(code, Some(1), "case matters by default");
    let (_, _, code) = run(&["find", path.to_str().unwrap(), "WORKBENCH", "-i"]);
    assert_eq!(code, Some(0), "-i waives it");
}

#[test]
fn an_unreadable_image_is_distinguished_from_a_search_that_found_nothing() {
    let (_, err, code) = run(&["find", "/nonexistent/nowhere.adf", "anything"]);
    assert_eq!(code, Some(3), "{err}");
}
