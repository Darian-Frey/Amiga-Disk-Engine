//! `ade mkdir`, `put`, `rm`, `rename` and `repair` — writing into a disk, at
//! the command line.
//!
//! The engine's side is checked in `src/api/tests/edit.rs`, against ADE's
//! health report and ADFlib. These check what only the command can get wrong:
//! that it has no in-place form (D-018), that the image it read is untouched
//! afterwards, and that where a file lands follows from the arguments.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_core::layers::endian::put_u32;

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
    let dir = std::env::temp_dir().join(format!("ade-edit-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A blank disk to edit.
fn disk(dir: &std::path::Path) -> String {
    let path = dir.join("blank.adf");
    let (_, err, code) = run(&["create", path.to_str().unwrap(), "--name=Work"]);
    assert_eq!(code, Some(0), "{err}");
    path.to_str().unwrap().to_owned()
}

#[test]
fn there_is_no_in_place_form() {
    let dir = scratch("inplace");
    let image = disk(&dir);
    let before = fs::read(&image).unwrap();

    let (_, err, code) = run(&["mkdir", &image, "New"]);
    assert_eq!(code, Some(2), "without --output it must refuse");
    assert!(err.contains("--output"), "{err}");

    let (_, err, code) = run(&["mkdir", &image, "New", &format!("--output={image}")]);
    assert_eq!(code, Some(2));
    assert!(err.contains("opened from"), "{err}");

    assert_eq!(
        fs::read(&image).unwrap(),
        before,
        "the original is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mkdir_then_put_builds_a_tree_every_command_can_read() {
    let dir = scratch("tree");
    let image = disk(&dir);
    let original = fs::read(&image).unwrap();
    let one = dir.join("one.adf");
    let two = dir.join("two.adf");
    let three = dir.join("three.adf");
    let host = dir.join("notes.txt");
    fs::write(&host, b"written on the host\n").unwrap();

    let (out, err, code) = run(&[
        "mkdir",
        &image,
        "Docs",
        &format!("--output={}", one.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("Docs"), "{out}");

    // Into an existing directory, under its own name.
    let (_, err, code) = run(&[
        "put",
        one.to_str().unwrap(),
        host.to_str().unwrap(),
        "docs",
        &format!("--output={}", two.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    // Under a name the destination path gives it.
    let (_, err, code) = run(&[
        "put",
        two.to_str().unwrap(),
        host.to_str().unwrap(),
        "Docs/ReadMe",
        &format!("--output={}", three.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let (listing, _, code) = run(&["ls", three.to_str().unwrap(), "Docs"]);
    assert_eq!(code, Some(0));
    assert!(listing.contains("notes.txt"), "{listing}");
    assert!(listing.contains("ReadMe"), "{listing}");

    let (report, _, code) = run(&["check", three.to_str().unwrap()]);
    assert_eq!(code, Some(0), "the result passes `ade check`: {report}");

    let out = dir.join("back.txt");
    let (_, err, code) = run(&[
        "extract",
        three.to_str().unwrap(),
        "Docs/ReadMe",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(&out).unwrap(), b"written on the host\n");

    assert_eq!(
        fs::read(&image).unwrap(),
        original,
        "the first image is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_is_refused_rather_than_ignored() {
    // BUG-007's lesson: a flag accepted and ignored lets a script mistake an
    // unsupported command for a successful one.
    let dir = scratch("json");
    let image = disk(&dir);
    let out = dir.join("out.adf");
    let (_, err, code) = run(&[
        "--format=json",
        "mkdir",
        &image,
        "New",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("JSON"), "{err}");
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_refusal_writes_nothing() {
    let dir = scratch("refuse");
    let image = disk(&dir);
    let out = dir.join("out.adf");

    // No parent to put it in.
    let (_, err, code) = run(&[
        "mkdir",
        &image,
        "Missing/Child",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("Missing"), "{err}");
    assert!(!out.exists(), "a refused edit saves nothing");

    // A name AmigaDOS cannot hold.
    let (_, _, code) = run(&[
        "mkdir",
        &image,
        &"x".repeat(31),
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(!out.exists());

    // Not Latin-1.
    let (_, err, code) = run(&[
        "mkdir",
        &image,
        "日本",
        &format!("--output={}", out.display()),
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("Latin-1"), "{err}");
    assert!(!out.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_file_put_under_an_accented_name_extracts_by_that_name() {
    // BUG-011, from the command line: `ls` listed `für.txt` and `extract`
    // said "no such entry".
    let dir = scratch("accent");
    let image = disk(&dir);
    let host = dir.join("für.txt");
    fs::write(&host, b"umlaut").unwrap();
    let edited = dir.join("edited.adf");
    let (_, err, code) = run(&[
        "put",
        &image,
        host.to_str().unwrap(),
        &format!("--output={}", edited.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let out = dir.join("back.txt");
    let (_, err, code) = run(&[
        "extract",
        edited.to_str().unwrap(),
        "für.txt",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(&out).unwrap(), b"umlaut");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn extracting_onto_an_existing_file_is_refused() {
    // IMP-010: every command that writes a file refuses one that is there,
    // and single-file `extract` was the last that replaced it silently.
    let dir = scratch("extract-noclobber");
    let image = disk(&dir);
    let host = dir.join("note.txt");
    fs::write(&host, b"from the host\n").unwrap();
    let with = dir.join("with.adf");
    let (_, err, code) = run(&[
        "put",
        &image,
        host.to_str().unwrap(),
        "note.txt",
        &format!("--output={}", with.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let out = dir.join("out.txt");
    fs::write(&out, b"do not lose me").unwrap();
    let (_, err, code) = run(&[
        "extract",
        with.to_str().unwrap(),
        "note.txt",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("refusing to overwrite"), "{err}");
    assert_eq!(
        fs::read(&out).unwrap(),
        b"do not lose me",
        "the existing file survives"
    );

    // A new name still works, and so does stdout, where the shell decides.
    let fresh = dir.join("fresh.txt");
    let (_, err, code) = run(&[
        "extract",
        with.to_str().unwrap(),
        "note.txt",
        fresh.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert_eq!(fs::read(&fresh).unwrap(), b"from the host\n");
    let (stdout, _, code) = run(&["extract", with.to_str().unwrap(), "note.txt"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "from the host\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one story, from a blank disk to a carved file, told in order"
)]
fn rm_and_rename_write_new_images_and_a_deleted_file_carves_back() {
    let dir = scratch("rm-rename");
    let blank = dir.join("ofs.adf");
    let (_, err, code) = run(&[
        "create",
        blank.to_str().unwrap(),
        "--name=Work",
        "--type=ofs",
    ]);
    assert_eq!(code, Some(0), "{err}");
    let host = dir.join("note.txt");
    fs::write(&host, b"a file to lose and find\n").unwrap();
    let one = dir.join("one.adf");
    let (_, err, code) = run(&[
        "put",
        blank.to_str().unwrap(),
        host.to_str().unwrap(),
        "Drawer",
        &format!("--output={}", one.display()),
    ]);
    // `Drawer` does not exist, so it is the file's new name.
    assert_eq!(code, Some(0), "{err}");
    let two = dir.join("two.adf");
    let (_, err, code) = run(&[
        "mkdir",
        one.to_str().unwrap(),
        "Box",
        &format!("--output={}", two.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    let three = dir.join("three.adf");
    let (_, err, code) = run(&[
        "put",
        two.to_str().unwrap(),
        host.to_str().unwrap(),
        "Box",
        &format!("--output={}", three.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");

    // Rename, and the file is found by its new name only.
    let renamed = dir.join("renamed.adf");
    let (out, err, code) = run(&[
        "rename",
        three.to_str().unwrap(),
        "Drawer",
        "Kept",
        &format!("--output={}", renamed.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("renamed Drawer to Kept"), "{out}");
    let (listing, _, _) = run(&["ls", renamed.to_str().unwrap()]);
    assert!(
        listing.contains("Kept") && !listing.contains("Drawer"),
        "{listing}"
    );

    // A folder with something in it needs --all.
    let refused = dir.join("refused.adf");
    let (_, err, code) = run(&[
        "rm",
        renamed.to_str().unwrap(),
        "Box",
        &format!("--output={}", refused.display()),
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("delete them first"), "{err}");
    assert!(!refused.exists(), "a refused edit saves nothing");
    let emptied = dir.join("emptied.adf");
    let (out, err, code) = run(&[
        "rm",
        renamed.to_str().unwrap(),
        "Box",
        "--all",
        &format!("--output={}", emptied.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("2 entries"), "{out}");

    // Delete the file, and carve it back out of the saved image.
    let gone = dir.join("gone.adf");
    let (_, err, code) = run(&[
        "rm",
        emptied.to_str().unwrap(),
        "Kept",
        &format!("--output={}", gone.display()),
    ]);
    assert_eq!(code, Some(0), "{err}");
    let (listing, _, _) = run(&["ls", gone.to_str().unwrap()]);
    assert!(
        listing.starts_with("0 entries") || listing.trim().ends_with("0 entries"),
        "{listing}"
    );
    let (check, _, code) = run(&["check", gone.to_str().unwrap()]);
    assert_eq!(code, Some(0), "the result passes `ade check`: {check}");
    let recovered = dir.join("recovered");
    let (out, err, code) = run(&[
        "carve",
        gone.to_str().unwrap(),
        "--all",
        recovered.to_str().unwrap(),
    ]);
    // Exit 1, because the deleted folder `Box` is found and, being a
    // directory, not written: carve says when not everything came back.
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(out.contains("not written"), "{out}");
    let found: Vec<_> = fs::read_dir(&recovered)
        .unwrap()
        .map(|e| fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(
        found.iter().any(|b| b == b"a file to lose and find\n"),
        "the deleted file carves back byte for byte"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn repair_saves_only_when_something_changed() {
    let dir = scratch("repair");
    let image = disk(&dir);
    let out = dir.join("mended.adf");
    let output = format!("--output={}", out.display());

    // A sound disk: nothing to do, so nothing is written.
    let (text, err, code) = run(&["repair", &image, &output]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("nothing to repair"), "{text}");
    assert!(!out.exists(), "nothing changed, so nothing is saved");

    // Clear the bitmap-valid flag and reseal the rootblock.
    let mut bytes = fs::read(&image).unwrap();
    let root = 880 * 512;
    let flag = root + 512 - 200;
    put_u32(&mut bytes, flag, 0).unwrap();
    put_u32(&mut bytes, root + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[root..root + 512], 20).unwrap();
    put_u32(&mut bytes, root + 20, sum).unwrap();
    let damaged = dir.join("damaged.adf");
    fs::write(&damaged, &bytes).unwrap();
    let damaged = damaged.to_str().unwrap();

    let (_, err, code) = run(&["repair", damaged]);
    assert_eq!(code, Some(2), "without --output it must refuse");
    assert!(err.contains("--output"), "{err}");

    let (text, err, code) = run(&["repair", damaged, &output]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("bitmap-valid flag"), "{text}");
    assert_eq!(
        fs::read(&out).unwrap(),
        fs::read(&image).unwrap(),
        "the disk as it was made"
    );
    assert_eq!(
        fs::read(damaged).unwrap(),
        bytes,
        "the original is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mv_format_and_wipe_write_new_images() {
    let dir = scratch("suite");
    let image = disk(&dir);
    let host = dir.join("notes.txt");
    fs::write(&host, b"moving notes").unwrap();
    let out = |n: &str| format!("--output={}", dir.join(n).display());
    let at = |n: &str| dir.join(n).to_str().unwrap().to_owned();
    assert_eq!(run(&["mkdir", &image, "Box", &out("1.adf")]).2, Some(0));
    assert_eq!(
        run(&["put", &at("1.adf"), host.to_str().unwrap(), &out("2.adf")]).2,
        Some(0)
    );

    // Move it into the folder; the original copy keeps it where it was.
    let (text, err, code) = run(&["mv", &at("2.adf"), "notes.txt", "Box", &out("3.adf")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("moved notes.txt into Box"), "{text}");
    let (listing, _, _) = run(&["ls", &at("3.adf"), "Box"]);
    assert!(listing.contains("notes.txt"), "{listing}");
    let (_, err, code) = run(&["mv", &at("3.adf"), "Box", "Box", &out("x.adf")]);
    assert_eq!(code, Some(2), "a folder does not go inside itself");
    assert!(err.contains("inside itself"), "{err}");

    // Wipe needs to be told what.
    let (_, err, code) = run(&["wipe", &at("3.adf"), &out("w.adf")]);
    assert_eq!(code, Some(2));
    assert!(err.contains("--slack"), "{err}");
    let (text, err, code) = run(&["wipe", &at("3.adf"), "--slack", "--free", &out("4.adf")]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        text.contains("file slack") && text.contains("free blocks"),
        "{text}"
    );

    // Format, under a new name; nothing is left to list.
    let (_, err, code) = run(&["format", &at("4.adf"), "--name=Fresh", &out("5.adf")]);
    assert_eq!(code, Some(0), "{err}");
    let (info, _, _) = run(&["info", &at("5.adf")]);
    assert!(info.contains("Fresh"), "{info}");
    let (listing, _, _) = run(&["ls", &at("5.adf")]);
    assert!(listing.contains("0 entries"), "{listing}");
    let _ = fs::remove_dir_all(&dir);
}
