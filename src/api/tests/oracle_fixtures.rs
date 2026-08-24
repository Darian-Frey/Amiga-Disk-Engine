//! The D-002 oracle over **generated** fixtures.
//!
//! Companion to `oracle.rs`, which runs ADFlib over the real corpus. This one
//! needs no corpus at all: it generates volumes with `ade-fixtures` and checks
//! that ADFlib reads them the same way ADE does.
//!
//! # Why this closes a loop I thought was open
//!
//! D-010 chose to generate fixtures rather than commit images, and its original
//! text worried that a generator written by the same hand as the parser would
//! encode the same misreading, with both agreeing. That worry assumed the
//! oracle could only be pointed at real disks.
//!
//! It cannot be — ADFlib reads any structurally valid volume regardless of who
//! wrote it, so an independent implementation can check the generator *and* the
//! parser without a single real image. See D-010's 2026-08-24 amendment.
//!
//! What this still cannot catch is what the *specification omits*: a fixture is
//! only ever as good as SPEC. Reality is the corpus's job.
//!
//! Skips when `unadf` is absent. Every invocation is resource-capped, because
//! an uncapped one once allocated 29 GB and killed the session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::{fs, process::Command};

use ade_core::Image;
use ade_fixtures::Volume as Fixture;

const MEM_KIB: u64 = 1_048_576;
const TIMEOUT_S: u64 = 20;

fn have_unadf() -> bool {
    Command::new("unadf").output().is_ok()
}

/// Run `unadf -lr` under hard caps, returning its listing.
fn oracle_list(image: &std::path::Path) -> Option<String> {
    let script = format!("ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf -lr \"$1\"");
    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("sh")
        .arg(image)
        .output()
        .ok()?;
    out.status.code().filter(|c| *c == 0)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn write_temp(bytes: &[u8], name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("ade-fixoracle-{name}-{}.adf", std::process::id()));
    fs::write(&p, bytes).expect("write fixture");
    p
}

#[test]
fn adflib_reads_our_generated_volumes() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping (apt install unadf)");
        return;
    }

    // Geometry, filesystem, and the dostype matrix. `DOS\4`..`DOS\7` are
    // generated with the classic layout — real dircache and LNFS structures are
    // Phase 2 — so what is checked here is identification and hashing, which is
    // where C-006 lives.
    let cases: Vec<(&str, Vec<u8>)> = (0u8..8)
        .map(|d| {
            let mut v = Fixture::dd(d).named("Matrix");
            // An accented name: international folding is the *only* difference
            // between the two hash functions, so this is the case that tells
            // them apart.
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_file("plain", b"ascii");
            v.add_dir("Tools");
            (
                match d {
                    0 => "dos0",
                    1 => "dos1",
                    2 => "dos2",
                    3 => "dos3",
                    4 => "dos4",
                    5 => "dos5",
                    6 => "dos6",
                    _ => "dos7",
                },
                v.build(),
            )
        })
        .chain(std::iter::once({
            let mut v = Fixture::hd(1).named("HighDensity");
            v.add_file("readme", b"high density");
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_dir("Tools");
            ("hd", v.build())
        }))
        .chain(std::iter::once({
            // An 8 MB hardfile: a raw volume with no floppy geometry, and five
            // bitmap blocks rather than one (BUG-006).
            let mut v = Fixture::new(512, 1, 32, 1).named("Hardfile");
            v.add_file("readme", b"a hardfile, not a floppy");
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_dir("Tools");
            ("hardfile", v.build())
        }))
        .collect();

    let mut checked = 0usize;
    for (name, bytes) in cases {
        let path = write_temp(&bytes, name);
        let listing = oracle_list(&path);
        let ours = Image::open(&path).ok().and_then(|i| {
            i.volume()
                .ok()
                .and_then(|v| v.walk(v.root()).ok())
                .map(|w| w.entries.len())
        });
        let _ = fs::remove_file(&path);

        let listing =
            listing.unwrap_or_else(|| panic!("{name}: ADFlib refused a volume we generated"));
        let ours =
            ours.unwrap_or_else(|| panic!("{name}: ADE could not read a volume we generated"));

        // ADFlib prints a banner, a Device line and a Volume line, then one
        // line per entry. Counting by exclusion is sturdier than matching the
        // date format, which is how the first version of this got it wrong.
        let theirs = listing
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("unADF")
                    && !t.starts_with("Device")
                    && !t.starts_with("Volume")
                    && !t.starts_with("Warning")
            })
            .count();
        assert_eq!(
            theirs, ours,
            "{name}: ADFlib found {theirs} entries, ADE found {ours}\n{listing}"
        );
        // The accented name must survive both readers, which it only does if
        // the international hash was chosen correctly (C-006).
        assert!(
            listing.contains("pfel"),
            "{name}: the accented entry did not survive ADFlib\n{listing}"
        );
        checked += 1;
    }
    eprintln!("ADFlib agreed with ADE on {checked} generated volumes");
    assert_eq!(checked, 10);
}

#[test]
fn adflib_and_ade_agree_on_generated_file_contents() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping");
        return;
    }
    // Multi-block files, exercising the reversed data_blocks[] table and the
    // OFS/FFS payload difference (C-005) against an independent reader.
    for (label, dostype) in [("ofs", 0u8), ("ffs", 1)] {
        let payload: Vec<u8> = (0..9000u32).map(|i| (i % 251) as u8).collect();
        let mut v = Fixture::dd(dostype).named("Contents");
        v.add_file("data.bin", &payload);
        let path = write_temp(&v.build(), label);

        let dir =
            std::env::temp_dir().join(format!("ade-fixoracle-out-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script =
            format!("ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf \"$1\" -d \"$2\"");
        let ok = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .arg("sh")
            .arg(&path)
            .arg(&dir)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "{label}: ADFlib could not extract from our fixture");

        let theirs = fs::read(dir.join("data.bin")).expect("ADFlib output");
        let ours = {
            let img = Image::open(&path).unwrap();
            let vol = img.volume().unwrap();
            let e = vol.lookup("data.bin").unwrap();
            vol.read_file(&e).unwrap()
        };
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(ours.bytes, payload, "{label}: ADE did not round-trip");
        assert_eq!(theirs, payload, "{label}: ADFlib did not round-trip");
    }
}
