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

/// The six AmigaDOS types ADE writes, as `(name, flags)`.
///
/// `DOS\6` and `DOS\7` are absent on purpose: LNFS is deferred by D-013 on
/// verifiability. Writing one would produce a format neither ADFlib nor this
/// corpus can check, which is what D-002 gave up ADFlib's knowledge to avoid.
const TYPES: [(&str, u8); 6] = [
    ("ofs", 0),
    ("ffs", 1),
    ("ofs-intl", 2),
    ("ffs-intl", 3),
    ("ofs-dc", 4),
    ("ffs-dc", 5),
];

fn dostype(flags: u8) -> Dostype {
    Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap()
}

#[test]
fn every_written_dostype_reads_back_as_itself() {
    // The whole matrix, because the flags byte is the only thing that differs
    // and getting it wrong is invisible until something hashes a name: `DOS\4`
    // and `DOS\5` carry the dircache bit with INTL *clear* yet are
    // international (C-006, BUG-001).
    for (name, flags) in TYPES {
        let image = format::blank(Geometry::DD_FLOPPY, dostype(flags), "Types", WHEN).unwrap();
        let health = ade_core::health::examine(image);
        let seen = health
            .inspection
            .bootblock
            .as_ref()
            .unwrap()
            .dostype
            .expect("a written dostype parses");

        assert_eq!(seen.raw() & 0xFF, u32::from(flags), "{name}");
        assert!(health.is_sound(), "{name}: {:?}", health.findings);
        // International whenever the INTL bit or the dircache bit is set —
        // never by reading bit 1 alone.
        assert_eq!(seen.is_international(), flags >= 2, "{name} international");
        assert_eq!(seen.has_dircache(), flags >= 4, "{name} dircache");
    }
}

#[test]
fn a_blank_dircache_volume_needs_no_cache_block() {
    // SPEC records the rootblock's `extension` as "first dircache block, else
    // 0", and a fresh volume has no entries to cache. Left unchecked this
    // would be an assumption; ADFlib mounting the result is what settles it,
    // and that is asserted in `adflib_mounts_every_type_it_knows` below.
    let image = format::blank(Geometry::DD_FLOPPY, dostype(5), "Cache", WHEN).unwrap();
    let health = ade_core::health::examine(image);
    assert!(health.is_sound(), "{:?}", health.findings);
    assert_eq!(health.files, 0);
    assert_eq!(health.directories, 0);
}

#[test]
fn a_five_and_a_quarter_inch_disk_is_the_formula_at_a_smaller_size() {
    // 40 cylinders, which the A1020 gave as 440 KB through trackdisk.device.
    // No corpus image is one and ADFlib does not know the size — it refuses
    // the *device* with "unknown device type", before reaching any
    // filesystem — so this leans on the two checks that are available: ADE
    // reads it back, and the fixture generator agrees.
    let geometry = Geometry::new(40, 2, 11, 512, 2).unwrap();
    assert_eq!(geometry.total_blocks(), 880);
    assert_eq!(geometry.total_bytes(), 450_560, "440 KB exactly");
    assert_eq!(geometry.root_block().0, 440, "(2 + 879) / 2");

    let created = format::blank(geometry, ffs(), "Small", WHEN).unwrap();
    let generated = ade_fixtures::Volume::new(40, 2, 11, 3)
        .named("Small")
        .build();

    let mine = ade_core::health::examine(created);
    let theirs = ade_core::health::examine(generated);
    assert!(mine.is_sound(), "{:?}", mine.findings);
    assert_eq!(
        mine.inspection
            .volume
            .as_ref()
            .map(|v| v.rootblock.name.clone()),
        theirs
            .inspection
            .volume
            .as_ref()
            .map(|v| v.rootblock.name.clone())
    );
    assert_eq!(
        mine.bitmap.map(|b| b.marked_used),
        theirs.bitmap.map(|b| b.marked_used)
    );
}

