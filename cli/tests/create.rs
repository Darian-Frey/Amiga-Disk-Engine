//! `ade create` — the first write path, at the command line (F-019).
//!
//! The engine's side is checked in `src/api/tests/create.rs`, three
//! independent ways. These check the things only the command can get wrong:
//! that it will not overwrite, that it says no clearly, and that what it
//! writes is immediately usable by the rest of the tool.

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

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-create-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_created_disk_is_immediately_usable_by_every_other_command() {
    // The point of the feature: a disk you can then do things with. If `ls`
    // and `check` cannot read what `create` wrote, the format is wrong however
    // well-formed it looks.
    let dir = scratch("usable");
    let disk = dir.join("new.adf");

    let (stdout, _, code) = run(&["create", disk.to_str().unwrap(), "--name=Fresh"]);
    assert_eq!(code, Some(0));
    assert!(stdout.contains("Fresh"), "{stdout}");
    assert_eq!(fs::metadata(&disk).unwrap().len(), 901_120);

    let (listing, _, ls_code) = run(&["ls", disk.to_str().unwrap()]);
    assert_eq!(ls_code, Some(0), "an empty disk lists cleanly");
    assert!(listing.contains("0 entries"), "{listing}");

    let (report, _, check_code) = run(&["check", disk.to_str().unwrap()]);
    assert_eq!(check_code, Some(0), "and passes its own health check");
    assert!(report.contains("findings    none"), "{report}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn it_will_not_overwrite() {
    // The whole safety story of a write path that cannot damage anything: it
    // only ever creates. A `create` that clobbered an image would be the
    // irreversible loss D-004 exists to prevent, arriving through the one
    // command nobody would suspect.
    let dir = scratch("overwrite");
    let disk = dir.join("precious.adf");
    fs::write(&disk, b"not a disk, but mine").unwrap();

    let (_, stderr, code) = run(&["create", disk.to_str().unwrap()]);
    assert_eq!(code, Some(2), "refusing is a usage error");
    assert!(stderr.contains("refusing to overwrite"), "{stderr}");
    assert_eq!(fs::read(&disk).unwrap(), b"not a disk, but mine");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_filesystem_and_density_are_chosen_not_assumed() {
    let dir = scratch("options");
    let ofs = dir.join("ofs.adf");
    let hd = dir.join("hd.adf");

    run(&["create", ofs.to_str().unwrap(), "--type=ofs"]);
    run(&["create", hd.to_str().unwrap(), "--hd"]);

    let (info, _, _) = run(&["info", ofs.to_str().unwrap()]);
    assert!(info.contains("OFS"), "{info}");
    assert_eq!(fs::metadata(&hd).unwrap().len(), 1_802_240);
    let (hd_info, _, _) = run(&["info", hd.to_str().unwrap()]);
    assert!(hd_info.contains("22 sectors"), "{hd_info}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_filesystem_ade_cannot_write_is_a_usage_error() {
    let dir = scratch("badtype");
    let (_, stderr, code) = run(&["create", dir.join("x.adf").to_str().unwrap(), "--type=pfs"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("expected ofs or ffs"), "{stderr}");
    assert!(!dir.join("x.adf").exists(), "and nothing is written");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_created_disk_carries_a_real_creation_date() {
    // Day zero is what Amiga software treats as unset, and ADE's own check
    // reports it: the first disk this command produced flagged three
    // `datestamp-day-zero` findings against itself.
    let dir = scratch("dated");
    let disk = dir.join("dated.adf");
    run(&["create", disk.to_str().unwrap()]);

    let (info, _, _) = run(&["info", disk.to_str().unwrap()]);
    assert!(info.contains("created"), "{info}");
    assert!(
        !info.contains("1978-01-01"),
        "a real date, not the epoch: {info}"
    );
    let _ = fs::remove_dir_all(&dir);
}
