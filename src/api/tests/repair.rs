//! The repair suite: the bitmap, and entries filed under the wrong hash slot.
//!
//! A repair puts back what the volume's own structures say, so the built
//! cases damage a disk ADE wrote and require the repair to give back **the
//! same bytes** — not a disk that merely passes ADE's check. Where ADFlib can
//! see the difference it is asked: a misfiled entry is one it cannot open by
//! name, and can after.
//!
//! The corpus half (local only) repairs every disk that needs it and then
//! **fills the free space** with one file. A bitmap that still marked a live
//! block free would put the filler over it, so every file ADFlib read before
//! must read the same after. That is the test of a bitmap that matters: not
//! what it says, but what the next write does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding; and whole disks are compared with assert!, since assert_eq! \
              would print 880 KB of each on failure"
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
use ade_core::layers::filesystem::hash::hash_name;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};
const ROOT: usize = 880 * 512;
const BITMAP: usize = 881 * 512;

/// A disk holding a directory and two files, made by ADE's writer.
fn disk(flags: u8) -> Vec<u8> {
    let dostype = Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap();
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Mend", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let dir = s.make_dir(root, b"Dir").unwrap();
    s.add_file(root, b"hello.txt", &[7u8; 3000]).unwrap();
    s.add_file(dir, b"inner", &[9u8; 700]).unwrap();
    s.bytes().to_vec()
}

fn session(bytes: Vec<u8>) -> Session {
    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    s
}

fn reseal_root(bytes: &mut [u8]) {
    put_u32(bytes, ROOT + 20, 0).unwrap();
    let sum = normal_at(&bytes[ROOT..ROOT + 512], 20).unwrap();
    put_u32(bytes, ROOT + 20, sum).unwrap();
}

fn reseal_bitmap(bytes: &mut [u8]) {
    put_u32(bytes, BITMAP, 0).unwrap();
    let sum = normal_at(&bytes[BITMAP..BITMAP + 512], 0).unwrap();
    put_u32(bytes, BITMAP, sum).unwrap();
}

/// Flip the bitmap bit for `block`; set means free.
fn flip(bytes: &mut [u8], block: u32) {
    let index = (block - 2) as usize;
    let at = BITMAP + 4 + (index / 32) * 4;
    let word = u32_at(bytes, at).unwrap() ^ (1 << (index % 32));
    put_u32(bytes, at, word).unwrap();
}

fn codes(bytes: &[u8]) -> Vec<&'static str> {
    let mut v: Vec<_> = ade_core::health::examine(bytes.to_vec())
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| f.code)
        .collect();
    v.sort_unstable();
    v
}

/// The first data block of the file at `path`.
fn data_block(bytes: &[u8], path: &str) -> u32 {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    volume.file_blocks(&entry).unwrap()[1]
}

#[test]
fn a_sound_disk_is_left_alone() {
    let mut s = session(disk(1));
    let before = s.bytes().to_vec();
    let done = s.repair(true).unwrap();
    assert!(done.is_empty() && done.declined.is_empty(), "{done:?}");
    assert_eq!(done.lines(), vec!["nothing to repair".to_owned()]);
    assert!(s.bytes() == before.as_slice());
    assert!(!s.is_dirty() && !s.can_undo());
}

#[test]
fn a_cleared_flag_is_set_again_and_nothing_else_changes() {
    // 259 corpus disks: the commonest damage there is, and harmless until a
    // write — which is why ADE refuses to write to one.
    for flags in 0..6 {
        let original = disk(flags);
        let mut bytes = original.clone();
        put_u32(&mut bytes, ROOT + 512 - 200, 0).unwrap();
        reseal_root(&mut bytes);
        assert_eq!(codes(&bytes), vec!["bitmap-flag-clear"]);

        let mut s = session(bytes);
        let root = s.root();
        assert!(matches!(
            s.make_dir(root, b"x"),
            Err(EditError::Refused(FsEditError::BitmapUntrusted(_)))
        ));
        let done = s.repair(false).unwrap();
        let b = done.bitmap.as_ref().unwrap();
        assert!(b.flag_set && b.rewritten.is_empty() && b.now_used.is_empty());
        assert!(
            s.bytes() == original.as_slice(),
            "DOS\\{flags}: the disk as it was"
        );
        s.make_dir(root, b"x").unwrap();
    }
}

