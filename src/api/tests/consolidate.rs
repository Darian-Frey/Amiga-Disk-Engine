//! Consolidating and diffing dumps of the same disk (Phase 4, F-008 and F-009).
//!
//! # What these tests are careful about
//!
//! F-008's wording is "merge N reads of the same disk into a best-estimate
//! image", which assumes the reads came from one physical disk so a
//! disagreement means a read failed. **The available material is not that.**
//! The corpus's multi-dump titles are independent dumps of possibly different
//! copies — several are TOSEC-tagged `[m files moved]` or `[m startup-sequence]`,
//! which are deliberate *edits*.
//!
//! So the tests pin agreement, not correctness, and they pin the honest edges:
//! two dumps cannot vote, and a tie is reported rather than silently broken.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::consolidate::ConsolidateError;
use ade_core::{consolidate, diff};

/// A disk-sized image filled from a seed, so differences are locatable.
fn image(seed: u8) -> Vec<u8> {
    (0..901_120usize)
        .map(|i| (i as u8).wrapping_add(seed))
        .collect()
}

/// Overwrite one sector with a constant.
fn damage(image: &mut [u8], sector: usize, fill: u8) {
    let at = sector * 512;
    image[at..at + 512].fill(fill);
}

#[test]
fn identical_dumps_agree_on_everything() {
    let a = image(0);
    let report = consolidate(&[a.clone(), a.clone(), a.clone()]).unwrap();

    assert!(report.is_unanimous());
    assert_eq!(report.unanimous_sectors, 1760);
    assert_eq!(report.resolved_sectors, 0);
    assert_eq!(report.unresolved_sectors, 0);
    assert!(report.tracks.is_empty(), "no track should be listed");
    assert_eq!(report.bytes, a);
}

#[test]
fn a_plurality_wins_and_is_reported_as_resolved() {
    // Two dumps agree, one differs: the majority version survives into the
    // merge and the sector is reported as resolved rather than as agreed.
    let good = image(0);
    let mut odd = good.clone();
    damage(&mut odd, 100, 0xFF);

    let report = consolidate(&[good.clone(), good.clone(), odd]).unwrap();

    assert_eq!(report.resolved_sectors, 1);
    assert_eq!(report.unresolved_sectors, 0);
    assert_eq!(report.unanimous_sectors, 1759);
    assert_eq!(report.bytes, good, "the majority version wins");

    assert_eq!(report.tracks.len(), 1);
    assert_eq!(report.tracks[0].track, 100 / 11);
    assert_eq!(report.tracks[0].disputed, vec![100 % 11]);
    assert!(report.tracks[0].unresolved.is_empty());
}

#[test]
fn two_dumps_cannot_vote() {
    // The honest edge. With two sources every disagreement is a tie by
    // definition, and saying "resolved" would be arithmetic dressed up as
    // judgement.
    let a = image(0);
    let mut b = a.clone();
    damage(&mut b, 500, 0xAA);

    let report = consolidate(&[a, b]).unwrap();

    assert_eq!(report.resolved_sectors, 0, "two dumps resolve nothing");
    assert_eq!(report.unresolved_sectors, 1);
    assert_eq!(report.tracks[0].unresolved, vec![500 % 11]);
}

#[test]
fn a_three_way_tie_is_unresolved() {
    let a = image(0);
    let mut b = a.clone();
    let mut c = a.clone();
    damage(&mut b, 7, 0x11);
    damage(&mut c, 7, 0x22);

    let report = consolidate(&[a, b, c]).unwrap();

    assert_eq!(report.unresolved_sectors, 1);
    assert_eq!(report.resolved_sectors, 0);
}

#[test]
fn damage_in_different_places_consolidates_to_a_whole_disk() {
    // The case F-008 exists for: three dumps, each damaged somewhere the
    // others are not, merging to a disk better than any of its inputs.
    let good = image(0);
    let mut a = good.clone();
    let mut b = good.clone();
    let mut c = good.clone();
    damage(&mut a, 10, 0xFF);
    damage(&mut b, 900, 0xFF);
    damage(&mut c, 1500, 0xFF);

    let report = consolidate(&[a, b, c]).unwrap();

    assert_eq!(report.resolved_sectors, 3);
    assert_eq!(report.unresolved_sectors, 0);
    assert_eq!(
        report.bytes, good,
        "each damaged sector is outvoted by the two that are not"
    );
}

