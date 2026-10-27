//! The rest of the write suite (F-031): moving between folders, formatting
//! in place, and wiping slack and free space.
//!
//! As `edit.rs` checks the rest of the suite: ADE's health report must find
//! nothing new, and ADFlib — which nobody here wrote — must read the result
//! the same way. A move ADFlib cannot find by name in its new folder is not
//! a move; a wipe that changes one byte ADFlib extracts is not a wipe.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding; whole disks are compared with assert!, since assert_eq! would \
              print 880 KB of each"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::edit::{EditError, Session};
use ade_core::layers::block::Geometry;
use ade_core::layers::block::checksum::normal_at;
use ade_core::layers::endian::{put_u32, u32_at};
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::edit::EditError as FsEditError;
use ade_core::layers::filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn session(flags: u8) -> Session {
    let dostype = Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap();
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Suite", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    s
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

fn warnings(bytes: &[u8]) -> Vec<String> {
    ade_core::health::examine(bytes.to_vec())
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

/// Every file ADE reads in full, by path.
fn ade_files(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let walk = volume.walk(volume.root()).unwrap();
    walk.entries
        .into_iter()
        .filter(|(_, e)| e.kind.is_file())
        .filter_map(|(path, e)| {
            let c = volume.read_file(&e).ok()?;
            (c.is_complete() && c.faults.is_empty()).then(|| (path, c.into_bytes()))
        })
        .collect()
}

/// Every file `unadf` extracts, by path; `None` when it cannot or is not
/// installed. Capped: an uncapped run once took a session down.
fn adflib_files(bytes: &[u8], scratch: &Path) -> Option<BTreeMap<String, Vec<u8>>> {
    let _ = std::fs::remove_dir_all(scratch);
    let out = scratch.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let image = scratch.join("disk.adf");
    std::fs::write(&image, bytes).unwrap();
    let status = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 1048576; exec timeout 20 unadf \"$1\" -d \"$2\" >/dev/null 2>&1")
        .arg("sh")
        .arg(&image)
        .arg(&out)
        .status()
        .ok()?;
    status.success().then_some(())?;
    let mut files = BTreeMap::new();
    let mut stack = vec![out.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(&out).ok()?.to_string_lossy().into_owned();
                files.insert(rel, std::fs::read(&p).ok()?);
            }
        }
    }
    let _ = std::fs::remove_dir_all(scratch);
    Some(files)
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ade-suite-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// What `unadf -p` prints for one path; `None` when it finds nothing.
fn adflib_read(bytes: &[u8], path: &str) -> Option<Vec<u8>> {
    let dir = scratch("p");
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("disk.adf");
    std::fs::write(&image, bytes).unwrap();
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 1048576; exec timeout 20 unadf -p \"$1\" \"$2\"")
        .arg("sh")
        .arg(&image)
        .arg(path)
        .output()
        .ok();
    let _ = std::fs::remove_dir_all(&dir);
    out.map(|o| o.stdout).filter(|b| !b.is_empty())
}

/// Every orphaned header `carve` finds: its name, and its contents when the
/// disk backs them up.
fn carved(bytes: &[u8]) -> Vec<(String, Option<Vec<u8>>)> {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    ade_core::carve::carve(&image)
        .iter()
        .map(|c| {
            let data =
                ade_core::carve::is_recoverable(c).then(|| ade_core::carve::contents(&image, c));
            (c.name.clone(), data)
        })
        .collect()
}

fn oracle() -> bool {
    Command::new("unadf").output().is_ok()
}

// --- Moving ------------------------------------------------------------------

