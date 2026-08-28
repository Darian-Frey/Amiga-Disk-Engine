//! The JSON surface of `diff`, `consolidate`, `formats` and `identify`.
//!
//! These four accepted `--format=json` and printed prose (BUG-007), so the
//! shapes below are new — and new means unconstrained exactly once. After
//! this, F-015 makes the field names a commitment: rename one and something
//! downstream breaks silently, because a missing key reads as a missing value
//! rather than as an error.
//!
//! The assertions are on exact strings rather than on a parsed structure,
//! since ADE has no JSON *parser* and will not gain one to test its writer.
//! That is stricter than parsing anyway: it pins field order, which is what
//! makes the output diffable between runs.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use ade_core::consolidate::{consolidate, diff};
use ade_core::convert::matrix_json;

/// Two images differing in one sector, so the shape has something to report.
fn two_dumps() -> (Vec<u8>, Vec<u8>) {
    let a = vec![0u8; 512 * 1760];
    let mut b = a.clone();
    // Sector 13 is on track 1, which proves the track index is derived rather
    // than copied from the sector number.
    b[512 * 13] = 0xFF;
    (a, b)
}

#[test]
fn a_diff_reports_which_sectors_moved_not_merely_how_many() {
    let (a, b) = two_dumps();
    let json = diff(&a, &b).unwrap().to_json().to_json();
    assert_eq!(
        json,
        r#"{"identical":false,"sectors_total":1760,"sectors_differing":1,"bytes_differing":1,"sectors":[13],"tracks":[1]}"#
    );
}

#[test]
fn identical_dumps_say_so_without_an_empty_field_being_ambiguous() {
    // `identical` is explicit rather than left to be inferred from an empty
    // `sectors` array, because an empty array is also what a caller gets from
    // a comparison that failed to run.
    let a = vec![0u8; 512 * 1760];
    let json = diff(&a, &a.clone()).unwrap().to_json().to_json();
    assert!(json.starts_with(r#"{"identical":true,"sectors_total":1760"#));
    assert!(json.ends_with(r#""sectors":[],"tracks":[]}"#));
}

#[test]
fn consolidation_says_when_the_dumps_could_not_vote() {
    // Two dumps tie on every disagreement by definition, so `unresolved` is
    // arithmetic rather than damage. A caller ranking by `unresolved` without
    // `can_vote` would call every two-dump run the most broken thing it had.
    let (a, b) = two_dumps();
    let report = consolidate(&[a, b]).unwrap();
    let json = report.to_json().to_json();
    assert!(json.contains(r#""sources":2"#));
    assert!(json.contains(r#""can_vote":false"#));
    assert!(json.contains(r#""unresolved_sectors":1"#));
    // Sector *2 of track 1*, which is absolute sector 13 — the numbers inside
    // a track object are relative to it, where `diff`'s are absolute. Both are
    // right for where they sit; the contract has to say which is which.
    assert!(json.contains(r#""tracks":[{"track":1,"disputed":[2],"unresolved":[2]}]"#));
}

#[test]
fn three_dumps_can_vote() {
    let (a, b) = two_dumps();
    let report = consolidate(&[a.clone(), a, b]).unwrap();
    let json = report.to_json().to_json();
    assert!(json.contains(r#""can_vote":true"#));
    assert!(json.contains(r#""resolved_sectors":1"#));
    assert!(json.contains(r#""unresolved_sectors":0"#));
}

#[test]
fn the_conversion_matrix_is_keyed_on_codes_not_on_prose() {
    let json = matrix_json().to_json();
    // `ADF (DD, 80 cylinders)` carries a geometry that varies between images
    // of one kind. Matching on it would be parsing prose.
    assert!(json.contains(r#""from":"scp","to":"adf""#));
    assert!(json.contains(r#""from_label":"SCP flux""#));
}

#[test]
fn a_conversion_separates_what_it_is_from_why() {
    let json = matrix_json().to_json();
    // F-016's whole distinction: refused is a decision that does not expire,
    // not-implemented is a gap with a cause. They invite opposite follow-up,
    // so a caller must be able to tell them apart without reading English.
    assert!(json.contains(r#""to":"ipf","from_label":"ADF (DD, 80 cylinders)","to_label":"IPF flux","conversion":{"kind":"refused""#));
    assert!(json.contains(r#""kind":"not implemented""#));
    assert!(json.contains(r#""kind":"lossless","possible":true,"reason":null"#));
}

#[test]
fn every_pair_of_formats_appears_except_a_format_with_itself() {
    let count = ade_core::convert::known_formats().len();
    let json = matrix_json().to_json();
    let rows = json.matches(r#"{"from":"#).count();
    assert_eq!(
        rows,
        count * (count - 1),
        "the matrix should be every ordered pair but the identities"
    );
}
