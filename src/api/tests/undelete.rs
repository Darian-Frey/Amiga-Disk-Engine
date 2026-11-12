//! Putting deleted entries back (F-012).
//!
//! Checked against AmigaDOS itself in `amigados.rs` (D-023), where AmigaDOS
//! does the deleting and reads the result. Here, where CI can see it: the
//! round trip through ADE's own delete on every dostype ADE writes, and each
//! refusal — a block in use again, a deleted directory, a name taken, an OFS
//! file only partly its own, an entry that is not deleted.

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

use ade_core::Image;
use ade_core::edit::{EditError, Session};
use ade_core::layers::block::Geometry;
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
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Undel", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    s
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

fn read(bytes: &[u8], path: &str) -> Vec<u8> {
    let image = Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup(path).unwrap();
    volume.read_file(&entry).unwrap().bytes
}

fn not_restorable(result: &Result<(), EditError>) -> bool {
    matches!(
        result,
        Err(EditError::Refused(FsEditError::NotRestorable(_)))
    )
}

#[test]
fn everything_ade_deletes_comes_back_as_it_was() {
    // Delete a file, one needing an extension block on OFS, a file in a
    // directory, and the directory; put them back, directory first. The
    // listing, every file's bytes and the bitmap must be as before.
    for flags in 0..6u8 {
        let mut s = session(flags);
        let root = s.root();
        let small = pattern(700, 1);
        let big = pattern(40_000, 2);
        let inner = pattern(1_500, 3);
        let a = s.add_file(root, b"small.bin", &small).unwrap();
        let b = s.add_file(root, b"big.bin", &big).unwrap();
        let dir = s.make_dir(root, b"Drawer").unwrap();
        let c = s.add_file(dir, b"inner.bin", &inner).unwrap();
        s.add_file(root, b"keep.bin", &small).unwrap();
        let before = s.bytes().to_vec();

        for block in [a, b, c, dir] {
            s.delete(block).unwrap();
        }
        // The file cannot come back before the directory it was in.
        assert!(not_restorable(&s.undelete(c)), "DOS\\{flags}");
        for block in [dir, c, b, a] {
            s.undelete(block)
                .unwrap_or_else(|e| panic!("DOS\\{flags}: block {block}: {e}"));
        }

        assert!(read(s.bytes(), "small.bin") == small, "DOS\\{flags}");
        assert!(read(s.bytes(), "big.bin") == big, "DOS\\{flags}");
        assert!(read(s.bytes(), "Drawer/inner.bin") == inner, "DOS\\{flags}");
        let health = ade_core::health::examine(s.bytes().to_vec());
        assert!(health.is_sound(), "DOS\\{flags}: {:?}", health.findings);
        // The bitmap is exactly as it was: the same blocks in use.
        let bitmap = |bytes: &[u8]| {
            let at = ade_core::layers::endian::u32_at(bytes, 880 * 512 + 512 - 196).unwrap();
            bytes[at as usize * 512 + 4..at as usize * 512 + 512].to_vec()
        };
        assert_eq!(
            bitmap(s.bytes()),
            bitmap(&before),
            "DOS\\{flags}: the bitmap"
        );
        // And undo takes the last one off again.
        assert!(s.undo());
        assert!(
            Image::from_bytes(s.bytes().to_vec())
                .unwrap()
                .volume()
                .unwrap()
                .lookup("small.bin")
                .is_err()
        );
    }
}

#[test]
fn an_entry_whose_blocks_are_in_use_again_stays_deleted() {
    for flags in [0u8, 1, 5] {
        let mut s = session(flags);
        let root = s.root();
        let gone = s.add_file(root, b"gone.bin", &pattern(5_000, 4)).unwrap();
        s.delete(gone).unwrap();
        // The next file is allocated from the same place, so it takes the
        // deleted one's blocks — its header block among them.
        s.add_file(root, b"new.bin", &pattern(5_000, 5)).unwrap();
        assert!(not_restorable(&s.undelete(gone)), "DOS\\{flags}");
    }
}

#[test]
fn a_name_taken_since_keeps_it_deleted() {
    // Taken by renaming another file, which allocates nothing: a new file
    // would be given the deleted one's header block first, and the refusal
    // would be about the block rather than the name.
    for flags in [0u8, 1, 5] {
        let mut s = session(flags);
        let root = s.root();
        let old = s.add_file(root, b"name.txt", b"old").unwrap();
        let other = s.add_file(root, b"other.txt", b"other").unwrap();
        s.delete(old).unwrap();
        s.rename(other, b"NAME.TXT").unwrap();
        let result = s.undelete(old);
        assert!(
            matches!(result, Err(EditError::Refused(FsEditError::Exists { .. }))),
            "DOS\\{flags}: {result:?}"
        );
    }
}

#[test]
fn an_ofs_file_only_partly_its_own_stays_deleted() {
    // An OFS data block names the header it belongs to. When a deleted
    // file's block has since been used by another file, it names that file
    // instead: putting the first back would show the other's bytes as its
    // own. Built directly — one data block re-pointed at another header and
    // resealed — because ADE's allocator, like AmigaDOS's, reuses a deleted
    // file's header before its data, so a real reuse rarely leaves this.
    use ade_core::layers::block::checksum::normal_at;
    use ade_core::layers::endian::{put_u32, u32_at};
    let mut s = session(0);
    let root = s.root();
    let first = s.add_file(root, b"first", &pattern(20_000, 7)).unwrap();
    s.add_file(root, b"keep", b"kept").unwrap();
    s.delete(first).unwrap();
    let mut bytes = s.bytes().to_vec();
    // The third data block, from the header's table (stored backwards).
    let header = first as usize * 512;
    let third = u32_at(&bytes, header + 24 + (72 - 3) * 4).unwrap() as usize * 512;
    put_u32(&mut bytes, third + 4, 1234).unwrap();
    put_u32(&mut bytes, third + 20, 0).unwrap();
    let sum = normal_at(&bytes[third..third + 512], 20).unwrap();
    put_u32(&mut bytes, third + 20, sum).unwrap();

    let mut s = Session::from_bytes(bytes.clone()).unwrap();
    let image = Image::from_bytes(bytes).unwrap();
    let carved = ade_core::carve::carve(&image);
    let entry = carved.iter().find(|c| c.block == first).unwrap();
    assert!(
        matches!(
            entry.evidence,
            ade_core::carve::Evidence::Partial { bad: 1, .. }
        ),
        "{:?}",
        entry.evidence
    );
    assert!(not_restorable(&s.undelete(first)));
}

#[test]
fn a_live_entry_or_a_block_that_is_no_entry_is_not_undeleted() {
    let mut s = session(1);
    let root = s.root();
    let live = s.add_file(root, b"here", b"still here").unwrap();
    assert!(not_restorable(&s.undelete(live)));
    assert!(not_restorable(&s.undelete(1500)));
    assert!(not_restorable(&s.undelete(root)));
}
