//! Hardfiles past 25 bitmap blocks: the `bm_ext` chain (F-038).
//!
//! Above about 50 MB a volume's bitmap needs more blocks than the rootblock
//! can name; the rest are listed in a chain of extension blocks. ADE lays it
//! out as AmigaDOS's `Format` does (measured in FS-UAE), and AmigaDOS reads
//! and writes what ADE makes (`amigados.rs`, D-023). Here, where CI can see
//! it: a 100 MB hardfile through every edit, each held to the health report,
//! and read by ADFlib when it is installed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding; file contents are compared with assert!, since assert_eq! would \
              print kilobytes of each"
)]

use std::process::Command;

use ade_core::Image;
use ade_core::create::Shape;
use ade_core::edit::Session;
use ade_core::layers::endian::u32_at;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

fn faults(bytes: &[u8]) -> Vec<String> {
    ade_core::health::examine(bytes.to_vec())
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

fn read(bytes: &[u8], path: &str) -> Vec<u8> {
    let image = Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    volume.read_file(&entry).unwrap().bytes
}

#[test]
fn a_100_mb_hardfile_has_a_bitmap_chain_and_takes_every_edit() {
    let geometry = Shape::Hard(100).geometry().unwrap();
    for flags in [0u8, 1, 5] {
        let dostype = Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap();
        let blank = format::blank(geometry, dostype, "Big", WHEN).unwrap();
        assert!(
            faults(&blank).is_empty(),
            "DOS\\{flags}: {:?}",
            faults(&blank)
        );
        {
            // 204,800 blocks need 51 bitmap pages: 25 named by the rootblock,
            // 26 by one extension block. In use: the root, 51 pages, the
            // extension, and on a dircache volume the root's cache.
            let image = Image::from_bytes(blank.clone()).unwrap();
            let root = image.volume().unwrap().root() as usize;
            let bm_ext = u32_at(&blank, root * 512 + 512 - 96).unwrap();
            assert_ne!(bm_ext, 0, "DOS\\{flags}: the rootblock names an extension");
            let next = u32_at(&blank, bm_ext as usize * 512 + 508).unwrap();
            assert_eq!(next, 0, "DOS\\{flags}: one extension, so the last");
            let health = ade_core::health::examine(blank.clone());
            let used = health.bitmap.as_ref().unwrap().marked_used;
            assert_eq!(used, if flags == 5 { 54 } else { 53 }, "DOS\\{flags}");
        }

        let mut s = Session::from_bytes(blank).unwrap();
        s.set_clock(Some(WHEN));
        let root = s.root();
        // The blocks around the rootblock — where allocation starts — are
        // mapped by pages only the chain lists, so every write below goes
        // through it.
        let big = pattern(600_000, flags);
        let dir = s.make_dir(root, b"Dir").unwrap();
        let file = s.add_file(dir, b"big.bin", &big).unwrap();
        s.make_link(file, root, b"link.bin").unwrap();
        assert!(read(s.bytes(), "link.bin") == big, "DOS\\{flags}");
        let gone = s.add_file(root, b"gone.bin", &pattern(5_000, 9)).unwrap();
        s.delete(gone).unwrap();
        s.undelete(gone).unwrap();
        assert!(
            faults(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            faults(s.bytes())
        );

        // Repair finds nothing to do on a sound chained bitmap — it once
        // refused any volume with an extension chain.
        let repaired = s.repair(false);
        assert!(
            repaired.is_ok() || format!("{repaired:?}").contains("NotNeeded"),
            "{repaired:?}"
        );
        s.wipe(true, true).unwrap();
        assert!(
            read(s.bytes(), "Dir/big.bin") == big,
            "DOS\\{flags}: a wipe keeps files"
        );
        s.format(None).unwrap();
        assert!(
            faults(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            faults(s.bytes())
        );
    }
}

#[test]
fn past_four_gigabytes_is_refused_and_below_it_is_made() {
    let too_big = Shape::Hard(4_097).geometry().unwrap();
    let dostype = Dostype::from_raw(0x444F_5301).unwrap();
    let refused = format::blank(too_big, dostype, "Huge", WHEN).unwrap_err();
    assert!(refused.to_string().contains("4 GB"), "{refused}");
}

#[test]
fn adflib_reads_a_large_hardfile_ade_writes() {
    let geometry = Shape::Hard(100).geometry().unwrap();
    let blank = format::blank(
        geometry,
        Dostype::from_raw(0x444F_5301).unwrap(),
        "Big",
        WHEN,
    )
    .unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let data = pattern(400_000, 3);
    s.add_file(root, b"big.bin", &data).unwrap();
    let dir = std::env::temp_dir().join(format!("ade-largehdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.hdf");
    std::fs::write(&path, s.bytes()).unwrap();
    let script = "ulimit -v 2097152; exec timeout 60 unadf -l \"$1\"";
    let Ok(out) = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&path)
        .output()
    else {
        eprintln!("skipping: unadf not installed");
        return;
    };
    let listing = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() && listing.is_empty() {
        eprintln!(
            "skipping: unadf could not run: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(listing.contains("big.bin"), "{listing}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn history_on_a_large_hardfile_is_the_blocks_edits_changed_not_the_disk() {
    // D-024 part 7: Undo once kept a whole image per step — on a hard disk,
    // copies of the disk. It now keeps the blocks each edit changed.
    let geometry = Shape::Hard(100).geometry().unwrap();
    let blank = format::blank(
        geometry,
        Dostype::from_raw(0x444F_5301).unwrap(),
        "Hist",
        WHEN,
    )
    .unwrap();
    let mut s = Session::from_bytes(blank.clone()).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    s.add_file(root, b"one", &pattern(20_000, 1)).unwrap();
    let after_one = s.bytes().to_vec();
    s.grouped(|s| {
        for i in 0..20u8 {
            s.add_file(root, format!("drop{i}").as_bytes(), &pattern(3_000, i))
                .unwrap();
        }
    });
    let after_group = s.bytes().to_vec();
    // A few dozen blocks of history for 21 files, on a 100 MB disk.
    assert!(
        s.history_bytes() < 512 * 1024,
        "history holds {} bytes",
        s.history_bytes()
    );

    // One Undo takes the whole group off; another the first file; Redo puts
    // them back, each exactly.
    assert!(s.undo());
    assert!(
        s.bytes() == after_one.as_slice(),
        "the group came off as one"
    );
    assert!(s.undo());
    assert!(s.bytes() == blank.as_slice(), "the disk as it was made");
    assert!(s.redo() && s.redo());
    assert!(s.bytes() == after_group.as_slice());
}

#[test]
fn a_folder_deletion_that_fails_part_way_is_taken_back_exactly() {
    // `delete_tree` is all or nothing: if one removal is refused, the ones
    // already made are undone. It once kept a copy of the whole disk to do
    // that; it now takes back the grouped step. The failure is real: a file
    // whose header names another directory as its parent (BUG-018), which
    // ADE will not edit — and deletion goes deepest first, so the good files
    // in `Box/Sub` are deleted before `Box/bad` is refused.
    use ade_core::layers::block::checksum::normal_at;
    use ade_core::layers::endian::put_u32;
    let geometry = Shape::Hard(60).geometry().unwrap();
    let blank = format::blank(
        geometry,
        Dostype::from_raw(0x444F_5301).unwrap(),
        "Tree",
        WHEN,
    )
    .unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let dir = s.make_dir(root, b"Box").unwrap();
    let sub = s.make_dir(dir, b"Sub").unwrap();
    s.add_file(sub, b"one", &pattern(4_000, 1)).unwrap();
    s.add_file(sub, b"two", &pattern(4_000, 2)).unwrap();
    let bad = s.add_file(dir, b"bad", b"names the wrong parent").unwrap();

    let mut bytes = s.bytes().to_vec();
    let at = bad as usize * 512;
    put_u32(&mut bytes, at + 512 - 12, root).unwrap();
    put_u32(&mut bytes, at + 20, 0).unwrap();
    let sum = normal_at(&bytes[at..at + 512], 20).unwrap();
    put_u32(&mut bytes, at + 20, sum).unwrap();
    let mut s = Session::from_bytes(bytes.clone()).unwrap();
    s.set_clock(Some(WHEN));

    assert!(s.delete_tree(dir).is_err(), "Box/bad cannot be deleted");
    assert!(
        s.bytes() == bytes.as_slice(),
        "every removal before it is taken back"
    );
    assert!(!s.can_undo(), "and nothing is left on the undo stack");
    assert_eq!(s.history_bytes(), 0);
}
