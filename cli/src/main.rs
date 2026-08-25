//! `ade` — the command-line front-end.
//!
//! Deliberately thin: every capability lives behind [`ade_core`] so the CLI and
//! the GUI share one engine (F-002). Nothing here parses a disk; this file
//! decides what to print, in which format, and with which exit code.
//!
//! # Output formats (F-015)
//!
//! `--format=text` (default) is for people and may be reworded freely.
//! `--format=json` is the scriptable surface and is a **commitment**: field
//! names and fault codes do not change once released. `ls` emits JSON Lines,
//! one object per entry, so a large directory streams; `info` emits a single
//! object.
//!
//! Text output is explicitly *not* parseable — Amiga filenames routinely
//! contain spaces, so no column layout can be split reliably. That was IMP-001,
//! and the fix is this flag rather than a cleverer layout.
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
//! should be, and many are simply not AmigaDOS disks.

use std::{
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use ade_core::{
    Conversion, Health, Image, Inspection, Severity, conversion, entry_to_json, examine_partition,
    inspect_path,
    layers::{
        container::Window,
        filesystem::{rdb::Partition, volume::Volume},
    },
};

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
/// `check` found something that would lose or corrupt data.
///
/// Distinct from 1 because the difference between "this disk is odd" and "do
/// not write to this disk" is the one a batch run most needs to act on.
const EXIT_DATA_AT_RISK: u8 = 5;

/// Write one line to stdout, reporting whether the stream is still open.
///
/// A closed pipe is how `| head` ends a command: ordinary, not an error. But
/// `println!` **panics** on it, which for a tool designed to be piped is
/// unacceptable — and restoring the default `SIGPIPE` disposition needs a
/// `libc` call behind `unsafe`, which the workspace forbids (D-001). So every
/// line of output goes through here, and a closed pipe simply stops the loop.
fn emit(out: &mut impl Write, line: &str) -> bool {
    match writeln!(out, "{line}") {
        Ok(()) => true,
        // Broken pipe or anything else: stop writing. There is nowhere left to
        // report the failure to.
        Err(e) => {
            debug_assert!(
                e.kind() == ErrorKind::BrokenPipe,
                "unexpected stdout failure: {e}"
            );
            false
        }
    }
}

/// How to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// For people. Layout may change; do not parse it.
    Text,
    /// For machines. Field names and fault codes are stable.
    Json,
}

/// The command line, after parsing.
struct Args {
    command: String,
    positional: Vec<String>,
    format: Format,
    /// Which partition of a hard disk to act on, by name or index.
    partition: Option<String>,
}

fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut format = Format::Text;
    let mut partition: Option<String> = None;
    for arg in raw {
        match arg.as_str() {
            "--format=json" | "--json" => format = Format::Json,
            "--format=text" => format = Format::Text,
            other if other.starts_with("--format=") => {
                return Err(format!(
                    "unknown format: {}",
                    other.trim_start_matches("--format=")
                ));
            }
            other if other.starts_with("--partition=") => {
                partition = Some(other.trim_start_matches("--partition=").to_owned());
            }
            "--version" | "-V" | "--help" | "-h" => positional.push(arg),
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            _ => positional.push(arg),
        }
    }
    let command = if positional.is_empty() {
        String::new()
    } else {
        positional.remove(0)
    };
    Ok(Args {
        command,
        positional,
        format,
        partition,
    })
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1).collect()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ade: {e}");
            usage();
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let p = |n: usize| args.positional.get(n).map_or("", String::as_str);

    match (args.command.as_str(), args.positional.len()) {
        ("info", 1) => info(Path::new(p(0)), args.format),
        ("check", 1) => check(Path::new(p(0)), args.format, args.partition.as_deref()),
        ("ls", 1) => list(
            Path::new(p(0)),
            None,
            args.format,
            args.partition.as_deref(),
        ),
        ("ls", 2) => list(
            Path::new(p(0)),
            Some(p(1)),
            args.format,
            args.partition.as_deref(),
        ),
        ("extract", 2) => extract(Path::new(p(0)), p(1), None, args.partition.as_deref()),
        ("extract", 3) => extract(
            Path::new(p(0)),
            p(1),
            Some(PathBuf::from(p(2))),
            args.partition.as_deref(),
        ),
        ("convert", 2) => convert(Path::new(p(0)), Path::new(p(1))),
        ("formats", 0) => formats(),
        ("--version" | "-V", 0) => {
            println!("ade {}", ade_core::version());
            ExitCode::from(EXIT_CLEAN)
        }
        ("--help" | "-h", 0) => {
            usage();
            ExitCode::from(EXIT_CLEAN)
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
    println!("    ade info <image>                   inspect a disk image");
    println!("    ade check <image>                  full health report (F-010)");
    println!("    ade ls <image> [path]              list a directory");
    println!("    ade extract <image> <path> [dest]  extract a file");
    println!("    ade convert <in> <out>             convert between containers (F-016)");
    println!("    ade formats                        what converts to what, and what it costs");
    println!("    ade --version");
    println!();
    println!("OPTIONS:");
    println!("    --format=text   human-readable (default); layout is not stable");
    println!("    --format=json   machine-readable; field names and fault codes are stable");
    println!("    --partition=P   which partition of a hard disk, by name (DH0) or index (0)");
    println!();
    println!("EXIT CODES:");
    println!("    0  clean   1  faults   2  usage   3  unreadable   4  no volume");
    println!();
    println!("`check` exits 1 on any warning, and 5 when an error would lose data.");
}

fn info(path: &Path, format: Format) -> ExitCode {
    let inspection = match inspect_path(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => {
            emit(&mut out, &inspection.to_json().to_json());
        }
        Format::Text => report_text(&mut out, path, &inspection),
    }
    let _ = out.flush();

    ExitCode::from(
        if inspection.volume.is_none() && inspection.partitions.is_empty() {
            EXIT_NO_VOLUME
        } else if inspection.faults().is_empty() {
            EXIT_CLEAN
        } else {
            EXIT_FAULTS
        },
    )
}

/// The human rendering.
///
/// Faults come from the engine, so the two output formats cannot drift apart
/// about what is wrong with an image.
/// The device sections of the text report: the Rigid Disk Block, and the
/// partition table it points at.
///
/// Neither appears for a floppy, which is most images — a disk having no
/// partition table is not a fault worth a line.
fn device_lines(lines: &mut Vec<String>, i: &Inspection) {
    if let Some(t) = &i.tracks {
        // A mix of kinds is the signature of copy protection, not a defect:
        // the raw tracks are what a plain ADF could not have carried.
        lines.push(format!("  tracks      {} declared", t.declared));
        lines.push(format!(
            "    sectors     {} ordinary AmigaDOS tracks",
            t.sectors
        ));
        lines.push(format!("    raw MFM     {} tracks", t.raw_mfm));
        if t.empty > 0 {
            lines.push(format!(
                "    empty       {} tracks (unformatted or not captured)",
                t.empty
            ));
        }
        if t.present < t.declared {
            lines.push(format!(
                "    present     {} of {} — the file does not reach the rest",
                t.present, t.declared
            ));
        }
        for fault in t.faults.iter().take(4) {
            lines.push(format!("    ! {fault}"));
        }
    }

    if let Some(d) = &i.description {
        // The disk's own words about itself, usually ASCII art from whoever
        // released it. Indented rather than quoted, because the layout is the
        // content.
        lines.push(format!(
            "  description {} ({} bytes)",
            d.file, d.declared_size
        ));
        for line in d.text.lines() {
            lines.push(format!("    | {}", line.trim_end()));
        }
        if d.truncated {
            lines.push("    | ... (truncated)".to_owned());
        }
    }

    if !i.boot_text.is_empty() {
        // Shown as found, not interpreted. Some of it is a publisher banner,
        // some a copy-protection notice, some a virus killer's menu — telling
        // them apart is a reader's job, not a scanner's (D-014).
        lines.push("  boot text".to_owned());
        for t in &i.boot_text {
            lines.push(format!("    {:>4}  {:?}", t.offset, t.text));
        }
    }

    if let Some(r) = &i.rdb {
        lines.push("  rigid disk block".to_owned());
        lines.push(format!("    at          block {}", r.block));
        lines.push(format!(
            "    checksum    {}",
            if r.checksum_valid { "valid" } else { "INVALID" }
        ));
        lines.push(format!(
            "    drive       {} cylinders x {} heads x {} sectors x {} bytes",
            r.cylinders, r.heads, r.sectors, r.block_size
        ));
        lines.push(format!("    reserved    up to block {}", r.high_rdsk_block));
        let drive = [r.vendor.as_str(), r.product.as_str(), r.revision.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !drive.is_empty() {
            lines.push(format!("    identity    {drive:?}"));
        }
    }

    if !i.partitions.is_empty() {
        lines.push(format!("  partitions  {}", i.partitions.len()));
        for p in &i.partitions {
            lines.push(format!(
                "    {:<8} cylinders {}..{}, block {} + {} x {} bytes",
                p.name, p.low_cylinder, p.high_cylinder, p.first_block, p.blocks, p.block_size
            ));
            lines.push(format!(
                "             dostype {:#010x}{}{}",
                p.dostype,
                if p.bootable { ", bootable" } else { "" },
                if p.checksum_valid {
                    ""
                } else {
                    ", PART checksum INVALID"
                }
            ));
            match (&p.volume_name, &p.mount_error) {
                (Some(name), _) => lines.push(format!("             volume {name:?}")),
                (None, Some(why)) => lines.push(format!("             does not mount — {why}")),
                (None, None) => {}
            }
        }
    }
    for f in &i.partition_faults {
        lines.push(format!("    ! partition table: {f}"));
    }
}

/// The bootblock section of the text report.
///
/// Absent on a device, whose block 0 is a Rigid Disk Block rather than a
/// bootblock — parsing one there produces a confident report about nothing.
fn bootblock_lines(lines: &mut Vec<String>, i: &Inspection) {
    if let Some(bb) = &i.bootblock {
        lines.push("  bootblock".to_owned());
        match &bb.dostype {
            Ok(d) => lines.push(format!("    dostype     {d}")),
            Err(e) => lines.push(format!("    dostype     none — {e}")),
        }
        if bb.checksum_valid {
            lines.push("    checksum    valid".to_owned());
        } else {
            lines.push(format!(
                "    checksum    invalid (stored {:#010x}) — normal for a non-bootable disk",
                bb.stored_checksum
            ));
        }
        lines.push(format!(
            "    boot code   {}",
            if bb.has_boot_code {
                "present (never executed)"
            } else {
                "none"
            }
        ));
        if !bb.is_dos() {
            lines.push(format!(
                "    prefix      {:?} — not DOS",
                bb.prefix_display()
            ));
        }
    } else if !i.detection.kind.has_bootblock() {
        // Not a defect: this container does not begin with one.
        lines.push(format!(
            "  bootblock   none — {} has no bootblock at block 0",
            i.detection.kind
        ));
    } else if i.rdb.is_none() {
        lines.push("  bootblock   absent — image too short".to_owned());
    }
}

fn report_text(out: &mut impl Write, path: &Path, i: &Inspection) {
    let mut lines: Vec<String> = vec![
        format!("{}", path.display()),
        format!("  container   {}", i.detection.kind),
        format!("  size        {} bytes", i.size),
    ];
    if let Some(g) = i.geometry {
        if i.rdb.is_some() {
            // A device is addressed linearly; the drive's own cylinder geometry
            // is reported below, where it belongs, since only the partition
            // extents are expressed in it.
            lines.push(format!(
                "  addressing  {} blocks x {} bytes, linear",
                g.total_blocks(),
                g.block_size()
            ));
        } else {
            lines.push(format!(
                "  geometry    {} cylinders x {} heads x {} sectors x {} bytes = {} blocks",
                g.cylinders(),
                g.heads(),
                g.sectors(),
                g.block_size(),
                g.total_blocks()
            ));
        }
    }
    if let Some(c) = &i.compression {
        match (c.decompressed_size, &c.error) {
            (Some(n), _) => lines.push(format!(
                "  compressed  {}, {} bytes on disk -> {n} bytes",
                c.kind, c.compressed_size
            )),
            (None, Some(why)) => lines.push(format!(
                "  compressed  {} — could not be decompressed: {why}",
                c.kind
            )),
            (None, None) => {}
        }
    }
    lines.push("  evidence".to_owned());
    for e in &i.detection.evidence {
        lines.push(format!("    - {e}"));
    }

    bootblock_lines(&mut lines, i);

    device_lines(&mut lines, i);

    if let Some(v) = &i.volume {
        let r = &v.rootblock;
        lines.push("  volume".to_owned());
        lines.push(format!("    name        {:?}", r.name_lossy()));
        lines.push(format!(
            "    rootblock   block {} (computed)",
            v.rootblock_at
        ));
        lines.push(format!(
            "    checksum    {}",
            if r.checksum_valid { "valid" } else { "INVALID" }
        ));
        lines.push(format!(
            "    bitmap      {}",
            if r.bitmap_flag_valid() {
                "flagged valid"
            } else {
                "flagged INVALID"
            }
        ));
        lines.push(format!("    created     {}", r.created));
        lines.push(format!("    modified    {}", r.volume_altered));
    } else if i.partitions.is_empty() {
        lines.push("  volume      none".to_owned());
        if let Some(why) = &i.volume_absent {
            lines.push(format!("              {why}"));
        }
    } else {
        // A partitioned device holds no volume of its own, by design.
        lines.push("  volume      none — the volumes are in the partitions".to_owned());
    }

    let faults = i.faults();
    if faults.is_empty() {
        lines.push("  faults      none".to_owned());
    } else {
        lines.push("  faults".to_owned());
        for f in &faults {
            lines.push(format!("    ! [{}] {f}", f.code));
        }
    }

    for line in &lines {
        if !emit(out, line) {
            return;
        }
    }
}

/// List a directory.
///
/// Faults found while walking are reported but do not stop the listing — a
/// directory with one broken hash chain still has usable entries in its other
/// slots.
fn list(path: &Path, dir: Option<&str>, format: Format, partition: Option<&str>) -> ExitCode {
    let image = match Image::open(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    let window = match select_partition(&image, partition) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_NO_VOLUME);
        }
    };
    let mounted = match &window {
        Some(w) => Volume::mount(w),
        None => image.volume(),
    };
    let volume = match mounted {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_NO_VOLUME);
        }
    };

    let start = match dir {
        Some(p) => match volume.lookup(p) {
            Ok(e) if e.kind.is_directory() => e.block,
            Ok(e) => {
                eprintln!("ade: {}: not a directory", e.name_lossy());
                return ExitCode::from(EXIT_FAULTS);
            }
            Err(e) => {
                eprintln!("ade: {e}");
                return ExitCode::from(EXIT_FAULTS);
            }
        },
        None => volume.root(),
    };

    let listing = match volume.list(start) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ade: {e}");
            return ExitCode::from(EXIT_FAULTS);
        }
    };
    let clean = listing.is_clean();
    let mut entries = listing.entries;
    entries.sort_by_key(|e| e.name_lossy().to_lowercase());

    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => {
            for e in &entries {
                let json = entry_to_json(e, &volume.path_components(e)).to_json();
                if !emit(&mut out, &json) {
                    break;
                }
            }
        }
        Format::Text => {
            for e in &entries {
                let size = if e.kind.is_file() {
                    format!("{:>9}", e.byte_size)
                } else {
                    format!("{:>9}", e.kind.to_string())
                };
                let mut line = format!(
                    "{size}  {}  {}  {}",
                    e.protection.to_amigados_string(),
                    e.altered,
                    e.name_lossy()
                );
                if e.kind.is_link() {
                    use std::fmt::Write as _;
                    match volume.resolve(e) {
                        Ok(t) => {
                            let _ = write!(line, "  -> {}", t.name_lossy());
                        }
                        Err(err) => {
                            let _ = write!(line, "  -> BROKEN ({err})");
                        }
                    }
                }
                if !e.comment.is_empty() {
                    use std::fmt::Write as _;
                    let _ = write!(line, "   ; {}", e.comment_lossy());
                }
                if !emit(&mut out, &line) {
                    break;
                }
            }
            emit(&mut out, &format!("{} entries", entries.len()));
        }
    }
    let _ = out.flush();

    // Cycles go to stderr, so they never contaminate JSON on stdout. They are
    // the AV-001 case: on a real disk they mean either a hard link to a
    // directory, or corruption.
    for c in listing.cycles.iter().chain(&listing.faults) {
        eprintln!("  ! {c}");
    }
    ExitCode::from(if clean { EXIT_CLEAN } else { EXIT_FAULTS })
}

