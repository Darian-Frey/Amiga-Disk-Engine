//! The D-002 oracle over **generated** fixtures.
//!
//! Companion to `oracle.rs`, which runs ADFlib over the real corpus. This one
//! needs no corpus at all: it generates volumes with `ade-fixtures` and checks
//! that ADFlib reads them the same way ADE does.
//!
//! # Why this closes a loop I thought was open
//!
//! D-010 chose to generate fixtures rather than commit images, and its original
//! text worried that a generator written by the same hand as the parser would
//! encode the same misreading, with both agreeing. That worry assumed the
//! oracle could only be pointed at real disks.
//!
//! It cannot be — ADFlib reads any structurally valid volume regardless of who
//! wrote it, so an independent implementation can check the generator *and* the
//! parser without a single real image. See D-010's 2026-08-24 amendment.
//!
//! What this still cannot catch is what the *specification omits*: a fixture is
//! only ever as good as SPEC. Reality is the corpus's job.
//!
//! Skips when `unadf` is absent. Every invocation is resource-capped, because
//! an uncapped one once allocated 29 GB and killed the session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::cast_possible_truncation,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::{fs, process::Command};

use ade_core::Image;
use ade_fixtures::{Volume as Fixture, device::Device};

const MEM_KIB: u64 = 1_048_576;
const TIMEOUT_S: u64 = 20;

fn have_unadf() -> bool {
    Command::new("unadf").output().is_ok()
}

/// Run `unadf -lr` under hard caps, returning its listing.
fn oracle_list(image: &std::path::Path) -> Option<String> {
    oracle_list_volume(image, None)
}

/// Run `unadf -c -l`, which lists a directory **from its dircache** rather
/// than by walking its hash chains.
///
/// This is the only oracle invocation that reads the cache at all, and so the
/// only external check that a cache ADE wrote is well-formed.
fn oracle_list_cached(image: &std::path::Path) -> Option<String> {
    let script = format!("ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf -c -l \"$1\"");
    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("sh")
        .arg(image)
        .output()
        .ok()?;
    out.status.code().filter(|c| *c == 0)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// As [`oracle_list`], but mounting one volume of a partitioned device.
///
/// ADFlib numbers partitions from zero in partition-list order, which is the
/// order [`Image::partitions`] returns them in, so the indices line up.
fn oracle_list_volume(image: &std::path::Path, volume: Option<usize>) -> Option<String> {
    let select = volume.map_or(String::new(), |v| format!("-v {v} "));
    let script = format!("ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf {select}-lr \"$1\"");
    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("sh")
        .arg(image)
        .output()
        .ok()?;
    out.status.code().filter(|c| *c == 0)?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn write_temp(bytes: &[u8], name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("ade-fixoracle-{name}-{}.adf", std::process::id()));
    fs::write(&p, bytes).expect("write fixture");
    p
}

#[test]
fn adflib_reads_our_generated_volumes() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping (apt install unadf)");
        return;
    }

    // Geometry, filesystem, and the dostype matrix. `DOS\4`..`DOS\7` are
    // generated with the classic layout — real dircache and LNFS structures are
    // Phase 2 — so what is checked here is identification and hashing, which is
    // where C-006 lives.
    let cases: Vec<(&str, Vec<u8>)> = (0u8..8)
        .map(|d| {
            let mut v = Fixture::dd(d).named("Matrix");
            // An accented name: international folding is the *only* difference
            // between the two hash functions, so this is the case that tells
            // them apart.
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_file("plain", b"ascii");
            v.add_dir("Tools");
            (
                match d {
                    0 => "dos0",
                    1 => "dos1",
                    2 => "dos2",
                    3 => "dos3",
                    4 => "dos4",
                    5 => "dos5",
                    6 => "dos6",
                    _ => "dos7",
                },
                v.build(),
            )
        })
        .chain(std::iter::once({
            let mut v = Fixture::hd(1).named("HighDensity");
            v.add_file("readme", b"high density");
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_dir("Tools");
            ("hd", v.build())
        }))
        .chain(std::iter::once({
            // An 8 MB hardfile: a raw volume with no floppy geometry, and five
            // bitmap blocks rather than one (BUG-006).
            let mut v = Fixture::new(512, 1, 32, 1).named("Hardfile");
            v.add_file("readme", b"a hardfile, not a floppy");
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_dir("Tools");
            ("hardfile", v.build())
        }))
        .chain(std::iter::once({
            // A file spanning several extension blocks, which the generator
            // could not build until IMP-004 — so this path had no oracle.
            let data: Vec<u8> = (0..200_000usize).map(|i| (i % 251) as u8).collect();
            let mut v = Fixture::new(512, 1, 32, 0).named("ExtChain");
            v.add_file("big.bin", &data);
            v.add_file("\u{e4}pfel", b"umlaut");
            v.add_dir("Tools");
            ("extension", v.build())
        }))
        .collect();

    let mut checked = 0usize;
    for (name, bytes) in cases {
        let path = write_temp(&bytes, name);
        let listing = oracle_list(&path);
        let ours = Image::open(&path).ok().and_then(|i| {
            i.volume()
                .ok()
                .and_then(|v| v.walk(v.root()).ok())
                .map(|w| w.entries.len())
        });
        let _ = fs::remove_file(&path);

        let listing =
            listing.unwrap_or_else(|| panic!("{name}: ADFlib refused a volume we generated"));
        let ours =
            ours.unwrap_or_else(|| panic!("{name}: ADE could not read a volume we generated"));

        // ADFlib prints a banner, a Device line and a Volume line, then one
        // line per entry. Counting by exclusion is sturdier than matching the
        // date format, which is how the first version of this got it wrong.
        let theirs = listing
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("unADF")
                    && !t.starts_with("Device")
                    && !t.starts_with("Volume")
                    && !t.starts_with("Warning")
            })
            .count();
        assert_eq!(
            theirs, ours,
            "{name}: ADFlib found {theirs} entries, ADE found {ours}\n{listing}"
        );
        // The accented name must survive both readers, which it only does if
        // the international hash was chosen correctly (C-006).
        assert!(
            listing.contains("pfel"),
            "{name}: the accented entry did not survive ADFlib\n{listing}"
        );
        checked += 1;
    }
    eprintln!("ADFlib agreed with ADE on {checked} generated volumes");
    assert_eq!(checked, 11);
}

