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
    // PFS is a real filesystem and not ADE's to write: it is one of the forty
    // or so non-AmigaDOS tags in SPEC's registry, and none of them appears in
    // the corpus.
    let (_, stderr, code) = run(&["create", dir.join("x.adf").to_str().unwrap(), "--type=pfs"]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("ofs-dc"), "the six it does write: {stderr}");
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

#[test]
fn every_type_the_command_offers_writes_that_dostype() {
    // Six, not two. `DOS\6` and `DOS\7` are refused by name rather than
    // written badly: LNFS is deferred by D-013 on verifiability.
    let dir = scratch("types");
    for (name, flags) in [
        ("ofs", "DOS\\0"),
        ("ffs", "DOS\\1"),
        ("ofs-intl", "DOS\\2"),
        ("ffs-intl", "DOS\\3"),
        ("ofs-dc", "DOS\\4"),
        ("ffs-dc", "DOS\\5"),
    ] {
        let path = dir.join(format!("{name}.adf"));
        let (_, err, code) = run(&[
            "create",
            path.to_str().unwrap(),
            &format!("--type={name}"),
            "--name=Types",
        ]);
        assert_eq!(code, Some(0), "{name}: {err}");

        let (out, _, code) = run(&["info", path.to_str().unwrap()]);
        assert_eq!(code, Some(0));
        assert!(out.contains(flags), "{name} should be {flags}: {out}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lnfs_is_refused_by_name_and_says_why() {
    // Not "unknown type": somebody asking for LNFS has asked for something
    // real, and the answer is that ADE will not write what it cannot check.
    let dir = scratch("lnfs");
    let (_, err, code) = run(&[
        "create",
        dir.join("l.adf").to_str().unwrap(),
        "--type=ffs-lnfs",
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert!(
        err.contains("D-013"),
        "the reason, not just a refusal: {err}"
    );
    assert!(!dir.join("l.adf").exists(), "and nothing was written");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_geometries_are_the_sizes_they_claim() {
    let dir = scratch("sizes");
    for (flag, bytes) in [
        (None, 901_120u64),
        (Some("--hd"), 1_802_240),
        (Some("--dd525"), 450_560),
    ] {
        let path = dir.join(format!("g{bytes}.adf"));
        let mut args = vec!["create", path.to_str().unwrap(), "--name=Geo"];
        if let Some(f) = flag {
            args.push(f);
        }
        let (_, err, code) = run(&args);
        assert_eq!(code, Some(0), "{flag:?}: {err}");
        assert_eq!(fs::metadata(&path).unwrap().len(), bytes, "{flag:?}");

        // And each reads back as a sound volume, which is the point of the
        // size being right.
        let (_, _, code) = run(&["check", path.to_str().unwrap()]);
        assert_eq!(code, Some(0), "{flag:?} should be sound");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_hard_disk_is_made_by_size_and_is_not_a_bigger_floppy() {
    let dir = scratch("hardfile");
    let path = dir.join("big.hdf");
    let (_, err, code) = run(&[
        "create",
        path.to_str().unwrap(),
        "--size=8",
        "--name=BigDisk",
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::metadata(&path).unwrap().len(), 8 * 1024 * 1024);

    let (out, _, code) = run(&["info", path.to_str().unwrap()]);
    assert_eq!(code, Some(0));
    assert!(out.contains("hardfile"), "not a floppy: {out}");
    let (_, _, code) = run(&["check", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "and sound, with its five bitmap blocks");

    // The floppy flags describe a shape a hard disk does not have.
    let (_, err, code) = run(&[
        "create",
        dir.join("clash.hdf").to_str().unwrap(),
        "--size=8",
        "--hd",
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("no floppy geometry"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_hard_disk_too_large_for_its_own_bitmap_is_refused() {
    // Past 25 bitmap pointers the rest belong in a `bm_ext` chain ADE does not
    // write. A volume whose bitmap is half described reports free blocks that
    // are not.
    let dir = scratch("toobig");
    let (_, err, code) = run(&[
        "create",
        dir.join("huge.hdf").to_str().unwrap(),
        "--size=64",
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("bitmap extension"), "{err}");
    assert!(!dir.join("huge.hdf").exists(), "nothing half-written");
    let _ = fs::remove_dir_all(&dir);
}
