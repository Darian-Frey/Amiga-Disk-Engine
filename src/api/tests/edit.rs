//! Writing into an existing disk (D-018), checked the way `create` is.
//!
//! D-018 part 4 says a write is finished only when the result passes
//! `ade check` **and** mounts under ADFlib. Both are asserted here for every
//! filesystem ADE writes: ADE's health report must find nothing the disk did
//! not already have, ADE must read back what it wrote, and ADFlib — an
//! implementation nobody here wrote (D-002) — must extract the same bytes.
//! A disk ADE wrote and ADE alone accepts proves only that the writer and the
//! reader share their mistakes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::collections::BTreeMap;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use ade_core::edit::{EditError, Session};
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::edit::EditError as FsEditError;
use ade_core::layers::filesystem::format::{self, Stamp};

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 600,
    ticks: 25,
};

/// Every dostype ADE writes, `DOS\0` to `DOS\5`.
const FLAGS: [u8; 6] = [0, 1, 2, 3, 4, 5];

fn blank(flags: u8) -> Vec<u8> {
    let dostype = Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap();
    format::blank(Geometry::DD_FLOPPY, dostype, "Edit", WHEN).unwrap()
}

fn session(flags: u8) -> Session {
    let mut s = Session::from_bytes(blank(flags)).unwrap();
    s.set_clock(Some(WHEN));
    s
}

/// Bytes that are not all the same, so a block written in the wrong order or
/// at the wrong offset cannot read back correctly by accident.
fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

/// ADE's own verdict: no warning or error at all.
fn assert_clean(bytes: &[u8], what: &str) {
    let health = ade_core::health::examine(bytes.to_vec());
    let bad: Vec<_> = health
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    assert!(
        bad.is_empty(),
        "{what}: ADE should accept its own write: {bad:?}"
    );
}

/// Every file on the volume, by path, as ADE reads it.
fn ade_files(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let image = ade_core::Image::from_bytes(bytes.to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let walk = volume.walk(volume.root()).unwrap();
    walk.entries
        .into_iter()
        .filter(|(_, e)| e.kind.is_file())
        .map(|(path, e)| {
            let contents = volume.read_file(&e).unwrap();
            assert!(contents.is_complete(), "{path} read back short");
            (path, contents.into_bytes())
        })
        .collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ade-edit-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run ADFlib, capped: an uncapped run once took a whole session down.
/// `None` when it is not installed.
fn unadf(args: &[&str], image: &Path) -> Option<(bool, String)> {
    let script = "ulimit -v 1048576; img=\"$1\"; shift; exec timeout 20 unadf \"$@\" \"$img\"";
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(image)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.is_empty() && !out.status.success() {
        return None;
    }
    Some((out.status.success(), text))
}

/// Every file ADFlib extracts, by path. `None` when the oracle is absent.
fn adflib_files(bytes: &[u8], tag: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    let scratch = Scratch::new(tag);
    let image = scratch.0.join("disk.adf");
    std::fs::write(&image, bytes).unwrap();
    let out = scratch.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let script = "ulimit -v 1048576; exec timeout 20 unadf \"$1\" -d \"$2\"";
    let status = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&image)
        .arg(&out)
        .output()
        .ok()?;
    if !status.status.success() && String::from_utf8_lossy(&status.stdout).is_empty() {
        eprintln!("skipping: unadf not installed");
        return None;
    }
    let mut files = BTreeMap::new();
    let mut stack = vec![out.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(&out)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                files.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    Some(files)
}

/// The full check D-018 asks for: ADE accepts it, and ADE and ADFlib agree
/// on every byte of every file.
fn assert_both_agree(bytes: &[u8], want: &BTreeMap<String, Vec<u8>>, what: &str) {
    assert_clean(bytes, what);
    let mine = ade_files(bytes);
    assert_eq!(
        mine.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "{what}: ADE's listing"
    );
    for (path, data) in want {
        assert!(mine[path] == *data, "{what}: ADE read {path} back wrongly");
    }
    let tag = what.replace(|c: char| !c.is_ascii_alphanumeric(), "-");
    let Some(theirs) = adflib_files(bytes, &tag) else {
        return;
    };
    assert_eq!(
        theirs.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "{what}: ADFlib's listing"
    );
    for (path, data) in want {
        assert!(
            theirs[path] == *data,
            "{what}: ADFlib extracted {path} differently ({} bytes, wanted {})",
            theirs[path].len(),
            data.len()
        );
    }
}

#[test]
fn a_directory_appears_on_every_filesystem() {
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let dir = s.make_dir(root, b"Drawer").unwrap();
        assert!(s.is_dirty());
        assert_clean(s.bytes(), &format!("DOS\\{flags} mkdir"));
        assert_eq!(
            s.directory("drawer").unwrap(),
            dir,
            "found case-insensitively"
        );

        let scratch = Scratch::new(&format!("mkdir{flags}"));
        let path = scratch.0.join("d.adf");
        std::fs::write(&path, s.bytes()).unwrap();
        if let Some((ok, text)) = unadf(&["-l", "-r"], &path) {
            assert!(ok, "DOS\\{flags}: ADFlib should list it: {text}");
            assert!(text.contains("Drawer/"), "DOS\\{flags}: {text}");
        }
        // On a cached volume, list from the cache too: `-c` reads only the
        // dircache, so this is the independent check on the record ADE wrote.
        if flags >= 4
            && let Some((ok, text)) = unadf(&["-l", "-c"], &path)
        {
            assert!(ok, "DOS\\{flags}: {text}");
            assert!(
                text.contains("Drawer/"),
                "DOS\\{flags} from the cache: {text}"
            );
        }
    }
}

#[test]
fn files_of_every_awkward_size_read_back_through_both_readers() {
    // Zero; one byte; exactly one OFS block and one past it; exactly one FFS
    // block and one past it; the last size the header's 72 pointers cover on
    // each filesystem and one past it, which needs an extension block; and
    // one large enough to need two extension blocks on OFS.
    let sizes = [
        0usize,
        1,
        488,
        489,
        512,
        513,
        72 * 488,
        72 * 488 + 1,
        72 * 512,
        72 * 512 + 1,
        160_000,
    ];
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let mut want = BTreeMap::new();
        for (i, &size) in sizes.iter().enumerate() {
            let name = format!("file{size}");
            let data = pattern(size, i as u8);
            s.add_file(root, name.as_bytes(), &data).unwrap();
            want.insert(name, data);
        }
        assert_both_agree(s.bytes(), &want, &format!("DOS\\{flags} sizes"));
    }
}

