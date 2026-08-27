//! End-to-end tests for `ade convert` and `ade formats` (F-016).
//!
//! What matters most here is the refusals. A conversion tool that quietly does
//! the lossy thing is the behaviour F-016 exists to replace, so these tests
//! check that ADE declines — and that it says why, in terms a person can act
//! on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, io::Write as _, path::PathBuf, process::Command, process::Stdio};

use ade_fixtures::Volume;

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

/// A uniquely named temporary path that does not exist yet.
fn temp(name: &str, ext: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ade-conv-{name}-{}-{n}.{ext}", std::process::id()));
    let _ = fs::remove_file(&p);
    p
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut child = Command::new("gzip")
        .args(["-9", "-c"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn gzip");
    let mut stdin = child.stdin.take().expect("stdin");
    let payload = data.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&payload).expect("write"));
    let out = child.wait_with_output().expect("gzip").stdout;
    writer.join().expect("writer");
    out
}

/// Run `ade convert in out`, returning (code, stdout, stderr).
fn convert(input: &PathBuf, output: &PathBuf) -> (i32, String, String) {
    let out = Command::new(ade())
        .arg("convert")
        .arg(input)
        .arg(output)
        .output()
        .expect("run ade");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_adz_converts_to_the_adf_inside_it() {
    let image = Volume::dd(1).named("RoundTrip").build();
    let input = temp("ok", "adz");
    let output = temp("ok", "adf");
    fs::write(&input, gzip(&image)).unwrap();

    let (code, out, err) = convert(&input, &output);
    let written = fs::read(&output).expect("output written");
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&output);

    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("lossless"), "{out}");
    assert_eq!(written, image, "the output must be the image byte for byte");
}

#[test]
fn an_existing_output_is_never_overwritten() {
    // A conversion that replaces a file is the irreversible damage D-004 is
    // about, and a source image is exactly the kind of thing someone points
    // this at by accident.
    let image = Volume::dd(1).named("Precious").build();
    let input = temp("noclob", "adz");
    let output = temp("noclob", "adf");
    fs::write(&input, gzip(&image)).unwrap();
    fs::write(&output, b"do not lose me").unwrap();

    let (code, _, err) = convert(&input, &output);
    let after = fs::read(&output).unwrap();
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&output);

    assert_ne!(code, 0);
    assert!(err.contains("refusing to overwrite"), "{err}");
    assert_eq!(after, b"do not lose me", "the existing file must survive");
}

#[test]
fn writing_ipf_is_refused_with_the_constraint_named() {
    let image = Volume::dd(1).named("NoIpf").build();
    let input = temp("ipf", "adz");
    let output = temp("ipf", "ipf");
    fs::write(&input, gzip(&image)).unwrap();

    let (code, out, _) = convert(&input, &output);
    let _ = fs::remove_file(&input);

    assert_ne!(code, 0);
    assert!(out.contains("refused"), "{out}");
    assert!(
        out.contains("C-003"),
        "the constraint should be named: {out}"
    );
    assert!(!output.exists(), "nothing should have been written");
}

#[test]
fn an_unknown_output_extension_is_an_error_not_a_guess() {
    let image = Volume::dd(1).named("Huh").build();
    let input = temp("ext", "adz");
    let output = temp("ext", "wibble");
    fs::write(&input, gzip(&image)).unwrap();

    let (code, _, err) = convert(&input, &output);
    let _ = fs::remove_file(&input);

    assert_ne!(code, 0);
    assert!(err.contains("cannot tell what format"), "{err}");
    assert!(!output.exists());
}

#[test]
fn a_plain_adf_copies_to_a_hardfile() {
    // Both are flat runs of sectors; the container is a naming convention.
    let image = Volume::dd(1).named("SameBytes").build();
    let input = temp("copy", "adf");
    let output = temp("copy", "hdf");
    fs::write(&input, &image).unwrap();

    let (code, out, err) = convert(&input, &output);
    let written = fs::read(&output).expect("written");
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&output);

    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(written, image);
}

#[test]
fn the_matrix_lists_every_format_with_a_reason() {
    let out = Command::new(ade())
        .arg("formats")
        .output()
        .expect("run ade");
    let text = String::from_utf8_lossy(&out.stdout);

    assert_eq!(out.status.code().unwrap_or(-1), 0);
    // Each verdict class must be visible, and every refusal must carry a why.
    // Each verdict class must be visible, and the refusals must name the
    // register entry behind them. D-005 is deliberately not on this list any
    // more: raw-MFM writing is implemented, so nothing defers to it.
    for expected in [
        "lossless",
        "lossy",
        "refused",
        "not implemented",
        "C-003",
        "D-009",
        "D-004",
    ] {
        assert!(text.contains(expected), "missing {expected:?}\n{text}");
    }
    assert!(
        text.contains("refused outright, not warned about"),
        "the policy should be stated\n{text}"
    );
}

