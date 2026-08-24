//! End-to-end tests for addressing partitions from the command line.
//!
//! A partitioned device has no volume of its own, so every subcommand that
//! reads a filesystem needs to be told which partition it means. These tests
//! cover that surface: the default, the two ways of naming a partition, and
//! what happens when the name is wrong.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_fixtures::device::Device;

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

/// A two-partition device: FFS then OFS, bootable then not.
fn device() -> Vec<u8> {
    let mut d = Device::new(64, 4, 32);
    d.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello from DH0");
        v.add_dir("Tools");
    });
    d.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });
    d.build()
}

/// Run `ade` over a temporary device. Returns (code, stdout, stderr).
fn run(name: &str, args: &[&str]) -> (i32, String, String) {
    let path = std::env::temp_dir().join(format!("ade-part-{name}-{}.hdf", std::process::id()));
    fs::write(&path, device()).expect("write fixture");
    let out = Command::new(ade())
        .args(args)
        .arg(&path)
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn info_reports_the_partition_table() {
    let (code, out, _) = run("info", &["info"]);

    // Having no volume of its own is not a fault on a partitioned device, so
    // this must not exit with the no-volume code.
    assert_eq!(code, 0, "a clean device exits 0\n{out}");
    assert!(out.contains("rigid disk block"), "{out}");
    assert!(out.contains("64 cylinders x 4 heads x 32 sectors"), "{out}");
    assert!(out.contains("partitions  2"), "{out}");
    assert!(out.contains("DH0"), "{out}");
    assert!(out.contains("DH1"), "{out}");
}

#[test]
fn info_does_not_report_an_rdb_as_a_bootblock() {
    // Block 0 of a device is an RDSK structure. Parsing it as a bootblock
    // yields a confident report about a checksum that was never a checksum.
    let (_, out, _) = run("info-bb", &["info"]);

    assert!(
        !out.contains("bootblock"),
        "a device has no bootblock of its own\n{out}"
    );
}

#[test]
fn json_carries_the_partition_table() {
    let (code, out, _) = run("json", &["info", "--json"]);

    assert_eq!(code, 0);
    // F-015: these field names are part of the scriptable surface.
    for field in [
        "\"rdb\"",
        "\"partitions\"",
        "\"partition_faults\"",
        "\"low_cylinder\"",
        "\"high_cylinder\"",
        "\"first_block\"",
        "\"volume_name\"",
    ] {
        assert!(out.contains(field), "missing {field}\n{out}");
    }
    assert!(out.contains("\"bootblock\":null"), "{out}");
}

#[test]
fn ls_defaults_to_the_first_partition() {
    let (code, out, _) = run("ls-default", &["ls"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("startup"), "{out}");
    assert!(!out.contains("data.bin"), "{out}");
}

#[test]
fn a_partition_can_be_named() {
    let (code, out, _) = run("ls-name", &["ls", "--partition=DH1"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("data.bin"), "{out}");
    assert!(!out.contains("startup"), "{out}");
}

#[test]
fn a_partition_name_is_case_insensitive() {
    // AmigaDOS device names are not case sensitive, and neither is anything
    // else in the filesystem (C-006).
    let (code, out, _) = run("ls-case", &["ls", "--partition=dh1"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("data.bin"), "{out}");
}

#[test]
fn a_partition_can_be_selected_by_index() {
    let (code, out, _) = run("ls-index", &["ls", "--partition=1"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("data.bin"), "{out}");
}

#[test]
fn an_unknown_partition_names_the_ones_that_exist() {
    let (code, _, err) = run("ls-bad", &["ls", "--partition=DH9"]);

    assert_ne!(code, 0, "naming a partition that is not there must fail");
    assert!(
        err.contains("DH0"),
        "the error should list what is there: {err}"
    );
    assert!(err.contains("DH1"), "{err}");
}

#[test]
fn extract_reads_from_the_named_partition() {
    let path = std::env::temp_dir().join(format!("ade-part-x-{}.hdf", std::process::id()));
    let dest = std::env::temp_dir().join(format!("ade-part-x-{}.out", std::process::id()));
    fs::write(&path, device()).expect("write fixture");

    let out = Command::new(ade())
        .args(["extract", "--partition=DH0"])
        .arg(&path)
        .arg("startup")
        .arg(&dest)
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);

    assert_eq!(
        out.status.code().unwrap_or(-1),
        0,
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = fs::read(&dest).expect("extracted file");
    let _ = fs::remove_file(&dest);
    assert_eq!(got, b"hello from DH0");
}

#[test]
fn extract_from_a_different_partition_reads_different_bytes() {
    // The same path could exist on both partitions; what distinguishes them is
    // the flag, so this checks the flag actually reaches the read.
    let path = std::env::temp_dir().join(format!("ade-part-y-{}.hdf", std::process::id()));
    fs::write(&path, device()).expect("write fixture");

    let out = Command::new(ade())
        .args(["extract", "--partition=DH1"])
        .arg(&path)
        .arg("data.bin")
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);

    assert_eq!(out.status.code().unwrap_or(-1), 0);
    assert_eq!(out.stdout.len(), 3000);
    assert!(out.stdout.iter().all(|&b| b == 0xAA));
}

#[test]
fn naming_a_partition_on_a_floppy_is_an_error() {
    // Silently ignoring the flag would read the floppy and report success for
    // something the caller did not ask for.
    let path = std::env::temp_dir().join(format!("ade-part-fl-{}.adf", std::process::id()));
    fs::write(
        &path,
        ade_fixtures::Volume::dd(1).named("Workbench").build(),
    )
    .expect("write");
    let out = Command::new(ade())
        .args(["ls", "--partition=DH0"])
        .arg(&path)
        .output()
        .expect("run ade");
    let _ = fs::remove_file(&path);

    assert_ne!(out.status.code().unwrap_or(-1), 0);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no partition table"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn check_examines_a_partition_not_the_device() {
    // A device has no volume at its own rootblock. Reporting "no volume" and
    // exiting non-zero would call a sound disk broken.
    let (code, out, _) = run("check", &["check"]);

    assert_eq!(code, 0, "a sound device exits 0\n{out}");
    assert!(out.contains("on partition DH0"), "{out}");
    assert!(out.contains("1 files"), "{out}");
    assert!(out.contains("findings    none"), "{out}");
}

#[test]
fn check_follows_the_partition_flag() {
    let (code, out, _) = run("check-p", &["check", "--partition=DH1"]);

    assert_eq!(code, 0, "{out}");
    assert!(out.contains("on partition DH1"), "{out}");
    assert!(out.contains("3000 bytes recovered"), "{out}");
}

#[test]
fn check_json_says_what_it_examined() {
    let (_, out, _) = run("check-json", &["check", "--json", "--partition=DH1"]);

    assert!(out.contains("\"examined\""), "{out}");
    assert!(out.contains("\"partition\":\"DH1\""), "{out}");
    assert!(out.contains("\"volume\":\"DH1\""), "{out}");
}

#[test]
fn checking_an_absent_partition_is_an_error() {
    let (code, out, _) = run("check-bad", &["check", "--partition=DH9"]);

    assert_ne!(code, 0);
    assert!(out.contains("no-such-partition"), "{out}");
    // Nothing was examined, so no tree figures should be reported at all.
    assert!(!out.contains("contents"), "{out}");
}
