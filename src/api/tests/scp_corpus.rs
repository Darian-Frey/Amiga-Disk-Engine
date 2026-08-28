//! SCP against the corpus: real disks, encoded to flux and read back.
//!
//! Companion to `scp_oracle.rs`, which uses one generated fixture. The
//! difference is the same one D-010 draws everywhere else: a fixture checks
//! conformance to the specification, and real disks check conformance to
//! reality — filenames, layouts and edge cases nobody thought to generate.
//!
//! Skips without `gw` or without `disks/`. Three disks by default, because
//! encoding one takes about nine seconds and produces 30 MB; set
//! `ADE_SCP_DISKS` higher for a deeper sweep, as `ADE_FUZZ_ITERS` does for the
//! fuzz harness.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::{fs, path::PathBuf, process::Command};

use ade_core::assemble::assemble_scp;
use ade_flux::scp::Scp;

/// Disks to sweep unless `ADE_SCP_DISKS` says otherwise.
const DEFAULT_DISKS: usize = 3;

fn corpus() -> Option<Vec<PathBuf>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("disks");
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "adf"))
        // Only standard double-density images: `gw` is being asked to encode
        // an AmigaDOS floppy, and an 81-cylinder image is not one.
        .filter(|p| fs::metadata(p).is_ok_and(|m| m.len() == 901_120))
        .collect();
    if found.is_empty() {
        return None;
    }
    found.sort();
    Some(found)
}

#[test]
fn real_disks_survive_the_round_trip_through_flux() {
    if Command::new("gw").arg("--version").output().is_err() {
        eprintln!("skipping: gw not installed");
        return;
    }
    let Some(disks) = corpus() else {
        eprintln!("skipping: no corpus in disks/");
        return;
    };
    let want: usize = std::env::var("ADE_SCP_DISKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_DISKS);

    let dir = std::env::temp_dir().join(format!("ade-scp-corpus-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let scp = dir.join("disk.scp");

    // Spread across the corpus rather than taking the first N, which would be
    // whatever sorts first — all the same publisher, often the same year.
    let step = (disks.len() / want.max(1)).max(1);
    let mut checked = 0usize;

    for adf in disks.iter().step_by(step).take(want) {
        let _ = fs::remove_file(&scp);
        let encoded = Command::new("gw")
            .args(["convert", "--format=amiga.amigados"])
            .arg(adf)
            .arg(&scp)
            .output()
            .is_ok_and(|o| o.status.success());
        if !encoded {
            eprintln!("skipping {}: gw would not encode it", adf.display());
            continue;
        }

        let original = fs::read(adf).unwrap();
        let bytes = fs::read(&scp).unwrap();
        let parsed = Scp::parse(&bytes).expect("gw's output must parse");
        let assembly = assemble_scp(&parsed, &bytes);

        assert_eq!(
            assembly.bytes,
            original,
            "{} did not survive ADF -> SCP -> ADF ({} of {} sectors recovered)",
            adf.display(),
            assembly.sectors_placed,
            assembly.sectors_total
        );
        checked += 1;
    }

    let _ = fs::remove_dir_all(&dir);
    assert!(
        checked > 0,
        "no disk could be encoded; the sweep proved nothing"
    );
    eprintln!("{checked} disks round-tripped byte-identically through SCP");
}