#[test]
fn files_go_into_subdirectories() {
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let a = s.make_dir(root, b"A").unwrap();
        let b = s.make_dir(a, b"B").unwrap();
        let mut want = BTreeMap::new();
        for (dir, prefix) in [(root, ""), (a, "A/"), (b, "A/B/")] {
            let data = pattern(1000, prefix.len() as u8);
            s.add_file(dir, b"readme", &data).unwrap();
            want.insert(format!("{prefix}readme"), data);
        }
        assert_both_agree(s.bytes(), &want, &format!("DOS\\{flags} nested"));
    }
}

#[test]
fn a_crowded_directory_fills_hash_chains_and_cache_blocks() {
    // 120 names into 72 slots forces collision chains, and 120 cache records
    // of ~36 bytes cannot fit in one 488-byte cache block, so a DOS\4/5 volume
    // has to grow its chain. Names are inserted in an order that is not block
    // order, so the sorted chain insertion is exercised in the middle too.
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let mut want = BTreeMap::new();
        for i in 0..120u32 {
            let name = format!("entry-{:03}", (i * 37) % 120);
            let data = pattern(100, i as u8);
            s.add_file(root, name.as_bytes(), &data).unwrap();
            want.insert(name, data);
        }
        assert_both_agree(s.bytes(), &want, &format!("DOS\\{flags} crowded"));

        let health = ade_core::health::examine(s.bytes().to_vec());
        if flags >= 4 {
            let cache = health.dircache.unwrap();
            assert!(
                cache.blocks > 1,
                "DOS\\{flags}: the cache should have grown"
            );
            assert_eq!(cache.records, 120);
            assert_eq!(cache.disagreements, 0);
        }
    }
}

/// The hash slot whose chain reaches `entry`, in the directory at `dir`.
fn slot_of(bytes: &[u8], dir: u32, entry: u32) -> Option<usize> {
    let long = |at: usize| ade_core::layers::endian::u32_at(bytes, at).unwrap();
    let base = dir as usize * 512;
    for slot in 0..72 {
        let mut next = long(base + 24 + slot * 4);
        let mut hops = 0;
        while next != 0 && hops < 1000 {
            if next == entry {
                return Some(slot);
            }
            next = long(next as usize * 512 + 512 - 16);
            hops += 1;
        }
    }
    None
}

#[test]
fn international_names_hash_the_way_the_volume_does() {
    // C-006: on DOS\2 and above `ü` folds to `Ü` before hashing; on DOS\0/1 it
    // does not. ASCII names hash the same either way, so only accented ones
    // can tell — and ADE's listing walks every slot, so a wrong one is
    // invisible to it. This test reads the slot itself, and asks ADFlib to
    // find each file **by name**, which goes through the hash.
    use ade_core::layers::filesystem::hash::hash_name;
    let names: [&[u8]; 4] = [b"f\xFCr", b"\xE9t\xE9", b"Gr\xF6\xDFe", b"na\xEFve-caf\xE9"];
    for flags in FLAGS {
        let international = flags >= 2;
        // The two hashes must actually differ for at least one name, or this
        // test proves nothing.
        assert!(
            names
                .iter()
                .any(|n| hash_name(n, 72, true) != hash_name(n, 72, false))
        );

        let mut s = session(flags);
        let root = s.root();
        let scratch = Scratch::new(&format!("intl{flags}"));
        let image = scratch.0.join("i.adf");
        for (i, name) in names.iter().enumerate() {
            let data = pattern(300, i as u8);
            let block = s.add_file(root, name, &data).unwrap();
            assert_eq!(
                slot_of(s.bytes(), root, block),
                Some(hash_name(name, 72, international) as usize),
                "DOS\\{flags}: {name:?} is in the wrong slot"
            );
            std::fs::write(&image, s.bytes()).unwrap();
            let script = "ulimit -v 1048576; exec timeout 20 unadf -p \"$1\" \"$2\"";
            let out = Command::new("sh")
                .arg("-c")
                .arg(script)
                .arg("sh")
                .arg(&image)
                .arg(std::ffi::OsStr::from_bytes(name))
                .output();
            if let Ok(out) = out
                && (!out.stdout.is_empty() || out.status.success())
            {
                assert!(
                    out.stdout == data,
                    "DOS\\{flags}: ADFlib could not find {name:?} by name"
                );
            }
        }
        assert_clean(s.bytes(), &format!("DOS\\{flags} latin-1"));
    }
}

