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
///
/// # This was a claim before it was true (BUG-008)
///
/// "Every line of output goes through here" was written when `info` and `ls`
/// were the only commands there were (IMP-001). Six arms added afterwards —
/// `--help`, `--version`, `convert`, `formats`, `diff` and `consolidate` —
/// each wrote with `println!`, and `ade formats | head` panicked. A rule that
/// lives only in a doc comment is one every later contributor has to
/// rediscover, so [`emit_lines`] now exists for the common case of a block of
/// output, and `cli/tests/pipes.rs` runs commands into a reader that stops.
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

/// Write one JSON document, stamped with the schema version (F-015, D-015).
///
/// **The single point where JSON reaches stdout.** Every command's machine
/// output goes through here, so the version cannot be forgotten by a command
/// added later — which is precisely how `--format=json` came to be silently
/// ignored by four commands (BUG-007) and how six commands came to panic on a
/// closed pipe (BUG-008). The same mistake twice is a design signal.
fn emit_json(out: &mut impl Write, value: ade_core::json::Value) -> bool {
    emit(out, &value.versioned().to_json())
}

/// Write a block of lines, stopping early if the stream closes.
fn emit_lines(out: &mut impl Write, lines: &[String]) {
    for line in lines {
        if !emit(out, line) {
            return;
        }
    }
    let _ = out.flush();
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
    /// Where `consolidate` should write its merged image, if anywhere.
    output: Option<PathBuf>,
    /// A directory of TOSEC datfiles, for identification (F-013).
    datfiles: Option<PathBuf>,
    /// Write tracks as raw MFM rather than as sectors.
    ///
    /// A flag rather than an output extension because an extended ADF is also
    /// called `.adf`: the format has no distinct one, and inventing a
    /// convention would be presumptuous.
    raw: bool,
}

fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut format = Format::Text;
    let mut partition: Option<String> = None;
    let mut raw_output = false;
    let mut output: Option<PathBuf> = None;
    let mut datfiles: Option<PathBuf> = None;
    for arg in raw {
        match arg.as_str() {
            "--raw" => raw_output = true,
            "--format=json" | "--json" => format = Format::Json,
            "--format=text" => format = Format::Text,
            other if other.starts_with("--format=") => {
                return Err(format!(
                    "unknown format: {}",
                    other.trim_start_matches("--format=")
                ));
            }
            other if other.starts_with("--datfiles=") => {
                datfiles = Some(PathBuf::from(other.trim_start_matches("--datfiles=")));
            }
            other if other.starts_with("--output=") => {
                output = Some(PathBuf::from(other.trim_start_matches("--output=")));
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
        raw: raw_output,
        output,
        datfiles,
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
        ("convert", 2) => convert(Path::new(p(0)), Path::new(p(1)), args.raw),
        ("formats", 0) => formats(args.format),
        ("batch", n) if n >= 1 => batch(&args.positional, args.format, args.datfiles.as_deref()),
        ("identify", n) if n >= 1 => {
            identify(&args.positional, args.datfiles.as_deref(), args.format)
        }
        ("diff", 2) => diff_images(Path::new(p(0)), Path::new(p(1)), args.format),
        ("consolidate", n) if n >= 2 => {
            consolidate_images(&args.positional, args.output.as_deref(), args.format)
        }
        ("--version" | "-V", 0) => {
            let mut out = std::io::stdout().lock();
            emit(&mut out, &format!("ade {}", ade_core::version()));
            let _ = out.flush();
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
    let lines = vec![
        format!("ade {} — Amiga Disk Engine", ade_core::version()),
        String::new(),
        "USAGE:".to_owned(),
        "    ade info <image>                   inspect a disk image".to_owned(),
        "    ade check <image>                  full health report (F-010)".to_owned(),
        "    ade ls <image> [path]              list a directory".to_owned(),
        "    ade extract <image> <path> [dest]  extract a file".to_owned(),
        "    ade convert <in> <out>             convert between containers (F-016)".to_owned(),
        "    ade formats                        what converts to what, and what it costs"
            .to_owned(),
        "    ade batch <dir|image>...           verify a whole corpus (F-014)".to_owned(),
        "    ade identify <image>...            name images from TOSEC datfiles (F-013)".to_owned(),
        "    ade diff <a> <b>                   where two dumps of a disk differ (F-009)"
            .to_owned(),
        "    ade consolidate <a> <b> [...]      what several dumps agree on (F-008)".to_owned(),
        "    ade --version".to_owned(),
        String::new(),
        "OPTIONS:".to_owned(),
        "    --format=text   human-readable (default); layout is not stable".to_owned(),
        "    --format=json   machine-readable; field names and fault codes are stable".to_owned(),
        "    --partition=P   which partition of a hard disk, by name (DH0) or index (0)".to_owned(),
        "    --raw           convert writes raw MFM tracks (an extended ADF)".to_owned(),
        "    --output=P      consolidate writes the merged image to P".to_owned(),
        "    --datfiles=D    directory of TOSEC .dat files, for identify".to_owned(),
        String::new(),
        "EXIT CODES:".to_owned(),
        "    0  clean   1  faults   2  usage   3  unreadable   4  no volume".to_owned(),
        String::new(),
        "`check` exits 1 on any warning, and 5 when an error would lose data.".to_owned(),
    ];
    emit_lines(&mut std::io::stdout().lock(), &lines);
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
            emit_json(&mut out, inspection.to_json());
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
/// The track-table section: what a raw-track container holds, and how much
/// of it is ordinary.
///
/// A mix of kinds is the signature of copy protection rather than a defect —
/// the raw tracks are what a plain ADF could not have carried.
fn track_lines(lines: &mut Vec<String>, i: &Inspection) {
    if let Some(f) = &i.flux {
        // How the capture was made, before what it holds. These describe the
        // *file* rather than the disk: two captures of one disk can differ in
        // every line here and still hold the same sectors.
        lines.push("  capture".to_owned());
        lines.push(format!(
            "    revolutions {} per track, {} RPM, {} ns resolution",
            f.revolutions, f.rpm, f.tick_ns
        ));
        let mut how = Vec::new();
        how.push(if f.index_aligned {
            "starts at the index pulse"
        } else {
            "starts at an arbitrary point"
        });
        if f.normalised {
            // The one line here a preservationist should read twice.
            how.push("timings normalised, not as captured");
        }
        if f.foreign_creator {
            how.push("written by other hardware, not a SuperCard Pro");
        }
        lines.push(format!("    flux        {}", how.join("; ")));
    }

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
        if t.raw_mfm > 0 {
            lines.push(format!(
                "    decoded     {} of {} raw tracks are ordinary, {} sound sectors",
                t.standard_tracks, t.raw_mfm, t.sound_sectors
            ));
            if t.illegally_encoded_sectors > 0 {
                lines.push(format!(
                    "    encoding    {} sectors are not legal MFM",
                    t.illegally_encoded_sectors
                ));
            }
            if t.stray_syncs > 0 {
                // Not a fault: a sync mark with nothing behind it is how a
                // custom loader finds its own data.
                lines.push(format!(
                    "    protection  {} sync marks lead to no sector",
                    t.stray_syncs
                ));
            }
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
}

fn device_lines(lines: &mut Vec<String>, i: &Inspection) {
    track_lines(lines, i);

    if let Some(a) = &i.assembly {
        // Said before the volume, not after: anyone reading the listing below
        // needs to know it is a reconstruction with holes in it.
        lines.push("  assembled".to_owned());
        lines.push(format!(
            "    recovered   {} of {} sectors ({}% of a disk)",
            a.sectors_placed,
            a.sectors_total,
            a.percent_complete()
        ));
        lines.push(format!(
            "    from        {} sector tracks, {} decoded from raw MFM",
            a.from_sector_tracks, a.from_raw_tracks
        ));
        // Only when something is actually missing: a complete reconstruction
        // is the disk, and warning about holes it does not have is noise.
        if a.sectors_placed < a.sectors_total {
            lines.push("    note        the volume below is reconstructed; missing".to_owned());
            lines.push("                sectors read as zeros".to_owned());
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
                if !emit_json(&mut out, entry_to_json(e, &volume.path_components(e))) {
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
            emit_json(&mut out, health.to_json());
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
fn convert(input: &Path, output: &Path, raw: bool) -> ExitCode {
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
    // An extended ADF is also called `.adf`, so the extension cannot say which
    // was meant; `--raw` does.
    let Some(to) = (if raw {
        Some(ade_core::layers::container::Kind::ExtendedAdf { tracks: 0 })
    } else {
        kind_from_extension(output)
    }) else {
        eprintln!(
            "ade: {}: cannot tell what format to write from that name — \
             use .adf, .hdf, .adz or .hdz",
            output.display()
        );
        return ExitCode::from(EXIT_USAGE);
    };

    let verdict = conversion(from, to);
    let target = if raw {
        "extended ADF (raw MFM tracks)".to_owned()
    } else {
        to.to_string()
    };
    emit_lines(
        &mut std::io::stdout().lock(),
        &[format!("{from}  ->  {target}"), format!("  {verdict}")],
    );

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

    // Decompression first, so `--raw` works on an ADZ as well as an ADF.
    let sectors = if matches!(from, ade_core::layers::container::Kind::Gzip) {
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

    let out_bytes = if raw {
        match encode_raw_mfm(&sectors) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("ade: {}: {e}", input.display());
                return ExitCode::from(EXIT_UNREADABLE);
            }
        }
    } else {
        sectors
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
    emit_lines(
        &mut std::io::stdout().lock(),
        &[format!(
            "  wrote {} bytes to {}",
            out_bytes.len(),
            output.display()
        )],
    );
    ExitCode::from(EXIT_CLEAN)
}

/// Print the conversion matrix (F-016).
fn formats(format: Format) -> ExitCode {
    if format == Format::Json {
        let mut out = std::io::stdout().lock();
        emit_json(&mut out, ade_core::convert::matrix_json());
        let _ = out.flush();
        return ExitCode::from(EXIT_CLEAN);
    }
    let kinds = ade_core::convert::known_formats();
    let mut lines = vec![
        "What ADE can convert, and what it would cost.".to_owned(),
        String::new(),
    ];
    for from in &kinds {
        lines.push(format!("  from {from}:"));
        for to in &kinds {
            // A format to itself is a copy; saying so for every row is noise.
            if std::mem::discriminant(from) == std::mem::discriminant(to) {
                continue;
            }
            let verdict = conversion(*from, *to);
            lines.push(format!("    -> {:<32} {verdict}", to.to_string()));
        }
        lines.push(String::new());
    }
    lines.push("Lossy conversions are refused outright, not warned about: the loss".to_owned());
    lines.push("is not recoverable, and a warning nobody reads is how it happens.".to_owned());
    emit_lines(&mut std::io::stdout().lock(), &lines);
    ExitCode::from(EXIT_CLEAN)
}

/// Encode a sector image as an extended ADF of raw MFM tracks.
///
/// Lossless, and provably so: reading the result back reassembles the input
/// byte for byte. What it cannot do is invent protection the source never had
/// — the output is an extended ADF holding an ordinary disk, which is a
/// container change rather than a preservation upgrade.
fn encode_raw_mfm(sectors: &[u8]) -> Result<Vec<u8>, String> {
    use ade_core::layers::container::extended::{self, TrackSource};
    use ade_core::layers::track::{SECTOR_BYTES, encode_track};

    // The sectors-per-track comes from the image's own geometry rather than
    // being assumed: an HD image has 22 where a DD one has 11.
    let inspection = ade_core::inspect_bytes(sectors.to_vec());
    let geometry = inspection
        .geometry
        .ok_or("cannot establish a geometry for this image")?;
    let per_track = usize::try_from(geometry.sectors()).unwrap_or(11).max(1);
    let track_bytes = per_track.saturating_mul(SECTOR_BYTES);
    let remainder = sectors
        .len()
        .checked_rem(track_bytes)
        .ok_or("a track cannot be zero bytes")?;
    if remainder != 0 {
        return Err(format!(
            "not a whole number of {per_track}-sector tracks — {} bytes",
            sectors.len()
        ));
    }
    let track_count = sectors
        .len()
        .checked_div(track_bytes)
        .ok_or("a track cannot be zero bytes")?;

    let mut encoded: Vec<Vec<u8>> = Vec::with_capacity(track_count);
    for index in 0..track_count {
        let base = index.saturating_mul(track_bytes);
        let mut slices: Vec<&[u8]> = Vec::with_capacity(per_track);
        for s in 0usize..per_track {
            let at = base.saturating_add(s.saturating_mul(SECTOR_BYTES));
            let end = at.saturating_add(SECTOR_BYTES);
            slices.push(sectors.get(at..end).ok_or("image ends mid-track")?);
        }
        let number = u8::try_from(index).unwrap_or(u8::MAX);
        encoded.push(encode_track(number, &slices).ok_or("a sector was the wrong size")?);
    }

    let sources: Vec<TrackSource<'_>> = encoded
        .iter()
        .map(|data| TrackSource::RawMfm {
            data,
            length_bits: u32::try_from(data.len().saturating_mul(8)).unwrap_or(u32::MAX),
        })
        .collect();
    extended::write(&sources).map_err(|e| e.to_string())
}

/// Read several images, reporting any that cannot be read.
fn read_all(paths: &[String]) -> Result<Vec<Vec<u8>>, ExitCode> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        match std::fs::read(path) {
            Ok(bytes) => out.push(bytes),
            Err(e) => {
                eprintln!("ade: {path}: {e}");
                return Err(ExitCode::from(EXIT_UNREADABLE));
            }
        }
    }
    Ok(out)
}

/// Report where two dumps of a disk differ (F-009).
fn diff_images(a: &Path, b: &Path, format: Format) -> ExitCode {
    let images = match read_all(&[a.display().to_string(), b.display().to_string()]) {
        Ok(i) => i,
        Err(code) => return code,
    };
    let (Some(left), Some(right)) = (images.first(), images.get(1)) else {
        return ExitCode::from(EXIT_UNREADABLE);
    };
    let report = match ade_core::diff(left, right) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ade: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    if format == Format::Json {
        let mut out = std::io::stdout().lock();
        emit_json(&mut out, report.to_json());
        let _ = out.flush();
        return ExitCode::from(if report.is_identical() {
            EXIT_CLEAN
        } else {
            EXIT_FAULTS
        });
    }

    let mut lines = vec![a.display().to_string(), b.display().to_string()];
    if report.is_identical() {
        lines.push(format!(
            "  identical — {} sectors compared",
            report.sectors_total
        ));
        emit_lines(&mut std::io::stdout().lock(), &lines);
        return ExitCode::from(EXIT_CLEAN);
    }
    lines.push(format!(
        "  {} of {} sectors differ, {} bytes",
        report.sectors.len(),
        report.sectors_total,
        report.bytes_differing
    ));
    lines.push(format!("  tracks  {}", summarise(&report.tracks)));
    emit_lines(&mut std::io::stdout().lock(), &lines);
    ExitCode::from(EXIT_FAULTS)
}

/// Report what several dumps of a disk agree on (F-008).
fn consolidate_images(paths: &[String], output: Option<&Path>, format: Format) -> ExitCode {
    let images = match read_all(paths) {
        Ok(i) => i,
        Err(code) => return code,
    };
    let report = match ade_core::consolidate(&images) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ade: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    if format == Format::Json {
        let mut out = std::io::stdout().lock();
        emit_json(&mut out, report.to_json());
        let _ = out.flush();
        // The merged image still goes to `--output` if asked for: JSON is the
        // report, not the disk.
        return write_merged(&report, output, format);
    }

    let mut lines = vec![
        format!(
            "{} dumps, {} sectors each",
            report.sources,
            report.total_sectors()
        ),
        format!("  agreed      {} sectors", report.unanimous_sectors),
        format!(
            "  resolved    {} sectors by plurality",
            report.resolved_sectors
        ),
        format!(
            "  unresolved  {} sectors with no majority",
            report.unresolved_sectors
        ),
    ];
    if report.sources == 2 && report.unresolved_sectors > 0 {
        // Worth saying plainly: with two dumps every disagreement is a tie by
        // definition, so "unresolved" here is arithmetic, not damage.
        lines.push("              (two dumps cannot vote — every difference ties)".to_owned());
    }

    if !report.tracks.is_empty() {
        lines.push("  tracks that disagree".to_owned());
        for track in report.tracks.iter().take(12) {
            let unresolved = if track.unresolved.is_empty() {
                String::new()
            } else {
                format!(", {} unresolved", track.unresolved.len())
            };
            lines.push(format!(
                "    {:3}  sectors {:?}{unresolved}",
                track.track, track.disputed
            ));
        }
        if report.tracks.len() > 12 {
            lines.push(format!(
                "    ... and {} more tracks",
                report.tracks.len().saturating_sub(12)
            ));
        }
    }

    // Agreement is not correctness: these may be dumps of different physical
    // copies, so a plurality says what most dumps hold, not what is right.
    lines.push(String::new());
    lines.push("This reports agreement between dumps, not which dump is correct.".to_owned());
    emit_lines(&mut std::io::stdout().lock(), &lines);

    write_merged(&report, output, format)
}

/// Write the merged image if `--output` asked for one, and report the outcome.
///
/// Shared by both output formats: the merge is the same work whether the
/// report that accompanies it is prose or JSON, and having one copy is what
/// keeps the two from disagreeing about whether a file was written.
///
/// # Where the confirmation goes depends on the format
///
/// Under JSON it goes to stderr, because a line of prose in the middle of a
/// JSON document means it is no longer one. Under text it stays on stdout,
/// where it has always been — fixing the JSON surface is no reason to move a
/// line a text-mode script may already be reading. A test caught this being
/// moved for both.
fn write_merged(
    report: &ade_core::Consolidation,
    output: Option<&Path>,
    format: Format,
) -> ExitCode {
    if let Some(path) = output {
        if path.exists() {
            eprintln!(
                "ade: {}: already exists, refusing to overwrite",
                path.display()
            );
            return ExitCode::from(EXIT_USAGE);
        }
        if let Err(e) = std::fs::write(path, &report.bytes) {
            eprintln!("ade: {}: {e}", path.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
        let line = format!("wrote {} bytes to {}", report.bytes.len(), path.display());
        if format == Format::Json {
            eprintln!("{line}");
        } else {
            emit_lines(&mut std::io::stdout().lock(), &[line]);
        }
    }

    ExitCode::from(if report.is_unanimous() {
        EXIT_CLEAN
    } else {
        EXIT_FAULTS
    })
}

/// Render a list of track numbers as ranges, so a long run reads as one.
fn summarise(tracks: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut index = 0usize;
    while let Some(&start) = tracks.get(index) {
        let mut end = start;
        while tracks.get(index.saturating_add(1)) == Some(&end.saturating_add(1)) {
            index = index.saturating_add(1);
            end = end.saturating_add(1);
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        index = index.saturating_add(1);
    }
    parts.join(", ")
}

/// Verify a whole corpus and summarise it (F-014).
///
/// Progress goes to stderr and the summary to stdout, so the machine-readable
/// output stays clean when a run is piped — a progress bar interleaved with
/// JSON is worse than no progress bar.
fn batch(paths: &[String], format: Format, datfiles: Option<&Path>) -> ExitCode {
    let inputs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let show_progress = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // Optional: a corpus run that also names every disk is F-013 and F-014
    // doing their jobs together.
    let catalogue = match datfiles {
        Some(dir) => match ade_core::layers::catalogue::Catalogue::load_dir(dir) {
            Ok(c) => {
                eprintln!("loaded {} entries from {} datfiles", c.len(), c.files());
                Some(c)
            }
            Err(e) => {
                eprintln!("ade: {}: {e}", dir.display());
                return ExitCode::from(EXIT_UNREADABLE);
            }
        },
        None => None,
    };

    let summary = ade_core::batch::run_with(&inputs, catalogue.as_ref(), |done, total| {
        if show_progress && (done % 25 == 0 || done == total) {
            eprint!("\r  {done} of {total} examined");
            let _ = std::io::stderr().flush();
        }
    });
    if show_progress {
        eprintln!();
    }

    if summary.examined == 0 {
        eprintln!("ade: nothing to examine");
        return ExitCode::from(EXIT_USAGE);
    }

    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => {
            // Records first as JSON Lines, then the summary object, so a
            // consumer can stream the former and still get the latter.
            for record in &summary.records {
                if !emit_json(&mut out, record.to_json()) {
                    return ExitCode::from(EXIT_CLEAN);
                }
            }
            emit_json(&mut out, summary.to_json());
        }
        Format::Text => report_batch(&mut out, &summary),
    }
    let _ = out.flush();

    ExitCode::from(if summary.unreadable > 0 || !summary.at_risk().is_empty() {
        EXIT_DATA_AT_RISK
    } else if summary.sound < summary.mounted {
        EXIT_FAULTS
    } else {
        EXIT_CLEAN
    })
}

/// The human-readable batch summary.
fn report_batch(out: &mut impl Write, summary: &ade_core::Summary) {
    let mut lines = vec![
        format!("{} images examined", summary.examined),
        format!(
            "  mounted     {} ({}%)",
            summary.mounted,
            percent(summary.mounted, summary.examined)
        ),
        format!(
            "  sound       {} ({}%)",
            summary.sound,
            percent(summary.sound, summary.examined)
        ),
        format!("  unreadable  {}", summary.unreadable),
        format!("  recovered   {} bytes", summary.bytes_recovered),
    ];
    if summary.identified > 0 {
        lines.push(format!(
            "  identified  {} ({}%) named from the dataset",
            summary.identified,
            percent(summary.identified, summary.examined)
        ));
    }

    lines.push("  containers".to_owned());
    let mut containers: Vec<(&String, &usize)> = summary.containers.iter().collect();
    containers.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in containers.iter().take(10) {
        lines.push(format!("    {count:>6}  {name}"));
    }

    if summary.findings.is_empty() {
        lines.push("  findings    none".to_owned());
    } else {
        lines.push("  findings".to_owned());
        let mut findings: Vec<(&&str, &usize)> = summary.findings.iter().collect();
        findings.sort_by(|a, b| b.1.cmp(a.1));
        for (code, count) in findings {
            lines.push(format!("    {count:>6}  {code}"));
        }
    }

    let at_risk = summary.at_risk();
    if !at_risk.is_empty() {
        lines.push(format!("  at risk      {} images", at_risk.len()));
        for record in at_risk.iter().take(10) {
            lines.push(format!("    {}", record.path.display()));
        }
        if at_risk.len() > 10 {
            lines.push(format!(
                "    ... and {} more",
                at_risk.len().saturating_sub(10)
            ));
        }
    }

    for line in &lines {
        if !emit(out, line) {
            return;
        }
    }
}

/// An integer percentage, because a proportion of a disk count is not a float.
fn percent(part: usize, whole: usize) -> usize {
    part.saturating_mul(100).checked_div(whole).unwrap_or(0)
}

/// Name images by content hash against a dataset (F-013).
fn identify(paths: &[String], datfiles: Option<&Path>, format: Format) -> ExitCode {
    let Some(dir) = datfiles else {
        eprintln!("ade: identify needs --datfiles=<directory of .dat files>");
        return ExitCode::from(EXIT_USAGE);
    };
    let catalogue = match ade_core::layers::catalogue::Catalogue::load_dir(dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ade: {}: {e}", dir.display());
            return ExitCode::from(EXIT_UNREADABLE);
        }
    };
    eprintln!(
        "loaded {} entries from {} datfiles",
        catalogue.len(),
        catalogue.files()
    );

    let mut named = 0usize;
    let mut unknown = 0usize;
    let mut out = std::io::stdout().lock();

    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("ade: {path}: {e}");
                unknown = unknown.saturating_add(1);
                continue;
            }
        };
        let matches = catalogue.identify(&bytes);
        if matches.is_empty() {
            unknown = unknown.saturating_add(1);
        } else {
            named = named.saturating_add(1);
        }

        if format == Format::Json {
            // One object per image, as JSON Lines — the same shape `batch`
            // uses, and for the same reason: a run over four thousand images
            // should be readable as it goes rather than only once it ends.
            if !emit_json(
                &mut out,
                ade_core::batch::identification_json(path, &matches),
            ) {
                return ExitCode::from(EXIT_CLEAN);
            }
            continue;
        }

        if matches.is_empty() {
            if !emit(&mut out, &format!("{path}\n  unknown — not in the dataset")) {
                return ExitCode::from(EXIT_CLEAN);
            }
            continue;
        }
        let mut lines = vec![path.clone()];
        for entry in &matches {
            lines.push(format!("  {}", entry.name));
            lines.push(format!("    from {}", entry.source));
        }
        if matches.len() > 1 {
            // A content hash is not an identity. Say so rather than pick.
            lines.push(format!(
                "    ! {} entries share this hash — ADE will not choose between them",
                matches.len()
            ));
        }
        for line in &lines {
            if !emit(&mut out, line) {
                return ExitCode::from(EXIT_CLEAN);
            }
        }
    }

    let _ = out.flush();
    eprintln!("{named} identified, {unknown} unknown");
    ExitCode::from(if unknown > 0 { EXIT_FAULTS } else { EXIT_CLEAN })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test scaffolding: a failure to set up is a test failure"
)]
mod tests {
    use super::{emit, emit_lines};
    use std::io::{Error, ErrorKind, Write};

    /// A writer that accepts whole lines until `allow` of them have gone
    /// through, then reports a closed pipe — which is what `| head -1` does to
    /// whatever is on the left of it.
    ///
    /// Counting lines rather than writes, because `writeln!` does not make one
    /// write per line: it writes the text and the newline separately, so a
    /// write counter would cut a line in half and prove nothing about output.
    struct ClosingPipe {
        allow: usize,
        buf: String,
    }

    impl ClosingPipe {
        fn new(allow: usize) -> Self {
            Self {
                allow,
                buf: String::new(),
            }
        }

        fn lines(&self) -> Vec<&str> {
            self.buf.lines().collect()
        }
    }

    impl Write for ClosingPipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.buf.matches('\n').count() >= self.allow {
                return Err(Error::new(ErrorKind::BrokenPipe, "closed"));
            }
            self.buf.push_str(&String::from_utf8_lossy(buf));
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_stops_the_output_rather_than_failing() {
        let mut pipe = ClosingPipe::new(0);
        assert!(!emit(&mut pipe, "anything"));
    }

    #[test]
    fn emit_lines_stops_at_the_line_the_pipe_closed_on() {
        // The behaviour that matters is not "does not panic" but "writes what
        // it can and stops". A loop that kept going would spend the rest of a
        // corpus run writing into a pipe nobody is reading.
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let mut pipe = ClosingPipe::new(3);
        emit_lines(&mut pipe, &lines);
        assert_eq!(pipe.lines(), vec!["line 0", "line 1", "line 2"]);
    }

    #[test]
    fn output_that_fits_is_written_in_full() {
        let lines: Vec<String> = (0..5).map(|i| format!("line {i}")).collect();
        let mut pipe = ClosingPipe::new(usize::MAX);
        emit_lines(&mut pipe, &lines);
        assert_eq!(pipe.lines().len(), 5);
        assert_eq!(pipe.lines()[4], "line 4");
    }
}
