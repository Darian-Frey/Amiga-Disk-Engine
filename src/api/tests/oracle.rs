//! Differential test against ADFlib's `unadf` — the D-002 oracle.
//!
//! D-002 chose to reimplement OFS/FFS rather than wrap ADFlib, accepting the
//! loss of twenty-five years of accumulated edge-case handling. This is how
//! that loss is recovered: ADFlib runs as a **separate binary** and its output
//! is diffed, never linked and never read. Running a GPL program and comparing
//! its output creates no derived work, which is what keeps ADE's licence free
//! (D-011).
//!
//! # The oracle is resource-capped, and that is not paranoia
//!
//! On 2026-08-22 an uncapped run of this test allocated **29 GB inside unadf**
//! on `Bomb Busters_Disk1.adf` — an ordinary 901,120-byte TOSEC game disk —
//! and the kernel OOM killer terminated the session. A subsequent capped survey
//! found unadf crashes on **15 of 4288** real images. ADE reads all of them.
//!
//! Every invocation therefore runs under `ulimit -v` and `timeout`. The cap is
//! applied by `sh` rather than by `setrlimit`, because the workspace forbids
//! `unsafe` and `Command::pre_exec` requires it — an inconvenience that is
//! itself the D-001 memory-safety posture working as intended.
//!
//! Skips when `unadf` is missing or the corpus is absent, so a fresh clone
//! passes offline. Install the oracle with `apt install unadf`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
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

/// Address-space ceiling for the oracle, in KiB. An 880 KB disk needs a
/// rounding error of this; 1 GiB is generous and still cannot hurt the host.
const ORACLE_MEM_KIB: u64 = 1_048_576;
/// Wall-clock ceiling for one oracle invocation.
const ORACLE_TIMEOUT_S: u64 = 20;

/// How an oracle invocation ended.
enum Oracle {
    /// It extracted, yielding these files.
    Files(BTreeMap<PathBuf, Vec<u8>>),
    /// It declined to mount the image — a legitimate answer.
    Refused,
    /// It crashed, hung, or exhausted its cap.
    Crashed(String),
}

fn have_unadf() -> bool {
    Command::new("unadf").output().is_ok()
}

fn corpus() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    d.is_dir().then_some(d)
}

/// Run the oracle under hard resource caps.
///
/// Arguments are passed positionally to `sh` so that image names containing
/// spaces, quotes and apostrophes — all common in TOSEC — need no escaping.
fn oracle_extract(image: &Path, into: &Path) -> Oracle {
    let script = format!(
        "ulimit -v {ORACLE_MEM_KIB}; exec timeout {ORACLE_TIMEOUT_S} unadf \"$1\" -d \"$2\""
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .arg("sh")
        .arg(image)
        .arg(into)
        .output();
    let Ok(out) = out else {
        return Oracle::Crashed("could not spawn".to_owned());
    };
    match out.status.code() {
        Some(0) => {}
        // unadf exits 1 when it cannot mount, which is an answer, not a fault.
        Some(1) => return Oracle::Refused,
        Some(124) => return Oracle::Crashed(format!("timeout after {ORACLE_TIMEOUT_S}s")),
        Some(c) => return Oracle::Crashed(format!("exit {c}")),
        // No exit code means a signal: segfault, abort, or the OOM killer.
        None => return Oracle::Crashed("killed by signal".to_owned()),
    }

    let mut found = BTreeMap::new();
    let mut stack = vec![into.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let (Ok(rel), Ok(bytes)) = (p.strip_prefix(into), fs::read(&p)) {
                found.insert(rel.to_path_buf(), bytes);
            }
        }
    }
    Oracle::Files(found)
}

/// Rebuild an entry's path from raw name bytes.
///
/// Amiga names are Latin-1 and `unadf` writes them to the filesystem verbatim,
/// so going through a lossy `String` would re-encode them as UTF-8 and never
/// match.
fn path_of(
    volume: &ade_filesystem::volume::Volume<'_>,
    entry: &ade_filesystem::entry::Entry,
) -> PathBuf {
    let mut rel = PathBuf::new();
    for part in volume.path_components(entry) {
        rel.push(OsStr::from_bytes(&part));
    }
    rel
}

