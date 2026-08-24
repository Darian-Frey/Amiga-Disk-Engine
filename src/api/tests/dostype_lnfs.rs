//! LNFS (`DOS\6`, `DOS\7`) identification — the trap ADFlib fell into.
//!
//! `DOS\6` is `0b110` and `DOS\7` is `0b111`. Decode those by bit pattern and
//! they read as INTL-plus-dircache, because LNFS took the two combinations the
//! classic flag encoding left spare. They are dostypes, not bit patterns
//! (BUG-001).
//!
//! ADFlib decodes by bit pattern and so reports an LNFS volume as
//! `FFS INTL DIRCACHE`. Asked to use the cache, it hunts for blocks that were
//! never there, prints an empty listing and **exits 0** — a caller scripting
//! against the exit code concludes the disk is empty
//! (SPEC §The oracle cannot check LNFS).
//!
//! These tests exist because the natural way to write `has_dircache` is the
//! way ADFlib wrote it, and because LNFS has no oracle to catch a regression:
//! it is absent from the corpus and unimplemented in the reference. This file
//! is the only thing holding the ordering in place.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests over data they construct"
)]

use ade_core::layers::filesystem::dostype::{Dostype, FileSystem, Mode};
use ade_core::{Image, examine};
use ade_fixtures::Volume as Fixture;

/// Both LNFS dostypes, with the filesystem each implies.
const LNFS: [(u8, FileSystem); 2] = [(6, FileSystem::Ofs), (7, FileSystem::Ffs)];

#[test]
fn lnfs_is_not_dircache() {
    // The whole point. Bit 2 is set on both, and testing it first is wrong.
    for (flags, _) in LNFS {
        let dostype = Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap();

        assert_eq!(dostype.mode(), Mode::LongNames, "DOS\\{flags}");
        assert!(
            !dostype.has_dircache(),
            "DOS\\{flags} has no directory cache — ADFlib's mistake"
        );
    }
}

#[test]
fn lnfs_is_international_and_so_are_the_cached_dostypes() {
    // C-006, and the two halves of it are different. LNFS carries the INTL bit
    // as well (6 is 0b110, 7 is 0b111), so reading the bit gets the *hash*
    // right by luck. `DOS\4` and `DOS\5` are where the bit alone fails: both
    // leave it clear and both are international, so deciding from it breaks
    // lookup silently — as "not found" rather than as an error.
    for flags in [6u8, 7] {
        let dostype = Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap();
        assert!(dostype.is_international(), "DOS\\{flags}");
        assert!(dostype.intl_flag_set(), "DOS\\{flags} does carry the bit");
    }
    for flags in [4u8, 5] {
        let dostype = Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap();
        assert!(dostype.is_international(), "DOS\\{flags}");
        assert!(
            !dostype.intl_flag_set(),
            "DOS\\{flags} is international with the bit clear — the real trap"
        );
    }
}

#[test]
fn the_filesystem_bit_still_reads_normally() {
    // Only bit 0 distinguishes the two, exactly as elsewhere.
    for (flags, filesystem) in LNFS {
        let dostype = Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap();
        assert_eq!(dostype.filesystem(), filesystem, "DOS\\{flags}");
    }
}

#[test]
fn a_volume_is_not_searched_for_a_cache_it_cannot_have() {
    // The behavioural half: ADFlib went looking and found garbage. ADE must
    // not look at all, because on a real LNFS disk the `extension` field is
    // `DirList` and reading it as a cache pointer walks whatever is there.
    for (flags, _) in LNFS {
        let mut v = Fixture::dd(flags).named("LongNames");
        v.add_file("startup", b"hello");
        v.add_dir("Tools");
        let bytes = v.build();

        let path =
            std::env::temp_dir().join(format!("ade-lnfs-{flags}-{}.adf", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let image = Image::open(&path).unwrap();
        let volume = image.volume().unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(
            !volume.has_dircache(),
            "DOS\\{flags} must not be treated as a cached volume"
        );
        let chain = volume.dircache(volume.root()).unwrap();
        assert!(
            chain.blocks.is_empty() && chain.faults.is_empty(),
            "DOS\\{flags}: nothing should have been read: {chain:?}"
        );

        // And the listing must not come back empty, which is the failure mode
        // that matters to a caller.
        let health = examine(bytes);
        assert_eq!(health.files, 1, "DOS\\{flags}");
        assert_eq!(health.directories, 1, "DOS\\{flags}");
        assert!(
            health.dircache.is_none(),
            "no cache cross-check belongs on an LNFS volume"
        );

        // D-013: the volume reads, but it must say the names are unreliable.
        // A classic read of a long-name block still checksums, so nothing
        // downstream would otherwise notice.
        let codes: Vec<&str> = health.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, ["lnfs-unsupported"], "DOS\\{flags}");
    }
}

#[test]
fn every_dostype_lands_in_exactly_one_mode() {
    // A table rather than a chain of ifs, so a future flag cannot quietly
    // change how an existing one is classified.
    let expected = [
        (0u8, Mode::Classic),
        (1, Mode::Classic),
        (2, Mode::Classic),
        (3, Mode::Classic),
        (4, Mode::DirCache),
        (5, Mode::DirCache),
        (6, Mode::LongNames),
        (7, Mode::LongNames),
    ];

    for (flags, mode) in expected {
        let dostype = Dostype::parse(&[b'D', b'O', b'S', flags], 0).unwrap();
        assert_eq!(dostype.mode(), mode, "DOS\\{flags}");
        // International for everything except plain OFS and plain FFS.
        assert_eq!(
            dostype.is_international(),
            flags >= 2,
            "DOS\\{flags} internationality"
        );
    }
}

#[test]
fn a_classic_volume_carries_no_lnfs_warning() {
    // The warning must be tied to the dostype, not sprayed at everything: a
    // warning that appears on sound disks is one people learn to ignore.
    for flags in [0u8, 1, 2, 3, 4, 5] {
        let mut v = Fixture::dd(flags).named("Ordinary");
        v.add_file("startup", b"hello");
        let health = examine(v.build());

        assert!(
            !health.findings.iter().any(|f| f.code == "lnfs-unsupported"),
            "DOS\\{flags} is not LNFS: {:?}",
            health.findings
        );
    }
}
