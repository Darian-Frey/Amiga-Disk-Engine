//! SCP reading, checked against Greaseweazle (D-002's oracle rule, applied to
//! a second format).
//!
//! # Why this is an oracle and not a self-test
//!
//! ADE does not generate the SCP files it reads here. `gw` does — an
//! independent implementation, written by people who have never seen this
//! code, encoding a sector image into flux the way its own authors believe
//! flux should look. If ADE's reading of the format were wrong, the two would
//! disagree; that they agree byte for byte over a whole disk is a statement
//! about the format rather than about this implementation.
//!
//! That answers what blocked SCP for weeks. The entry said "no material", and
//! the material was one command away the whole time: every sector image is a
//! potential capture.
//!
//! # What it cannot check
//!
//! Everything flux exists for. `gw` encodes an ordinary AmigaDOS disk, so
//! these files hold no weak bits, no long tracks and no deliberate illegality
//! — the protections a real capture is made to preserve. What passes here is
//! "ADE reads a clean capture correctly", which is necessary and not
//! sufficient. Only a real protected disk closes that gap, and that needs the
//! hardware F-006 is waiting on.
//!
//! Skips when `gw` is absent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::{fs, path::Path, process::Command};

use ade_core::assemble::assemble_scp;
use ade_flux::scp::Scp;

fn have_gw() -> bool {
    Command::new("gw").arg("--version").output().is_ok()
}

/// Encode a sector image as SCP flux with the oracle.
fn encode(adf: &Path, scp: &Path) -> bool {
    Command::new("gw")
        .args(["convert", "--format=amiga.amigados"])
        .arg(adf)
        .arg(scp)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A generated fixture image, since D-010 commits none.
fn fixture(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fixture.adf");
    let mut volume = ade_fixtures::Volume::dd(1).named("SCPTEST");
    volume.add_file("startup", b"hello from a generated fixture\n");
    volume.add_dir("Tools");
    fs::write(&path, volume.build()).unwrap();
    path
}

#[test]
fn a_generated_capture_decodes_back_to_the_image_it_came_from() {
    if !have_gw() {
        eprintln!("skipping: gw not installed");
        return;
    }
    let dir = tempdir("roundtrip");
    let adf = fixture(&dir);
    let scp = dir.join("fixture.scp");
    if !encode(&adf, &scp) {
        eprintln!("skipping: gw could not encode the fixture");
        return;
    }

    let original = fs::read(&adf).unwrap();
    let bytes = fs::read(&scp).unwrap();
    let parsed = Scp::parse(&bytes).expect("the oracle's own output must parse");
    let assembly = assemble_scp(&parsed, &bytes);

    assert_eq!(
        assembly.sectors_placed, assembly.sectors_total,
        "every sector of a clean capture should decode"
    );
    assert_eq!(
        assembly.bytes, original,
        "a decoded capture must be the image it was made from, byte for byte"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_header_the_oracle_writes_is_the_header_the_spec_describes() {
    if !have_gw() {
        eprintln!("skipping: gw not installed");
        return;
    }
    let dir = tempdir("header");
    let adf = fixture(&dir);
    let scp = dir.join("fixture.scp");
    if !encode(&adf, &scp) {
        eprintln!("skipping: gw could not encode the fixture");
        return;
    }
    let bytes = fs::read(&scp).unwrap();
    let parsed = Scp::parse(&bytes).unwrap();

    assert_eq!(parsed.tracks.len(), 160, "80 cylinders, two heads");
    assert!(parsed.revolutions >= 1);
    assert_eq!(parsed.tick_ns(), 25);
    assert!(parsed.index_aligned());
    // 200 ms per revolution is 300 RPM, and 8,000,000 ticks of 25 ns is 200 ms.
    let first = &parsed.tracks[0].revolutions[0];
    let ms = first.duration_ticks / 40_000;
    assert!(
        (195..=205).contains(&ms),
        "a revolution should take about 200 ms, not {ms}"
    );
    // The disk-type byte is *not* usable for detection: the oracle writes
    // "other" for an Amiga disk it has just encoded as AmigaDOS MFM.
    assert_ne!(
        parsed.disk_type, 0x04,
        "if this ever becomes 0x04, the note in Scp::disk_type should change"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_capture_reads_as_a_browsable_volume() {
    if !have_gw() {
        eprintln!("skipping: gw not installed");
        return;
    }
    let dir = tempdir("browse");
    let adf = fixture(&dir);
    let scp = dir.join("fixture.scp");
    if !encode(&adf, &scp) {
        eprintln!("skipping: gw could not encode the fixture");
        return;
    }

    // The whole point of F-007 for flux: a 30 MB pile of timings opens as a
    // disk, not as a container someone else has to convert first.
    let inspection = ade_core::inspect_path(&scp).expect("a capture should open");
    assert!(
        inspection.volume.is_some(),
        "a clean capture should mount: {:?}",
        inspection.volume_absent
    );
    assert!(
        inspection.assembly.is_some(),
        "and must say it is a reconstruction"
    );
    assert!(inspection.flux.is_some(), "and how it was captured");
    let _ = fs::remove_dir_all(&dir);
}

/// A scratch directory of this test's own, since the workspace has no temp-dir
/// crate and will not gain one for a test.
///
/// `tag` is not decoration. These tests run **concurrently in one process**,
/// so a directory named only for the process is shared, and each test removing
/// it at the end deletes the fixtures the others are still using. That failed
/// roughly one run in three — a flake that looks like an intermittent bug in
/// the code under test and is nothing of the kind.
fn tempdir(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("ade-scp-{}-{tag}", std::process::id()));
    fs::create_dir_all(&base).unwrap();
    base
}
