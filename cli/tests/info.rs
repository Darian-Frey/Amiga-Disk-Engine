//! End-to-end tests for `ade info`.
//!
//! Exercises the binary itself, so the exit codes F-015 commits to are covered
//! by tests rather than by intention. Fixtures are generated (D-010); no image
//! is read from the repository.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
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
