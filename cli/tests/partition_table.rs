//! `ade partition` and `ade format --type`: editing a hard disk's partition
//! table from the command line (F-038, D-024 part 5). The engine's side is
//! `src/api/tests/partition_table.rs`; AmigaDOS mounting an edited table is
//! in `src/api/tests/amigados.rs`.

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

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-ptable-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn at(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).to_str().unwrap().to_owned()
}

#[test]
fn a_table_is_edited_one_step_per_command_each_saved_as_a_new_image() {
    let dir = scratch("steps");
    // One image per step, named for what it holds.
    let made = at(&dir, "made.hdf");
    let added = at(&dir, "added.hdf");
    let renamed = at(&dir, "renamed.hdf");
    let booted = at(&dir, "booted.hdf");
    let reformatted = at(&dir, "reformatted.hdf");
    let deleted = at(&dir, "deleted.hdf");
    let (code, _, err) = run(&["create", &made, "--size=20", "--partition=DH0:8"]);
    assert_eq!(code, 0, "{err}");

    let (code, out, err) = run(&[
        "partition",
        "add",
        &made,
        "DH1:rest:ofs:Data",
        &format!("--output={added}"),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("added DH1") && out.contains("not bootable"),
        "{out}"
    );
    let (code, _, err) = run(&[
        "partition",
        "rename",
        &added,
        "DH1",
        "WORK",
        &format!("--output={renamed}"),
    ]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = run(&[
        "partition",
        "boot",
        &renamed,
        "WORK",
        "-2",
        &format!("--output={booted}"),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("priority -2"), "{out}");
    let (code, _, err) = run(&[
        "format",
        &booted,
        "--partition=WORK",
        "--type=ffs-dc",
        "--name=Cached",
        &format!("--output={reformatted}"),
    ]);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = run(&[
        "partition",
        "delete",
        &reformatted,
        "DH0",
        &format!("--output={deleted}"),
    ]);
    assert_eq!(code, 0, "{err}");

    let (_, info, _) = run(&["info", &deleted]);
    assert!(
        info.contains("partitions  1") && info.contains("WORK"),
        "{info}"
    );
    assert!(info.contains("0x444f5305, bootable"), "{info}");
    assert!(info.contains("volume \"Cached\""), "{info}");
    let (code, report, _) = run(&["check", &deleted, "--partition=WORK"]);
    assert_eq!(code, 0, "{report}");
    // Each step left the image before it as it was.
    let (_, info, _) = run(&["info", &made]);
    assert!(
        info.contains("partitions  1") && info.contains("DH0"),
        "{info}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_table_edit_that_is_not_one_is_refused_and_nothing_is_written() {
    let dir = scratch("refused");
    let a = at(&dir, "a.hdf");
    let out = at(&dir, "out.hdf");
    run(&[
        "create",
        &a,
        "--size=12",
        "--partition=DH0:4",
        "--partition=DH1:rest",
    ]);
    let floppy = at(&dir, "f.adf");
    run(&["create", &floppy]);
    let output = format!("--output={out}");
    for (args, says) in [
        (vec!["partition", "add", &a, "DH2:1"], "largest is 0 MB"),
        (vec!["partition", "add", &a, "dh0:1"], "already named"),
        (
            vec!["partition", "rename", &a, "DH1", "DH0"],
            "already named",
        ),
        (
            vec!["partition", "rename", &a, "DH9", "X"],
            "no partition \"DH9\"",
        ),
        (
            vec!["partition", "delete", &a, "DH9"],
            "no partition \"DH9\"",
        ),
        (vec!["partition", "boot", &a, "DH1", "200"], "-128 to 127"),
        (vec!["partition", "boot", &a, "DH1", "maybe"], "-128 to 127"),
        (vec!["partition", "frob", &a], "partition add"),
        (
            vec!["partition", "add", &floppy, "DH0:1"],
            "no partition table",
        ),
        (
            vec!["partition", "add", &a, "DH2:1:lnfs"],
            "expected one of",
        ),
    ] {
        let mut full = args.clone();
        full.push(&output);
        let (code, _, err) = run(&full);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains(says), "{args:?}: {err}");
        assert!(!dir.join("out.hdf").exists(), "{args:?} wrote a file");
    }
    // And --output is required, as for every edit.
    let (code, _, err) = run(&["partition", "delete", &a, "DH1"]);
    assert_eq!(code, 2);
    assert!(err.contains("--output"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partition_grows_with_its_files_and_refuses_what_it_cannot() {
    let dir = scratch("grow");
    let (made, filled, grown, out) = (
        at(&dir, "made.hdf"),
        at(&dir, "filled.hdf"),
        at(&dir, "grown.hdf"),
        at(&dir, "out.hdf"),
    );
    run(&["create", &made, "--size=16", "--partition=DH0:4"]);
    let note = dir.join("note.txt");
    fs::write(&note, b"still here").unwrap();
    let (code, _, err) = run(&[
        "put",
        &made,
        note.to_str().unwrap(),
        "--partition=DH0",
        &format!("--output={filled}"),
    ]);
    assert_eq!(code, 0, "{err}");

    let (code, out_text, err) = run(&[
        "partition",
        "grow",
        &filled,
        "DH0",
        "rest",
        &format!("--output={grown}"),
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(out_text.contains("grew DH0"), "{out_text}");
    let (_, info, _) = run(&["info", &grown]);
    assert!(info.contains("block 64 + 32704 x 512"), "{info}");
    let (code, report, _) = run(&["check", &grown, "--partition=DH0"]);
    assert_eq!(code, 0, "{report}");
    let (_, listing, _) = run(&["ls", &grown, "--partition=DH0"]);
    assert!(listing.contains("note.txt"), "{listing}");

    let output = format!("--output={out}");
    for (args, says) in [
        (
            vec!["partition", "grow", &grown, "DH0", "rest"],
            "no unused cylinders",
        ),
        (
            vec!["partition", "grow", &filled, "DH0", "2"],
            "never shrinks",
        ),
        (
            vec!["partition", "grow", &filled, "DH0", "40"],
            "can grow by",
        ),
        (
            vec!["partition", "grow", &filled, "DH0", "big"],
            "megabytes, or rest",
        ),
    ] {
        let mut full = args.clone();
        full.push(&output);
        let (code, _, err) = run(&full);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(err.contains(says), "{args:?}: {err}");
        assert!(!dir.join("out.hdf").exists(), "{args:?} wrote a file");
    }
    let _ = fs::remove_dir_all(&dir);
}