/// Extract one file to disk, or to stdout when no destination is given.
fn extract(path: &Path, inner: &str, dest: Option<PathBuf>, partition: Option<&str>) -> ExitCode {
    let image = match Image::open(path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    let window = match select_partition(&image, partition) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_NO_VOLUME);
        }
    };
    let mounted = match &window {
        Some(w) => Volume::mount(w),
        None => image.volume(),
    };
    let volume = match mounted {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_NO_VOLUME);
        }
    };
    let entry = match volume.lookup(inner) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("ade: {e}");
            return ExitCode::from(EXIT_FAULTS);
        }
    };
    let data = match volume.read_file(&entry) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ade: {inner}: {e}");
            return ExitCode::from(EXIT_FAULTS);
        }
    };
    if !data.is_full_length() {
        eprintln!(
            "ade: {inner}: recovered {} of {} declared bytes — {} short",
            data.bytes.len(),
            data.declared_size,
            data.short_by
        );
    }
    // Structural faults are reported even when the byte count came out right:
    // a file can be exactly its declared length and still have stopped being a
    // file part-way through (IMP-002).
    for fault in &data.faults {
        eprintln!("ade: {inner}: {fault}");
    }
    let incomplete = !data.is_complete();
    let data = data.into_bytes();

    if let Some(out) = dest {
        if let Err(e) = std::fs::write(&out, &data) {
            eprintln!("ade: {}: {e}", out.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
        eprintln!("{} bytes -> {}", data.len(), out.display());
    } else {
        let mut stdout = std::io::stdout().lock();
        // A closed pipe here is ordinary: `ade extract ... | head -c 100`.
        if stdout.write_all(&data).is_err() {
            return ExitCode::from(EXIT_CLEAN);
        }
        let _ = stdout.flush();
    }
    ExitCode::from(if incomplete { EXIT_FAULTS } else { EXIT_CLEAN })
}

/// Report an image's condition (F-010).
fn check(path: &Path, format: Format, partition: Option<&str>) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    let health = examine_partition(bytes, partition);
    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => {
            emit(&mut out, &health.to_json().to_json());
        }
        Format::Text => report_health(&mut out, path, &health),
    }
    let _ = out.flush();

    ExitCode::from(match health.worst() {
        Some(Severity::Error) => EXIT_DATA_AT_RISK,
        Some(Severity::Warning) => EXIT_FAULTS,
        // A device holds no volume at its own rootblock, so what matters is
        // whether anything was examined, not whether the image itself mounted.
        _ if health.examined.is_none() => EXIT_NO_VOLUME,
        _ => EXIT_CLEAN,
    })
}

