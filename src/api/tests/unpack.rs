//! Writing every file off a disk into a folder (F-024).
//!
//! The interesting half is the names. Every rule below was chosen from a
//! measurement over all 4,652 corpus images and 83,487 distinct filenames, and
//! the counts are in the module's own documentation.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests over volumes they construct"
)]

use std::fs;
use std::path::PathBuf;

use ade_core::Image;
use ade_core::unpack::{host_name, unpack};
use ade_fixtures::Volume as Fixture;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-unpack-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn an_ordinary_name_is_left_exactly_alone() {
    for name in ["startup-sequence", "SUPERSHEET", "READ ME", "4-GET-IT.info"] {
        assert_eq!(host_name(name), name);
    }
}

#[test]
fn an_accented_name_survives_as_utf8() {
    // Measured: names like `Effekte für AE 2 Deutsch.info` and `CD³²_Prefs`
    // are real and meaningful. A name is Latin-1 and already decoded by the
    // time it reaches here; decoding it a second time turns `für` into `fÃ¼r`,
    // which loses the name while appearing to preserve it.
    assert_eq!(host_name("Effekte für AE 2"), "Effekte für AE 2");
    assert_eq!(host_name("CD³²_Prefs"), "CD³²_Prefs");
}

#[test]
fn a_name_that_cannot_be_a_filename_is_escaped_reversibly() {
    // A NUL: three corpus names carry one, all on `Xenon 2 - Megablast_Disk1`.
    // It cannot go in a POSIX filename at all.
    assert_eq!(host_name("bad\u{0}name"), "bad%00name");
    // The separator. Zero corpus names contain one — AmigaDOS forbids it — but
    // it is escaped because writing it is structurally impossible, not because
    // it was seen.
    assert_eq!(host_name("a/b"), "a%2Fb");
    // The escape character itself, so the escaping stays reversible. Three
    // corpus names contain one, such as `The Only 100% Version.`.
    assert_eq!(host_name("100%"), "100%25");
    assert_eq!(
        host_name("The Only 100% Version."),
        "The Only 100%25 Version."
    );
}

#[test]
fn a_name_that_is_a_direction_rather_than_a_thing_is_escaped() {
    // `.` and `..` would resolve to a different directory rather than fail,
    // which is the one case here where a quiet mistake is possible. One corpus
    // image carries such a name.
    assert_eq!(host_name("."), "%2E");
    assert_eq!(host_name(".."), "%2E%2E");
    assert_eq!(host_name(""), "%00");
    // But a name that merely starts with a dot is an ordinary name, and the
    // corpus is full of them.
    assert_eq!(host_name(".info"), ".info");
    assert_eq!(host_name("4-GET-IT.info.."), "4-GET-IT.info..");
}

#[test]
fn names_that_are_awkward_but_legal_are_left_alone() {
    // The judgement call, and the one worth being able to point at. These are
    // legal on POSIX: 62 corpus names carry a Windows-illegal character and
    // 328 end in a dot or a space or are nothing but spaces. Escaping them
    // would mangle 390 real names to buy portability to a platform ADE has
    // never been built on.
    assert_eq!(host_name(">>> BY AEON <<<"), ">>> BY AEON <<<");
    assert_eq!(host_name(" * DRAGO & AMADEUS * "), " * DRAGO & AMADEUS * ");
    assert_eq!(host_name("C.D.I."), "C.D.I.");
    assert_eq!(host_name("        "), "        ");
}

#[test]
fn every_file_and_drawer_reaches_the_folder() {
    let dir = scratch("whole");
    let mut v = Fixture::dd(1).named("Whole");
    v.add_file("readme", b"hello");
    v.add_dir("Tools");
    v.add_file("Tools/deep", b"inside a drawer");
    v.add_dir("Empty");
    let image = Image::from_bytes(v.build()).unwrap();
    let volume = image.volume().unwrap();

    let out = unpack(&volume, &dir).unwrap();
    assert_eq!(out.files, 2);
    assert_eq!(out.bytes, 5 + 15);
    assert!(out.skipped.is_empty(), "{:?}", out.skipped);

    assert_eq!(fs::read(dir.join("readme")).unwrap(), b"hello");
    assert_eq!(
        fs::read(dir.join("Tools/deep")).unwrap(),
        b"inside a drawer"
    );
    // An empty drawer is a fact about the disk, so it is recovered too.
    assert!(
        dir.join("Empty").is_dir(),
        "an empty drawer is still a drawer"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nothing_is_ever_overwritten() {
    // Two files in one drawer cannot share a name — AmigaDOS's hash table
    // prevents it — so a collision means a case-insensitive host or two names
    // that escaped alike. Either way the second is skipped and said so:
    // replacing the first would destroy recovered data to make room for more.
    let dir = scratch("collide");
    let mut v = Fixture::dd(1).named("Collide");
    v.add_file("readme", b"from the disk");
    let image = Image::from_bytes(v.build()).unwrap();
    let volume = image.volume().unwrap();

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("readme"), b"already here").unwrap();

    let out = unpack(&volume, &dir).unwrap();
    assert_eq!(out.files, 0, "nothing written");
    assert_eq!(out.skipped.len(), 1);
    assert_eq!(out.skipped[0].path, "readme");
    assert!(out.skipped[0].reason.contains("already exists"));
    assert_eq!(
        fs::read(dir.join("readme")).unwrap(),
        b"already here",
        "the file that was there is the file that is there"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn one_failure_does_not_stop_the_rest() {
    // A run over a damaged disk that stops at the first bad file has recovered
    // nothing, which is the same reasoning as `ade batch`. Driven here through
    // a write that cannot succeed, because that is the failure this side can
    // cause on purpose: a folder that already holds a *file* called `Tools`
    // has nowhere to put a drawer of that name.
    let dir = scratch("damaged");
    let mut v = Fixture::dd(1).named("Damaged");
    v.add_file("first", b"one");
    v.add_dir("Tools");
    v.add_file("Tools/deep", b"inside");
    v.add_file("third", b"three");
    let image = Image::from_bytes(v.build()).unwrap();
    let volume = image.volume().unwrap();

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Tools"), b"in the way").unwrap();

    let out = unpack(&volume, &dir).unwrap();
    assert_eq!(
        out.files, 2,
        "the two at the top level still arrive: {out:?}"
    );
    assert_eq!(fs::read(dir.join("first")).unwrap(), b"one");
    assert_eq!(fs::read(dir.join("third")).unwrap(), b"three");

    // And both the drawer and the file inside it are named as skipped, rather
    // than quietly missing from a run that reported success.
    let skipped: Vec<&str> = out.skipped.iter().map(|s| s.path.as_str()).collect();
    assert!(skipped.contains(&"Tools"), "{skipped:?}");
    assert!(skipped.contains(&"Tools/deep"), "{skipped:?}");
    assert_eq!(
        fs::read(dir.join("Tools")).unwrap(),
        b"in the way",
        "and what was there is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}