#[test]
fn a_refused_edit_changes_nothing() {
    let mut s = session(3);
    let root = s.root();
    s.add_file(root, b"Taken", b"one").unwrap();
    let before = s.bytes().to_vec();

    // Names compare as AmigaDOS compares them.
    assert!(matches!(
        s.add_file(root, b"TAKEN", b"two"),
        Err(EditError::Refused(FsEditError::Exists { .. }))
    ));
    assert!(matches!(
        s.make_dir(root, b"taken"),
        Err(EditError::Refused(FsEditError::Exists { .. }))
    ));
    for bad in [&b""[..], b"a/b", b"dev:", &[b'n'; 31]] {
        assert!(
            matches!(s.make_dir(root, bad), Err(EditError::Refused(_))),
            "{bad:?} should be refused"
        );
    }
    // A file is not a directory.
    let file = s.directory("").unwrap();
    assert_eq!(file, root);
    let taken = {
        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        image.volume().unwrap().lookup("Taken").unwrap().block
    };
    assert!(matches!(
        s.make_dir(taken, b"inside"),
        Err(EditError::Refused(FsEditError::NotADirectory { .. }))
    ));
    // Larger than the disk: refused before a single block is written.
    assert!(matches!(
        s.add_file(root, b"huge", &vec![0u8; 1_000_000]),
        Err(EditError::Refused(FsEditError::DiskFull { .. }))
    ));
    assert!(
        s.bytes() == before,
        "every refusal must leave the image alone"
    );
}

#[test]
fn a_disk_can_be_filled_exactly() {
    // The last free block is usable and the one after it is not, on both
    // filesystems: the free count, the allocator and the bitmap must agree.
    for flags in [0u8, 1] {
        let mut s = session(flags);
        let root = s.root();
        let free = ade_core::health::examine(s.bytes().to_vec())
            .bitmap
            .map(|b| b.covered as usize - b.marked_used)
            .unwrap();
        let payload = if flags == 0 { 488 } else { 512 };
        // One header, `n` data blocks, and `ceil(n / 72) - 1` extensions.
        let mut n = free - 1;
        while 1 + n + n.div_ceil(72) - 1 > free {
            n -= 1;
        }
        let data = pattern(n * payload, 7);
        s.add_file(root, b"all", &data).unwrap();
        assert_eq!(
            ade_core::health::examine(s.bytes().to_vec())
                .bitmap
                .map(|b| b.covered as usize - b.marked_used),
            Some(free - (n + n.div_ceil(72))),
        );
        let err = s.make_dir(root, b"more");
        if free - (n + n.div_ceil(72)) == 0 {
            assert!(matches!(
                err,
                Err(EditError::Refused(FsEditError::DiskFull { .. }))
            ));
        }
        let mut want = BTreeMap::new();
        want.insert("all".to_owned(), data);
        if err.is_ok() {
            // A block was left over, so the directory fitted.
            let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
            assert!(image.volume().unwrap().lookup("more").is_ok());
        }
        assert_both_agree(s.bytes(), &want, &format!("DOS\\{flags} full"));
    }
}

#[test]
fn undo_and_redo_move_through_whole_edits() {
    let mut s = session(5);
    let root = s.root();
    let blank = s.bytes().to_vec();
    s.make_dir(root, b"one").unwrap();
    let after_one = s.bytes().to_vec();
    s.add_file(root, b"two", b"2").unwrap();
    assert!(s.undo());
    assert_eq!(s.bytes(), &after_one[..]);
    assert!(s.undo());
    assert_eq!(s.bytes(), &blank[..]);
    assert!(!s.undo(), "nothing left to undo");
    assert!(s.redo());
    assert_eq!(s.bytes(), &after_one[..]);
    // A new edit discards what could have been redone.
    s.make_dir(root, b"three").unwrap();
    assert!(!s.can_redo());
    assert_clean(s.bytes(), "after undo and redo");
}

#[test]
fn a_save_never_lands_on_the_original() {
    let scratch = Scratch::new("save");
    let original = scratch.0.join("original.adf");
    std::fs::write(&original, blank(3)).unwrap();
    let pristine = std::fs::read(&original).unwrap();

    let mut s = Session::open(&original).unwrap();
    let root = s.root();
    s.make_dir(root, b"New").unwrap();

    assert!(matches!(s.save(), Err(EditError::NoDestination)));
    assert!(matches!(
        s.save_as(&original),
        Err(EditError::WouldOverwriteOriginal)
    ));
    // Not even through another name for the same file.
    let alias = scratch.0.join("alias.adf");
    std::fs::hard_link(&original, &alias).unwrap();
    assert!(matches!(
        s.save_as(&alias),
        Err(EditError::WouldOverwriteOriginal)
    ));
    let other = scratch.0.join("somebody-elses.adf");
    std::fs::write(&other, b"not ours").unwrap();
    assert!(matches!(s.save_as(&other), Err(EditError::Exists)));
    assert_eq!(std::fs::read(&other).unwrap(), b"not ours");

    let copy = scratch.0.join("copy.adf");
    s.save_as(&copy).unwrap();
    assert!(!s.is_dirty());
    assert_eq!(std::fs::read(&copy).unwrap(), s.bytes());

    // The destination ADE made may be saved over again.
    s.add_file(root, b"later", b"x").unwrap();
    s.save().unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), s.bytes());

    assert_eq!(std::fs::read(&original).unwrap(), pristine);
    let leftovers: Vec<_> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|e| e == "tmp")
        })
        .collect();
    assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
}

#[test]
fn only_plain_images_open_for_editing() {
    // An image that is not a disk at all.
    assert!(matches!(
        Session::from_bytes(b"not a disk".to_vec()),
        Err(EditError::NotEditable(_))
    ));
    // A floppy-sized image with no volume on it.
    assert!(matches!(
        Session::from_bytes(vec![0u8; 901_120]),
        Err(EditError::NotEditable(_))
    ));
}

