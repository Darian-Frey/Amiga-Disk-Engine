//! The health report (F-010), against generated fixtures.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over data they construct"
)]

use ade_core::{Severity, examine};
use ade_fixtures::{Volume as Fixture, corrupt};

fn codes(h: &ade_core::Health) -> Vec<&'static str> {
    h.findings.iter().map(|f| f.code).collect()
}

#[test]
fn a_clean_volume_is_sound() {
    let mut v = Fixture::dd(1).named("Healthy");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    let h = examine(v.build());
    assert!(h.is_sound(), "{:?}", h.findings);
    assert_eq!(h.files, 1);
    assert_eq!(h.directories, 1);
    assert!(h.worst().is_none_or(|s| s == Severity::Info));
}

#[test]
fn the_bitmap_cross_check_counts_overhead_as_used() {
    // The rootblock and bitmap blocks are allocated but never *reached* by a
    // tree walk. Counting them as orphans would fire on every healthy disk.
    let mut v = Fixture::dd(0).named("Overhead");
    v.add_file("f", &vec![1u8; 4000]);
    let h = examine(v.build());
    let b = h.bitmap.expect("bitmap");
    assert_eq!(
        b.orphaned, 0,
        "filesystem overhead must not read as orphaned"
    );
    assert_eq!(b.referenced_but_free, 0);
    assert_eq!(b.marked_used, b.actually_used);
}

#[test]
fn a_stale_bitmap_flag_is_a_warning_not_an_error() {
    // 260 of 4652 real images have one; treating it as an error would drown
    // the disks that are genuinely broken (AV-003).
    let v = Fixture::dd(1).named("Unclean");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root);
    let h = examine(img);
    assert!(codes(&h).contains(&"bitmap-flag-clear"));
    assert_eq!(h.worst(), Some(Severity::Warning));
}

#[test]
fn cosmetic_findings_do_not_outrank_structural_ones() {
    let mut v = Fixture::dd(1).named("Mixed");
    v.add_file("f", b"x");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root); // warning
    corrupt::clear_created_date(&mut img, root); // info
    let h = examine(img);
    // Findings are ordered worst-first, so the first is what a reader sees.
    assert_eq!(
        h.findings.first().map(|f| f.severity),
        Some(Severity::Warning)
    );
    assert!(h.findings.iter().any(|f| f.severity == Severity::Info));
}

#[test]
fn a_damaged_rootblock_checksum_is_an_error() {
    let v = Fixture::dd(0).named("Damaged");
    let root = v.root();
    let mut img = v.build();
    corrupt::block_checksum(&mut img, root);
    let h = examine(img);
    assert_eq!(h.worst(), Some(Severity::Error));
    assert!(codes(&h).contains(&"rootblock-checksum"));
}

#[test]
fn an_image_with_no_volume_is_reported_not_failed() {
    // A quarter of real images are not AmigaDOS disks. That is information,
    // not a fault.
    let v = Fixture::dd(0);
    let root = v.root();
    let mut img = v.build();
    corrupt::rootblock_wrong_type(&mut img, root);
    let h = examine(img);
    assert!(codes(&h).contains(&"no-volume"));
    assert_eq!(
        h.worst(),
        Some(Severity::Info),
        "not a fault: {:?}",
        h.findings
    );
}

#[test]
fn structural_data_faults_reach_the_report() {
    let mut v = Fixture::dd(0).named("Faulty");
    let hdr = v.add_file("victim", &vec![3u8; 4000]);
    let mut img = v.build();
    // Zero the first data block, whose pointer sits at BSIZE-204.
    let o = hdr as usize * 512 + 512 - 204;
    let first = ade_core::layers::endian::u32_at(&img, o).unwrap();
    corrupt::zero_block(&mut img, first);
    let h = examine(img);
    assert!(
        codes(&h).contains(&"data-block-structure"),
        "{:?}",
        h.findings
    );
    assert!(codes(&h).contains(&"file-short"));
}

#[test]
fn degenerate_images_produce_a_report_rather_than_a_panic() {
    for bytes in [
        Vec::new(),
        vec![0u8; 3],
        corrupt::zeroed_volume(),
        corrupt::truncated(&Fixture::dd(0).build(), 176),
        corrupt::with_trailing_junk(&Fixture::dd(0).build(), 1),
    ] {
        let h = examine(bytes);
        // Whatever it is, it produces a report.
        let _ = h.to_json().to_json();
    }
}

#[test]
fn the_json_surface_carries_severities_and_codes() {
    let v = Fixture::dd(1).named("Json");
    let root = v.root();
    let mut img = v.build();
    corrupt::bitmap_flag_invalid(&mut img, root);
    let json = examine(img).to_json().to_json();
    assert!(json.is_ascii());
    assert!(json.contains(r#""severity":"warning""#), "{json}");
    assert!(json.contains(r#""code":"bitmap-flag-clear""#));
    assert!(json.contains(r#""worst":"warning""#));
    assert!(json.contains(r#""bitmap":{"#));
}
