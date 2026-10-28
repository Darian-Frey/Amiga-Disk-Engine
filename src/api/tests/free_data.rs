//! A volume with no entries whose free blocks hold data (IMP-019).
//!
//! A game that loads its own tracks can carry a valid, empty root, and leave
//! the blocks it reads marked free. Measured 2026-10-28 over the corpus: 87
//! volumes, every one a game. `ade check` called them empty and nearly unused,
//! and `ade put` wrote over the game with no finding raised. The corpus half
//! is local only and skips without the corpus; the built cases run in CI.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::edit::{EditError, Session};
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn blank() -> Vec<u8> {
    let dostype = Dostype::from_raw(0x444F_5300).unwrap();
    format::blank(Geometry::DD_FLOPPY, dostype, "Loader", WHEN).unwrap()
}

fn findings(bytes: Vec<u8>) -> Vec<ade_core::Finding> {
    ade_core::health::examine(bytes)
        .findings
        .into_iter()
        .filter(|f| f.code == "free-blocks-hold-data")
        .collect()
}

fn held(bytes: Vec<u8>) -> usize {
    ade_core::health::examine(bytes)
        .bitmap
        .map_or(usize::MAX, |b| b.free_holding_data)
}

/// Fill `block` with `byte`, then put a 1 in its last byte so it is not one
/// word repeated.
fn game_data(img: &mut [u8], block: usize, byte: u8) {
    img[block * 512..(block + 1) * 512].fill(byte);
    img[(block + 1) * 512 - 1] = 1;
}

#[test]
fn an_empty_volume_whose_free_blocks_hold_data_is_a_warning() {
    let mut img = blank();
    assert_eq!(held(img.clone()), 0);
    assert_eq!(findings(img.clone()), Vec::new());

    game_data(&mut img, 2, 0x4E);
    game_data(&mut img, 1500, 0x60);
    let found = findings(img.clone());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, ade_core::Severity::Warning);
    assert_eq!(
        found[0].message,
        "the volume holds no files, but 2 blocks the bitmap marks free hold data (2, 1500) — \
         on a disk like this it is usually a game its own loader reads, and writing to the \
         volume would overwrite it"
    );
    assert_eq!(held(img), 2);
}

#[test]
fn a_formatter_s_fill_is_not_data() {
    let mut img = blank();
    // AmigaDOS's Format writes `DOS\0` over every block; a PC's writes 0xE5.
    for (b, word) in [(2usize, *b"DOS\0"), (3, [0xE5; 4]), (1700, [0xFF; 4])] {
        for at in (b * 512..(b + 1) * 512).step_by(4) {
            img[at..at + 4].copy_from_slice(&word);
        }
    }
    assert_eq!(held(img.clone()), 0);
    assert_eq!(findings(img), Vec::new());
}

#[test]
fn free_blocks_holding_data_beside_files_are_not_reported() {
    // Deleted files leave data in free blocks on any disk that has been used;
    // only a volume with nothing on it says "a loader's".
    let mut s = Session::from_bytes(blank()).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    s.add_file(root, b"readme", b"text").unwrap();
    let mut img = s.bytes().to_vec();
    game_data(&mut img, 1500, 0x60);
    assert_eq!(held(img.clone()), 0);
    assert_eq!(findings(img), Vec::new());
}

#[test]
fn writing_over_free_data_is_refused_until_allowed() {
    let mut img = blank();
    for b in 882..900 {
        game_data(&mut img, b, 0x4E);
    }
    let mut s = Session::from_bytes(img).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    assert_eq!(s.free_blocks_holding_data(), 18);

    // Everything that takes or wipes a free block is refused, saying why.
    let refused = |e: EditError| matches!(e, EditError::FreeBlocksHoldData(18));
    assert!(refused(s.add_file(root, b"readme", b"text").unwrap_err()));
    assert!(refused(s.make_dir(root, b"Box").unwrap_err()));
    assert!(refused(s.wipe(false, true).unwrap_err()));
    let said = s
        .add_file(root, b"readme", b"text")
        .unwrap_err()
        .to_string();
    assert!(
        said.contains("18 blocks its bitmap marks free hold data"),
        "{said}"
    );
    assert!(!s.can_undo() && !s.is_dirty());

    // What writes no free block is not: slack has no file to wipe, and a
    // format gives the disk up as a whole.
    s.wipe(true, false).unwrap();
    s.format(Some(b"Reused")).unwrap();
    assert_eq!(s.free_blocks_holding_data(), 18);
    assert!(s.add_file(root, b"readme", b"text").is_err());

    // Allowed: the file goes onto the disk, and with an entry on it the
    // volume is no longer an empty one.
    s.allow_overwriting_free_data();
    s.add_file(root, b"readme", b"text").unwrap();
    assert_eq!(s.free_blocks_holding_data(), 0);
    assert_eq!(findings(s.bytes().to_vec()), Vec::new());

    // Undone, the count comes back with the empty volume; the allowance
    // stays for the session.
    assert!(s.undo());
    assert_eq!(s.free_blocks_holding_data(), 18);
    s.make_dir(root, b"Box").unwrap();
}

