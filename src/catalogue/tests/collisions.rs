//! What several matches against the dataset mean (F-013).
//!
//! # The measurement that produced these tests
//!
//! ADE recorded from 2026-08-27 that the TOSEC Amiga dataset contained "71
//! CRC32 collisions", and `identify` returned every match and declined to
//! choose. Checking that claim on 2026-08-29 found it wrong. There are **77
//! groups sharing a CRC32 and a size, and every member of every one of them
//! carries the same SHA-1 *and* the same MD5**: they are duplicate content
//! under different names — the same CD audio track listed as track 6 and
//! track 10, the same ISO in two sets — not collisions. Zero genuine
//! collisions exist in the set, and not one group involves an `.adf`.
//!
//! So the useful distinction is not "resolve the ambiguity" but "say which
//! kind of several this is": duplicate names are a property of the catalogue
//! and every name is correct, while different content sharing a hash would be
//! a reason to distrust the match entirely.
//!
//! # No collision is constructed here, and none is needed
//!
//! An early attempt searched for two byte strings sharing a CRC32. Wrong
//! problem twice over: for a fixed length CRC32 is a bijection over four
//! varying bytes, so exactly one solution exists and the search cannot find a
//! second — and what is being modelled is not two files that collide but
//! **two dataset entries claiming one CRC**, which needs no search at all.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use ade_block::checksum::crc32;
use ade_catalogue::{Catalogue, Match, sha1};

const PAYLOAD: &[u8] = b"the bytes on the disk in front of us";

/// A datfile whose two entries claim the same CRC32 and size, as 71 real pairs
/// in the TOSEC set do, differing only in their SHA-1.
fn two_entries(first_sha1: Option<&str>, second_sha1: Option<&str>) -> String {
    let attr = |s: Option<&str>| s.map_or(String::new(), |v| format!(r#" sha1="{v}""#));
    format!(
        r#"<datafile>
 <game name="First"><rom name="first.adf" size="{len}" crc="{crc:08x}"{a}/></game>
 <game name="Second"><rom name="second.adf" size="{len}" crc="{crc:08x}"{b}/></game>
</datafile>"#,
        len = PAYLOAD.len(),
        crc = crc32(PAYLOAD),
        a = attr(first_sha1),
        b = attr(second_sha1),
    )
}

#[test]
fn sha1_separates_two_entries_that_share_a_crc32() {
    // The hypothetical the old claim described: one entry is these bytes and
    // the other is a different file with the same CRC32 and size. Never seen
    // in the Amiga set, and still the case worth handling.
    let real = sha1::hex(&sha1::sha1(PAYLOAD));
    let other = sha1::hex(&sha1::sha1(b"some other disk entirely"));
    let mut catalogue = Catalogue::default();
    catalogue.add(&two_entries(Some(&real), Some(&other)), "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Named);
    assert_eq!(
        found.entries.len(),
        1,
        "the tie should be broken, not reported"
    );
    assert_eq!(found.entries[0].name, "first.adf");
}

#[test]
fn identical_content_under_two_names_is_a_duplicate_not_an_ambiguity() {
    // What the dataset *actually* contains, 77 times over. Both names are
    // correct, so both are returned — but calling this ambiguous would say
    // ADE could not tell which disk it was holding, which is false.
    let same = sha1::hex(&sha1::sha1(PAYLOAD));
    let mut catalogue = Catalogue::default();
    catalogue.add(&two_entries(Some(&same), Some(&same)), "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Duplicated);
    assert_eq!(found.entries.len(), 2, "both names are right");
    assert!(found.kind.is_named());
}

#[test]
fn it_is_the_bytes_that_decide_which_entry_wins() {
    // The same dataset, the other way round: nothing about ordering or
    // position is doing the work.
    let real = sha1::hex(&sha1::sha1(PAYLOAD));
    let other = sha1::hex(&sha1::sha1(b"some other disk entirely"));
    let mut catalogue = Catalogue::default();
    catalogue.add(&two_entries(Some(&other), Some(&real)), "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Named);
    assert_eq!(found.entries.len(), 1);
    assert_eq!(found.entries[0].name, "second.adf");
}

#[test]
fn a_tie_with_no_sha1_to_check_is_still_reported_rather_than_dropped() {
    // The bytes *are* in the dataset; the entries simply carry nothing that
    // could separate them. Reporting nothing would be a worse answer than
    // reporting the ambiguity, which is what ADE did before SHA-1 existed.
    let mut catalogue = Catalogue::default();
    catalogue.add(&two_entries(None, None), "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(
        found.kind,
        Match::Unverified,
        "not a collision, an unanswerable question"
    );
    assert_eq!(
        found.entries.len(),
        2,
        "an unresolvable tie is still the truth"
    );
}

#[test]
fn two_different_files_claiming_one_crc_is_reported_as_a_collision() {
    // Both entries claim this CRC32 and size, and neither is these bytes. The
    // disk in hand is *not* in the dataset, and saying "identified, ambiguous"
    // would be the worst of the available answers.
    let a = sha1::hex(&sha1::sha1(b"neither"));
    let b = sha1::hex(&sha1::sha1(b"nor this"));
    let mut catalogue = Catalogue::default();
    catalogue.add(&two_entries(Some(&a), Some(&b)), "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Collision);
    assert!(!found.kind.is_named(), "a collision names nothing");
}

#[test]
fn a_single_match_is_returned_without_consulting_sha1() {
    // The common case, and the reason SHA-1 is not computed up front: an entry
    // with a deliberately wrong SHA-1 still matches, because nothing looked.
    let dat = format!(
        r#"<datafile><game name="Only"><rom name="only.adf" size="{}" crc="{:08x}" sha1="{}"/></game></datafile>"#,
        PAYLOAD.len(),
        crc32(PAYLOAD),
        "0000000000000000000000000000000000000000"
    );
    let mut catalogue = Catalogue::default();
    catalogue.add(&dat, "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Named);
    assert_eq!(found.entries.len(), 1);
}

#[test]
fn a_wrong_size_still_rules_an_entry_out_before_any_of_this() {
    let dat = format!(
        r#"<datafile><game name="Only"><rom name="only.adf" size="999999" crc="{:08x}"/></game></datafile>"#,
        crc32(PAYLOAD)
    );
    let mut catalogue = Catalogue::default();
    catalogue.add(&dat, "test.dat");

    let found = catalogue.identify_detailed(PAYLOAD);
    assert_eq!(found.kind, Match::Unknown);
    assert!(found.entries.is_empty());
}