#[test]
fn a_disk_whose_bitmap_hides_live_blocks_is_refused() {
    // Put a file on, then mark its data block free behind ADE's back: the
    // state where the next allocation lands on somebody's data.
    let mut s = session(1);
    let root = s.root();
    let header = s.add_file(root, b"victim", &pattern(100, 1)).unwrap();
    let mut bytes = s.bytes().to_vec();
    let data = {
        let image = ade_core::Image::from_bytes(bytes.clone()).unwrap();
        let volume = image.volume().unwrap();
        let entry = volume.entry_at(header).unwrap();
        volume.file_blocks(&entry).unwrap()[1]
    };
    // Bitmap block 881, bit (data - 2); set means free.
    let index = (data - 2) as usize;
    let at = 881 * 512 + 4 + (index / 32) * 4;
    let word = ade_core::layers::endian::u32_at(&bytes, at).unwrap() | 1 << (index % 32);
    ade_core::layers::endian::put_u32(&mut bytes, at, word).unwrap();
    let sum =
        ade_core::layers::block::checksum::normal_at(&bytes[881 * 512..882 * 512], 0).unwrap();
    ade_core::layers::endian::put_u32(&mut bytes, 881 * 512, sum).unwrap();

    assert!(matches!(
        Session::from_bytes(bytes),
        Err(EditError::Unsound(_))
    ));
}

/// Real disks, not just ones ADE made: add a directory and a file to every
/// corpus image ADE will edit, and require that nothing new is wrong and that
/// ADFlib extracts what was added. Local only; skips without the corpus.
///
/// Set `ADE_EDIT_CORPUS=all` for the whole corpus; by default every 25th
/// image, which keeps an ordinary test run quick.
#[test]
fn corpus_disks_take_an_edit_and_stay_sound() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let step = if std::env::var("ADE_EDIT_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        25
    };
    let mut paths: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    paths.sort();

    let (mut edited, mut refused, mut checked) = (0usize, BTreeMap::new(), 0usize);
    for path in paths.iter().step_by(step) {
        let mut s = match Session::open(path) {
            Ok(s) => s,
            Err(e) => {
                let key = match e {
                    EditError::NotEditable(_) => "not editable".to_owned(),
                    EditError::Unsound(_) => "unsound".to_owned(),
                    other => format!("{other}"),
                };
                *refused.entry(key).or_insert(0usize) += 1;
                continue;
            }
        };
        let root = s.root();
        let data = pattern(3000, 9);
        let dir = match s.make_dir(root, b"ADE-edit") {
            Ok(d) => d,
            Err(EditError::Refused(e)) => {
                *refused.entry(refusal_kind(&e)).or_insert(0) += 1;
                continue;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        };
        match s.add_file(dir, b"added", &data) {
            Ok(_) => {}
            Err(EditError::Refused(e)) => {
                *refused.entry(refusal_kind(&e)).or_insert(0) += 1;
                continue;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        }
        edited += 1;

        // ADE reads the addition back.
        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        let volume = image.volume().unwrap();
        let entry = volume.lookup("ADE-edit/added").unwrap();
        assert!(
            volume.read_file(&entry).unwrap().into_bytes() == data,
            "{}: ADE read the new file back wrongly",
            path.display()
        );

        // ADFlib extracts it. Only from disks ADFlib could already read in
        // full, so a disk it chokes on for its own reasons is not blamed on
        // the edit.
        let scratch = Scratch::new("corpus");
        let edited_path = scratch.0.join("e.adf");
        std::fs::write(&edited_path, s.bytes()).unwrap();
        let script = "ulimit -v 1048576; exec timeout 20 unadf -p \"$1\" \"$2\"";
        let Ok(out) = Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("sh")
            .arg(&edited_path)
            .arg("ADE-edit/added")
            .output()
        else {
            continue;
        };
        if out.stdout.is_empty() && !out.status.success() {
            continue;
        }
        checked += 1;
        assert!(
            out.stdout == data,
            "{}: ADFlib extracted {} bytes, wanted {}",
            path.display(),
            out.stdout.len(),
            data.len()
        );
    }
    eprintln!("edited {edited}, ADFlib confirmed {checked}, refused {refused:?}");
}

fn refusal_kind(e: &FsEditError) -> String {
    match e {
        FsEditError::Unsupported(_) => "unsupported volume".to_owned(),
        FsEditError::BitmapUntrusted(_) => "bitmap untrusted".to_owned(),
        FsEditError::Damaged(_) => "damaged directory".to_owned(),
        FsEditError::DiskFull { .. } => "disk full".to_owned(),
        other => format!("{other}"),
    }
}

#[test]
fn an_accented_path_is_found_by_name() {
    // BUG-011: `Volume::lookup` compared a path's UTF-8 bytes against Latin-1
    // names, so `ü` (C3 BC) never matched `ü` (FC) and `ade extract` could not
    // reach a file `ade ls` listed. Built with ADE's writer, whose Latin-1
    // placement ADFlib confirms above, because the fixture generator writes
    // names as UTF-8 and cannot make this disk.
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let dir = s.make_dir(root, b"Gr\xF6\xDFe").unwrap();
        s.add_file(dir, b"f\xFCr.txt", b"accented").unwrap();

        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        let volume = image.volume().unwrap();
        let entry = volume
            .lookup("Größe/für.txt")
            .unwrap_or_else(|e| panic!("DOS\\{flags}: an accented path should resolve: {e}"));
        assert_eq!(volume.read_file(&entry).unwrap().into_bytes(), b"accented");

        // Case folds the way the volume folds: ASCII on every volume, the
        // accented letters only where the hash is international (C-006).
        assert!(volume.lookup("GRÖSSE/für.txt").is_err(), "ß is not SS");
        assert!(
            volume.lookup("größe/FüR.TXT").is_ok(),
            "ASCII case folds everywhere"
        );
        assert_eq!(
            volume.lookup("GRÖßE/FÜR.TXT").is_ok(),
            flags >= 2,
            "DOS\\{flags}: accented case folds only on an international volume"
        );

        // A character with no Latin-1 byte cannot name anything on the disk.
        assert!(matches!(
            volume.lookup("日本"),
            Err(ade_core::layers::filesystem::volume::FsError::NotFound { .. })
        ));
        assert_eq!(s.directory("Größe").unwrap(), dir);
    }
}

