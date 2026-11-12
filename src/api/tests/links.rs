//! Writing hard links, and checking their chains.
//!
//! Against AmigaDOS itself in `amigados.rs` (D-023): links ADE makes are read
//! and deleted by AmigaDOS, and links AmigaDOS makes are deleted by ADE as
//! AmigaDOS would. Here, where CI can see it: every dostype, each refusal,
//! promotion with links left over, and the health report's link checks.

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
    let blank = format::blank(Geometry::DD_FLOPPY, dostype, "Links", WHEN).unwrap();
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
    let entry = volume
        .lookup(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    volume.read_file(&entry).unwrap().bytes
}

fn exists(bytes: &[u8], path: &str) -> bool {
    let image = Image::from_bytes(bytes.to_vec()).unwrap();
    image.volume().unwrap().lookup(path).is_ok()
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

/// A long `back` bytes from the end of `block`.
fn field(bytes: &[u8], block: u32, back: usize) -> u32 {
    u32_at(bytes, block as usize * 512 + 512 - back).unwrap()
}

#[test]
fn links_are_made_read_through_and_deleted_on_every_type() {
    for flags in 0..6u8 {
        let mut s = session(flags);
        let root = s.root();
        let data = pattern(3_000, flags);
        let file = s.add_file(root, b"target.bin", &data).unwrap();
        let sub = s.make_dir(root, b"Sub").unwrap();
        s.add_file(sub, b"other.bin", b"other").unwrap();
        let older = s.make_link(file, root, b"link.bin").unwrap();
        let newer = s.make_link(file, sub, b"d\xe9ep.bin").unwrap();
        s.make_link(sub, root, b"dirlink").unwrap();
        assert!(
            codes(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            codes(s.bytes())
        );

        // The chain, as AmigaDOS makes it: the object names the newest link,
        // which names the older one.
        assert_eq!(field(s.bytes(), file, 40), newer, "DOS\\{flags}");
        assert_eq!(field(s.bytes(), newer, 40), older);
        assert_eq!(field(s.bytes(), older, 40), 0);
        assert_eq!(field(s.bytes(), older, 44), file, "real_entry");

        for path in ["link.bin", "Sub/d\u{e9}ep.bin", "dirlink/other.bin"] {
            let want: &[u8] = if path.ends_with("other.bin") {
                b"other"
            } else {
                &data
            };
            assert!(read(s.bytes(), path) == want, "DOS\\{flags}: {path}");
        }

        // Deleting the older link unhooks it from the chain.
        s.delete(older).unwrap();
        assert_eq!(field(s.bytes(), newer, 40), 0, "DOS\\{flags}");
        assert!(!exists(s.bytes(), "link.bin"));
        let dirlink = s.entry("dirlink").unwrap();
        s.delete(dirlink).unwrap();
        assert_eq!(
            field(s.bytes(), sub, 40),
            0,
            "DOS\\{flags}: the directory's chain"
        );
        assert!(
            codes(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            codes(s.bytes())
        );
    }
}

#[test]
fn deleting_a_linked_file_promotes_the_newest_link() {
    // As AmigaDOS's `Delete` was measured to: the object's header block stays,
    // taking the newest link's name, directory and successor; the other links
    // still reach it; its datestamp, protection and comment are its own.
    for flags in [0u8, 1, 5] {
        let mut s = session(flags);
        let root = s.root();
        let data = pattern(2_000, 3);
        let file = s.add_file(root, b"f.bin", &data).unwrap();
        let d = s.make_dir(root, b"D").unwrap();
        let a = s.make_link(file, root, b"a.bin").unwrap();
        let b = s.make_link(file, d, b"b.bin").unwrap();
        let before_date = (field(s.bytes(), file, 92), field(s.bytes(), file, 88));
        s.delete(file).unwrap();

        assert!(!exists(s.bytes(), "f.bin"), "DOS\\{flags}");
        assert_eq!(
            s.entry("D/b.bin").unwrap(),
            file,
            "the header block is kept"
        );
        assert_eq!(
            field(s.bytes(), file, 12),
            d,
            "it moved to the link's directory"
        );
        assert_eq!(
            field(s.bytes(), file, 40),
            a,
            "and took the link's successor"
        );
        assert_eq!(
            field(s.bytes(), file, 4),
            0xFFFF_FFFD,
            "and is a file again"
        );
        assert_eq!(
            (field(s.bytes(), file, 92), field(s.bytes(), file, 88)),
            before_date,
            "its own datestamp"
        );
        assert!(read(s.bytes(), "D/b.bin") == data);
        assert!(
            read(s.bytes(), "a.bin") == data,
            "the other link still reaches it"
        );
        assert!(
            codes(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            codes(s.bytes())
        );
        let _ = b;

        // And the last one: deleting the promoted file promotes `a.bin`.
        s.delete(file).unwrap();
        assert!(read(s.bytes(), "a.bin") == data);
        assert_eq!(field(s.bytes(), file, 40), 0);
        assert!(
            codes(s.bytes()).is_empty(),
            "DOS\\{flags}: {:?}",
            codes(s.bytes())
        );
    }
}

#[test]
fn a_linked_empty_directory_promotes_too() {
    let mut s = session(5);
    let root = s.root();
    let e = s.make_dir(root, b"E").unwrap();
    s.make_link(e, root, b"EL").unwrap();
    s.delete(e).unwrap();
    assert_eq!(s.entry("EL").unwrap(), e);
    assert!(codes(s.bytes()).is_empty(), "{:?}", codes(s.bytes()));
}

#[test]
fn links_and_linked_entries_rename_and_move() {
    let mut s = session(3);
    let root = s.root();
    let data = pattern(800, 4);
    let file = s.add_file(root, b"f", &data).unwrap();
    let d = s.make_dir(root, b"D").unwrap();
    let link = s.make_link(file, root, b"l").unwrap();
    s.rename(link, b"renamed").unwrap();
    s.move_entry(link, d).unwrap();
    s.move_entry(file, d).unwrap();
    s.rename(file, b"moved").unwrap();
    assert!(read(s.bytes(), "D/renamed") == data);
    assert!(read(s.bytes(), "D/moved") == data);
    assert!(codes(s.bytes()).is_empty(), "{:?}", codes(s.bytes()));
}

#[test]
fn what_cannot_be_linked_is_refused() {
    let mut s = session(1);
    let root = s.root();
    let file = s.add_file(root, b"f", b"x").unwrap();
    let d = s.make_dir(root, b"D").unwrap();
    let inner = s.make_dir(d, b"Inner").unwrap();
    let link = s.make_link(file, root, b"l").unwrap();
    let refused = |r: Result<u32, EditError>| {
        matches!(r, Err(EditError::Refused(FsEditError::NotRemovable(_))))
    };
    assert!(refused(s.make_link(link, root, b"l2")), "a link to a link");
    assert!(refused(s.make_link(root, d, b"top")), "the volume itself");
    assert!(
        refused(s.make_link(d, d, b"self")),
        "a directory inside itself"
    );
    assert!(refused(s.make_link(d, inner, b"loop")), "or below itself");
    assert!(matches!(
        s.make_link(file, root, b"F"),
        Err(EditError::Refused(FsEditError::Exists { .. }))
    ));
    assert_eq!(codes(s.bytes()), Vec::<&str>::new());
}

/// Rewrite the long `back` bytes from the end of `block`, and reseal it.
fn poke(bytes: &mut [u8], block: u32, back: usize, value: u32) {
    let at = block as usize * 512;
    put_u32(bytes, at + 512 - back, value).unwrap();
    put_u32(bytes, at + 20, 0).unwrap();
    let sum = normal_at(&bytes[at..at + 512], 20).unwrap();
    put_u32(bytes, at + 20, sum).unwrap();
}

#[test]
fn a_broken_chain_or_a_dangling_link_is_reported() {
    let mut s = session(1);
    let root = s.root();
    let file = s.add_file(root, b"f", b"x").unwrap();
    let link = s.make_link(file, root, b"l").unwrap();
    let sound = s.bytes().to_vec();

    // The object forgets its link.
    let mut bytes = sound.clone();
    poke(&mut bytes, file, 40, 0);
    assert_eq!(codes(&bytes), vec!["link-chain-broken"]);

    // The object names something that is not a link to it.
    let mut bytes = sound.clone();
    poke(&mut bytes, file, 40, root);
    assert_eq!(codes(&bytes), vec!["link-chain-broken"]);

    // The chain loops.
    let mut bytes = sound.clone();
    poke(&mut bytes, link, 40, link);
    assert_eq!(codes(&bytes), vec!["link-chain-broken"]);

    // The link names a block that is no entry.
    let mut bytes = sound;
    poke(&mut bytes, link, 44, 1500);
    assert!(
        codes(&bytes).contains(&"link-target-missing"),
        "{:?}",
        codes(&bytes)
    );
}

#[test]
fn a_directory_link_lists_and_resolves_as_its_directory() {
    // BUG-026: a path through a directory link found nothing, and listing the
    // link — `ade ls disk dirlink`, or opening it in the window — showed an
    // empty directory, where AmigaDOS shows what the directory holds.
    let mut s = session(1);
    let root = s.root();
    let sub = s.make_dir(root, b"Sub").unwrap();
    s.add_file(sub, b"inside", b"held").unwrap();
    let link = s.make_link(sub, root, b"dirlink").unwrap();
    let image = Image::from_bytes(s.bytes().to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let listed: Vec<String> = volume
        .list(link)
        .unwrap()
        .entries
        .iter()
        .map(ade_core::layers::filesystem::entry::Entry::name_lossy)
        .collect();
    assert_eq!(listed, vec!["inside".to_owned()]);
    assert!(read(s.bytes(), "dirlink/inside") == b"held");
}
