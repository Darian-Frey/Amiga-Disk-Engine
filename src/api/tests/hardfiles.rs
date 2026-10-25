//! Real hard-disk images, against ADFlib (D-002) — local only.
//!
//! The corpus is floppies: not one of its 4,652 images carries an RDB, so the
//! partition path had only ever been checked against devices ADE generated
//! itself. Amiga Forever's FS-UAE hardfiles are the first real material:
//! `workbench-135.hdf` is an RDB device with one FFS partition, and
//! `workbench-211.hdf` and `workbench-311.hdf` are raw OFS and FFS hardfiles.
//! All three use 512-byte blocks, so they say nothing about larger ones.
//!
//! Measured 2026-10-25 by hand before this was written: ADE mounts DH0 at the
//! blocks ADFlib reports (2020–6059), `ade check` finds nothing, and the two
//! list the same 365 entries. This checks every file's bytes.
//!
//! The images are not ADE's to commit (D-010). The test reads the directory
//! named by `ADE_HARDFILES`, else `disks/hardfiles`, and skips cleanly when
//! there is none — or when `unadf` is not installed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use ade_core::Image;
use ade_core::layers::filesystem::volume::Volume;

const MEM_KIB: u64 = 1_048_576;
const TIMEOUT_S: u64 = 60;

fn hardfiles() -> Option<PathBuf> {
    let dir = std::env::var_os("ADE_HARDFILES").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks/hardfiles"),
        PathBuf::from,
    );
    dir.is_dir().then_some(dir)
}

/// Every file `unadf` extracts from one volume, by path. `-v` picks a
/// partition of a device; ADFlib numbers them from zero in list order, which
/// is the order `Image::partitions` returns.
fn oracle(image: &Path, volume: Option<usize>, into: &Path) -> Option<BTreeMap<PathBuf, Vec<u8>>> {
    let select = volume.map_or(String::new(), |v| format!("-v {v} "));
    let script = format!(
        "ulimit -v {MEM_KIB}; exec timeout {TIMEOUT_S} unadf {select}\"$1\" -d \"$2\" >/dev/null"
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("sh")
        .arg(image)
        .arg(into)
        .status()
        .ok()?;
    status.success().then_some(())?;
    let mut found = BTreeMap::new();
    let mut stack = vec![into.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                found.insert(p.strip_prefix(into).ok()?.to_path_buf(), fs::read(&p).ok()?);
            }
        }
    }
    Some(found)
}

/// Compare one mounted volume with ADFlib's extraction of it, file by file.
fn agree(name: &str, volume: &Volume<'_>, mut theirs: BTreeMap<PathBuf, Vec<u8>>) -> usize {
    let walked = volume.walk(volume.root()).unwrap();
    let mut compared = 0usize;
    for (_, entry) in walked.entries {
        if !entry.kind.is_file() {
            continue;
        }
        // Raw name bytes: Latin-1, written by unadf verbatim.
        let mut rel = PathBuf::new();
        for part in volume.path_components(&entry) {
            rel.push(OsStr::from_bytes(&part));
        }
        let ours = volume.read_file(&entry).unwrap();
        let their = theirs
            .remove(&rel)
            .unwrap_or_else(|| panic!("{name}: ADFlib has no {}", rel.display()));
        assert!(ours.bytes == their, "{name}: {} differs", rel.display());
        compared += 1;
    }
    assert!(
        theirs.is_empty(),
        "{name}: only ADFlib found {:?}",
        theirs.keys().take(5).collect::<Vec<_>>()
    );
    compared
}

#[test]
fn real_hardfiles_agree_with_adflib_file_for_file() {
    if Command::new("unadf").output().is_err() {
        eprintln!("skipping: unadf not installed");
        return;
    }
    let Some(dir) = hardfiles() else {
        eprintln!("skipping: no hardfiles (set ADE_HARDFILES, or put them in disks/hardfiles)");
        return;
    };
    let mut images: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("hdf")))
        .collect();
    images.sort();
    let tmp = std::env::temp_dir().join(format!("ade-hardfiles-{}", std::process::id()));
    let (mut volumes, mut files, mut partitioned) = (0usize, 0usize, 0usize);
    for path in &images {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = fs::read(path).unwrap();
        let image = Image::open(path).unwrap();
        let (parts, faults) = image.partitions().unwrap();
        assert!(
            faults.is_empty(),
            "{name}: partition table faults {faults:?}"
        );
        if parts.is_empty() {
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp).unwrap();
            let theirs = oracle(path, None, &tmp).expect("ADFlib reads it");
            files += agree(&name, &image.volume().unwrap(), theirs);
            let health = ade_core::health::examine(bytes);
            assert!(health.findings.is_empty(), "{name}: {:?}", health.findings);
            volumes += 1;
            continue;
        }
        partitioned += 1;
        for (i, part) in parts.iter().enumerate() {
            let _ = fs::remove_dir_all(&tmp);
            fs::create_dir_all(&tmp).unwrap();
            let theirs = oracle(path, Some(i), &tmp).expect("ADFlib reads the partition");
            let window = image.partition_window(part).unwrap();
            let volume = Volume::mount(&window).unwrap();
            files += agree(&format!("{name} {}", part.name_lossy()), &volume, theirs);
            let health =
                ade_core::health::examine_partition(bytes.clone(), Some(&part.name_lossy()));
            assert!(health.findings.is_empty(), "{name}: {:?}", health.findings);
            volumes += 1;
        }
    }
    let _ = fs::remove_dir_all(&tmp);
    eprintln!(
        "{} hardfiles, {partitioned} with an RDB: {volumes} volumes, {files} files byte-identical \
         to ADFlib",
        images.len()
    );
    assert!(volumes > 0, "{}: no .hdf images in it", dir.display());
}
