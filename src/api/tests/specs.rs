//! What a disk says it needs (F-028).
//!
//! The feature's whole value is that it does not guess, so most of what is
//! pinned here is what it declines to say.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over images they construct"
)]

use ade_core::Image;
use ade_core::specs::{Specs, UNKNOWABLE};
use ade_fixtures::Volume as Fixture;

fn read(bytes: &[u8]) -> Specs {
    let image = Image::from_bytes(bytes.to_vec()).expect("opens");
    Specs::of(&image, bytes)
}

fn says(specs: &Specs, fragment: &str) -> bool {
    specs.facts.iter().any(|f| f.what.contains(fragment))
}

#[test]
fn every_claim_carries_its_evidence() {
    // The rule the whole report rests on. A claim without evidence is asking
    // to be believed; one that names the library is asking to be checked.
    let mut v = Fixture::dd(1).named("Evidence");
    v.add_file("readme", b"hello");
    let specs = read(&v.build());

    assert!(!specs.facts.is_empty());
    for fact in &specs.facts {
        assert!(!fact.what.trim().is_empty());
        assert!(
            !fact.because.trim().is_empty(),
            "no evidence for: {}",
            fact.what
        );
    }
}

#[test]
fn the_media_is_read_from_the_geometry() {
    assert!(says(
        &read(&Fixture::dd(1).named("D").build()),
        "double-density"
    ));
    let hd = read(&Fixture::new(80, 2, 22, 1).named("H").build());
    assert!(says(&hd, "high-density"), "{:?}", hd.facts);
    assert!(says(&hd, "needs an HD drive"), "and what that costs");
    assert!(says(
        &read(&Fixture::new(40, 2, 11, 1).named("S").build()),
        "5.25-inch"
    ));
}

#[test]
fn a_bootblock_that_is_not_amigados_is_not_called_self_booting() {
    // Two readings and no way to choose between them without running the code:
    // a custom loader that takes the machine over, or an AmigaDOS bootblock
    // that has been damaged. `Abandoned Places_Disk2` begins `\x00OS\x00`,
    // which is the second — and the first draft of this called it self-booting.
    let mut bytes = Fixture::dd(1).named("Custom").build();
    bytes[0] = 0x00;
    let specs = read(&bytes);
    assert!(says(&specs, "Does not start through AmigaDOS"));
    assert!(says(&specs, "or its bootblock is damaged"), "both readings");

    // An ordinary disk is not accused of either.
    let sound = read(&Fixture::dd(1).named("Sound").build());
    assert!(says(&sound, "Starts through AmigaDOS"));
    assert!(!says(&sound, "damaged"));

    // And an empty bootblock is neither.
    let mut blank = Fixture::dd(1).named("Blank").build();
    blank[..4].fill(0);
    assert!(says(&read(&blank), "Not bootable"));
}

#[test]
fn a_release_2_library_puts_a_floor_under_the_kickstart_version() {
    let mut v = Fixture::dd(1).named("Modern");
    v.add_file("tool", b"\x00\x00\x03\xf3 open asl.library now");
    let specs = read(&v.build());

    assert!(says(&specs, "at least Kickstart 2.0"));
    assert!(
        specs
            .facts
            .iter()
            .any(|f| f.because.contains("asl.library")),
        "and names which library said so"
    );
    assert_eq!(specs.libraries, vec!["asl.library"]);
}

#[test]
fn a_library_name_inside_another_word_is_not_a_library() {
    // Measured: without a word boundary, a scan of 400 corpus images reported
    // `udos.library`, `ugraphics.library` and `uintuition.library`. None of
    // those exists — they were real names with whatever byte came before them.
    let mut v = Fixture::dd(1).named("Boundary");
    v.add_file(
        "tool",
        b"xasl.library and my_utility.library and z.iffparse.library",
    );
    let specs = read(&v.build());

    assert!(specs.libraries.is_empty(), "{:?}", specs.libraries);
    // Not "Kickstart" alone: the bootblock fact legitimately mentions one.
    // What must be absent is the *version floor* those names would have set.
    assert!(
        !says(&specs, "at least Kickstart"),
        "so no claim is made from them"
    );
}

#[test]
fn a_disk_that_says_nothing_gets_no_kickstart_claim() {
    // Most disks are this: 4,339 of 4,651 make no claim, because their bytes
    // support none. Silence is the honest answer, not a default of "1.3".
    let mut v = Fixture::dd(1).named("Quiet");
    v.add_file("data", &vec![0x42u8; 2000]);
    let specs = read(&v.build());
    assert!(!says(&specs, "Kickstart 2.0"));
    assert!(specs.libraries.is_empty());
}

#[test]
fn what_cannot_be_known_is_listed_with_its_reason() {
    // Listed rather than omitted: a report that simply stops reads as "there
    // is nothing more to know", and somebody infers from the silence that the
    // disk runs on anything.
    assert!(UNKNOWABLE.len() >= 4);
    for (what, why) in UNKNOWABLE {
        assert!(!what.is_empty());
        assert!(why.len() > 30, "{what}: a reason, not a shrug");
    }
    let named: Vec<&str> = UNKNOWABLE.iter().map(|(w, _)| *w).collect();
    for expected in ["Memory", "Processor", "Chipset", "Video standard"] {
        assert!(named.contains(&expected), "{expected} must be admitted to");
    }
}