#[test]
fn free_blocks_holding_data_across_the_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let mut disks: BTreeMap<String, usize> = BTreeMap::new();
    let mut emptied = 0usize;
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let report = ade_core::health::examine(bytes);
        let Some(n) = report.bitmap.as_ref().map(|b| b.free_holding_data) else {
            continue;
        };
        if n == 0 {
            continue;
        }
        // Only ever an empty volume.
        assert_eq!(
            (report.files, report.directories),
            (0, 0),
            "{}",
            path.display()
        );
        // Every one a game: none only what deleting left.
        if report
            .findings
            .iter()
            .any(|f| f.code == "free-blocks-hold-deleted-files")
        {
            emptied += 1;
        }
        disks.insert(path.file_name().unwrap().to_string_lossy().into_owned(), n);
    }
    eprintln!("{} volumes: {disks:?}", disks.len());
    assert_eq!(emptied, 0);
    assert_eq!(disks.len(), 87);
    assert_eq!(disks.get("Paperboy.adf"), Some(&1720));
    // All but one hold hundreds of blocks there, not a stray one: Skyfox,
    // with 23.
    assert_eq!(disks.values().filter(|&&n| n < 100).count(), 1);
    assert_eq!(disks.get("Skyfox.adf"), Some(&23));
}

#[test]
fn a_volume_emptied_by_its_own_edits_is_not_refused() {
    // Deleting the last file leaves its data in free blocks on an empty
    // volume. Its header still lists them, so the report says what deleting
    // left, as information, not a game; and the session, which did the
    // deleting, does not hold the disk.
    let mut s = Session::from_bytes(blank()).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let file = s.add_file(root, b"readme", &[0x4E; 2000]).unwrap();
    s.delete(file).unwrap();
    assert_eq!(held(s.bytes().to_vec()), 6);
    assert_eq!(findings(s.bytes().to_vec()), Vec::new());
    let info: Vec<_> = ade_core::health::examine(s.bytes().to_vec())
        .findings
        .into_iter()
        .filter(|f| f.code == "free-blocks-hold-deleted-files")
        .collect();
    assert_eq!(info.len(), 1, "{info:?}");
    assert_eq!(info[0].severity, ade_core::Severity::Info);
    assert_eq!(
        info[0].message,
        "the volume holds no files, but 6 blocks the bitmap marks free hold what deleting \
         left (882, 883, 884, 885, 886, 887) — `ade carve` may recover it, and writing to the \
         volume would overwrite it"
    );
    assert_eq!(s.free_blocks_holding_data(), 0);

    // Opened afresh, the disk is held: a game mastered as files and then
    // cleared looks the same, and a question costs less than the game.
    let reopened = Session::from_bytes(s.bytes().to_vec()).unwrap();
    assert_eq!(reopened.free_blocks_holding_data(), 6);

    // A block no header lists is a loader's again, and then every block
    // counts — a disk mastered by writing files and clearing the directory
    // leaves its loader reading what the headers list.
    let mut mastered = s.bytes().to_vec();
    game_data(&mut mastered, 1500, 0x60);
    let found = findings(mastered.clone());
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0]
            .message
            .contains("hold data (882, 883, 884, 885, 886, 887, and 1 more; 6 of them listed by"),
        "{}",
        found[0].message
    );
    assert_eq!(held(mastered), 7);

    s.add_file(root, b"again", b"text").unwrap();
    s.format(None).unwrap();
    s.make_dir(root, b"Box").unwrap();
}
