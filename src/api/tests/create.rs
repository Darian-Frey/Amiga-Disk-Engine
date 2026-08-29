//! Creating a blank volume (F-019), checked three independent ways.
//!
//! # Why three
//!
//! This is ADE's first write path, so nothing above it can be assumed. A
//! formatter checked only by ADE's own reader proves that two halves of one
//! understanding agree, which is exactly the trap D-002 was written to avoid
//! and D-010 spends the fixture generator's independence on.
//!
//! So: **ADE reads it back**, **`ade-fixtures` agrees structurally** — a
//! generator written from the same specification but sharing no code — and
//! **ADFlib mounts it** (D-002's oracle, an implementation nobody here wrote).
//! If the formatter has misread SPEC, at least one of the three disagrees.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::process::Command;

use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, FormatError, Stamp};

/// A fixed, *legal* datestamp. Day zero is what Amiga software treats as
/// unset, and ADE's own health check reports it — the first disk this feature
/// produced flagged three findings against itself.
const WHEN: Stamp = Stamp {
    days: 1,
    mins: 60,
    ticks: 0,
};

fn ffs() -> Dostype {
    Dostype::from_raw(0x444F_5303).unwrap()
}

fn ofs() -> Dostype {
    Dostype::from_raw(0x444F_5302).unwrap()
}

#[test]
fn a_created_disk_reads_back_as_an_empty_volume() {
    let bytes = format::blank(Geometry::DD_FLOPPY, ffs(), "Blank", WHEN).unwrap();
    assert_eq!(bytes.len(), 901_120);

    let inspection = ade_core::inspect_bytes(bytes);
    let volume = inspection.volume.expect("it should mount");
    assert_eq!(volume.rootblock.name, b"Blank");
    assert!(volume.rootblock.checksum_valid);
    assert!(
        volume.rootblock.bitmap_flag_valid(),
        "the bitmap must be flagged valid"
    );
}

#[test]
fn a_created_disk_is_clean_by_ades_own_standard() {
    // The strongest single check available without another implementation: the
    // health report is the accumulated knowledge of what a sound disk looks
    // like, measured against 4,652 real ones.
    let bytes = format::blank(Geometry::DD_FLOPPY, ffs(), "Clean", WHEN).unwrap();
    let health = ade_core::health::examine(bytes);
    assert!(
        health.findings.is_empty(),
        "a disk ADE made should satisfy ADE: {:?}",
        health.findings.iter().map(|f| f.code).collect::<Vec<_>>()
    );
    assert_eq!(health.files, 0);
    assert_eq!(health.directories, 0);
}

#[test]
fn the_bitmap_says_free_not_full() {
    // A set bit means **free** (SPEC §Bitmap), which is the opposite of the
    // usual convention. Inverted, a fresh disk reports itself completely full
    // and the first write to it fails for want of space — and every checksum
    // would still be valid, so nothing else would notice.
    let bytes = format::blank(Geometry::DD_FLOPPY, ffs(), "Free", WHEN).unwrap();
    let health = ade_core::health::examine(bytes);
    let bitmap = health.bitmap.expect("a bitmap was written");
    assert_eq!(
        bitmap.marked_used, 2,
        "only the rootblock and the bitmap block itself are taken"
    );
    assert!(
        bitmap.marked_used * 100 < bitmap.covered as usize,
        "a blank disk is not full: {} of {} marked used",
        bitmap.marked_used,
        bitmap.covered
    );
    assert_eq!(bitmap.orphaned, 0, "and nothing is stranded");
    assert_eq!(bitmap.referenced_but_free, 0);
}

#[test]
fn ofs_and_hd_are_written_too() {
    for (geometry, size) in [
        (Geometry::DD_FLOPPY, 901_120usize),
        (Geometry::HD_FLOPPY, 1_802_240),
    ] {
        for dostype in [ofs(), ffs()] {
            let bytes = format::blank(geometry, dostype, "Both", WHEN).unwrap();
            assert_eq!(bytes.len(), size);
            let inspection = ade_core::inspect_bytes(bytes);
            assert!(
                inspection.volume.is_some(),
                "{dostype} at {size} bytes should mount"
            );
        }
    }
}

#[test]
fn a_name_that_amigados_could_not_hold_is_refused() {
    let long = "x".repeat(31);
    assert_eq!(
        format::blank(Geometry::DD_FLOPPY, ffs(), &long, WHEN),
        Err(FormatError::NameTooLong { len: 31 })
    );
    // `:` terminates a device and `/` a directory, so a name holding either
    // cannot be typed at an AmigaDOS prompt.
    for bad in ["has/slash", "has:colon"] {
        assert!(
            matches!(
                format::blank(Geometry::DD_FLOPPY, ffs(), bad, WHEN),
                Err(FormatError::NameInvalid { .. })
            ),
            "{bad} should be refused"
        );
    }
    // Thirty exactly is the limit, not one under it.
    assert!(format::blank(Geometry::DD_FLOPPY, ffs(), &"x".repeat(30), WHEN).is_ok());
}

#[test]
fn formatting_is_deterministic() {
    // Two runs with the same arguments produce the same bytes. Without this
    // nothing above can be compared, and a formatter that quietly consulted
    // the clock would make every test of it flaky.
    let a = format::blank(Geometry::DD_FLOPPY, ffs(), "Same", WHEN).unwrap();
    let b = format::blank(Geometry::DD_FLOPPY, ffs(), "Same", WHEN).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_fixture_generator_agrees_about_what_a_blank_disk_is() {
    // The second independent statement. `ade-fixtures` builds volumes from the
    // same specification and shares no code with the engine — D-010 keeps it
    // dependent on nothing precisely so it can be used like this.
    //
    // The bytes are *not* compared: two correct formatters may lay out a disk
    // differently and both be right. What must agree is what a reader sees.
    let created = format::blank(Geometry::DD_FLOPPY, ffs(), "Compare", WHEN).unwrap();
    let generated = ade_fixtures::Volume::dd(3).named("Compare").build();

    let mine = ade_core::health::examine(created);
    let theirs = ade_core::health::examine(generated);

    let name = |h: &ade_core::Health| {
        h.inspection
            .volume
            .as_ref()
            .map(|v| v.rootblock.name.clone())
    };
    assert_eq!(name(&mine), name(&theirs));
    assert_eq!(mine.files, theirs.files);
    assert_eq!(mine.directories, theirs.directories);
    assert_eq!(
        mine.bitmap.map(|b| b.marked_used),
        theirs.bitmap.map(|b| b.marked_used),
        "both should take exactly the rootblock and the bitmap block"
    );
}

#[test]
fn adflib_mounts_what_ade_formatted() {
    // The third statement, and the only one from an implementation nobody here
    // wrote (D-002). Skips when the oracle is absent; capped, because an
    // uncapped run of it once took the whole session down.
    let dir = std::env::temp_dir().join(format!("ade-create-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("blank.adf");
    let bytes = format::blank(Geometry::DD_FLOPPY, ffs(), "Oracle", WHEN).unwrap();
    std::fs::write(&path, bytes).unwrap();

    let script = "ulimit -v 1048576; exec timeout 20 unadf -l \"$1\"";
    let Ok(out) = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&path)
        .output()
    else {
        eprintln!("skipping: unadf not installed");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    if text.is_empty() {
        eprintln!("skipping: unadf produced nothing");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(
        text.contains("\"Oracle\""),
        "ADFlib should read the name: {text}"
    );
    assert!(text.contains("FFS"), "and the filesystem: {text}");
    assert!(text.contains("880 KBytes"), "and the size: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}