fn report_health(out: &mut impl Write, path: &Path, h: &Health) {
    let mut lines = vec![
        format!("{}", path.display()),
        format!("  container   {}", h.inspection.detection.kind),
    ];
    if let Some(e) = &h.examined {
        // On a device this names the partition, so a report cannot be mistaken
        // for one covering the whole disk.
        match &e.partition {
            Some(p) => lines.push(format!(
                "  volume      {:?} on partition {p}  (rootblock {})",
                e.volume, e.rootblock
            )),
            None => lines.push(format!(
                "  volume      {:?}  (rootblock {})",
                e.volume, e.rootblock
            )),
        }
        lines.push(format!(
            "  contents    {} files, {} directories, {} bytes recovered",
            h.files, h.directories, h.bytes_recovered
        ));
    } else {
        lines.push("  volume      none".to_owned());
    }

    if let Some(d) = &h.dircache {
        // Stated even when it finds nothing: "compared and matched" is a
        // different fact from "not checked", and only one of them is reassuring.
        lines.push("  dircache".to_owned());
        lines.push(format!(
            "    cached      {} records in {} blocks across {} directories",
            d.records, d.blocks, d.directories
        ));
        lines.push(format!(
            "    cross-check {}",
            if d.disagreements == 0 {
                "agrees with the directory entries".to_owned()
            } else {
                format!(
                    "{} DISAGREEMENTS with the directory entries",
                    d.disagreements
                )
            }
        ));
    }

    if let Some(b) = &h.bitmap {
        lines.push("  bitmap".to_owned());
        lines.push(format!(
            "    flag        {}",
            if b.flagged_valid {
                "valid"
            } else {
                "CLEAR — may be stale"
            }
        ));
        let percent = (b.marked_used as u64)
            .saturating_mul(100)
            .checked_div(u64::from(b.covered))
            .unwrap_or(0);
        lines.push(format!(
            "    usage       {} blocks marked used, {} reachable, {percent}% full",
            b.marked_used, b.actually_used
        ));
        if b.orphaned > 0 {
            lines.push(format!("    orphaned    {} blocks", b.orphaned));
        }
        if b.referenced_but_free > 0 {
            lines.push(format!(
                "    AT RISK     {} blocks in use but marked free",
                b.referenced_but_free
            ));
        }
    }

    let (info, warning, error) = h.counts();
    if h.findings.is_empty() {
        lines.push("  findings    none".to_owned());
    } else {
        lines.push(format!(
            "  findings    {error} error, {warning} warning, {info} info"
        ));
        for f in &h.findings {
            let mark = match f.severity {
                Severity::Error => "!!",
                Severity::Warning => " !",
                Severity::Info => "  ",
            };
            let at = f
                .block
                .map_or_else(String::new, |b| format!(" [block {b}]"));
            lines.push(format!("    {mark} [{}] {}{at}", f.code, f.message));
        }
    }

    for line in &lines {
        if !emit(out, line) {
            return;
        }
    }
}