#[test]
fn a_file_and_a_folder_move_on_every_filesystem_and_adflib_finds_them() {
    for flags in 0..6u8 {
        let mut s = session(flags);
        let root = s.root();
        let from = s.make_dir(root, b"From").unwrap();
        let to = s.make_dir(root, b"To").unwrap();
        // An accented name: hashed differently by the international function,
        // so a move that filed it with the wrong one shows by name lookup.
        let name: &[u8] = b"r\xE9sum\xE9";
        let data = pattern(1500, flags);
        let file = s.add_file(from, name, &data).unwrap();
        let inner = s.make_dir(from, b"Inner").unwrap();
        s.add_file(inner, b"deep", b"deep data").unwrap();

        s.move_entry(file, to).unwrap();
        s.move_entry(inner, to).unwrap();
        assert_eq!(warnings(s.bytes()), Vec::<String>::new(), "DOS\\{flags}");

        let files = ade_files(s.bytes());
        assert_eq!(files["To/résumé"], data, "DOS\\{flags}");
        assert_eq!(files["To/Inner/deep"], b"deep data");
        assert!(!files.keys().any(|k| k.starts_with("From/")), "{files:?}");
        if oracle() {
            assert_eq!(
                adflib_read(s.bytes(), "To/Inner/deep").as_deref(),
                Some(&b"deep data"[..]),
                "DOS\\{flags}: ADFlib finds the moved folder's file by name"
            );
            // ADFlib writes the name's Latin-1 bytes as they are, which a
            // UTF-8 path reads back lossily.
            let theirs = adflib_files(s.bytes(), &scratch("move")).unwrap();
            assert!(
                theirs.get("To/r\u{FFFD}sum\u{FFFD}") == Some(&data),
                "DOS\\{flags}: ADFlib has {:?}",
                theirs.keys().collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn a_folder_does_not_go_inside_itself_and_a_taken_name_is_refused() {
    let mut s = session(3);
    let root = s.root();
    let a = s.make_dir(root, b"A").unwrap();
    let b = s.make_dir(a, b"B").unwrap();
    let before = s.bytes().to_vec();
    assert!(matches!(
        s.move_entry(a, b),
        Err(EditError::Refused(FsEditError::NotRemovable(_)))
    ));
    assert!(matches!(
        s.move_entry(a, a),
        Err(EditError::Refused(FsEditError::NotRemovable(_)))
    ));
    assert!(matches!(
        s.move_entry(b, a),
        Err(EditError::Refused(FsEditError::NotNeeded(_)))
    ));
    s.make_dir(root, b"b").unwrap();
    let before_clash = s.bytes().to_vec();
    assert!(matches!(
        s.move_entry(b, root),
        Err(EditError::Refused(FsEditError::Exists { .. }))
    ));
    assert!(
        s.bytes() == before_clash.as_slice(),
        "a refusal changes nothing"
    );
    assert!(matches!(
        s.move_entry(root, a),
        Err(EditError::Refused(FsEditError::NotRemovable(_)))
    ));
    // And undo takes the one move back.
    s.move_entry(b, root).unwrap_err();
    assert!(s.undo());
    assert!(s.bytes() == before.as_slice());
}

// --- Formatting in place -----------------------------------------------------

#[test]
fn a_format_in_place_empties_the_disk_and_leaves_its_blocks_to_be_carved() {
    for flags in [0u8, 1, 4, 5] {
        let mut s = session(flags);
        let root = s.root();
        let data = pattern(2000, 7);
        s.add_file(root, b"keepsake", &data).unwrap();
        let dir = s.make_dir(root, b"Folder").unwrap();
        s.add_file(dir, b"inner", b"x").unwrap();

        s.format(None).unwrap();
        assert_eq!(warnings(s.bytes()), Vec::<String>::new(), "DOS\\{flags}");
        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        let volume = image.volume().unwrap();
        assert_eq!(volume.rootblock().name, b"Suite", "the name is kept");
        assert_eq!(
            volume.dostype().unwrap().raw(),
            0x444F_5300 | u32::from(flags)
        );
        assert_eq!(volume.list(volume.root()).unwrap().entries.len(), 0);
        if oracle() {
            assert_eq!(
                adflib_files(s.bytes(), &scratch("fmt")),
                Some(BTreeMap::new())
            );
        }
        // Quick, as AmigaDOS's is: the old file is still there to carve. OFS
        // proves its blocks; FFS can only offer the header.
        let found = carved(s.bytes());
        let keepsake = found
            .iter()
            .find(|(name, _)| name == "keepsake")
            .unwrap_or_else(|| panic!("DOS\\{flags}: {found:?}"));
        if flags % 2 == 0 {
            assert_eq!(keepsake.1.as_deref(), Some(&data[..]));
        }
        assert!(s.undo());
        assert_eq!(ade_files(s.bytes())["keepsake"], data);
    }
}

#[test]
fn a_format_takes_a_new_name_and_mends_a_disk_nothing_else_would_edit() {
    let mut s = session(1);
    let root = s.root();
    let header = s.add_file(root, b"victim", &pattern(100, 1)).unwrap();
    let mut bytes = s.bytes().to_vec();
    // Mark the file's data block free: unsound, so no ordinary edit is made.
    let data = {
        let image = ade_core::Image::from_bytes(bytes.clone()).unwrap();
        let volume = image.volume().unwrap();
        let entry = volume.entry_at(header).unwrap();
        volume.file_blocks(&entry).unwrap()[1]
    };
    let index = (data - 2) as usize;
    let at = 881 * 512 + 4 + (index / 32) * 4;
    let word = u32_at(&bytes, at).unwrap() | 1 << (index % 32);
    put_u32(&mut bytes, at, word).unwrap();
    let sum = normal_at(&bytes[881 * 512..882 * 512], 0).unwrap();
    put_u32(&mut bytes, 881 * 512, sum).unwrap();

    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    assert!(matches!(s.make_dir(root, b"x"), Err(EditError::Unsound(_))));
    s.format(Some(b"Fresh")).unwrap();
    assert_eq!(warnings(s.bytes()), Vec::<String>::new());
    let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
    assert_eq!(image.volume().unwrap().rootblock().name, b"Fresh");
    s.make_dir(root, b"x").unwrap();
    assert!(matches!(
        s.format(Some(b"bad:name")),
        Err(EditError::NotEditable(_))
    ));
}

// --- Wiping ------------------------------------------------------------------

/// Fill the slack of `path`'s last data block with `$AA`, as an older,
/// longer version of the file would have left it.
fn dirty_slack(bytes: &mut [u8], path: &str, ofs: bool) {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    let last = *volume.data_blocks(&entry).unwrap().last().unwrap() as usize;
    let cap = if ofs { 488 } else { 512 };
    let used = (entry.byte_size as usize - 1) % cap + 1;
    let from = last * 512 + if ofs { 24 } else { 0 } + used;
    bytes[from..(last + 1) * 512].fill(0xAA);
    if ofs {
        put_u32(bytes, last * 512 + 20, 0).unwrap();
        let sum = normal_at(&bytes[last * 512..(last + 1) * 512], 20).unwrap();
        put_u32(bytes, last * 512 + 20, sum).unwrap();
    }
}

#[test]
fn wiping_slack_zeroes_file_tails_and_changes_no_file() {
    for flags in 0..6u8 {
        let ofs = flags % 2 == 0;
        let mut s = session(flags);
        let root = s.root();
        s.add_file(root, b"short", &pattern(100, 1)).unwrap();
        s.add_file(root, b"exact", &pattern(if ofs { 976 } else { 1024 }, 2))
            .unwrap();
        s.add_file(root, b"long", &pattern(40_000, 3)).unwrap();
        let mut bytes = s.bytes().to_vec();
        dirty_slack(&mut bytes, "short", ofs);
        dirty_slack(&mut bytes, "long", ofs);
        let files = ade_files(&bytes);

        let mut s = Session::from_bytes(bytes.clone()).unwrap();
        let wiped = s.wipe(true, false).unwrap().slack.unwrap();
        assert_eq!(wiped.blocks, 2, "DOS\\{flags}: {wiped:?}");
        assert!(wiped.skipped.is_empty(), "{wiped:?}");
        assert_eq!(ade_files(s.bytes()), files, "DOS\\{flags}");
        assert_eq!(warnings(s.bytes()), Vec::<String>::new());
        assert!(!s.bytes().contains(&0xAA) || s.bytes().windows(4).all(|w| w != [0xAA; 4]));
        if oracle() {
            assert_eq!(
                adflib_files(s.bytes(), &scratch("slack")).unwrap(),
                files,
                "DOS\\{flags}"
            );
        }
        // Nothing left to wipe the second time.
        assert_eq!(s.wipe(true, false).unwrap().slack.unwrap().blocks, 0);
    }
}

#[test]
fn wiping_free_space_destroys_what_a_deleted_file_left_and_nothing_else() {
    for flags in [0u8, 1] {
        let mut s = session(flags);
        let root = s.root();
        let kept = pattern(3000, 9);
        s.add_file(root, b"kept", &kept).unwrap();
        let gone = s.add_file(root, b"gone", &pattern(3000, 5)).unwrap();
        s.delete(gone).unwrap();
        // Deleted, but still there to carve.
        assert!(carved(s.bytes()).iter().any(|(name, _)| name == "gone"));

        let wiped = s.wipe(false, true).unwrap().free.unwrap();
        assert!(wiped.blocks >= 7, "DOS\\{flags}: {wiped:?}");
        assert_eq!(ade_files(s.bytes())["kept"], kept);
        assert_eq!(warnings(s.bytes()), Vec::<String>::new());
        let after = carved(s.bytes());
        assert!(!after.iter().any(|(name, _)| name == "gone"), "{after:?}");
        assert_eq!(s.wipe(false, true).unwrap().free.unwrap().blocks, 0);
    }
}

#[test]
fn free_space_is_not_wiped_through_a_bitmap_that_cannot_be_trusted() {
    let mut s = session(1);
    let mut bytes = s.bytes().to_vec();
    put_u32(&mut bytes, 880 * 512 + 512 - 200, 0).unwrap();
    put_u32(&mut bytes, 880 * 512 + 20, 0).unwrap();
    let sum = normal_at(&bytes[880 * 512..881 * 512], 20).unwrap();
    put_u32(&mut bytes, 880 * 512 + 20, sum).unwrap();
    s = Session::from_bytes(bytes.clone()).unwrap();
    assert!(matches!(
        s.wipe(false, true),
        Err(EditError::Refused(FsEditError::BitmapUntrusted(_)))
    ));
    assert!(s.bytes() == bytes.as_slice());

    // Both or neither: slack that could be wiped is not, when free space
    // cannot be.
    let root = s.root();
    let mut fresh = session(1);
    fresh.add_file(root, b"f", &pattern(100, 4)).unwrap();
    let mut bytes = fresh.bytes().to_vec();
    dirty_slack(&mut bytes, "f", false);
    put_u32(&mut bytes, 880 * 512 + 512 - 200, 0).unwrap();
    put_u32(&mut bytes, 880 * 512 + 20, 0).unwrap();
    let sum = normal_at(&bytes[880 * 512..881 * 512], 20).unwrap();
    put_u32(&mut bytes, 880 * 512 + 20, sum).unwrap();
    let mut s = Session::from_bytes(bytes.clone()).unwrap();
    assert!(s.wipe(true, true).is_err());
    assert!(s.bytes() == bytes.as_slice() && !s.can_undo());
    assert_eq!(s.wipe(true, false).unwrap().blocks(), 1);
}

// --- The corpus, local only --------------------------------------------------

#[test]
fn corpus_disks_wipe_without_changing_a_file() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let use_oracle = oracle();
    let (mut disks, mut slack, mut free, mut confirmed) = (0usize, 0usize, 0usize, 0usize);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(mut s) = Session::open(path) else {
            continue;
        };
        let before = s.bytes().to_vec();
        let ours = ade_files(&before);
        let theirs = if use_oracle {
            adflib_files(&before, &scratch("corpus-before"))
        } else {
            None
        };
        disks += 1;
        match s.wipe(true, false) {
            Ok(w) => slack += usize::from(w.blocks() > 0),
            Err(EditError::WouldDamage(found)) => panic!("{}: {found:?}", path.display()),
            Err(e) => {
                *refused
                    .entry(format!("slack: {e}").chars().take(40).collect())
                    .or_default() += 1;
            }
        }
        match s.wipe(false, true) {
            Ok(w) => free += usize::from(w.blocks() > 0),
            Err(EditError::WouldDamage(found)) => panic!("{}: {found:?}", path.display()),
            Err(e) => {
                *refused
                    .entry(format!("free: {e}").chars().take(40).collect())
                    .or_default() += 1;
            }
        }
        let after = ade_files(s.bytes());
        for (p, data) in &ours {
            assert!(
                after.get(p) == Some(data),
                "{}: {p} changed in a wipe",
                path.display()
            );
        }
        let Some(theirs) = theirs else { continue };
        let Some(now) = adflib_files(s.bytes(), &scratch("corpus-after")) else {
            panic!(
                "{}: ADFlib read it before the wipe and not after",
                path.display()
            );
        };
        for (p, data) in ours.iter().filter(|(p, d)| theirs.get(*p) == Some(*d)) {
            assert!(
                now.get(p) == Some(data),
                "{}: ADFlib reads {p} differently after the wipe",
                path.display()
            );
        }
        confirmed += 1;
    }
    eprintln!(
        "{disks} editable disks: slack wiped on {slack}, free space on {free}; ADFlib confirmed \
         {confirmed}; refused {refused:?}"
    );
    assert!(disks > 3000, "{disks}");
}

#[test]
fn corpus_files_move_into_a_new_folder_and_adflib_finds_them_there() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let use_oracle = oracle();
    let (mut moved, mut confirmed) = (0usize, 0usize);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(mut s) = Session::open(path) else {
            continue;
        };
        // The first root file ADE reads in full with a plain ASCII name, so
        // ADFlib can be asked for it by name on the command line.
        let files = ade_files(s.bytes());
        let Some((name, data)) = files.iter().find(|(p, _)| {
            !p.contains('/')
                && !p.starts_with('-')
                && p.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        }) else {
            continue;
        };
        let root = s.root();
        let block = match s.entry(name) {
            Ok(b) => b,
            Err(e) => {
                *refused
                    .entry(format!("lookup: {e}").chars().take(40).collect())
                    .or_default() += 1;
                continue;
            }
        };
        let dir = match s.make_dir(root, b"ADE-moved") {
            Ok(d) => d,
            Err(e) => {
                *refused
                    .entry(format!("{e}").chars().take(40).collect())
                    .or_default() += 1;
                continue;
            }
        };
        match s.move_entry(block, dir) {
            Ok(()) => {}
            Err(EditError::WouldDamage(found)) => panic!("{}: {found:?}", path.display()),
            Err(e) => {
                *refused
                    .entry(format!("move: {e}").chars().take(40).collect())
                    .or_default() += 1;
                continue;
            }
        }
        moved += 1;
        let target = format!("ADE-moved/{name}");
        assert!(
            ade_files(s.bytes()).get(&target) == Some(data),
            "{}: {target}",
            path.display()
        );
        if use_oracle && let Some(theirs) = adflib_read(s.bytes(), &target) {
            assert!(
                theirs == *data,
                "{}: ADFlib reads {target} differently",
                path.display()
            );
            confirmed += 1;
        }
    }
    eprintln!(
        "moved a file on {moved} disks, ADFlib found {confirmed} by name; refused {refused:?}"
    );
    assert!(moved > 2500, "{moved}");
}

