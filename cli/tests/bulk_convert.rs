//! `ade batch --convert=` — F-014's bulk clause, under F-016's rules.
//!
//! # What a bulk conversion has to get right
//!
//! A corpus is heterogeneous. One target format is lossless for most images,
//! refused for the flux captures, and unimplemented for the compressed ones,
//! so the interesting behaviour is not "it converted things" but **what it
//! does with the ones it cannot**: report each, convert the rest, abort on
//! none. A run that stopped at the first refusal would convert nothing over a
//! real collection.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
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

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A scratch directory of this test's own — these run concurrently in one
/// process, so a shared name means one test deleting another's inputs.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-bulk-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn plain_adf() -> Vec<u8> {
    let mut volume = Volume::dd(1).named("BULK");
    volume.add_file("startup", b"bulk converted");
    volume.add_dir("Tools");
    volume.build()
}

#[test]
fn a_directory_of_images_converts_in_one_run() {
    let dir = scratch("many");
    let input = dir.join("in");
    let output = dir.join("out");
    fs::create_dir_all(&input).unwrap();
    for name in ["one.adf", "two.adf", "three.adf"] {
        fs::write(input.join(name), plain_adf()).unwrap();
    }

    let (stdout, _, code) = run(&[
        "--convert=hdf",
        &format!("--output={}", output.display()),
        "batch",
        input.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("3  converted"), "{stdout}");

    for name in ["one.hdf", "two.hdf", "three.hdf"] {
        assert!(
            output.join(name).exists(),
            "{name} should have been written"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_ever_overwritten() {
    // In bulk this matters more than it does for one file: silently replacing
    // an output is the irreversible damage D-004 is about, repeated once per
    // image before anyone notices.
    let dir = scratch("overwrite");
    let input = dir.join("in");
    let output = dir.join("out");
    fs::create_dir_all(&input).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(input.join("disk.adf"), plain_adf()).unwrap();
    fs::write(output.join("disk.hdf"), b"precious").unwrap();

    let (stdout, _, code) = run(&[
        "--convert=hdf",
        &format!("--output={}", output.display()),
        "batch",
        input.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "an existing output is not a failure");
    assert!(stdout.contains("exists"), "{stdout}");
    assert_eq!(
        fs::read(output.join("disk.hdf")).unwrap(),
        b"precious",
        "the existing file must be untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_refusal_is_reported_and_the_run_continues() {
    // The case a real corpus hits: an extended ADF cannot be flattened to a
    // plain one without discarding the raw tracks that are the reason it
    // exists, so F-016 refuses — and the ordinary disks beside it still
    // convert.
    let dir = scratch("refusal");
    let input = dir.join("in");
    let output = dir.join("out");
    fs::create_dir_all(&input).unwrap();
    let plain = input.join("plain.adf");
    fs::write(&plain, plain_adf()).unwrap();

    // Make an extended ADF with ADE itself.
    let raw = input.join("raw.adf");
    let (_, _, code) = run(&[
        "convert",
        "--raw",
        plain.to_str().unwrap(),
        raw.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "the fixture conversion should succeed");

    let (stdout, _, code) = run(&[
        "--convert=hdf",
        &format!("--output={}", output.display()),
        "batch",
        input.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "a refusal is not a failed run");
    assert!(stdout.contains("1  converted"), "{stdout}");
    assert!(stdout.contains("1  lossy"), "{stdout}");
    assert!(
        stdout.contains("raw MFM"),
        "the reason should be in the report: {stdout}"
    );
    assert!(
        output.join("plain.hdf").exists(),
        "the ordinary disk converts"
    );
    assert!(!output.join("raw.hdf").exists(), "the refused one does not");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bulk_and_single_conversion_produce_the_same_bytes() {
    // Two paths to one answer, so they must agree. They share
    // `convert::convert_bytes` now (IMP-007); before that the CLI owned the
    // encoding and `batch` could not have called it at all.
    let dir = scratch("agree");
    let input = dir.join("in");
    let output = dir.join("out");
    fs::create_dir_all(&input).unwrap();
    let source = input.join("disk.adf");
    fs::write(&source, plain_adf()).unwrap();

    let single = dir.join("single.hdf");
    run(&[
        "convert",
        source.to_str().unwrap(),
        single.to_str().unwrap(),
    ]);
    run(&[
        "--convert=hdf",
        &format!("--output={}", output.display()),
        "batch",
        input.to_str().unwrap(),
    ]);

    assert_eq!(
        fs::read(&single).unwrap(),
        fs::read(output.join("disk.hdf")).unwrap()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_target_without_a_destination_is_a_usage_error() {
    let dir = scratch("nodest");
    fs::write(dir.join("disk.adf"), plain_adf()).unwrap();
    let (_, stderr, code) = run(&["--convert=adf", "batch", dir.to_str().unwrap()]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("--output"), "{stderr}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_container_ade_cannot_write_is_a_usage_error_not_a_silent_skip() {
    let dir = scratch("badtarget");
    fs::write(dir.join("disk.adf"), plain_adf()).unwrap();
    let (_, stderr, code) = run(&[
        "--convert=ipf",
        &format!("--output={}", dir.join("out").display()),
        "batch",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("not a container ADE can write"), "{stderr}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_machine_surface_carries_the_outcome_per_image() {
    let dir = scratch("json");
    let input = dir.join("in");
    let output = dir.join("out");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("disk.adf"), plain_adf()).unwrap();

    let (stdout, _, _) = run(&[
        "--format=json",
        "--convert=hdf",
        &format!("--output={}", output.display()),
        "batch",
        input.to_str().unwrap(),
    ]);
    let first = stdout.lines().next().unwrap();
    assert!(
        first.contains(r#""conversion":{"code":"converted""#),
        "{first}"
    );
    // The full path, so match the tail rather than a quoted whole.
    assert!(
        first.contains(r#"disk.hdf""#),
        "the destination is named: {first}"
    );

    let summary = stdout.lines().last().unwrap();
    assert!(
        summary.contains(r#""conversions":[{"code":"converted","images":1}]"#),
        "{summary}"
    );
    let _ = fs::remove_dir_all(&dir);
}