#[test]
fn a_live_block_marked_free_in_a_bitmap_that_fails_its_checksum_comes_back() {
    for flags in 0..6 {
        let original = disk(flags);
        let block = data_block(&original, "Dir/inner");
        let mut bytes = original.clone();
        flip(&mut bytes, block);
        assert_eq!(
            codes(&bytes),
            vec!["bitmap-checksum", "referenced-but-free"]
        );

        let mut s = session(bytes.clone());
        let done = s.repair(false).unwrap();
        let b = done.bitmap.as_ref().unwrap();
        assert_eq!(b.now_used, vec![block]);
        assert_eq!(b.rewritten, vec![881]);
        assert!(!b.flag_set);
        assert!(
            s.bytes() == original.as_slice(),
            "DOS\\{flags}: the disk as it was"
        );

        // One step, and undo is the damaged disk again.
        assert!(s.undo());
        assert!(s.bytes() == bytes.as_slice());
    }
}

#[test]
fn an_orphan_stays_used_unless_asked_and_then_is_freed() {
    let original = disk(3);
    let mut bytes = original.clone();
    flip(&mut bytes, 1500);
    reseal_bitmap(&mut bytes);
    assert_eq!(codes(&bytes), vec!["orphaned-blocks"]);

    let mut s = session(bytes);
    let kept = s.repair(false).unwrap();
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(kept.orphans_kept, 1);

    let freed = s.repair(true).unwrap();
    assert_eq!(freed.bitmap.unwrap().now_free, vec![1500]);
    assert!(s.bytes() == original.as_slice());

    // Kept even when the map is being rewritten for another reason.
    let mut bytes = original.clone();
    flip(&mut bytes, 1500);
    flip(&mut bytes, data_block(&original, "hello.txt"));
    reseal_bitmap(&mut bytes);
    let mut s = session(bytes);
    let done = s.repair(false).unwrap();
    let b = done.bitmap.unwrap();
    assert!(b.now_free.is_empty() && b.now_used.len() == 1, "{b:?}");
    let health = ade_core::health::examine(s.bytes().to_vec());
    assert_eq!(health.bitmap.unwrap().orphaned_blocks, vec![1500]);
}

#[test]
fn a_bitmap_block_that_only_fails_its_checksum_is_resealed() {
    let original = disk(0);
    let mut bytes = original.clone();
    bytes[BITMAP + 1] ^= 0x40;
    assert_eq!(codes(&bytes), vec!["bitmap-checksum"]);
    let mut s = session(bytes);
    let done = s.repair(false).unwrap();
    let b = done.bitmap.clone().unwrap_or_else(|| panic!("{done:?}"));
    assert_eq!(b.rewritten, vec![881]);
    assert!(b.now_used.is_empty() && b.now_free.is_empty());
    assert!(s.bytes() == original.as_slice());
}

/// Move whatever the root's hash slot `from` holds into slot `to`.
fn move_slot(bytes: &mut [u8], from: u32, to: u32) {
    let at = |slot: u32| ROOT + 24 + slot as usize * 4;
    let entry = u32_at(bytes, at(from)).unwrap();
    put_u32(bytes, at(from), 0).unwrap();
    put_u32(bytes, at(to), entry).unwrap();
    reseal_root(bytes);
}

/// What `unadf -p` prints for `path`; `None` when it finds nothing or is
/// not installed. Capped: an uncapped run once took a session down.
fn adflib_read(bytes: &[u8], path: &str, tag: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("ade-repair-{tag}-{}", std::process::id()));
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