#[test]
fn adflib_and_ade_agree_on_generated_file_contents() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping");
        return;
    }
    // Multi-block files, exercising the reversed data_blocks[] table and the
    // OFS/FFS payload difference (C-005) against an independent reader.
    // 9 KB fits a single header block; 200 KB spans several extension blocks,
    // which is the path IMP-004 brought under the oracle.
    for (label, dostype, size) in [
        ("ofs", 0u8, 9_000usize),
        ("ffs", 1, 9_000),
        ("ofs-ext", 0, 200_000),
        ("ffs-ext", 1, 200_000),
    ] {
        let payload: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut v = if size > 100_000 {
            Fixture::new(512, 1, 32, dostype).named("Contents")
        } else {
            Fixture::dd(dostype).named("Contents")
        };
        v.add_file("data.bin", &payload);
        let path = write_temp(&v.build(), label);

        let dir =
            std::env::temp_dir().join(format!("ade-fixoracle-out-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script =
            format!("ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf \"$1\" -d \"$2\"");
        let ok = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .arg("sh")
            .arg(&path)
            .arg(&dir)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "{label}: ADFlib could not extract from our fixture");

        let theirs = fs::read(dir.join("data.bin")).expect("ADFlib output");
        let ours = {
            let img = Image::open(&path).unwrap();
            let vol = img.volume().unwrap();
            let e = vol.lookup("data.bin").unwrap();
            vol.read_file(&e).unwrap()
        };
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(ours.bytes, payload, "{label}: ADE did not round-trip");
        assert_eq!(theirs, payload, "{label}: ADFlib did not round-trip");
    }
}