#[test]
fn dumps_of_different_sizes_are_refused() {
    // Not dumps of one disk, so consolidating them would be meaningless.
    let a = image(0);
    let b = vec![0u8; 1024];

    assert!(matches!(
        consolidate(&[a, b]).unwrap_err(),
        ConsolidateError::SizeMismatch { .. }
    ));
}

#[test]
fn one_dump_is_not_a_consolidation() {
    assert_eq!(
        consolidate(&[image(0)]).unwrap_err(),
        ConsolidateError::TooFewSources
    );
    assert_eq!(
        consolidate(&[]).unwrap_err(),
        ConsolidateError::TooFewSources
    );
}

#[test]
fn a_partial_sector_is_refused() {
    let odd = vec![0u8; 700];
    assert!(matches!(
        consolidate(&[odd.clone(), odd]).unwrap_err(),
        ConsolidateError::NotWholeSectors { .. }
    ));
}

#[test]
fn diffing_identical_images_finds_nothing() {
    let a = image(0);
    let report = diff(&a, &a).unwrap();

    assert!(report.is_identical());
    assert!(report.sectors.is_empty());
    assert_eq!(report.sectors_total, 1760);
}

#[test]
fn a_diff_locates_the_sector_and_counts_the_bytes() {
    let a = image(0);
    let mut b = a.clone();
    b[512 * 3 + 10] ^= 0xFF;
    b[512 * 3 + 20] ^= 0xFF;

    let report = diff(&a, &b).unwrap();

    assert_eq!(report.sectors, vec![3]);
    assert_eq!(report.tracks, vec![0]);
    assert_eq!(report.bytes_differing, 2);
}

#[test]
fn a_diff_groups_sectors_into_tracks() {
    let a = image(0);
    let mut b = a.clone();
    // Two sectors on one track, one on another.
    damage(&mut b, 0, 0xFF);
    damage(&mut b, 1, 0xFF);
    damage(&mut b, 50, 0xFF);

    let report = diff(&a, &b).unwrap();

    assert_eq!(report.sectors, vec![0, 1, 50]);
    assert_eq!(report.tracks, vec![0, 50 / 11]);
}

#[test]
fn the_corpus_multi_dump_titles_consolidate() {
    // Real material: 46 titles have dumps that genuinely differ. This checks
    // the reporting holds up on them, not that any particular answer is right
    // — which is precisely what consolidation cannot tell you here.
    let corpus = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    // Group by title with TOSEC's bracketed tags stripped.
    let mut groups: std::collections::BTreeMap<String, Vec<std::path::PathBuf>> =
        std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&corpus).expect("read corpus").flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("adf") {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut base = String::new();
        let mut depth = 0i32;
        for ch in name.chars() {
            match ch {
                '[' => depth += 1,
                ']' => depth -= 1,
                c if depth == 0 => base.push(c),
                _ => {}
            }
        }
        groups.entry(base).or_default().push(path);
    }

    let mut consolidated = 0usize;
    let mut with_disagreement = 0usize;
    for paths in groups.values().filter(|p| p.len() > 1) {
        let dumps: Vec<Vec<u8>> = paths
            .iter()
            .filter_map(|p| std::fs::read(p).ok())
            .filter(|d| d.len() == 901_120)
            .collect();
        if dumps.len() < 2 {
            continue;
        }
        let report = consolidate(&dumps).expect("consolidates");
        consolidated += 1;
        if !report.is_unanimous() {
            with_disagreement += 1;
        }

        // The invariants that must hold whatever the disks say.
        assert_eq!(report.bytes.len(), 901_120);
        assert_eq!(report.total_sectors(), 1760);
        assert_eq!(report.sources, dumps.len());
        // A track is listed only if it has a disputed sector.
        for track in &report.tracks {
            assert!(
                !track.disputed.is_empty(),
                "track {} listed empty",
                track.track
            );
            assert!(track.unresolved.len() <= track.disputed.len());
        }
        // With two dumps nothing can carry a plurality.
        if dumps.len() == 2 {
            assert_eq!(
                report.resolved_sectors, 0,
                "two dumps cannot resolve anything"
            );
        }
    }

    eprintln!("consolidated {consolidated} multi-dump titles, {with_disagreement} disagreed");
    assert!(
        consolidated >= 20,
        "expected the corpus's duplicates, got {consolidated}"
    );
}