#[test]
fn a_misfiled_entry_goes_back_to_its_slot_and_adflib_opens_it() {
    for flags in 0..6 {
        let original = disk(flags);
        let right = hash_name(b"hello.txt", 72, flags >= 2);
        let mut bytes = original.clone();
        move_slot(&mut bytes, right, (right + 9) % 72);
        assert_eq!(codes(&bytes), vec!["entry-wrong-slot"]);
        let unadf_before = adflib_read(&bytes, "hello.txt", "before");

        let mut s = session(bytes);
        let done = s.repair(false).unwrap();
        assert_eq!(done.refiled, vec!["hello.txt".to_owned()]);
        assert!(
            s.bytes() == original.as_slice(),
            "DOS\\{flags}: the disk as it was"
        );
        if let Some(after) = adflib_read(s.bytes(), "hello.txt", "after") {
            assert!(unadf_before.is_none(), "ADFlib could not open it before");
            assert_eq!(after, vec![7u8; 3000]);
        }
    }
}

#[test]
fn a_misfiled_entry_whose_header_fails_its_checksum_is_left_where_it_is() {
    // Refiling rewrites its chain pointer and reseals it, which would make a
    // damaged header look sound.
    let original = disk(1);
    let right = hash_name(b"hello.txt", 72, false);
    let mut bytes = original.clone();
    move_slot(&mut bytes, right, (right + 9) % 72);
    let header = u32_at(&bytes, ROOT + 24 + ((right + 9) % 72) as usize * 4).unwrap() as usize;
    bytes[header * 512 + 20] ^= 1;

    let mut s = session(bytes.clone());
    let done = s.repair(false).unwrap();
    assert_eq!(done.refiled, Vec::<String>::new());
    assert!(
        done.declined[0].contains("fails its checksum"),
        "{:?}",
        done.declined
    );
    assert!(s.bytes() == bytes.as_slice());
}

#[test]
fn a_misfiled_entry_sharing_its_name_is_left_where_it_is() {
    // Every misfiled entry the corpus declines is this: refiling would
    // change which copy AmigaDOS opens, which is the owner's choice.
    let original = disk(1);
    let mut s = session(original);
    let root = s.root();
    let second = s.add_file(root, b"bbb", b"other").unwrap();
    let mut bytes = s.bytes().to_vec();
    let at = second as usize * 512 + 512 - 80;
    bytes[at] = 9;
    bytes[at + 1..at + 10].copy_from_slice(b"HELLO.TXT");
    put_u32(&mut bytes, second as usize * 512 + 20, 0).unwrap();
    let sum = normal_at(&bytes[second as usize * 512..][..512], 20).unwrap();
    put_u32(&mut bytes, second as usize * 512 + 20, sum).unwrap();

    let mut s = session(bytes.clone());
    let done = s.repair(false).unwrap();
    assert!(done.is_empty());
    assert_eq!(done.declined.len(), 1, "{done:?}");
    assert!(
        done.declined[0].contains("another entry is also called"),
        "{}",
        done.declined[0]
    );
    assert!(s.bytes() == bytes.as_slice());
}

#[test]
fn the_bitmap_is_left_when_the_tree_cannot_be_read_in_full() {
    // A block the walk missed would be marked free: AmigaDOS's validator
    // stops here too.
    let mut bytes = disk(1);
    put_u32(&mut bytes, ROOT + 512 - 200, 0).unwrap();
    let empty = (0..72)
        .find(|&s| u32_at(&bytes, ROOT + 24 + s * 4).unwrap() == 0)
        .unwrap();
    put_u32(&mut bytes, ROOT + 24 + empty * 4, 5_000_000).unwrap();
    reseal_root(&mut bytes);

    let mut s = session(bytes.clone());
    let done = s.repair(false).unwrap();
    assert!(done.bitmap.is_none());
    assert!(
        done.declined[0].contains("cannot be listed in full"),
        "{:?}",
        done.declined
    );
    assert!(s.bytes() == bytes.as_slice());
}

#[test]
fn a_rootblock_failing_its_checksum_is_not_resealed() {
    let mut bytes = disk(1);
    put_u32(&mut bytes, ROOT + 512 - 200, 0).unwrap();
    reseal_root(&mut bytes);
    bytes[ROOT + 20] ^= 1;

    let mut s = session(bytes.clone());
    let done = s.repair(false).unwrap();
    assert!(done.bitmap.is_none());
    assert!(
        done.declined[0].contains("fails its checksum"),
        "{:?}",
        done.declined
    );
    assert!(s.bytes() == bytes.as_slice());
}