#[test]
fn extraction_agrees_with_adflib() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping the D-002 oracle (apt install unadf)");
        return;
    }
    let Some(root) = corpus() else {
        eprintln!("no corpus — skipping (D-010: images are never committed)");
        return;
    };

    let mut paths: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let step = (paths.len() / 120).max(1);
    let paths: Vec<_> = paths.iter().step_by(step).take(120).cloned().collect();

    let tmp = std::env::temp_dir().join(format!("ade-oracle-{}", std::process::id()));
    let mut disks = 0usize;
    let mut compared = 0usize;
    let mut identical = 0usize;
    let mut oracle_crashes: Vec<String> = Vec::new();
    let mut differing: Vec<String> = Vec::new();
    let mut only_ade: Vec<String> = Vec::new();

    for image in &paths {
        let disk = image.file_name().unwrap().to_string_lossy().into_owned();
        let _ = fs::remove_dir_all(&tmp);
        if fs::create_dir_all(&tmp).is_err() {
            continue;
        }
        let mut theirs = match oracle_extract(image, &tmp) {
            Oracle::Files(f) => f,
            Oracle::Refused => continue,
            Oracle::Crashed(why) => {
                oracle_crashes.push(format!("{disk} — {why}"));
                continue;
            }
        };
        let Ok(img) = Image::open(image) else {
            continue;
        };
        let Ok(volume) = img.volume() else { continue };
        let Ok(walked) = volume.walk(volume.root()) else {
            continue;
        };
        disks += 1;

        for (_, entry) in walked.entries {
            if !entry.kind.is_file() {
                continue;
            }
            let rel = path_of(&volume, &entry);
            let Some(their_bytes) = theirs.remove(&rel) else {
                only_ade.push(format!("{disk}:{}", rel.display()));
                continue;
            };
            let Ok(ours) = volume.read_file(&entry) else {
                only_ade.push(format!("{disk}:{} (read failed)", rel.display()));
                continue;
            };
            compared += 1;
            if ours.bytes == their_bytes {
                identical += 1;
            } else {
                differing.push(format!(
                    "{disk}:{} — ade {}, adflib {}",
                    rel.display(),
                    ours.bytes.len(),
                    their_bytes.len()
                ));
            }
        }
    }
    let _ = fs::remove_dir_all(&tmp);

    eprintln!("D-002 oracle: {disks} disks compared against ADFlib");
    eprintln!("  files compared:    {compared}");
    eprintln!(
        "  byte-identical:    {identical} ({:.2}%)",
        100.0 * identical as f64 / compared.max(1) as f64
    );
    eprintln!("  differing:         {}", differing.len());
    eprintln!("  only ADE found:    {}", only_ade.len());
    eprintln!("  ORACLE CRASHED on: {} disks", oracle_crashes.len());
    for c in oracle_crashes.iter().take(10) {
        eprintln!("    x {c}");
    }
    for d in differing.iter().take(10) {
        eprintln!("    ! {d}");
    }

    assert!(
        compared > 200,
        "expected a meaningful sample, compared {compared}"
    );
    let agreement = identical as f64 / compared as f64;
    assert!(
        agreement > 0.99,
        "ADE and ADFlib disagree on {:.2}% of files — that is a reader bug",
        (1.0 - agreement) * 100.0
    );
}

/// ADE must survive every image the oracle cannot.
///
/// This is F-001's claim stated as a test: the disks that crash ADFlib are
/// exactly the ones a never-crash core has to handle.
#[test]
fn ade_survives_what_the_oracle_does_not() {
    if !have_unadf() {
        eprintln!("unadf not installed — skipping");
        return;
    }
    let Some(root) = corpus() else {
        eprintln!("no corpus — skipping");
        return;
    };
    // Known from the 2026-08-22 corpus survey. Named rather than rediscovered,
    // because rediscovering means crashing unadf 4288 times on every run.
    let known = [
        "Bomb Busters_Disk1.adf",
        "Bomb Busters_Disk2.adf",
        "Elf (MicroValue).adf",
        "Fascination_Disk2.adf",
        "Heliosfera_Disk1.adf",
        "Midwinter II - Flames of Freedom_Disk1.adf",
        "Return to Genesis.adf",
        "Road Rash_Disk1.adf",
        "Sexy Droids.adf",
        "ThunderCats.adf",
        "Transworld.adf",
    ];
    let mut checked = 0usize;
    for name in known {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        checked += 1;
        // The claim is simply that these return.
        let Ok(image) = Image::open(&path) else {
            continue;
        };
        if let Ok(volume) = image.volume() {
            let walked = volume.walk(volume.root()).unwrap_or_default();
            for (_, entry) in walked.entries {
                if entry.kind.is_file() {
                    let _ = volume.read_file(&entry);
                }
            }
        }
    }
    eprintln!("ADE handled {checked} images that crash ADFlib, without incident");
    assert!(checked > 0, "none of the known-bad images were present");
}

