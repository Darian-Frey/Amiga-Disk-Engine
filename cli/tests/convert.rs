//! End-to-end tests for `ade convert` and `ade formats` (F-016).
//!
//! What matters most here is the refusals. A conversion tool that quietly does
//! the lossy thing is the behaviour F-016 exists to replace, so these tests
//! check that ADE declines — and that it says why, in terms a person can act
//! on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
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
    for expected in [
        "lossless",
        "lossy",
        "refused",
        "not implemented",
        "C-003",
        "D-009",
        "D-005",
    ] {
        assert!(text.contains(expected), "missing {expected:?}\n{text}");
    }
    assert!(
        text.contains("refused outright, not warned about"),
        "the policy should be stated\n{text}"
    );
}