/// The last data block of the file at `path`.
fn last_block(bytes: &[u8], path: &str) -> usize {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    *volume.data_blocks(&entry).unwrap().last().unwrap() as usize
}

#[test]
fn slack_is_left_in_a_block_another_file_uses_or_another_header_owns() {
    use ade_core::layers::filesystem::wipe;
    // OFS, so a data block names its owner.
    let mut s = session(0);
    let root = s.root();
    let a = s.add_file(root, b"a", &pattern(100, 1)).unwrap();
    s.add_file(root, b"b", &pattern(100, 2)).unwrap();
    let mut bytes = s.bytes().to_vec();
    dirty_slack(&mut bytes, "a", true);
    dirty_slack(&mut bytes, "b", true);

    // `b`'s only data block claimed by `a`'s header too: cross-linked.
    let (la, lb) = (last_block(&bytes, "a"), last_block(&bytes, "b"));
    let mut shared = bytes.clone();
    put_u32(&mut shared, a as usize * 512 + 24 + 71 * 4, lb as u32).unwrap();
    put_u32(&mut shared, a as usize * 512 + 16, lb as u32).unwrap();
    put_u32(&mut shared, a as usize * 512 + 20, 0).unwrap();
    let sum = normal_at(&shared[a as usize * 512..][..512], 20).unwrap();
    put_u32(&mut shared, a as usize * 512 + 20, sum).unwrap();
    let w = wipe::slack(&mut shared, Geometry::DD_FLOPPY).unwrap();
    assert!(
        w.skipped
            .iter()
            .filter(|s| s.contains("another file uses"))
            .count()
            == 2,
        "{w:?}"
    );
    assert_eq!(w.blocks, 0);

    // `a`'s last block naming another header as its owner.
    let mut owned = bytes.clone();
    put_u32(&mut owned, la * 512 + 4, 999).unwrap();
    put_u32(&mut owned, la * 512 + 20, 0).unwrap();
    let sum = normal_at(&owned[la * 512..][..512], 20).unwrap();
    put_u32(&mut owned, la * 512 + 20, sum).unwrap();
    let w = wipe::slack(&mut owned, Geometry::DD_FLOPPY).unwrap();
    assert_eq!(w.blocks, 1, "only b's: {w:?}");
    assert!(w.skipped[0].starts_with("a:"), "{w:?}");
}

#[test]
fn a_free_space_wipe_never_zeroes_a_block_in_use_whatever_the_bitmap_says() {
    use ade_core::layers::filesystem::wipe;
    // Below the session, which refuses such a disk: the filesystem layer
    // does not rely on that.
    let mut s = session(1);
    let root = s.root();
    let data = pattern(100, 3);
    s.add_file(root, b"live", &data).unwrap();
    let mut bytes = s.bytes().to_vec();
    let block = last_block(&bytes, "live");
    let index = block - 2;
    let at = 881 * 512 + 4 + (index / 32) * 4;
    let word = u32_at(&bytes, at).unwrap() | 1 << (index % 32);
    put_u32(&mut bytes, at, word).unwrap();
    let sum = normal_at(&bytes[881 * 512..882 * 512], 0).unwrap();
    put_u32(&mut bytes, 881 * 512, sum).unwrap();
    let in_use: std::collections::HashSet<u32> = [block as u32].into();
    wipe::free_space(&mut bytes, Geometry::DD_FLOPPY, &in_use).unwrap();
    assert_eq!(ade_files(&bytes)["live"], data);
}
