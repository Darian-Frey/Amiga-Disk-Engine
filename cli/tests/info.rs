//! End-to-end tests for `ade info`.
//!
//! Exercises the binary itself, so the exit codes F-015 commits to are covered
//! by tests rather than by intention. Fixtures are generated (D-010); no image
//! is read from the repository.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_fixtures::{Volume, corrupt};

fn ade() -> PathBuf {
    // The test binary sits beside the CLI it is testing.
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

/// Write bytes to a uniquely named temporary image.
fn write_temp(bytes: &[u8], name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ade-test-{name}-{}.adf", std::process::id()));
    fs::write(&path, bytes).expect("write fixture");
    path
}

/// Run `ade` with arbitrary arguments over a temporary image.
fn info_args(bytes: &[u8], name: &str, args: &[&str]) -> (i32, String) {
    let path = write_temp(bytes, name);
    let out = Command::new(ade())
        .args(args)
        .arg(&path)
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run `ade info` over bytes written to a temporary file. Returns (code, stdout).
fn info(bytes: &[u8], name: &str) -> (i32, String) {
    let path = std::env::temp_dir().join(format!("ade-test-{name}-{}.adf", std::process::id()));
    fs::write(&path, bytes).expect("write fixture");
    let out = Command::new(ade())
        .arg("info")
        .arg(&path)
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn a_clean_volume_exits_zero() {
    let (code, out) = info(&Volume::dd(0).named("Workbench").build(), "clean");
    assert_eq!(code, 0, "clean image must exit 0\n{out}");
    assert!(out.contains(r#"name        "Workbench""#), "{out}");
    assert!(out.contains("faults      none"), "{out}");
    assert!(out.contains("rootblock   block 880 (computed)"), "{out}");
}

#[test]
fn a_stale_bitmap_flag_exits_one() {
    let v = Volume::dd(1).named("Unclean");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root);
    let (code, out) = info(&img, "bitmap");
    assert_eq!(code, 1, "faults must exit 1\n{out}");
    assert!(out.contains("bitmap-valid flag is clear"), "{out}");
}

#[test]
fn no_rootblock_exits_four_not_zero() {
    // The distinction that matters: this is not "clean".
    let v = Volume::dd(0);
    let root = v.root();
    let mut img = v.build();
    corrupt::rootblock_wrong_type(&mut img, root);
    let (code, out) = info(&img, "novol");
    assert_eq!(
        code, 4,
        "no volume must be distinguishable from clean\n{out}"
    );
    assert!(out.contains("volume      none"), "{out}");
    assert!(out.contains("no rootblock at block 880"), "{out}");
}

#[test]
fn an_unreadable_path_exits_three() {
    let out = Command::new(ade())
        .arg("info")
        .arg("/nonexistent/definitely/not/here.adf")
        .output()
        .expect("run ade");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn a_bad_command_line_exits_two() {
    for args in [vec![], vec!["wat"], vec!["info"], vec!["info", "a", "b"]] {
        let out = Command::new(ade()).args(&args).output().expect("run ade");
        assert_eq!(out.status.code(), Some(2), "args {args:?}");
    }
}

#[test]
fn a_foreign_bootblock_still_reports_its_volume() {
    let mut img = Volume::dd(1).named("QUARTEX").build();
    corrupt::non_dos_bootblock(&mut img, b"ATN!");
    let (code, out) = info(&img, "foreign");
    assert_eq!(
        code, 0,
        "a custom loader does not make a disk faulty\n{out}"
    );
    assert!(out.contains("not DOS"), "{out}");
    assert!(out.contains(r#"name        "QUARTEX""#), "{out}");
}

#[test]
fn evidence_is_always_shown() {
    let (_, out) = info(&Volume::dd(3).build(), "evidence");
    assert!(out.contains("evidence"), "{out}");
    assert!(out.contains("bootblock begins DOS\\3"), "{out}");
    assert!(out.contains("exactly 80 cylinders"), "{out}");
}

#[test]
fn degenerate_images_do_not_crash_the_binary() {
    for (name, bytes) in [
        ("empty", Vec::new()),
        ("tiny", vec![0u8; 3]),
        ("zeroed", corrupt::zeroed_volume()),
        ("truncated", corrupt::truncated(&Volume::dd(0).build(), 176)),
        (
            "plusone",
            corrupt::with_trailing_junk(&Volume::dd(0).build(), 1),
        ),
    ] {
        let (code, _) = info(&bytes, name);
        assert!(
            (0..=4).contains(&code),
            "{name} exited {code}, not a defined code"
        );
    }
}

// --- machine-readable output (F-015, IMP-001) ---------------------------

/// A crude JSON field reader. Enough to assert on shape without giving the
/// test suite a JSON dependency the engine itself does not have.
fn field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let start = json.find(&needle)? + needle.len();
    let rest = json.get(start..)?;
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    let end = rest.find(['"', ',', '}']).unwrap_or(rest.len());
    rest.get(..end)
}

#[test]
fn ls_json_emits_one_object_per_entry() {
    let mut v = Volume::dd(1).named("Machine");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    let (code, out) = info_args(&v.build(), "lsjson", &["ls", "--format=json"]);
    assert_eq!(code, 0, "{out}");
    let lines: Vec<_> = out.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "one line per entry:\n{out}");
    for line in &lines {
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
        assert!(line.is_ascii(), "output must be pure ASCII: {line}");
    }
    assert!(lines.iter().any(|l| field(l, "name") == Some("readme")));
    assert!(lines.iter().any(|l| field(l, "kind") == Some("dir")));
}

#[test]
fn json_escapes_latin1_names_losslessly() {
    // 0xE9 is ISO 8859-1 'e-acute'. It must appear as an escape, never as a
    // raw byte, or the line stops being valid JSON.
    let mut v = Volume::dd(1);
    v.add_file("Caf", b"x");
    let mut img = v.build();
    // Rewrite the name in place to a genuine Latin-1 byte the builder would
    // have re-encoded.
    let needle = b"Caf";
    if let Some(pos) = img.windows(3).position(|w| w == needle) {
        img[pos + 3] = 0xE9;
        img[pos - 1] = 4; // name length
    }
    let (_, out) = info_args(&img, "latin1", &["ls", "--format=json"]);
    assert!(out.is_ascii(), "non-ASCII leaked into JSON:\n{out}");
    assert!(out.contains("\\u00e9"), "expected an escaped byte:\n{out}");
}

#[test]
fn info_json_carries_stable_fault_codes() {
    let v = Volume::dd(1).named("Unclean");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root);
    let (code, out) = info_args(&img, "faultcodes", &["info", "--format=json"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.starts_with('{') && out.trim_end().ends_with('}'),
        "{out}"
    );
    assert!(
        out.contains("\"bitmap-flag-clear\""),
        "stable code missing:\n{out}"
    );
    // The code is the contract; the message is not.
    assert!(out.contains("\"code\":"), "{out}");
    assert!(out.contains("\"message\":"), "{out}");
}

#[test]
fn json_and_text_agree_on_whether_an_image_is_faulty() {
    let v = Volume::dd(0).named("Same");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root);
    let (text_code, _) = info_args(&img, "agree-t", &["info"]);
    let (json_code, _) = info_args(&img, "agree-j", &["info", "--format=json"]);
    assert_eq!(
        text_code, json_code,
        "formats must not disagree on the verdict"
    );
}

#[test]
fn an_unknown_format_is_a_usage_error() {
    let (code, _) = info_args(&Volume::dd(1).build(), "badfmt", &["info", "--format=yaml"]);
    assert_eq!(code, 2);
    let (code, _) = info_args(&Volume::dd(1).build(), "badopt", &["info", "--wat"]);
    assert_eq!(code, 2);
}

#[test]
fn a_closed_pipe_does_not_panic() {
    // `ade ls --format=json big.adf | head` must end quietly, not abort.
    let mut v = Volume::dd(1);
    for i in 0..40 {
        v.add_file(&format!("file{i:03}"), b"x");
    }
    let path = write_temp(&v.build(), "pipe");
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{} ls --format=json {} 2>&1 | head -2",
            ade().display(),
            path.display()
        ))
        .output()
        .expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    let _ = fs::remove_file(&path);
    assert!(
        !text.contains("panicked"),
        "panicked on a closed pipe:\n{text}"
    );
    assert_eq!(text.lines().count(), 2);
}
