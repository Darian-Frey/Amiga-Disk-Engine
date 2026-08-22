//! `ade` — the command-line front-end.
//!
//! Deliberately thin: every capability lives behind [`ade_core`] so the CLI and
//! the GUI share one engine (F-002). Nothing here parses a disk; this file
//! decides what to print and with which exit code.
//!
//! # Exit codes (F-015)
//!
//! Part of the scriptable surface from the first command, not an afterthought.
//! They distinguish *the tool failed* from *the image has problems*, because a
//! batch run over thousands of images needs to tell those apart.
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | Inspected; an AmigaDOS volume was found and has no faults |
//! | 1 | Inspected; the volume has faults |
//! | 2 | Usage error |
//! | 3 | The image could not be read at all |
//! | 4 | Inspected; no AmigaDOS volume found |
//!
//! 4 is separate from both 0 and 1 deliberately. Reporting "clean" for an image
//! whose filesystem could not be read would be misleading, and calling it a
//! fault would be wrong — 1054 of 4288 real images have no rootblock where one
//! should be, and many are simply not AmigaDOS disks. A batch run (F-014) needs
//! to bucket clean, faulty, not-AmigaDOS and unreadable separately, so the
//! exit codes distinguish them.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use ade_core::{Inspection, inspect_path};

/// No faults found.
const EXIT_CLEAN: u8 = 0;
/// The image was read, and has faults.
const EXIT_FAULTS: u8 = 1;
/// The command line was wrong.
const EXIT_USAGE: u8 = 2;
/// The image could not be read.
const EXIT_UNREADABLE: u8 = 3;
/// The image was read, but holds no AmigaDOS volume.
const EXIT_NO_VOLUME: u8 = 4;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("info") if args.len() == 2 => match args.get(1) {
            Some(p) => info(&PathBuf::from(p)),
            None => ExitCode::from(EXIT_USAGE),
        },
        Some("--version" | "-V") => {
            println!("ade {}", ade_core::version());
            ExitCode::from(EXIT_CLEAN)
        }
        Some("--help" | "-h") | None => {
            usage();
            ExitCode::from(if args.is_empty() {
                EXIT_USAGE
            } else {
                EXIT_CLEAN
            })
        }
        _ => {
            usage();
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn usage() {
    println!("ade {} — Amiga Disk Engine", ade_core::version());
    println!();
    println!("USAGE:");
    println!("    ade info <image>     inspect a disk image");
    println!("    ade --version");
    println!();
    println!("EXIT CODES:");
    println!("    0  clean   1  faults   2  usage   3  unreadable   4  no volume");
}

fn info(path: &Path) -> ExitCode {
    let inspection = match inspect_path(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    let faults = report(path, &inspection);
    // No volume outranks faults: it is the more fundamental fact, and the
    // faults are printed either way.
    ExitCode::from(if inspection.volume.is_none() {
        EXIT_NO_VOLUME
    } else if faults {
        EXIT_FAULTS
    } else {
        EXIT_CLEAN
    })
}

/// Print the report. Returns whether anything was found worth flagging.
///
/// Container and volume are reported as two independent facts, never collapsed
/// into one verdict (C-008) — a `DOS` prefix does not imply a mountable volume
/// and its absence does not preclude one.
fn report(path: &Path, i: &Inspection) -> bool {
    let mut faults = Vec::new();
    println!("{}", path.display());
    report_container(i);
    report_bootblock(i, &mut faults);
    report_volume(i, &mut faults);

    if faults.is_empty() {
        println!("  faults      none");
        false
    } else {
        println!("  faults");
        for f in &faults {
            println!("    ! {f}");
        }
        true
    }
}

fn report_container(i: &Inspection) {
    println!("  container   {}", i.detection.kind);
    println!("  size        {} bytes", i.size);
    if let Some(g) = i.geometry {
        println!(
            "  geometry    {} cylinders x {} heads x {} sectors x {} bytes = {} blocks",
            g.cylinders(),
            g.heads(),
            g.sectors(),
            g.block_size(),
            g.total_blocks()
        );
    }
    println!("  evidence");
    for e in &i.detection.evidence {
        println!("    - {e}");
    }
}

fn report_bootblock(i: &Inspection, faults: &mut Vec<String>) {
    let Some(bb) = &i.bootblock else {
        println!("  bootblock   absent — image too short");
        return;
    };
    println!("  bootblock");
    match &bb.dostype {
        Ok(d) => println!("    dostype     {d}"),
        Err(e) => println!("    dostype     none — {e}"),
    }
    if bb.checksum_valid {
        println!("    checksum    valid");
    } else {
        // Not a fault: only bootable disks need one, and 26% of real images
        // lack it (C-008).
        println!(
            "    checksum    invalid (stored {:#010x}) — normal for a non-bootable disk",
            bb.stored_checksum
        );
    }
    println!(
        "    boot code   {}",
        if bb.has_boot_code {
            "present (never executed)"
        } else {
            "none"
        }
    );
    if !bb.is_dos() {
        println!("    prefix      {:?} — not DOS", bb.prefix_display());
    }
    if let Ok(d) = &bb.dostype
        && d.unrecognised_flags() != 0
    {
        faults.push(format!(
            "dostype carries undocumented bits {:#04x}",
            d.unrecognised_flags()
        ));
    }
}

fn report_volume(i: &Inspection, faults: &mut Vec<String>) {
    let Some(v) = &i.volume else {
        println!("  volume      none");
        if let Some(why) = &i.volume_absent {
            println!("              {why}");
        }
        return;
    };
    let r = &v.rootblock;
    println!("  volume");
    println!("    name        {:?}", r.name_lossy());
    println!("    rootblock   block {} (computed)", v.rootblock_at);
    println!(
        "    checksum    {}",
        if r.checksum_valid { "valid" } else { "INVALID" }
    );
    println!(
        "    bitmap      {}",
        if r.bitmap_flag_valid() {
            "flagged valid"
        } else {
            "flagged INVALID"
        }
    );
    println!("    created     {}", r.created);
    println!("    modified    {}", r.volume_altered);

    if !r.checksum_valid {
        faults.push("rootblock checksum does not match".to_owned());
    }
    if !r.bitmap_flag_valid() {
        faults.push("bitmap-valid flag is clear — the bitmap may be stale".to_owned());
    }
    if r.name_length_overflows() {
        faults.push(format!(
            "volume name length claims {} bytes in a 30-byte field",
            r.declared_name_len
        ));
    }
    for (label, stamp) in [
        ("created", r.created),
        ("modified", r.volume_altered),
        ("root altered", r.root_altered),
    ] {
        for fault in stamp.faults() {
            faults.push(format!("{label} datestamp: {fault}"));
        }
    }
}
