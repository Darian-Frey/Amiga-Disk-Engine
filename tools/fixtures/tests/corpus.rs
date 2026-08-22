//! Differential check of the generator against real images.
//!
//! The generator is an independent statement of the format, but it was written
//! by the same hand as the parser from the same sources, so a misreading of
//! SPEC survives in both (see the crate docs). This test is the other half of
//! D-010: it holds the generator's arithmetic against several thousand disks
//! that Commodore's own filesystem wrote.
//!
//! Skips when the corpus is absent, so a fresh clone still passes.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test code over data it constructs or has already length-checked"
)]

use ade_fixtures::{BSIZE, bootblock_checksum, get_u32, normal_checksum};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const CANONICAL: u64 = 901_120;

fn corpus() -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    d.is_dir().then(|| d.canonicalize().ok()).flatten()
}

fn images(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn checksum_algorithms_agree_with_real_disks() {
    let Some(root) = corpus() else {
        eprintln!("no corpus at ../../disks — skipping (D-010: images are never committed)");
        return;
    };

    let (mut dos, mut boot_ok, mut typed_roots, mut root_ok) = (0u32, 0u32, 0u32, 0u32);

    for path in images(&root) {
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if meta.len() < CANONICAL {
            continue;
        }
        let Ok(mut f) = fs::File::open(&path) else {
            continue;
        };
        let mut head = vec![0u8; BSIZE * 2];
        if f.read_exact(&mut head).is_err() {
            continue;
        }
        if &head[..3] != b"DOS" {
            continue;
        }
        dos += 1;
        if get_u32(&head, 4) == bootblock_checksum(&head) {
            boot_ok += 1;
        }

        // Rootblock at the computed midpoint, not the bootblock's claim (C-007).
        let mut rb = vec![0u8; BSIZE];
        if f.seek(SeekFrom::Start(880 * BSIZE as u64)).is_err() {
            continue;
        }
        if f.read_exact(&mut rb).is_err() {
            continue;
        }
        if get_u32(&rb, 0) == 2 && get_u32(&rb, BSIZE - 4) == 1 {
            typed_roots += 1;
            if get_u32(&rb, 20) == normal_checksum(&rb) {
                root_ok += 1;
            }
        }
    }

    assert!(
        dos > 100,
        "corpus too small to conclude anything ({dos} images)"
    );
    eprintln!("corpus: {dos} DOS images");
    eprintln!(
        "  bootblock checksum valid: {boot_ok} ({:.1}%)",
        100.0 * f64::from(boot_ok) / f64::from(dos)
    );
    eprintln!("  rootblocks with correct type/sec_type: {typed_roots}");
    eprintln!("  ...of which checksum valid: {root_ok}");

    // The real assertion. A block that identifies as a rootblock was written by
    // AmigaDOS, so its checksum must validate under our implementation. If the
    // algorithm were wrong, this would collapse rather than sit near 100%.
    let rate = f64::from(root_ok) / f64::from(typed_roots.max(1));
    assert!(
        rate > 0.995,
        "normal_checksum disagrees with real disks: only {root_ok}/{typed_roots} validate ({:.2}%)",
        rate * 100.0
    );

    // The bootblock algorithm is different, and most disks are not bootable, so
    // a low rate is expected and is itself the finding behind C-008. But it must
    // not be zero: that would mean the algorithm never matches anything.
    assert!(
        boot_ok > dos / 10,
        "bootblock_checksum matches almost nothing — likely wrong"
    );
}
