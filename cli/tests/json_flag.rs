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

/// Run `ade`, keeping stdout, stderr and the exit code.
fn run_full(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A fresh, empty scratch directory for one test.
fn scratch(tag: &str) -> PathBuf {
    let dir = temp(&format!("dir-{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn create_convert_and_extract_report_in_json_when_asked() {
    // BUG-012: all three took the flag, printed prose and exited 0.
    let dir = scratch("bug012");
    let disk = dir.join("new.adf");
    let (out, err, code) = run_full(&["--format=json", "create", disk.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with(VERSIONED), "create: {out:.80}");
    assert!(out.contains(r#""type":"ffs-intl""#), "{out}");
    assert_eq!(out.lines().count(), 1, "one document, nothing else: {out}");

    let hdf = dir.join("new.hdf");
    let (out, err, code) = run_full(&[
        "--format=json",
        "convert",
        disk.to_str().unwrap(),
        hdf.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with(VERSIONED), "convert: {out:.80}");
    assert!(out.contains(r#""conversion":{"kind":"lossless""#), "{out}");
    assert_eq!(
        out.lines().count(),
        1,
        "the verdict travels inside it: {out}"
    );

    let mut volume = Volume::dd(1).named("JSONTEST");
    volume.add_file("a", b"contents");
    let source = dir.join("source.adf");
    fs::write(&source, volume.build()).unwrap();
    let dest = dir.join("a.bin");
    let (out, err, code) = run_full(&[
        "--format=json",
        "extract",
        source.to_str().unwrap(),
        "a",
        dest.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with(VERSIONED), "extract: {out:.80}");
    assert!(out.contains(r#""complete":true"#), "{out}");
    assert!(
        err.is_empty(),
        "the prose summary is the document's job now: {err}"
    );
    assert_eq!(fs::read(&dest).unwrap(), b"contents");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn extract_to_stdout_refuses_json_rather_than_mixing_it_into_the_file() {
    let dir = scratch("stdout");
    let mut volume = Volume::dd(1).named("JSONTEST");
    volume.add_file("a", b"contents");
    let source = dir.join("source.adf");
    fs::write(&source, volume.build()).unwrap();
    let (out, err, code) = run_full(&["--format=json", "extract", source.to_str().unwrap(), "a"]);
    assert_eq!(code, Some(2));
    assert!(out.is_empty(), "nothing on stdout: {out}");
    assert!(err.contains("JSON"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

/// What a command does with `--format=json`.
#[derive(Clone, Copy, Debug)]
enum Json {
    /// Emits a versioned document.
    Emits,
    /// Refuses with exit 2 and says so: it reports in text only.
    Refuses,
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one row per command; splitting the table would hide the one that is missing"
)]
fn every_command_either_emits_json_or_refuses_it() {
    // BUG-007 and BUG-012 were the same bug twice: a command added without
    // thinking about the flag accepted it, printed prose and exited 0. This
    // reads the command list from `ade --help`, so a new command fails here
    // until somebody decides which it does — the decision is the point.
    let dir = scratch("every");
    let mut volume = Volume::dd(1).named("JSONTEST");
    volume.add_file("a", b"contents");
    let disk = dir.join("disk.adf");
    fs::write(&disk, volume.build()).unwrap();
    let d = disk.to_str().unwrap().to_owned();
    // An extended ADF, for the one command that needs track-level data.
    let ext = dir.join("raw.adf");
    let (_, err, code) = run_full(&["convert", &d, ext.to_str().unwrap(), "--raw"]);
    assert_eq!(code, Some(0), "{err}");
    // A one-entry datfile, for identify.
    let dats = dir.join("dats");
    fs::create_dir_all(&dats).unwrap();
    fs::write(
        dats.join("t.dat"),
        r#"<?xml version="1.0"?><datafile><header><name>t</name></header><game name="t"><rom name="t.adf" size="1" crc="00000000"/></game></datafile>"#,
    )
    .unwrap();
    let out_path = |name: &str| dir.join(name).to_str().unwrap().to_owned();

    let cases: Vec<(&str, Vec<String>, Json)> = vec![
        ("info", vec![d.clone()], Json::Emits),
        ("check", vec![d.clone()], Json::Emits),
        ("ls", vec![d.clone()], Json::Emits),
        (
            "extract",
            vec![d.clone(), "a".into(), out_path("a.bin")],
            Json::Emits,
        ),
        ("convert", vec![d.clone(), out_path("c.hdf")], Json::Emits),
        ("create", vec![out_path("n.adf")], Json::Emits),
        ("scan", vec![d.clone()], Json::Emits),
        ("find", vec![d.clone(), "contents".into()], Json::Emits),
        ("layout", vec![d.clone()], Json::Emits),
        ("specs", vec![d.clone()], Json::Emits),
        (
            "surface",
            vec![ext.to_str().unwrap().to_owned()],
            Json::Emits,
        ),
        ("carve", vec![d.clone()], Json::Emits),
        ("formats", vec![], Json::Emits),
        ("batch", vec![d.clone()], Json::Emits),
        (
            "identify",
            vec![format!("--datfiles={}", dats.display()), d.clone()],
            Json::Emits,
        ),
        ("diff", vec![d.clone(), d.clone()], Json::Emits),
        ("consolidate", vec![d.clone(), d.clone()], Json::Emits),
        ("disasm", vec![d.clone()], Json::Refuses),
        ("decode", vec![d.clone(), "startup".into()], Json::Refuses),
        (
            "rm",
            vec![
                d.clone(),
                "X".into(),
                format!("--output={}", out_path("rm.adf")),
            ],
            Json::Refuses,
        ),
        (
            "rename",
            vec![
                d.clone(),
                "X".into(),
                "Y".into(),
                format!("--output={}", out_path("rn.adf")),
            ],
            Json::Refuses,
        ),
        (
            "mkdir",
            vec![
                d.clone(),
                "X".into(),
                format!("--output={}", out_path("m.adf")),
            ],
            Json::Refuses,
        ),
        (
            "repair",
            vec![d.clone(), format!("--output={}", out_path("rp.adf"))],
            Json::Refuses,
        ),
        (
            "put",
            vec![
                d.clone(),
                d.clone(),
                format!("--output={}", out_path("p.adf")),
            ],
            Json::Refuses,
        ),
    ];

    // Every command `--help` lists, and nothing else, is in the table.
    let (help, _, _) = run_full(&["--help"]);
    let listed: std::collections::BTreeSet<String> = help
        .lines()
        .skip_while(|l| !l.starts_with("USAGE"))
        .take_while(|l| !l.starts_with("OPTIONS"))
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|w| !w.starts_with('-'))
        .map(str::to_owned)
        .collect();
    let covered: std::collections::BTreeSet<String> =
        cases.iter().map(|(c, _, _)| (*c).to_owned()).collect();
    assert_eq!(
        listed, covered,
        "a command is listed in --help but not classified here, or the reverse"
    );

    for (command, rest, expected) in &cases {
        let mut args = vec!["--format=json", command];
        args.extend(rest.iter().map(String::as_str));
        let (out, err, code) = run_full(&args);
        match expected {
            Json::Emits => assert!(
                out.starts_with(VERSIONED),
                "{command}: expected a JSON document, got stdout {out:.100} stderr {err:.200}"
            ),
            Json::Refuses => {
                assert_eq!(code, Some(2), "{command}: {err}");
                assert!(out.is_empty(), "{command}: {out}");
                assert!(err.contains("JSON"), "{command}: {err}");
            }
        }
    }
    let _ = fs::remove_dir_all(&dir);
}