/// Resolve `--partition` against a device's partition table.
///
/// `None` means the image is a single volume, so nothing is selected. Naming a
/// partition on an image with no table is an error rather than a silent
/// fallback: the caller asked for something that does not exist.
fn select_partition<'a>(
    image: &'a Image,
    wanted: Option<&str>,
) -> Result<Option<Window<'a>>, String> {
    let (parts, faults) = image.partitions().map_err(|e| e.to_string())?;
    for f in &faults {
        eprintln!("ade: partition table: {f}");
    }
    let Some(wanted) = wanted else {
        // No selection: use the first partition if the device has one, since a
        // partitioned device has no volume of its own.
        if parts.is_empty() {
            return Ok(None);
        }
        let first = parts.first().ok_or("no partitions")?;
        return image
            .partition_window(first)
            .map(Some)
            .map_err(|e| e.to_string());
    };
    if parts.is_empty() {
        return Err(format!("no partition table, so no partition {wanted:?}"));
    }
    let found = parts
        .iter()
        .find(|p| p.name_lossy().eq_ignore_ascii_case(wanted))
        .or_else(|| wanted.parse::<usize>().ok().and_then(|i| parts.get(i)))
        .ok_or_else(|| {
            let names: Vec<String> = parts.iter().map(Partition::name_lossy).collect();
            format!(
                "no partition {wanted:?}; this device has {}",
                names.join(", ")
            )
        })?;
    image
        .partition_window(found)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Guess the container a path names, from its extension.