// --- The corpus, local only -------------------------------------------------

/// Every file ADE reads in full, by path.
fn complete_files(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
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

/// Every file `unadf` extracts, by path; `None` when it cannot.
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
    Some(files)
}

/// Add one file as large as the free space takes. Returns its size.
fn fill(s: &mut Session) -> Option<usize> {
    let health = ade_core::health::examine(s.bytes().to_vec());
    let bitmap = health.bitmap?;
    let free = (bitmap.covered as usize).saturating_sub(bitmap.marked_used);
    let root = s.root();
    for share in [97usize, 90, 75, 50, 25] {
        let len = free * 488 * share / 100;
        match s.add_file(root, b"ADE-fill", &vec![0xA5; len]) {
            Ok(_) => return Some(len),
            Err(EditError::Refused(FsEditError::DiskFull { .. })) => {}
            Err(EditError::WouldDamage(found)) => {
                panic!("filling a repaired disk damaged it: {found:?}")
            }
            Err(_) => return None,
        }
    }
    None
}

#[test]
fn corpus_disks_repair_and_take_a_full_write_without_losing_a_file() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let oracle = Command::new("unadf").output().is_ok();
    let scratch = std::env::temp_dir().join(format!("ade-repair-corpus-{}", std::process::id()));
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut repaired, mut filled, mut confirmed, mut declined) = (0usize, 0usize, 0usize, 0usize);
    let (mut flags, mut live, mut refiled) = (0usize, 0usize, 0usize);
    for path in &paths {
        let Ok(mut s) = Session::open(path) else {
            continue;
        };
        let before = s.bytes().to_vec();
        let done = s
            .repair(false)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        declined += usize::from(!done.declined.is_empty());
        if done.is_empty() {
            assert!(s.bytes() == before.as_slice(), "{}", path.display());
            continue;
        }
        repaired += 1;
        refiled += done.refiled.len();
        if let Some(b) = &done.bitmap {
            flags += usize::from(b.flag_set);
            live += usize::from(!b.now_used.is_empty());
            assert!(b.now_free.is_empty(), "{}", path.display());
        }

        // A repair changes no file.
        let ours = complete_files(&before);
        let after = complete_files(s.bytes());
        for (p, data) in &ours {
            assert!(
                after.get(p) == Some(data),
                "{}: {p} changed in the repair",
                path.display()
            );
        }

        // ADFlib's reading before, of the files it and ADE agree on.
        let theirs = if oracle {
            adflib_files(&before, &scratch)
        } else {
            None
        };
        if fill(&mut s).is_none() {
            continue;
        }
        filled += 1;
        let after = complete_files(s.bytes());
        for (p, data) in &ours {
            assert!(
                after.get(p) == Some(data),
                "{}: the fill overwrote {p}",
                path.display()
            );
        }
        let Some(theirs) = theirs else { continue };
        let Some(now) = adflib_files(s.bytes(), &scratch) else {
            panic!(
                "{}: ADFlib read it before the fill and not after",
                path.display()
            );
        };
        for (p, data) in ours.iter().filter(|(p, d)| theirs.get(*p) == Some(*d)) {
            assert!(
                now.get(p) == Some(data),
                "{}: ADFlib reads {p} differently after the fill",
                path.display()
            );
        }
        confirmed += 1;
    }
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!(
        "repaired {repaired} ({flags} flags set, {live} with live blocks marked used, {refiled} \
         entries refiled), {declined} with something declined; filled {filled}, ADFlib confirmed \
         {confirmed}"
    );
    // Measured 2026-10-26: 270 repaired, 259 flags, 12 with live blocks.
    assert!((250..300).contains(&repaired), "{repaired}");
    assert!(live >= 10, "{live}");
    assert!(
        filled * 10 >= repaired * 9,
        "most repaired disks take a write: {filled}"
    );
}