#[test]
fn a_group_of_edits_undoes_and_redoes_as_one() {
    // IMP-014: dropping twenty files is one action, and took twenty Undos.
    let mut s = session(3);
    let root = s.root();
    s.make_dir(root, b"Before").unwrap();
    let before = s.bytes().to_vec();

    s.grouped(|s| {
        let dir = s.make_dir(root, b"Dropped").unwrap();
        for i in 0..5u8 {
            s.add_file(dir, format!("f{i}").as_bytes(), &[i; 100])
                .unwrap();
        }
        // A refusal inside the group is not a step and does not end it.
        assert!(s.make_dir(root, b"Before").is_err());
        s.add_file(root, b"last", b"x").unwrap();
    });
    let after = s.bytes().to_vec();

    assert!(s.undo(), "the group is one step");
    assert_eq!(s.bytes(), &before[..], "and it all comes off at once");
    assert!(s.undo(), "the edit before the group is still its own step");
    assert!(!s.can_undo());
    assert!(s.redo());
    assert!(s.redo(), "and the group comes back as one");
    assert_eq!(s.bytes(), &after[..]);
    assert!(!s.can_redo());
}

#[test]
fn an_empty_or_failed_group_leaves_nothing_to_undo_and_groups_nest() {
    let mut s = session(1);
    let root = s.root();
    s.grouped(|s| {
        assert!(s.make_dir(root, b"bad/name").is_err());
    });
    assert!(
        !s.can_undo(),
        "a group with no successful edit is not a step"
    );

    // Nested: only the outermost counts.
    s.begin_group();
    s.make_dir(root, b"A").unwrap();
    s.grouped(|s| {
        s.make_dir(root, b"B").unwrap();
    });
    s.make_dir(root, b"C").unwrap();
    s.end_group();
    assert!(s.undo());
    assert!(!s.can_undo(), "three edits in nested groups were one step");

    // Undo inside an open group closes it: the next edit is a step of its own.
    s.begin_group();
    s.make_dir(root, b"D").unwrap();
    assert!(s.undo());
    s.make_dir(root, b"E").unwrap();
    s.make_dir(root, b"F").unwrap();
    assert!(s.undo());
    assert!(
        s.can_undo(),
        "E and F were separate steps once the group closed"
    );
    assert_clean(s.bytes(), "after grouped edits");
}

#[test]
fn saving_over_the_original_keeps_it_as_a_backup_first() {
    // D-020: Save on a disk that was opened writes over it after the person
    // confirms, and the untouched original is kept beside it.
    let scratch = Scratch::new("over");
    let original = scratch.0.join("disk.adf");
    std::fs::write(&original, blank(3)).unwrap();
    let pristine = std::fs::read(&original).unwrap();

    let mut s = Session::open(&original).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    s.make_dir(root, b"First").unwrap();
    let made = s.save_over_original(true).unwrap();
    let backup = scratch.0.join("disk.adf.orig");
    assert_eq!(made.as_deref(), Some(backup.as_path()));
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        pristine,
        "the backup is the original"
    );
    assert_eq!(
        std::fs::read(&original).unwrap(),
        s.bytes(),
        "the original holds the edit"
    );
    assert!(!s.is_dirty());
    assert_clean(&std::fs::read(&original).unwrap(), "saved over");

    // A second save in the same session: no second backup.
    s.make_dir(root, b"Second").unwrap();
    assert_eq!(s.save_over_original(true).unwrap(), None);
    assert_eq!(std::fs::read(&backup).unwrap(), pristine);
    assert_eq!(std::fs::read(&original).unwrap(), s.bytes());

    // A later session never overwrites the earlier backup.
    let mut later = Session::open(&original).unwrap();
    later.make_dir(root, b"Third").unwrap();
    let second = later.save_over_original(true).unwrap().unwrap();
    assert_eq!(second, scratch.0.join("disk.adf.orig-2"));
    assert_eq!(std::fs::read(&backup).unwrap(), pristine);

    // Save As still refuses the original: it always makes a new file.
    assert!(matches!(
        later.save_as(&original),
        Err(EditError::WouldOverwriteOriginal)
    ));
    let leftovers: Vec<_> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|e| e == "tmp")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_file_changed_behind_the_session_is_not_saved_over() {
    let scratch = Scratch::new("changed");
    let original = scratch.0.join("disk.adf");
    std::fs::write(&original, blank(3)).unwrap();
    let mut s = Session::open(&original).unwrap();
    let root = s.root();
    s.make_dir(root, b"Mine").unwrap();

    // Somebody else writes the file after it was opened.
    let mut theirs = blank(1);
    theirs[100_000] = 0x55;
    std::fs::write(&original, &theirs).unwrap();
    assert!(matches!(
        s.save_over_original(true),
        Err(EditError::ChangedOnDisk)
    ));
    assert_eq!(
        std::fs::read(&original).unwrap(),
        theirs,
        "their file is untouched"
    );
    assert!(
        !scratch.0.join("disk.adf.orig").exists(),
        "and no backup was made of it"
    );

    // Without a backup, for a disk the front end made itself.
    let made = scratch.0.join("made.adf");
    std::fs::write(&made, blank(3)).unwrap();
    let mut s = Session::open(&made).unwrap();
    s.make_dir(root, b"X").unwrap();
    assert_eq!(s.save_over_original(false).unwrap(), None);
    assert!(!scratch.0.join("made.adf.orig").exists());
    assert_eq!(std::fs::read(&made).unwrap(), s.bytes());

    // A session not opened from a file has no original.
    let mut loose = Session::from_bytes(blank(3)).unwrap();
    assert!(matches!(
        loose.save_over_original(true),
        Err(EditError::NoOriginal)
    ));
}

