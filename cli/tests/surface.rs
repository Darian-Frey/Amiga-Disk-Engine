//! `ade surface` at the command line (F-029).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "integration tests: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

// Through `ade-endian`, because C-001 is a clippy tripwire and not a
// convention: raw `to_be_bytes` fails the build outside that crate, and it
// caught this test rather than a shipping path — the guard doing its job.
use ade_core::layers::endian::{put_u16, put_u32};

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
    let dir = std::env::temp_dir().join(format!("ade-surface-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A plain disk wrapped as an extended ADF carrying `tracks` ordinary tracks.
fn extended(tracks: usize) -> Vec<u8> {
    let mut v = ade_fixtures::Volume::dd(1).named("Surface");
    v.add_file("readme", b"raw tracks");
    let plain = v.build();

    let mut out = vec![0u8; 12];
    out[..8].copy_from_slice(b"UAE-1ADF");
    put_u16(&mut out, 10, u16::try_from(tracks).unwrap()).unwrap();
    for _ in 0..tracks {
        let at = out.len();
        out.extend_from_slice(&[0u8; 12]);
        put_u32(&mut out, at + 4, 11 * 512).unwrap();
        put_u32(&mut out, at + 8, 11 * 512 * 8).unwrap();
    }
    out.extend_from_slice(&plain[..tracks * 11 * 512]);
    out
}

#[test]
fn a_plain_adf_says_it_has_no_surface_rather_than_showing_a_full_one() {
    // The distinction the whole command rests on. A plain ADF is already
    // sectors: every one is present by construction and nothing recorded how
    // it was read. Drawing 160 whole tracks would claim a measurement nobody
    // made.
    let dir = scratch("plain");
    let path = dir.join("plain.adf");
    fs::write(&path, ade_fixtures::Volume::dd(1).named("Plain").build()).unwrap();

    let (out, err, code) = run(&["surface", path.to_str().unwrap()]);
    assert_eq!(code, Some(4), "stdout={out} stderr={err}");
    assert!(err.contains("no track-level information"), "{err}");
    assert!(
        err.contains("extended ADF or a flux capture"),
        "and what does: {err}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_raw_track_container_draws_two_rows_of_eighty() {
    // Two rows because that is the shape of the medium: the same cylinder on
    // side 0 and side 1 are different tracks and fail independently.
    let dir = scratch("raw");
    let path = dir.join("raw.adf");
    fs::write(&path, extended(160)).unwrap();

    let (out, err, code) = run(&["surface", path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("1760 of 1760 sectors recovered (100%)"),
        "{out}"
    );

    let rows: Vec<&str> = out.lines().filter(|l| l.starts_with("  head ")).collect();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let cells = row
            .trim_start_matches("  head 0   ")
            .trim_start_matches("  head 1   ");
        assert_eq!(cells.len(), 80, "one cell per cylinder: {row}");
        assert!(cells.chars().all(|c| c == '#'), "all whole: {row}");
    }

    // The ruler is the same width as the disk. Written by hand it was 100
    // columns for 80 cylinders, which invites counting to the wrong track.
    let units = out
        .lines()
        .find(|l| l.trim_start().starts_with("0123456789"))
        .expect("a ruler");
    assert_eq!(units.trim_start().len(), 80);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_track_the_container_never_mentioned_is_shown_as_absent() {
    // "Nothing was recovered here" and "nobody looked here" are the same
    // picture otherwise, and only one of them is a fact about the disk.
    let dir = scratch("short");
    let path = dir.join("short.adf");
    fs::write(&path, extended(40)).unwrap();

    let (out, _, code) = run(&["surface", path.to_str().unwrap()]);
    assert_eq!(code, Some(0));
    assert!(out.contains("440 of 1760"), "{out}");

    let row = out
        .lines()
        .find(|l| l.starts_with("  head 0"))
        .expect("a row");
    let cells: Vec<char> = row.chars().skip("  head 0   ".len()).collect();
    assert_eq!(cells[0], '#', "the first cylinder is whole");
    assert_eq!(cells[79], '.', "and the last was never in the container");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_carries_every_track_including_the_absent_ones() {
    let dir = scratch("json");
    let path = dir.join("raw.adf");
    fs::write(&path, extended(40)).unwrap();

    let (out, _, _) = run(&["--format=json", "surface", path.to_str().unwrap()]);
    assert_eq!(
        out.matches("\"track\"").count(),
        160,
        "one per track, always"
    );
    assert!(out.contains("\"source\":\"absent\""), "{out}");
    assert!(out.contains("\"source\":\"sectors\""), "{out}");
    assert!(out.contains("\"sectors_placed\":440"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}