///
/// Only used for the **output**, which does not exist yet and so cannot be
/// sniffed. Inputs are always identified by their content — an extension is a
/// claim, and C-008's habit of trusting evidence over labels applies here too.
fn kind_from_extension(path: &Path) -> Option<ade_core::layers::container::Kind> {
    use ade_core::layers::container::Kind;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "adf" => Some(Kind::Adf {
            cylinders: 80,
            sectors: 11,
        }),
        "hdf" | "hda" => Some(Kind::Hardfile),
        "adz" | "hdz" | "gz" => Some(Kind::Gzip),
        "dms" => Some(Kind::Dms),
        "scp" => Some(Kind::Scp),
        "ipf" => Some(Kind::Ipf),
        _ => None,
    }
}

/// Convert one container into another, refusing anything that would lose data
/// silently (F-016).
fn convert(input: &Path, output: &Path) -> ExitCode {
    let bytes = match std::fs::read(input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ade: {}: {e}", input.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };

    // The input is identified by content; only the output is taken on trust,
    // because it does not exist yet.
    let from = ade_core::layers::container::sniff(
        bytes.get(..bytes.len().min(512 * 16)).unwrap_or(&[]),
        bytes.len() as u64,
    )
    .kind;
    let Some(to) = kind_from_extension(output) else {
        eprintln!(
            "ade: {}: cannot tell what format to write from that name — \
             use .adf, .hdf, .adz or .hdz",
            output.display()
        );
        return ExitCode::from(EXIT_USAGE);
    };

    let verdict = conversion(from, to);
    println!("{from}  ->  {to}");
    println!("  {verdict}");

    if !verdict.is_possible() {
        return ExitCode::from(EXIT_USAGE);
    }
    if let Conversion::Lossy { lost } = &verdict {
        // Refused rather than warned. A lossy conversion that runs anyway is
        // exactly the silence F-016 exists to break, and the loss is not
        // recoverable afterwards.
        eprintln!("ade: refusing to convert: this would discard {lost}");
        eprintln!("ade: lossy conversion is not available yet — no flag enables it");
        return ExitCode::from(EXIT_USAGE);
    }

    // The only lossless path with a proven reader: decompression (D-004).
    let out_bytes = if matches!(from, ade_core::layers::container::Kind::Gzip) {
        match ade_core::layers::container::inflate::gunzip(&bytes, ade_core::MAX_DECOMPRESSED) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("ade: {}: {e}", input.display());
                return ExitCode::from(EXIT_UNREADABLE);
            }
        }
    } else {
        bytes
    };

    // Never overwrite. A conversion that silently replaces a source image is
    // the irreversible damage D-004 is about.
    if output.exists() {
        eprintln!(
            "ade: {}: already exists, refusing to overwrite",
            output.display()
        );
        return ExitCode::from(EXIT_USAGE);
    }
    if let Err(e) = std::fs::write(output, &out_bytes) {
        eprintln!("ade: {}: {e}", output.display());
        return ExitCode::from(EXIT_UNREADABLE);
    }
    println!("  wrote {} bytes to {}", out_bytes.len(), output.display());
    ExitCode::from(EXIT_CLEAN)
}

/// Print the conversion matrix (F-016).
fn formats() -> ExitCode {
    let kinds = ade_core::convert::known_formats();
    println!("What ADE can convert, and what it would cost.");
    println!();
    for from in &kinds {
        println!("  from {from}:");
        for to in &kinds {
            let verdict = conversion(*from, *to);
            // A format to itself is a copy; saying so for every row is noise.
            if std::mem::discriminant(from) == std::mem::discriminant(to) {
                continue;
            }
            println!("    -> {:<32} {verdict}", to.to_string());
        }
        println!();
    }
    println!("Lossy conversions are refused outright, not warned about: the loss");
    println!("is not recoverable, and a warning nobody reads is how it happens.");
    ExitCode::from(EXIT_CLEAN)
}