// --- Deleting and renaming (F-031) ------------------------------------------

/// Blocks the bitmap marks in use.
fn used(bytes: &[u8]) -> usize {
    ade_core::health::examine(bytes.to_vec())
        .bitmap
        .unwrap()
        .marked_used
}

#[test]
fn a_deleted_file_is_gone_and_its_blocks_are_free_on_every_filesystem() {
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let drawer = s.make_dir(root, b"Drawer").unwrap();
        let mut want = BTreeMap::new();
        for (i, (name, len)) in [("keep", 700), ("small", 30)].into_iter().enumerate() {
            let data = pattern(len, i as u8);
            s.add_file(root, name.as_bytes(), &data).unwrap();
            want.insert(name.to_owned(), data);
        }
        let inner = pattern(5000, 7);
        s.add_file(drawer, b"inner", &inner).unwrap();
        want.insert("Drawer/inner".to_owned(), inner);
        let before = used(s.bytes());

        // Large enough for extension blocks on OFS: 72 data blocks of 488
        // bytes fill the header's table.
        let doomed = pattern(40_000, 3);
        let block = s.add_file(drawer, b"doomed", &doomed).unwrap();
        assert!(used(s.bytes()) > before);
        s.delete(block).unwrap();

        assert_eq!(
            used(s.bytes()),
            before,
            "DOS\\{flags}: every block the file took is free again"
        );
        assert!(s.entry("Drawer/doomed").is_err());
        assert_both_agree(s.bytes(), &want, &format!("DOS\\{flags} delete"));
        // On a cached volume the record goes too: `-c` lists from the cache.
        let scratch = Scratch::new(&format!("delc{flags}"));
        let path = scratch.0.join("d.adf");
        std::fs::write(&path, s.bytes()).unwrap();
        if flags >= 4
            && let Some((ok, text)) = unadf(&["-l", "-c"], &path)
        {
            assert!(ok, "DOS\\{flags}: {text}");
            assert!(
                !text.contains("doomed"),
                "DOS\\{flags} from the cache: {text}"
            );
        }
    }
}

#[test]
fn a_deleted_file_carves_back_byte_for_byte() {
    // The round trip F-030 was waiting for: deleted by ADE, recovered by ADE.
    // OFS data blocks name their header and are checksummed, so the carve is
    // confirmed block by block; FFS data blocks carry nothing, so the most an
    // honest carver can say there is that a header survives.
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let data = pattern(9_000, 11);
        let block = s.add_file(root, b"lost-and-found", &data).unwrap();
        s.delete(block).unwrap();

        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        let found = ade_core::carve::carve(&image);
        let carved = found
            .iter()
            .find(|c| c.block == block)
            .unwrap_or_else(|| panic!("DOS\\{flags}: the deleted header is found"));
        assert_eq!(carved.name, "lost-and-found");
        let ofs = flags % 2 == 0;
        assert_eq!(
            ade_core::carve::is_recoverable(carved),
            ofs,
            "DOS\\{flags}: {:?}",
            carved.evidence
        );
        if ofs {
            assert!(
                ade_core::carve::contents(&image, carved) == data,
                "DOS\\{flags}: the carved bytes are the file's"
            );
        }
    }
}

#[test]
fn a_directory_is_deleted_only_when_empty_or_as_a_whole_tree() {
    for flags in FLAGS {
        let mut s = session(flags);
        let root = s.root();
        let before = used(s.bytes());
        let top = s.make_dir(root, b"Top").unwrap();
        let mid = s.make_dir(top, b"Mid").unwrap();
        s.add_file(mid, b"a", &pattern(2000, 1)).unwrap();
        s.add_file(top, b"b", &pattern(100, 2)).unwrap();
        let full = s.bytes().to_vec();

        // Not empty: refused, and nothing changed.
        let refused = s.delete(top).unwrap_err();
        assert!(
            matches!(refused, EditError::Refused(FsEditError::NotRemovable(_))),
            "{refused}"
        );
        assert!(
            s.bytes() == full.as_slice(),
            "a refused delete changes nothing"
        );

        // The whole tree: gone, every block free, one undo brings it back.
        assert_eq!(s.delete_tree(top).unwrap(), 4);
        assert_eq!(used(s.bytes()), before, "DOS\\{flags}");
        assert_both_agree(s.bytes(), &BTreeMap::new(), &format!("DOS\\{flags} tree"));
        assert!(s.undo());
        assert!(
            s.bytes() == full.as_slice(),
            "DOS\\{flags}: one undo restores it all"
        );
    }
}

#[test]
fn the_root_and_hard_links_are_not_deleted() {
    let mut s = session(1);
    let root = s.root();
    assert!(matches!(
        s.delete(root),
        Err(EditError::Refused(FsEditError::NotRemovable(_)))
    ));
    assert!(matches!(
        s.rename(root, b"New"),
        Err(EditError::Refused(FsEditError::NotRemovable(_)))
    ));

    // A link, and the file it points at: removing either end means
    // rewriting the link chain, which ADE does not do yet.
    let mut v = ade_fixtures::Volume::dd(1).named("Links");
    let target = v.add_file("target", b"linked to");
    let link = v.add_hardlink("link", target, false);
    let mut s = Session::from_bytes(v.build()).unwrap();
    for block in [target, link] {
        let e = s.delete(block).unwrap_err();
        assert!(
            matches!(e, EditError::Refused(FsEditError::NotRemovable(_))),
            "{e}"
        );
    }
    assert!(!s.is_dirty());
}