#[test]
fn a_hard_disk_gets_as_many_bitmap_blocks_as_it_needs() {
    // BUG-006 in the other direction: one bitmap block maps 4064 blocks, so a
    // volume above about 2 MB needs a second and an 8 MB one needs five. A
    // volume that names one of its five describes an eighth of its own free
    // space, and every block past that reads as allocated.
    for (megabytes, expected) in [(1u32, 1u64), (2, 2), (4, 3), (8, 5), (32, 17)] {
        let geometry = Geometry::new(megabytes * 64, 1, 32, 512, 2).unwrap();
        let image = format::blank(geometry, ffs(), "Big", WHEN).unwrap();

        let mounted = ade_core::Image::from_bytes(image).unwrap();
        let map = ade_core::layout::Layout::of(&mounted);
        let pages: u64 = map
            .spans
            .iter()
            .filter(|s| s.region == ade_core::layout::Region::Bitmap)
            .map(|s| s.blocks)
            .sum();
        assert_eq!(pages, expected, "{megabytes} MB");

        let health = ade_core::health::examine(mounted.read_range(0, geometry.total_bytes()));
        assert!(health.is_sound(), "{megabytes} MB: {:?}", health.findings);
    }
}

#[test]
fn a_volume_needing_a_bitmap_extension_chain_is_refused_not_half_written() {
    // Past 25 bitmap pointers the rest belong in a `bm_ext` chain, which ADE
    // does not write. Refusing is the only safe answer: a volume whose bitmap
    // is half described reports free blocks that are not, and that is how a
    // write path destroys data.
    // The boundary is exact rather than "roughly 50 MB": 25 pages of 4064
    // blocks each is 101,600 mapped, plus 2 reserved, which at 2048 blocks to
    // the megabyte is 49 MB fitting and 50 not.
    let ok = Geometry::new(49 * 64, 1, 32, 512, 2).unwrap();
    assert!(format::blank(ok, ffs(), "Fits", WHEN).is_ok(), "49 MB fits");

    let over = Geometry::new(50 * 64, 1, 32, 512, 2).unwrap();
    assert!(
        matches!(
            format::blank(over, ffs(), "Over", WHEN),
            Err(FormatError::TooLarge { .. })
        ),
        "50 MB is one page past what a rootblock can name"
    );

    let too_big = Geometry::new(64 * 64, 1, 32, 512, 2).unwrap();
    assert!(matches!(
        format::blank(too_big, ffs(), "TooBig", WHEN),
        Err(FormatError::TooLarge { .. })
    ));
}

#[test]
fn adflib_mounts_every_type_and_size_it_knows() {
    // The oracle across the whole matrix, not just FFS DD. This is what
    // settles the dircache question that `a_blank_dircache_volume_needs_no_
    // cache_block` leaves open: `DOS\4` and `DOS\5` are written with a zero
    // dircache pointer, and an implementation nobody here wrote mounts them.
    //
    // 5.25" is deliberately absent. ADFlib refuses that size outright —
    // "adfMountDev : unknown device type", before it looks at any filesystem —
    // so it can say nothing about those bytes either way. That is a gap in the
    // oracle, not a pass.
    let dir = std::env::temp_dir().join(format!("ade-create-matrix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut checked = 0;
    for (name, flags) in TYPES {
        for (label, geometry, size) in [
            ("dd", Geometry::DD_FLOPPY, "880 KBytes"),
            ("hd", Geometry::HD_FLOPPY, "1760 KBytes"),
        ] {
            let path = dir.join(format!("{name}-{label}.adf"));
            let bytes = format::blank(geometry, dostype(flags), "Matrix", WHEN).unwrap();
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
                text.contains("\"Matrix\""),
                "{name} {label}: ADFlib should read the name: {text}"
            );
            assert!(text.contains(size), "{name} {label}: size: {text}");
            checked += 1;
        }
    }
    assert_eq!(checked, 12, "six types across two floppy sizes");

    // And a hard disk, where the bitmap runs to five blocks. ADFlib reports it
    // as a hardfile rather than a floppy, which is what it is.
    let path = dir.join("big.hdf");
    let geometry = Geometry::new(8 * 64, 1, 32, 512, 2).unwrap();
    std::fs::write(
        &path,
        format::blank(geometry, ffs(), "BigDisk", WHEN).unwrap(),
    )
    .unwrap();
    let script = "ulimit -v 1048576; exec timeout 20 unadf -l \"$1\"";
    if let Ok(out) = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&path)
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.is_empty() {
            assert!(text.contains("Hardfile"), "read as a hard disk: {text}");
            assert!(text.contains("8192.0 KBytes"), "at its full size: {text}");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}
