//! `ade carve` at the command line (F-030).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
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
    let dir = std::env::temp_dir().join(format!("ade-carve-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An OFS disk whose root hash table has been cleared: the files are still
/// there, and nothing points at them.
fn lost(dir: &std::path::Path, files: &[(&str, &[u8])]) -> PathBuf {
    let mut v = ade_fixtures::Volume::dd(0).named("Lost");
    for (name, body) in files {
        v.add_file(name, body);
    }
    let mut bytes = v.build();
    let root = 880usize * 512;
    for slot in 0..72usize {
        put_u32(&mut bytes, root + 24 + slot * 4, 0).unwrap();
    }
    let block = &mut bytes[root..root + 512];
    put_u32(block, 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(block, 20).unwrap();
    put_u32(block, 20, sum).unwrap();

    let path = dir.join("lost.adf");
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn orphaned_files_are_listed_with_how_far_they_are_believable() {
    let dir = scratch("list");
    let body: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let path = lost(&dir, &[("secret", &body)]);

    let (out, err, code) = run(&["carve", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr={err}");
    assert!(out.contains("secret"), "{out}");
    assert!(out.contains("self-evident"), "{out}");
    // The grading is explained every time, because the difference is the point.
    assert!(
        out.contains("names this header back and checksums"),
        "{out}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_disk_with_nothing_lost_exits_non_zero_like_grep() {
    let dir = scratch("nothing");
    let mut v = ade_fixtures::Volume::dd(0).named("Healthy");
    v.add_file("ordinary", b"still linked");
    let path = dir.join("ok.adf");
    fs::write(&path, v.build()).unwrap();

    let (out, _, code) = run(&["carve", path.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{out}");
    assert!(out.contains("0 orphaned headers"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn writing_out_never_writes_what_it_cannot_confirm() {
    // Header-only carves are not written at all. A file on disk with the right
    // name and unconfirmed bytes is worse than no file, because somebody will
    // believe it.
    let dir = scratch("write");
    let body: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let path = lost(&dir, &[("secret", &body)]);
    let out_dir = dir.join("out");

    let (out, _, _) = run(&[
        "carve",
        path.to_str().unwrap(),
        "--all",
        out_dir.to_str().unwrap(),
    ]);
    assert!(out.contains("written to"), "{out}");

    let names: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    // The block number is in the name, because two lost files can share one
    // and the block is what makes each answer unique.
    assert!(names[0].ends_with("-secret"), "{names:?}");
    assert!(
        names[0][..5].chars().all(|c| c.is_ascii_digit()),
        "{names:?}"
    );

    // And the bytes are the bytes.
    assert_eq!(fs::read(out_dir.join(&names[0])).unwrap(), body);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_partial_recovery_says_so_in_the_filename() {
    // Handing over a truncated file under its own name gives somebody
    // something that looks whole. `L2_MAP` on a real corpus disk claims 40,000
    // bytes and confirms 12,688.
    let dir = scratch("partial");
    let body: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
    let path = lost(&dir, &[("holey", &body)]);

    // Reuse one of the file's data blocks, as a later write would.
    let mut bytes = fs::read(&path).unwrap();
    let image = ade_core::Image::from_bytes(bytes.clone()).unwrap();
    let found = ade_core::carve::carve(&image);
    let victim = found
        .iter()
        .find(|c| c.name == "holey")
        .expect("the lost file")
        .blocks[1];
    ade_fixtures::corrupt::data_block_owner(&mut bytes, victim, 999);
    fs::write(&path, &bytes).unwrap();

    let out_dir = dir.join("out");
    let (_, _, code) = run(&[
        "carve",
        path.to_str().unwrap(),
        "--all",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(
        code,
        Some(1),
        "something was not written, so not a clean exit"
    );

    let names: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with(".partial"), "{names:?}");
    assert!(
        fs::metadata(out_dir.join(&names[0])).unwrap().len() < body.len() as u64,
        "and it really is short"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_carries_the_grading() {
    let dir = scratch("json");
    let body: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
    let path = lost(&dir, &[("secret", &body)]);

    let (out, _, _) = run(&["--format=json", "carve", path.to_str().unwrap()]);
    assert!(out.contains("\"evidence\":\"self-evident\""), "{out}");
    assert!(out.contains("\"name\":\"secret\""), "{out}");
    assert!(out.contains("\"data_blocks\":"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}