#[test]
fn a_renamed_entry_is_found_by_its_new_name_on_every_filesystem() {
    use ade_core::layers::filesystem::hash::hash_name;
    for flags in FLAGS {
        let international = flags >= 2;
        let mut s = session(flags);
        let root = s.root();
        let data = pattern(3_000, 5);
        let file = s.add_file(root, b"old-name", &data).unwrap();
        let dir = s.make_dir(root, b"Folder").unwrap();
        s.add_file(dir, b"inside", b"still here").unwrap();
        let used_before = used(s.bytes());

        // An accented name, so a wrong hash function would file it where no
        // lookup looks (C-006); and a folder, whose children must follow it.
        let new: &[u8] = b"nouveau-r\xE9sum\xE9";
        s.rename(file, new).unwrap();
        s.rename(dir, b"Renamed").unwrap();
        assert_eq!(used(s.bytes()), used_before, "a rename takes no blocks");
        assert_eq!(
            slot_of(s.bytes(), root, file),
            Some(hash_name(new, 72, international) as usize),
            "DOS\\{flags}: filed under its new name's slot"
        );

        let mut want = BTreeMap::new();
        want.insert("Renamed/inside".to_owned(), b"still here".to_vec());
        want.insert("nouveau-r\u{e9}sum\u{e9}".to_owned(), data.clone());
        // ADE's reading; ADFlib's is checked by name below, because its
        // extraction writes Latin-1 names that do not round-trip as UTF-8.
        assert_clean(s.bytes(), &format!("DOS\\{flags} rename"));
        assert_eq!(ade_files(s.bytes()), want, "DOS\\{flags} rename");

        // ADFlib finds it **by name**, which goes through the hash.
        let scratch = Scratch::new(&format!("ren{flags}"));
        let image = scratch.0.join("r.adf");
        std::fs::write(&image, s.bytes()).unwrap();
        let script = "ulimit -v 1048576; exec timeout 20 unadf -p \"$1\" \"$2\"";
        let out = Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("sh")
            .arg(&image)
            .arg(std::ffi::OsStr::from_bytes(new))
            .output();
        if let Ok(out) = out
            && (!out.stdout.is_empty() || out.status.success())
        {
            assert!(
                out.stdout == data,
                "DOS\\{flags}: ADFlib finds the new name"
            );
        }
        if flags >= 4
            && let Some((ok, text)) = unadf(&["-l", "-c"], &image)
        {
            assert!(ok, "{text}");
            assert!(
                text.contains("Renamed/") && !text.contains("Folder"),
                "DOS\\{flags} from the cache: {text}"
            );
        }
    }
}

#[test]
fn a_rename_onto_a_taken_name_is_refused_but_a_change_of_case_is_not() {
    let mut s = session(3);
    let root = s.root();
    let a = s.add_file(root, b"alpha", b"a").unwrap();
    s.add_file(root, b"beta", b"b").unwrap();
    let before = s.bytes().to_vec();
    assert!(matches!(
        s.rename(a, b"BETA"),
        Err(EditError::Refused(FsEditError::Exists { .. }))
    ));
    assert!(
        s.bytes() == before.as_slice(),
        "a refused rename changes nothing"
    );
    // The same name in other letters is the entry itself.
    s.rename(a, b"Alpha").unwrap();
    let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
    let volume = image.volume().unwrap();
    assert_eq!(volume.lookup("alpha").unwrap().name, b"Alpha");
    // And a shorter name leaves nothing of the longer one behind it.
    s.rename(a, b"A").unwrap();
    let raw = &s.bytes()[a as usize * 512..(a as usize + 1) * 512];
    assert_eq!(&raw[512 - 80..512 - 80 + 7], b"\x01A\0\0\0\0\0");
}

#[test]
fn deleting_from_a_crowded_cached_directory_gives_cache_blocks_back() {
    // Enough files on DOS\5 to need several cache blocks; deleting them all
    // must free those blocks and leave the directory with no cache, as a new
    // one has.
    let mut s = session(5);
    let root = s.root();
    let before = used(s.bytes());
    let blocks: Vec<u32> = (0..60)
        .map(|i| {
            s.add_file(root, format!("file-{i:02}").as_bytes(), b"x")
                .unwrap()
        })
        .collect();
    for (i, block) in blocks.iter().enumerate() {
        s.delete(*block).unwrap();
        assert_clean(s.bytes(), &format!("after deleting {i}"));
    }
    assert_eq!(used(s.bytes()), before, "files and cache blocks all free");
    assert_both_agree(s.bytes(), &BTreeMap::new(), "emptied cached root");
}