#[test]
fn a_volume_smaller_than_its_file_extracts_the_same_as_adflib() {
    // BUG-009. A disk with extra cylinders is not a larger volume: it is an
    // 80-cylinder filesystem in a file that carries extra tracks, and its
    // rootblock is still at 880. ADE used to compute the rootblock from the
    // *file's* block count, land on a file header, and report no volume.
    //
    // ADFlib gets this right and says so plainly — `Volume : Floppy 880
    // KBytes, "LINGO" between sectors [0-1759]` for a 912,384-byte file — so
    // it is exactly the right thing to check the fix against. Mounting such an
    // image is not enough on its own: a wrongly adopted geometry can find a
    // rootblock-shaped block and produce confident nonsense.
    //
    // **This is not the regression guard.** With the bug present no image is
    // rescued, so there is nothing to compare and this passes vacuously. What
    // it proves is that the images the fix rescues are read *correctly*;
    // `tests/oversize.rs` is what fails if the fix is removed.
    if !have_unadf() {
        eprintln!("unadf not installed — skipping the D-002 oracle");
        return;
    }
    let Some(root) = corpus() else {
        eprintln!("no corpus — skipping (D-010: images are never committed)");
        return;
    };

    let tmp = std::env::temp_dir().join(format!("ade-oracle-oversize-{}", std::process::id()));
    let mut examined = 0usize;
    let mut compared = 0usize;
    let mut identical = 0usize;
    let mut differing: Vec<String> = Vec::new();

    let mut paths: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();

    for image in &paths {
        // Only plain images whose file is a little larger than a DD floppy but
        // smaller than an HD one — the shape this bug was about. An extended
        // ADF is also "larger than its volume" and is a different mechanism.
        let Ok(size) = fs::metadata(image).map(|m| m.len()) else {
            continue;
        };
        if !(901_121..1_802_240).contains(&size) {
            continue;
        }
        let Ok(img) = Image::open(image) else {
            continue;
        };
        if img.geometry().total_bytes() != 901_120 {
            continue;
        }
        let Ok(volume) = img.volume() else { continue };
        let Ok(walked) = volume.walk(volume.root()) else {
            continue;
        };

        let disk = image.file_name().unwrap().to_string_lossy().into_owned();
        let _ = fs::remove_dir_all(&tmp);
        if fs::create_dir_all(&tmp).is_err() {
            continue;
        }
        let mut theirs = match oracle_extract(image, &tmp) {
            Oracle::Files(f) => f,
            // ADFlib declining tells us nothing either way about the bytes.
            Oracle::Refused | Oracle::Crashed(_) => continue,
        };
        examined += 1;

        for (_, entry) in walked.entries {
            if !entry.kind.is_file() {
                continue;
            }
            let rel = path_of(&volume, &entry);
            let Some(their_bytes) = theirs.remove(&rel) else {
                continue;
            };
            let Ok(ours) = volume.read_file(&entry) else {
                continue;
            };
            compared += 1;
            if ours.bytes == their_bytes {
                identical += 1;
            } else {
                differing.push(format!("{disk}:{}", rel.display()));
            }
        }
    }
    let _ = fs::remove_dir_all(&tmp);

    eprintln!("BUG-009: {examined} images whose volume is smaller than their file");
    eprintln!("  files compared: {compared}, byte-identical: {identical}");
    if examined == 0 {
        eprintln!("  none in this corpus — nothing to compare");
        return;
    }
    assert!(
        differing.is_empty(),
        "ADE and ADFlib disagree on a rescued image: {differing:?}"
    );
    assert!(
        compared > 0,
        "images were rescued but no file could be compared, which proves nothing"
    );
}
