//! A program's names and comments, kept per user and found by the
//! program's bytes (D-022). Every call here names its folder, so nothing
//! reads or writes the user's own.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    clippy::indexing_slicing,
    reason = "test scaffolding: programs built by hand"
)]

use std::path::PathBuf;

use ade_core::project::{self, Change};

/// A one-hunk program: `moveq #0,d0; rts`.
fn program() -> Vec<u8> {
    let mut out = Vec::new();
    for l in [0x3F3u32, 0, 1, 0, 0, 1, 0x3E9, 1, 0x7000_4E75, 0x3F2] {
        out.extend_from_slice(&l.to_be_bytes());
    }
    out
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ade-projects-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn notes_are_kept_by_program_and_read_back() {
    let dir = folder("keep");
    let bytes = program();
    let file = project::change_in(&dir, &bytes, false, (0, 2), Change::Name(Some("Done"))).unwrap();
    assert_eq!(file, project::file_in(&dir, &bytes));
    assert!(file.ends_with(format!("{}.adeproj", project::key(&bytes))));
    project::change_in(
        &dir,
        &bytes,
        false,
        (0, 0),
        Change::Comment(Some("clears d0")),
    )
    .unwrap();
    let (notes, problems) = project::load_from(&dir, &bytes);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(notes.names.get(&(0, 2)).map(String::as_str), Some("Done"));
    assert_eq!(
        notes.comments.get(&(0, 0)).map(String::as_str),
        Some("clears d0")
    );
    // The file says which program it is for.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&format!("program {} {}", project::key(&bytes), bytes.len())));

    // A program a byte different is another program, with no notes.
    let mut other = bytes.clone();
    other[32] ^= 1;
    assert!(project::load_from(&dir, &other).0.is_empty());

    // Clearing the last of them removes the file.
    project::change_in(&dir, &bytes, false, (0, 2), Change::Name(None)).unwrap();
    project::change_in(&dir, &bytes, false, (0, 0), Change::Comment(None)).unwrap();
    assert!(!file.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_breaks_a_rule_is_refused_and_nothing_is_written() {
    let dir = folder("refuse");
    let bytes = program();
    for (place, change, why) in [
        ((0, 0), Change::Name(Some("a6")), "register"),
        ((0, 0), Change::Name(Some("h0_0000")), "ADE makes"),
        ((1, 0), Change::Name(Some("x")), "not a place"),
        ((0, 4), Change::Name(Some("x")), "not a place"),
        ((0, 0), Change::Comment(Some("two\nlines")), "one line"),
    ] {
        let err = project::change_in(&dir, &bytes, false, place, change).unwrap_err();
        assert!(err.contains(why), "{err}");
    }
    assert!(!dir.exists());
    // Boot code's places run from byte 12.
    let boot = vec![0u8; 1024];
    assert!(project::change_in(&dir, &boot, true, (0, 4), Change::Name(Some("x"))).is_err());
    project::change_in(&dir, &boot, true, (0, 12), Change::Name(Some("Boot"))).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_with_lines_ade_cannot_read_says_so_and_is_not_written_over() {
    let dir = folder("bad");
    let bytes = program();
    std::fs::create_dir_all(&dir).unwrap();
    let file = project::file_in(&dir, &bytes);
    std::fs::write(&file, "name h0_0002 Done\nname h0_0000 d0\n").unwrap();
    let (notes, problems) = project::load_from(&dir, &bytes);
    assert_eq!(notes.names.len(), 1);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("line 2"), "{problems:?}");
    let err =
        project::change_in(&dir, &bytes, false, (0, 0), Change::Name(Some("Go"))).unwrap_err();
    assert!(err.contains("would lose"), "{err}");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("h0_0000 d0")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_listing_line_is_found_back_as_its_place() {
    let bytes = program();
    // The code hunk's bytes start 32 bytes into the file.
    assert_eq!(project::place_at(&bytes, 32), Some((0, 0)));
    assert_eq!(project::place_at(&bytes, 34), Some((0, 2)));
    assert_eq!(project::place_at(&bytes, 36), None);
    assert_eq!(project::place_at(&bytes, 0), None);
    assert_eq!(project::boot_place_at(12), Some((0, 12)));
    assert_eq!(project::boot_place_at(11), None);
    assert_eq!(project::boot_place_at(1024), None);
}