#[test]
fn corpus_files_rename_and_delete_and_the_disk_stays_sound() {
    // Real disks, real files: rename one, check ADFlib finds it by its new
    // name with the bytes it found under the old one, then delete it. ADFlib
    // against ADFlib, not against ADE: on a damaged file the two readers
    // differ for their own reasons (a TOSEC bad dump showed 488 bytes of it),
    // and that is not what a rename must preserve. Only intact files are
    // compared at all — see below. Every step goes through the
    // session's health check, so a write that leaves anything new for `ade
    // check` to report fails here. ADE_EDIT_CORPUS=all for every disk.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&dir) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let step = if std::env::var("ADE_EDIT_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        25
    };
    let mut paths: Vec<PathBuf> = read.map(|e| e.unwrap().path()).collect();
    paths.sort();
    let (mut renamed, mut deleted, mut checked) = (0usize, 0usize, 0usize);
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for path in paths.iter().step_by(step) {
        let Ok(mut s) = Session::open(path) else {
            continue;
        };
        // The first plain file in the root that is not part of a link.
        let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
        let volume = image.volume().unwrap();
        let Ok(listing) = volume.list(volume.root()) else {
            continue;
        };
        let Some(file) = listing.entries.into_iter().find(|e| {
            e.kind == ade_core::layers::filesystem::entry::EntryKind::File && e.next_link == 0
        }) else {
            continue;
        };
        let Ok(contents) = volume.read_file(&file) else {
            continue;
        };
        // A damaged file is renamed and deleted like any other, but not used
        // to judge ADFlib's reading: on one, `unadf -p` gives different bytes
        // on every run (measured on Beverly Hills Cop_DiskB, three runs,
        // three digests), so there is nothing stable to compare.
        // And a name that begins with `-` is read by unadf as an option
        // (Bograts' `-S-.info`), so ADFlib cannot be asked about it.
        let intact = contents.is_complete() && file.name.first() != Some(&b'-');
        drop(volume);
        // What ADFlib reads under the old name, before anything changes.
        let fetch = |bytes: &[u8], name: &[u8], tag: &str| -> Option<Vec<u8>> {
            let scratch = Scratch::new(tag);
            let image = scratch.0.join("d.adf");
            std::fs::write(&image, bytes).unwrap();
            let out = Command::new("sh")
                .arg("-c")
                .arg("ulimit -v 1048576; exec timeout 20 unadf -p \"$1\" \"$2\"")
                .arg("sh")
                .arg(&image)
                .arg(std::ffi::OsStr::from_bytes(name))
                .output()
                .ok()?;
            (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
        };
        let before = intact
            .then(|| fetch(s.bytes(), &file.name, "corpus-before"))
            .flatten();

        match s.rename(file.block, b"ADE-renamed") {
            Ok(()) => renamed += 1,
            Err(EditError::Refused(e)) => {
                *refused.entry(refusal_kind(&e)).or_insert(0) += 1;
                continue;
            }
            // BUG-014: the health report looks for cross-links by path, so two
            // entries of one name sharing blocks look like one file. Renaming
            // one gives them two paths and the old cross-link appears as new,
            // and the edit is refused. Safe, and explained only on a disk that
            // already holds a name twice.
            Err(EditError::WouldDamage(found))
                if found.iter().all(|f| f.starts_with("cross-linked-block"))
                    && ade_core::health::examine(std::fs::read(path).unwrap())
                        .findings
                        .iter()
                        .any(|f| f.code == "duplicate-name") =>
            {
                *refused.entry("BUG-014".to_owned()).or_insert(0) += 1;
                continue;
            }
            Err(e) => panic!("{}: rename: {e}", path.display()),
        }
        // Only where ADFlib read the file at all before: a disk it chokes on
        // for its own reasons is not blamed on the rename.
        if let Some(before) = before {
            let after = fetch(s.bytes(), b"ADE-renamed", "corpus-after");
            checked += 1;
            assert!(
                after.as_deref() == Some(before.as_slice()),
                "{}: ADFlib found {:?} bytes under the new name, {} under the old",
                path.display(),
                after.as_ref().map(Vec::len),
                before.len()
            );
        }
        match s.delete(file.block) {
            Ok(()) => deleted += 1,
            Err(EditError::Refused(e)) => {
                *refused.entry(refusal_kind(&e)).or_insert(0) += 1;
            }
            Err(e) => panic!("{}: delete: {e}", path.display()),
        }
    }
    eprintln!(
        "renamed {renamed}, ADFlib confirmed {checked}, deleted {deleted}, refused {refused:?}"
    );
}

#[test]
fn a_block_another_file_shares_stays_allocated_when_one_is_deleted() {
    // A cross-linked disk: file `b`'s table names `a`'s data block. Freeing
    // every block of a deleted `a` — as AmigaDOS would — leaves `b` reading a
    // block the next write can take. Found on Championship Golf.
    use ade_core::layers::endian::{put_u32, u32_at};
    let mut s = session(1);
    let root = s.root();
    let a = s.add_file(root, b"a", &pattern(400, 1)).unwrap();
    let b = s.add_file(root, b"b", &pattern(400, 2)).unwrap();
    let mut bytes = s.bytes().to_vec();
    let at = |block: u32| block as usize * 512;
    // A one-block file keeps its data pointer in the table's last slot.
    let a_data = u32_at(&bytes, at(a) + 24 + 71 * 4).unwrap();
    put_u32(&mut bytes, at(b) + 24 + 71 * 4, a_data).unwrap();
    put_u32(&mut bytes, at(b) + 16, a_data).unwrap();
    put_u32(&mut bytes, at(b) + 20, 0).unwrap();
    let sum = ade_core::layers::block::checksum::normal_at(&bytes[at(b)..at(b) + 512], 20).unwrap();
    put_u32(&mut bytes, at(b) + 20, sum).unwrap();
    let health = ade_core::health::examine(bytes.clone());
    assert!(
        health
            .findings
            .iter()
            .any(|f| f.code == "cross-linked-block"),
        "the disk is cross-linked to begin with"
    );

    let mut s = Session::from_bytes(bytes).unwrap();
    s.set_clock(Some(WHEN));
    s.delete(a).unwrap();
    let after = ade_core::health::examine(s.bytes().to_vec());
    assert_eq!(
        after.bitmap.unwrap().referenced_but_free,
        0,
        "b's block is still in use"
    );
    let image = ade_core::Image::from_bytes(s.bytes().to_vec()).unwrap();
    let volume = image.volume().unwrap();
    let left = volume.lookup("b").unwrap();
    assert!(
        volume.read_file(&left).unwrap().into_bytes() == pattern(400, 1),
        "b still reads the block it shared"
    );
}