#[test]
fn diff_reports_identical_images_as_identical() {
    let image = Volume::dd(1).named("Same").build();
    let a = temp("same-a", "adf");
    let b = temp("same-b", "adf");
    fs::write(&a, &image).unwrap();
    fs::write(&b, &image).unwrap();

    let out = Command::new(ade())
        .arg("diff")
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);

    assert_eq!(out.status.code().unwrap_or(-1), 0, "{text}");
    assert!(text.contains("identical"), "{text}");
}

#[test]
fn diff_locates_a_difference() {
    let image = Volume::dd(1).named("Differ").build();
    let mut other = image.clone();
    other[512 * 3 + 7] ^= 0xFF;

    let a = temp("diff-a", "adf");
    let b = temp("diff-b", "adf");
    fs::write(&a, &image).unwrap();
    fs::write(&b, &other).unwrap();

    let out = Command::new(ade())
        .arg("diff")
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);

    assert_ne!(
        out.status.code().unwrap_or(-1),
        0,
        "a difference is a finding"
    );
    assert!(text.contains("1 of 1760 sectors differ"), "{text}");
    assert!(text.contains("1 bytes"), "{text}");
}

#[test]
fn consolidate_says_two_dumps_cannot_vote() {
    // The thing a caller most needs to understand about a two-dump merge.
    let image = Volume::dd(1).named("Pair").build();
    let mut other = image.clone();
    other[512 * 9..512 * 10].fill(0xAA);

    let a = temp("cons-a", "adf");
    let b = temp("cons-b", "adf");
    fs::write(&a, &image).unwrap();
    fs::write(&b, &other).unwrap();

    let out = Command::new(ade())
        .arg("consolidate")
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);

    assert!(text.contains("unresolved  1 sectors"), "{text}");
    assert!(text.contains("two dumps cannot vote"), "{text}");
    assert!(
        text.contains("not which dump is correct"),
        "the limit must be stated: {text}"
    );
}

#[test]
fn consolidate_writes_only_when_asked_and_never_overwrites() {
    let image = Volume::dd(1).named("Merged").build();
    let mut other = image.clone();
    other[512 * 4..512 * 5].fill(0x11);

    let a = temp("w-a", "adf");
    let b = temp("w-b", "adf");
    let c = temp("w-c", "adf");
    let merged = temp("w-out", "adf");
    fs::write(&a, &image).unwrap();
    fs::write(&b, &image).unwrap();
    fs::write(&c, &other).unwrap();

    // Without --output, nothing is written.
    let out = Command::new(ade())
        .arg("consolidate")
        .arg(&a)
        .arg(&b)
        .arg(&c)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("resolved    1 sectors"));
    assert!(!merged.exists(), "report-only by default");

    // With it, the majority version is written.
    let out = Command::new(ade())
        .arg("consolidate")
        .arg(format!("--output={}", merged.display()))
        .arg(&a)
        .arg(&b)
        .arg(&c)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("wrote"));
    assert_eq!(fs::read(&merged).unwrap(), image, "two votes beat one");

    // And a second run refuses to clobber it.
    let out = Command::new(ade())
        .arg("consolidate")
        .arg(format!("--output={}", merged.display()))
        .arg(&a)
        .arg(&b)
        .arg(&c)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("refusing to overwrite"));

    for p in [&a, &b, &c, &merged] {
        let _ = fs::remove_file(p);
    }
}

#[test]
fn batch_summarises_a_directory() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ade-batch-cli-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let mut sound = Volume::dd(1).named("Sound");
    sound.add_file("startup", b"hello");
    fs::write(dir.join("a.adf"), sound.build()).unwrap();
    fs::write(dir.join("b.adf"), Volume::dd(1).named("Bare").build()).unwrap();

    let out = Command::new(ade()).arg("batch").arg(&dir).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_dir_all(&dir);

    assert!(text.contains("2 images examined"), "{text}");
    assert!(text.contains("mounted     2"), "{text}");
    assert!(text.contains("containers"), "{text}");
}

#[test]
fn batch_json_emits_records_then_a_summary() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ade-batchjson-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.adf"), Volume::dd(1).named("Jason").build()).unwrap();

    let out = Command::new(ade())
        .args(["batch", "--json"])
        .arg(&dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_dir_all(&dir);

    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "one record then one summary\n{text}");
    assert!(lines[0].contains("\"path\""), "{}", lines[0]);
    assert!(lines[1].contains("\"examined\":1"), "{}", lines[1]);
}

#[test]
fn batch_on_nothing_is_a_usage_error() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ade-batchempty-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let out = Command::new(ade()).arg("batch").arg(&dir).output().unwrap();
    let _ = fs::remove_dir_all(&dir);

    assert_eq!(out.status.code().unwrap_or(-1), 2, "usage");
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing to examine"));
}
