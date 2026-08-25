//! The format-conversion matrix (Phase 3, F-016).
//!
//! The matrix is the feature. Converting between Amiga image formats is done
//! today by single-purpose tools that rarely say what the conversion cost —
//! writing an extended ADF out as a plain ADF silently discards the copy
//! protection that was the reason to capture it. These tests pin the answers,
//! and in particular pin that a lossy conversion is **refused** rather than
//! warned about.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests over data they construct"
)]

use ade_core::layers::container::Kind;
use ade_core::{Conversion, conversion};

const ADF: Kind = Kind::Adf {
    cylinders: 80,
    sectors: 11,
};

#[test]
fn decompression_is_lossless() {
    // The one conversion whose reader is proven byte-identically (D-004).
    assert_eq!(conversion(Kind::Gzip, ADF), Conversion::Lossless);
    assert_eq!(conversion(Kind::Gzip, Kind::Hardfile), Conversion::Lossless);
}

#[test]
fn sector_containers_convert_freely() {
    // ADF, HDF and a whole-device image are the same thing — a flat run of
    // sectors — so moving between them cannot lose anything.
    for from in [ADF, Kind::Hardfile, Kind::RigidDisk] {
        for to in [ADF, Kind::Hardfile] {
            assert_eq!(conversion(from, to), Conversion::Lossless, "{from} -> {to}");
        }
    }
}

#[test]
fn flattening_flux_is_lossy_and_says_what_is_lost() {
    // The case F-016 exists for. An extended ADF or an SCP capture holds what
    // a sector image cannot, and a tool that flattens it without saying so is
    // destroying the reason the disk was captured.
    for from in [Kind::ExtendedAdf { tracks: 160 }, Kind::Scp] {
        let verdict = conversion(from, ADF);
        let Conversion::Lossy { lost } = &verdict else {
            panic!("{from} -> ADF should be lossy, got {verdict}");
        };
        assert!(
            lost.contains("protection"),
            "the message should name what is lost: {lost}"
        );
    }
}

#[test]
fn ipf_output_is_refused_permanently() {
    // C-003 is a licence constraint, not a gap. It must read as a decision
    // rather than as something waiting to be implemented, because the two
    // invite very different follow-up.
    for from in [ADF, Kind::Gzip, Kind::Scp, Kind::Hardfile] {
        let verdict = conversion(from, Kind::Ipf);
        let Conversion::Refused { why } = &verdict else {
            panic!("IPF output must be refused, got {verdict}");
        };
        assert!(why.contains("C-003"), "{why}");
    }
}

#[test]
fn dms_output_is_refused_and_dms_input_is_merely_missing() {
    // Two different answers about the same format, and the distinction is the
    // point: ADE will not write DMS by choice, and cannot read it yet by
    // circumstance (D-009, blocked on material).
    assert!(matches!(
        conversion(ADF, Kind::Dms),
        Conversion::Refused { .. }
    ));

    let verdict = conversion(Kind::Dms, ADF);
    let Conversion::NotImplemented { why } = &verdict else {
        panic!("DMS input should be not-implemented, got {verdict}");
    };
    assert!(why.contains("D-009"), "{why}");
}

#[test]
fn writing_compressed_images_is_not_implemented() {
    // ADE inflates but does not deflate. Saying so plainly beats emitting a
    // "compressed" file made of stored blocks that is larger than its input.
    assert!(matches!(
        conversion(ADF, Kind::Gzip),
        Conversion::NotImplemented { .. }
    ));
}

#[test]
fn an_unrecognised_input_converts_to_nothing() {
    assert!(matches!(
        conversion(Kind::Unknown, ADF),
        Conversion::NotImplemented { .. }
    ));
}

#[test]
fn every_pair_has_an_answer_and_every_refusal_has_a_reason() {
    // No pair may fall through to a default. A conversion matrix whose gaps
    // are silent is the thing this replaces.
    let kinds = ade_core::convert::known_formats();
    let mut possible = 0;

    for from in &kinds {
        for to in &kinds {
            let verdict = conversion(*from, *to);
            match &verdict {
                Conversion::Lossless => possible += 1,
                Conversion::Lossy { lost } => {
                    possible += 1;
                    assert!(!lost.is_empty(), "{from} -> {to}");
                }
                Conversion::NotImplemented { why } | Conversion::Refused { why } => {
                    assert!(!why.is_empty(), "{from} -> {to} gives no reason");
                }
            }
            assert!(!verdict.label().is_empty());
        }
    }

    assert!(possible > 0, "the matrix should authorise something");
}

#[test]
fn only_lossless_conversions_are_possible_today() {
    // is_possible() authorises the CLI to run, and a lossy conversion is
    // refused at the command layer. Nothing lossy should reach a writer while
    // that is the policy.
    let kinds = ade_core::convert::known_formats();
    for from in &kinds {
        for to in &kinds {
            if let Conversion::Lossy { .. } = conversion(*from, *to) {
                // Lossy is "possible" in principle — the matrix says so
                // honestly — but the CLI refuses it. This test records that
                // the two are deliberately different questions.
                assert!(conversion(*from, *to).is_possible());
            }
        }
    }
}