#[test]
fn adflib_reads_our_generated_device() {
    // A partitioned device is the one shape with no counterpart in the corpus:
    // every image we hold is a floppy. Without the oracle, the RDB parser and
    // the RDB generator would only ever be checked against each other.
    if !have_unadf() {
        eprintln!("unadf not installed — skipping (apt install unadf)");
        return;
    }

    let mut device = Device::new(64, 4, 32);
    device.add_partition("DH0", 2, 30, 1, true, |v| {
        v.add_file("startup", b"hello from DH0");
        v.add_file("\u{e4}pfel", b"umlaut");
        v.add_dir("Tools");
    });
    device.add_partition("DH1", 31, 63, 0, false, |v| {
        v.add_file("data.bin", &[0xAA; 3000]);
    });

    let path = write_temp(&device.build(), "device");
    let image = Image::open(&path).expect("open device");
    let (parts, faults) = image.partitions().expect("read partition table");
    assert!(faults.is_empty(), "clean device: {faults:?}");
    assert_eq!(parts.len(), 2, "ADE found the wrong number of partitions");

    let expected = [3usize, 1];
    for (index, part) in parts.iter().enumerate() {
        let listing = oracle_list_volume(&path, Some(index))
            .unwrap_or_else(|| panic!("ADFlib refused partition {index} of a device we generated"));

        // ADFlib reports the label from the partition's own rootblock, so this
        // also checks that both readers placed that rootblock in the same
        // place — the computation C-007 exists for.
        let label = part.name_lossy();
        assert!(
            listing.contains(&format!("\"{label}\"")),
            "partition {index}: ADFlib did not report the label {label:?}\n{listing}"
        );

        let window = image.partition_window(part).expect("window");
        let volume = ade_core::layers::filesystem::volume::Volume::mount(&window)
            .unwrap_or_else(|e| panic!("partition {index} did not mount: {e}"));
        let ours = volume.walk(volume.root()).expect("walk").entries.len();

        let theirs = listing
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.starts_with("unADF")
                    && !t.starts_with("Device")
                    && !t.starts_with("Volume")
                    && !t.starts_with("Warning")
            })
            .count();

        assert_eq!(
            theirs, ours,
            "partition {index}: ADFlib found {theirs} entries, ADE found {ours}\n{listing}"
        );
        assert_eq!(ours, expected[index], "partition {index} entry count");
    }

    let _ = fs::remove_file(&path);
    eprintln!("ADFlib agreed with ADE on both partitions of a generated device");
}

#[test]
fn adflib_reads_our_generated_dircache() {
    // `unadf -c` lists from the dircache instead of walking the hash chains,
    // so it parses the cache records byte for byte. Nothing else does: the
    // plain listing would pass even if every cache block were garbage.
    //
    // Worth its own test because the corpus holds no `DOS\4` at all, and the
    // 21 `DOS\5` images it does hold are read-only evidence — they cannot
    // check that a cache *we write* is well-formed.
    if !have_unadf() {
        eprintln!("unadf not installed — skipping (apt install unadf)");
        return;
    }

    for dostype in [4u8, 5] {
        let mut v = Fixture::dd(dostype).named("Cached");
        v.add_file("startup", b"hello");
        v.add_file("\u{e4}pfel", b"umlaut");
        v.add_dir("Tools");
        v.add_file("plain", b"x");
        let path = write_temp(&v.build(), &format!("dircache{dostype}"));

        let from_cache = oracle_list_cached(&path)
            .unwrap_or_else(|| panic!("DOS\\{dostype}: ADFlib refused a cache we generated"));
        let from_hash = oracle_list(&path)
            .unwrap_or_else(|| panic!("DOS\\{dostype}: ADFlib refused the volume"));
        let _ = fs::remove_file(&path);

        // ADFlib says so explicitly when it uses the cache; without this the
        // test would pass on a silent fallback to the hash chains and prove
        // nothing at all.
        assert!(
            from_cache.contains("Using dir cache blocks"),
            "DOS\\{dostype}: ADFlib did not use the cache\n{from_cache}"
        );
        assert!(
            from_cache.contains("DIRCACHE"),
            "DOS\\{dostype}: the volume was not identified as cached\n{from_cache}"
        );

        // The two listings describe the same directory, so they must agree —
        // that is the whole premise of a cache.
        let entries = |listing: &str| -> Vec<String> {
            listing
                .lines()
                .map(str::trim)
                .filter(|t| {
                    !t.is_empty()
                        && !t.starts_with("unADF")
                        && !t.starts_with("Device")
                        && !t.starts_with("Volume")
                        && !t.starts_with("Warning")
                        && !t.starts_with("Using dir cache")
                })
                .map(ToOwned::to_owned)
                .collect()
        };
        let mut cached = entries(&from_cache);
        let mut hashed = entries(&from_hash);
        cached.sort();
        hashed.sort();

        assert_eq!(
            cached, hashed,
            "DOS\\{dostype}: the cache and the hash chains disagree, \
             which means the cache we wrote is wrong\n{from_cache}\n{from_hash}"
        );
        assert_eq!(cached.len(), 4, "DOS\\{dostype}");
    }

    eprintln!("ADFlib read our generated dircache on both DOS\\4 and DOS\\5");
}
