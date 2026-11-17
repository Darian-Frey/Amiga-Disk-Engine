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
fn a_hard_disk_past_49_mb_is_made_and_past_4_gb_is_refused() {
    // Past 25 bitmap pointers the rest go in a `bm_ext` chain, which ADE
    // writes (F-038); past 4 GB an AmigaDOS volume cannot address its blocks
    // (C-002), and nothing is written.
    let dir = scratch("bigdisk");
    let made = dir.join("big.hdf");
    let (_, err, code) = run(&["create", made.to_str().unwrap(), "--size=64"]);
    assert_eq!(code, Some(0), "{err}");
    let (check, _, code) = run(&["check", made.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{check}");

    let huge = dir.join("huge.hdf");
    let (_, err, code) = run(&["create", huge.to_str().unwrap(), "--size=4097"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("4 GB"), "{err}");
    assert!(!huge.exists(), "nothing written");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_accented_name_is_stored_in_latin1_and_one_latin1_lacks_is_refused() {
    // BUG-017: `--name=Größe` was stored as its UTF-8 bytes.
    let dir = scratch("latin1");
    let disk = dir.join("named.adf");
    let (_, err, code) = run(&["create", disk.to_str().unwrap(), "--name=Größe"]);
    assert_eq!(code, Some(0), "{err}");
    let bytes = fs::read(&disk).unwrap();
    let root = 880 * 512 + 512 - 80;
    assert_eq!(bytes.get(root..root + 6), Some(&b"\x05Gr\xF6\xDFe"[..]));
    let (info, _, _) = run(&["info", disk.to_str().unwrap()]);
    assert!(info.contains("Größe"), "read back as it was given: {info}");

    let refused = dir.join("refused.adf");
    let (_, err, code) = run(&["create", refused.to_str().unwrap(), "--name=日本"]);
    assert_eq!(code, Some(2));
    assert!(err.contains("Latin-1"), "{err}");
    assert!(!refused.exists(), "nothing is written");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partitioned_disk_is_made_one_partition_per_flag_and_every_command_reads_it() {
    // F-038: `--partition=NAME:SIZE[:TYPE[:VOLUME]]`, once per partition.
    let dir = scratch("rdb");
    let disk = dir.join("work.hdf");
    let path = disk.to_str().unwrap();
    let (stdout, stderr, code) = run(&[
        "create",
        path,
        "--size=20",
        "--partition=DH0:8",
        "--partition=DH1:rest:ofs:Data",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("2 partitions"), "{stdout}");
    // The rest is 11.97 MB: tenths, so whole megabytes do not round it away.
    assert!(
        stdout.contains("11.9 MB") && stdout.contains("\"Data\""),
        "{stdout}"
    );
    assert!(
        stdout.contains("bootable") && stdout.contains("ROM"),
        "{stdout}"
    );
    assert_eq!(fs::metadata(&disk).unwrap().len(), 20 * 1024 * 1024);

    let (info, _, _) = run(&["info", path]);
    assert!(
        info.contains("partitions  2") && info.contains("volume \"Data\""),
        "{info}"
    );
    for partition in ["--partition=DH0", "--partition=DH1"] {
        let (report, _, code) = run(&["check", path, partition]);
        assert_eq!(code, Some(0), "{report}");
        assert!(report.contains("findings    none"), "{report}");
    }

    // And it takes an edit in either partition.
    let note = dir.join("note.txt");
    fs::write(&note, b"hello").unwrap();
    let edited = dir.join("edited.hdf");
    let (_, stderr, code) = run(&[
        "put",
        path,
        note.to_str().unwrap(),
        "--partition=DH1",
        &format!("--output={}", edited.display()),
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    let (listing, _, _) = run(&["ls", edited.to_str().unwrap(), "--partition=DH1"]);
    assert!(listing.contains("note.txt"), "{listing}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partitioned_disk_reports_its_partitions_in_json() {
    let dir = scratch("rdb-json");
    let disk = dir.join("work.hdf");
    let (stdout, stderr, code) = run(&[
        "create",
        disk.to_str().unwrap(),
        "--size=8",
        "--partition=DH0:3:ffs",
        "--partition=DH1:rest",
        "--type=ofs-intl",
        "--format=json",
    ]);
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stdout.contains("\"partitions\":["), "{stdout}");
    assert!(
        stdout.contains("\"name\":\"DH0\",\"type\":\"ffs\""),
        "{stdout}"
    );
    // --type is the default for a partition that names none.
    assert!(
        stdout.contains("\"name\":\"DH1\",\"type\":\"ofs-intl\""),
        "{stdout}"
    );
    assert!(
        stdout.contains("\"first_block\":64,\"blocks\":6144,\"bootable\":true"),
        "{stdout}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partitioned_disk_that_is_not_one_is_refused_and_nothing_is_written() {
    let dir = scratch("rdb-refused");
    let disk = dir.join("work.hdf");
    let path = disk.to_str().unwrap();
    for (args, says) in [
        (vec!["--partition=DH0:4"], "--size"),
        (vec!["--hd", "--partition=DH0:1"], "--size"),
        (
            vec!["--size=10", "--name=X", "--partition=DH0:4"],
            "each partition's",
        ),
        (vec!["--size=10", "--partition=DH0"], "NAME:SIZE"),
        (
            vec!["--size=10", "--partition=DH0:rest", "--partition=DH1:2"],
            "only the last",
        ),
        (
            vec!["--size=10", "--partition=DH0:9", "--partition=DH1:2"],
            "do not fit",
        ),
    ] {
        let mut full = vec!["create", path];
        full.extend(args.iter().copied());
        let (_, stderr, code) = run(&full);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(stderr.contains(says), "{args:?}: {stderr}");
        assert!(!disk.exists(), "{args:?} wrote a file");
    }
    let _ = fs::remove_dir_all(&dir);
}
