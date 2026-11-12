//! AmigaDOS itself as an oracle (D-023).
//!
//! Every other check on what ADE writes is a reader — ADE's health report and
//! ADFlib — and neither boots a disk or is AmigaDOS. Here Workbench 3.1 runs
//! in FS-UAE: ADE extracts the Workbench disk into a folder, FS-UAE mounts it
//! as the boot drive, a startup-sequence runs the commands and writes what
//! they say into that folder, and a marker file tells this harness to stop
//! the emulator.
//!
//! Configured, never found: `ADE_KICKSTART` (a Kickstart 3.1 ROM),
//! `ADE_WORKBENCH` (a Workbench 3.1 disk) and `fs-uae` on the path or named
//! by `ADE_FSUAE`. Without them every test here prints why and passes, so CI
//! and a fresh clone are unaffected. The ROM and Workbench are copyrighted
//! and never enter the repository (D-010).
//!
//! Three things a run must respect, each found by a run that did not:
//! - **A requester waits forever.** "Please insert volume" stalls the script,
//!   so every run has a limit and fails with the emulator's log when the
//!   marker has not appeared.
//! - **AmigaDOS writes lazily.** Stopped at the marker, a floppy is left
//!   half-written; the script locks it and waits before leaving the marker.
//! - **Scripts are Latin-1**, as AmigaDOS reads them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::panic,
    clippy::manual_assert_eq,
    reason = "test scaffolding; file contents are compared with assert!, since assert_eq! would \
              print tens of kilobytes of each"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ade_core::Image;
use ade_core::edit::Session;
use ade_core::layers::block::Geometry;
use ade_core::layers::filesystem::dostype::Dostype;
use ade_core::layers::filesystem::format::{self, Stamp};
use ade_core::unpack::unpack;

const WHEN: Stamp = Stamp {
    days: 17_000,
    mins: 1,
    ticks: 0,
};

/// One emulator at a time: each is a whole machine, and two windows racing
/// for the screen make a failure harder to read (D-023 part 5).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct Setup {
    kickstart: PathBuf,
    workbench: PathBuf,
    fsuae: String,
    /// `xvfb-run`, when installed, so the emulator draws to no screen.
    xvfb: bool,
}

fn setup() -> Option<Setup> {
    let why = |reason: &str| {
        eprintln!("skipping: {reason} (D-023)");
        None
    };
    let Some(kickstart) = std::env::var_os("ADE_KICKSTART").map(PathBuf::from) else {
        return why("ADE_KICKSTART names no Kickstart 3.1 ROM");
    };
    let Some(workbench) = std::env::var_os("ADE_WORKBENCH").map(PathBuf::from) else {
        return why("ADE_WORKBENCH names no Workbench 3.1 disk");
    };
    if !kickstart.is_file() || !workbench.is_file() {
        return why("ADE_KICKSTART or ADE_WORKBENCH is not a file");
    }
    let fsuae = std::env::var("ADE_FSUAE").unwrap_or_else(|_| "fs-uae".to_owned());
    let runs = |program: &str, arg: &str| {
        Command::new(program)
            .arg(arg)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    };
    if !runs(&fsuae, "--version") {
        return why("no fs-uae on the path (set ADE_FSUAE)");
    }
    let xvfb = runs("xvfb-run", "--help");
    if !xvfb && std::env::var_os("DISPLAY").is_none() {
        return why("no display and no xvfb-run");
    }
    Some(Setup {
        kickstart,
        workbench,
        fsuae,
        xvfb,
    })
}

/// What a run left behind.
struct Outcome {
    /// The boot folder's `out`, where the script wrote.
    out: PathBuf,
    /// Times Kickstart started: 1 for a machine that booted once.
    resets: usize,
    /// Illegal instructions the CPU met — what running zeros comes to.
    illegal: usize,
}

/// The assigns Workbench's own startup-sequence makes that the commands
/// used here need: `Format` will not start without `ENV:`.
const PRELUDE: &str = "C:SetPatch QUIET
FailAt 1000
C:MakeDir RAM:T RAM:ENV RAM:ENV/Sys
Assign ENV: RAM:ENV
Assign T: RAM:T
Assign LOCALE: SYS:Locale
Assign KEYMAPS: DEVS:Keymaps
";

/// Boot Workbench 3.1 with `floppies` in DF0 and DF1, run `commands`, and
/// return when the script says it is done. `inputs` are copied into the boot
/// folder's `in` first, for the script to read as `SYS:in/<name>`.
fn run(
    setup: &Setup,
    name: &str,
    floppies: &[&Path],
    inputs: &[(&str, &[u8])],
    commands: &str,
    limit: Duration,
) -> Result<Outcome, String> {
    run_with(setup, name, (floppies, &[]), inputs, commands, limit)
}

/// As [`run`], with `hardfiles` mounted as `DH1:`, `DH2:` … beside the
/// floppies. A raw hardfile is given FS-UAE's default geometry — 32 sectors,
/// 1 surface, 2 reserved blocks — which is the shape ADE writes, and which
/// stops at 65,535 cylinders: a little under 1 GB.
fn run_with(
    setup: &Setup,
    name: &str,
    (floppies, hardfiles): (&[&Path], &[&Path]),
    inputs: &[(&str, &[u8])],
    commands: &str,
    limit: Duration,
) -> Result<Outcome, String> {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("ade-amigados-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let boot = dir.join("boot");
    fs::create_dir_all(&boot).unwrap();

    let workbench = Image::from_bytes(fs::read(&setup.workbench).unwrap()).unwrap();
    unpack(&workbench.volume().unwrap(), &boot).unwrap();
    let out = boot.join("out");
    fs::create_dir_all(&out).unwrap();
    fs::create_dir_all(boot.join("in")).unwrap();
    for (file, bytes) in inputs {
        fs::write(boot.join("in").join(file), bytes).unwrap();
    }
    fs::write(boot.join("S/cr"), b"\n").unwrap();

    let mut script = String::from(PRELUDE);
    script.push_str(commands);
    for drive in 0..floppies.len() {
        let _ = writeln!(script, "C:Lock DF{drive}: ON");
    }
    for unit in 1..=hardfiles.len() {
        let _ = writeln!(script, "C:Lock DH{unit}: ON");
    }
    script.push_str("C:Wait 3\nEcho \"done\" >SYS:out/done\nEndCLI\n");
    fs::write(boot.join("S/Startup-Sequence"), latin1(&script)).unwrap();

    let mut config = format!(
        "[fs-uae]\namiga_model = A1200\nkickstart_file = {}\n\
         hard_drive_0 = {}\nhard_drive_0_label = Boot\n\
         writable_floppy_images = 1\nfloppy_drive_speed = 0\n\
         fullscreen = 0\nuae_sound_output = none\nbase_dir = {}\n",
        setup.kickstart.display(),
        boot.display(),
        dir.join("base").display(),
    );
    for (drive, floppy) in floppies.iter().enumerate() {
        let _ = writeln!(config, "floppy_drive_{drive} = {}", floppy.display());
    }
    for (unit, hardfile) in hardfiles.iter().enumerate() {
        let _ = writeln!(config, "hard_drive_{} = {}", unit + 1, hardfile.display());
    }
    let cfg = dir.join("run.fs-uae");
    fs::write(&cfg, config).unwrap();

    let mut command = if setup.xvfb {
        let mut c = Command::new("xvfb-run");
        c.args(["-a", &setup.fsuae]);
        c
    } else {
        Command::new(&setup.fsuae)
    };
    // Its own process group, so stopping it stops xvfb-run's children too.
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command
        .arg(&cfg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("starting fs-uae: {e}"))?;

    let done = out.join("done");
    let start = Instant::now();
    while !done.exists() && start.elapsed() < limit {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // A run that left no marker is stuck on something only the screen shows
    // — every hang so far was a requester or a crash there (IMP-022) — so
    // the screen is kept before the emulator goes.
    let screen = if done.exists() || setup.xvfb {
        None
    } else {
        screenshot(&dir)
    };
    stop(&mut child);

    let log = fs::read_to_string(dir.join("base/Cache/Logs/fs-uae.log.txt")).unwrap_or_default();
    let resets = log.matches("Reset at").count();
    let illegal = log.matches("Illegal instruction").count();
    if !done.exists() {
        let tail: Vec<&str> = log.lines().rev().take(25).collect();
        let seen = screen.map_or_else(
            || {
                "no screenshot (needs xwininfo and ImageMagick's import, and a visible \
                display)"
                    .to_owned()
            },
            |p| format!("the screen is in {}", p.display()),
        );
        return Err(format!(
            "{name}: no marker after {:?}: {resets} resets, {illegal} illegal instructions; \
             {seen}; the log ends\n{}",
            start.elapsed(),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(Outcome {
        out,
        resets,
        illegal,
    })
}

/// Stop the emulator and everything it started.
fn stop(child: &mut std::process::Child) {
    // Asked to quit first, and given time to: FS-UAE keeps written floppy
    // tracks in memory and writes them back to the image when it quits. A
    // KILL straight after the marker intermittently left disks half-written
    // — a link deleted from its directory but still named by its object's
    // chain, blocks marked used that nothing reached — states AmigaDOS
    // never wrote, failing tests that passed when the same steps ran again.
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-TERM", "--", &group]).status();
    let quitting = Instant::now();
    while quitting.elapsed() < Duration::from_secs(15) {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
    // And the child itself, should the group kill not have reached it: a
    // harness that waits on a live emulator never returns.
    let _ = child.kill();
    let _ = child.wait();
}

/// The FS-UAE window, saved as `timeout.png` in `dir`, if the tools to take
/// it are installed (IMP-022). Only the emulator's window, found by its
/// title — never the whole screen, which is somebody's desktop. A window
/// partly covered by another may show what covers it.
fn screenshot(dir: &Path) -> Option<PathBuf> {
    let tree = Command::new("xwininfo")
        .args(["-root", "-tree"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let listing = String::from_utf8_lossy(&tree.stdout);
    let id = listing
        .lines()
        .find(|l| l.contains("\"FS-UAE"))?
        .split_whitespace()
        .next()?
        .to_owned();
    let path = dir.join("timeout.png");
    let taken = Command::new("import")
        .args(["-window", &id])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    (taken && path.is_file()).then_some(path)
}

/// AmigaDOS reads a script as Latin-1; every character here must be one.
fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).expect("a script must be Latin-1"))
        .collect()
}

fn dostype(flags: u8) -> Dostype {
    Dostype::from_raw(0x444F_5300 | u32::from(flags)).unwrap()
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ (i >> 8) as u8)
        .collect()
}

fn floppy(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ade-amigados-disks-{}-{name}", std::process::id()))
}

#[test]
fn a_disk_ade_formats_does_not_boot_and_kickstart_passes_over_it() {
    // BUG-023's regression. A valid boot checksum over zeroed code made
    // Kickstart run the zeros and reset forever; with the folder at the
    // default boot priority, the floppy is offered first, so a disk that
    // boots would never let the script run.
    let Some(setup) = setup() else { return };
    let blank = format::blank(Geometry::DD_FLOPPY, dostype(1), "Blank", WHEN).unwrap();
    let disk = floppy(&scratch("boot"), "blank.adf", &blank);

    let outcome = run(
        &setup,
        "boot",
        &[&disk],
        &[],
        "C:Info DF0: >SYS:out/info.txt\n",
        Duration::from_secs(60),
    )
    .unwrap();
    assert_eq!(
        outcome.illegal, 0,
        "the CPU ran something it should not have"
    );
    assert_eq!(outcome.resets, 1, "the machine started more than once");
    let info = fs::read(outcome.out.join("info.txt")).unwrap();
    assert!(
        String::from_utf8_lossy(&info).contains("Blank"),
        "AmigaDOS mounted the disk: {}",
        String::from_utf8_lossy(&info)
    );
}

#[test]
fn amigados_finds_and_reads_what_ade_writes_by_name() {
    // Walking a directory cannot see a wrong hash slot (CLAUDE.md, F-031):
    // `Copy` with a path makes AmigaDOS hash each name and walk that one
    // chain. Accented names, on an international volume and on one hashed
    // the old way, because the two hash `ü` differently.
    let Some(setup) = setup() else { return };
    let dir = scratch("read");
    let mut disks = Vec::new();
    let mut wanted = Vec::new();
    for (drive, (flags, label)) in [(3u8, "Intl"), (0u8, "Plain")].into_iter().enumerate() {
        let blank = format::blank(Geometry::DD_FLOPPY, dostype(flags), label, WHEN).unwrap();
        let mut s = Session::from_bytes(blank).unwrap();
        s.set_clock(Some(WHEN));
        let folder = s.make_dir(s.root(), b"St\xfcff").unwrap();
        // Long enough to need an extension block on OFS: 72 data blocks.
        let data = pattern(40_000 + drive * 777, flags);
        s.add_file(folder, b"f\xfcr.bin", &data).unwrap();
        disks.push(floppy(&dir, &format!("{label}.adf"), s.bytes()));
        wanted.push((label, data));
    }

    let commands = "C:Copy \"Intl:Stüff/für.bin\" SYS:out/intl.bin\n\
                    C:Copy \"Plain:Stüff/für.bin\" SYS:out/plain.bin\n";
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    let outcome = run(
        &setup,
        "read",
        &paths,
        &[],
        commands,
        Duration::from_secs(60),
    )
    .unwrap();
    for (label, data) in wanted {
        let got = fs::read(outcome.out.join(format!("{}.bin", label.to_lowercase())))
            .unwrap_or_else(|_| panic!("{label}: AmigaDOS did not find the file by name"));
        assert!(got == data, "{label}: AmigaDOS read different bytes");
    }
}

#[test]
fn ade_reads_what_amigados_formats_and_writes() {
    // The other direction, from a disk ADE never touched until the end:
    // Workbench's own `Format` on a zeroed image, a folder, and a file copied
    // on. ADE must find no fault, and read the file back byte for byte.
    let Some(setup) = setup() else { return };
    let disk = floppy(&scratch("write"), "zeros.adf", &vec![0u8; 901_120]);
    let data = pattern(30_000, 7);
    let commands = "SYS:System/Format DRIVE DF0: NAME Real FFS NOICONS <S:cr >NIL:\n\
                    C:MakeDir Real:Dätä\n\
                    C:Copy SYS:in/data.bin Real:Dätä/ünïcode.bin\n";
    let outcome = run(
        &setup,
        "write",
        &[&disk],
        &[("data.bin", &data)],
        commands,
        Duration::from_secs(120),
    );
    outcome.unwrap();

    let bytes = fs::read(&disk).unwrap();
    // What `Format` writes in block 0 is what ADE's formatter writes
    // (BUG-023): the dostype, then zeros. Block 1 is not compared: a full
    // format fills every block on the disk with `DOS`-and-a-counter, and
    // leaves that in block 1.
    let ours = format::blank(Geometry::DD_FLOPPY, dostype(1), "Real", WHEN).unwrap();
    assert_eq!(bytes[..512], ours[..512], "block 0 differs");

    let health = ade_core::health::examine(bytes.clone());
    let faults: Vec<String> = health
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    assert!(faults.is_empty(), "{faults:?}");
    let image = Image::from_bytes(bytes).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup("Dätä/ünïcode.bin").unwrap();
    assert!(volume.read_file(&entry).unwrap().bytes == data);
}

#[test]
fn a_disk_amigados_has_only_formatted_reads_as_blank() {
    // A full `Format` writes a fill — `DOS` and a counter — into every block
    // on the disk, then the rootblock and bitmap over it. Nothing has been
    // put on the disk, so nothing on it is data.
    let Some(setup) = setup() else { return };
    let disk = floppy(&scratch("fresh"), "zeros.adf", &vec![0u8; 901_120]);
    run(
        &setup,
        "fresh",
        &[&disk],
        &[],
        "SYS:System/Format DRIVE DF0: NAME Fresh FFS NOICONS <S:cr >NIL:\n",
        Duration::from_secs(120),
    )
    .unwrap();

    let health = ade_core::health::examine(fs::read(&disk).unwrap());
    let faults: Vec<String> = health
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    assert!(faults.is_empty(), "{faults:?}");
}

#[test]
fn amigados_writes_into_every_directory_ade_makes_on_a_dircache_disk() {
    // BUG-025. AmigaDOS's filesystem crashed writing into a directory with
    // no cache chain — the root of a blank `DOS\5` disk, and a directory
    // `ade mkdir` made. Every directory ADE makes now has one, and one
    // emptied by deleting keeps an empty block rather than none.
    let Some(setup) = setup() else { return };
    let blank = format::blank(Geometry::DD_FLOPPY, dostype(5), "Cached", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    s.make_dir(root, b"Made").unwrap();
    let emptied = s.make_dir(root, b"Emptied").unwrap();
    let file = s.add_file(emptied, b"gone", b"soon deleted").unwrap();
    s.delete(file).unwrap();
    let disk = floppy(&scratch("cached"), "cached.adf", s.bytes());

    let data = pattern(5_000, 3);
    let commands = "C:Copy SYS:in/data.bin Cached:top.bin\n\
                    C:Copy SYS:in/data.bin Cached:Made/inner.bin\n\
                    C:Copy SYS:in/data.bin Cached:Emptied/again.bin\n\
                    C:MakeDir Cached:Made/Deeper\n\
                    C:Copy SYS:in/data.bin Cached:Made/Deeper/deep.bin\n";
    let outcome = run(
        &setup,
        "cached",
        &[&disk],
        &[("data.bin", &data)],
        commands,
        Duration::from_secs(60),
    );
    outcome.unwrap();

    let bytes = fs::read(&disk).unwrap();
    let health = ade_core::health::examine(bytes.clone());
    let faults: Vec<String> = health
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    assert!(faults.is_empty(), "{faults:?}");
    let image = Image::from_bytes(bytes).unwrap();
    let volume = image.volume().unwrap();
    for path in [
        "top.bin",
        "Made/inner.bin",
        "Emptied/again.bin",
        "Made/Deeper/deep.bin",
    ] {
        let entry = volume
            .lookup(path)
            .unwrap_or_else(|e| panic!("{path}: AmigaDOS did not write it: {e}"));
        assert!(volume.read_file(&entry).unwrap().bytes == data, "{path}");
    }
}

#[test]
fn a_blank_dircache_disk_is_laid_out_as_format_lays_it_out() {
    // `Format … FFS DIRCACHE QUICK` on a zeroed image writes block 0, the
    // rootblock at 880, the root's empty cache at 881 and the bitmap at 882,
    // and nothing else. ADE's blank `DOS\5` must use the same blocks, with
    // the cache and bitmap identical; the rootblock differs only in its
    // datestamps, so those are compared field by field instead.
    let Some(setup) = setup() else { return };
    let disk = floppy(&scratch("dcformat"), "zeros.adf", &vec![0u8; 901_120]);
    run(
        &setup,
        "dcformat",
        &[&disk],
        &[],
        "SYS:System/Format DRIVE DF0: NAME Same FFS DIRCACHE NOICONS QUICK <S:cr >NIL:\n",
        Duration::from_secs(60),
    )
    .unwrap();
    let theirs = fs::read(&disk).unwrap();
    let ours = format::blank(Geometry::DD_FLOPPY, dostype(5), "Same", WHEN).unwrap();

    let used = |image: &[u8]| -> Vec<usize> {
        (0..1760)
            .filter(|b| image[b * 512..(b + 1) * 512].iter().any(|&x| x != 0))
            .collect()
    };
    assert_eq!(used(&theirs), used(&ours), "the blocks a blank disk uses");
    for block in [0usize, 881] {
        assert_eq!(
            theirs[block * 512..(block + 1) * 512],
            ours[block * 512..(block + 1) * 512],
            "block {block}"
        );
    }
    // The bitmap by its map bits for blocks 2–1759, set meaning free. Not
    // byte for byte: AmigaDOS sets the bits past the volume's end, which
    // ADE's formatter clears (SPEC §Repairing a volume), and leaves whatever
    // was in memory in the block's unused tail — code and colour tables, on
    // the run that measured it.
    let free = |image: &[u8], block: usize| {
        let index = block - 2;
        let at = 882 * 512 + 4 + (index / 32) * 4;
        let word = u32::from(image[at]) << 24
            | u32::from(image[at + 1]) << 16
            | u32::from(image[at + 2]) << 8
            | u32::from(image[at + 3]);
        word & (1 << (index % 32)) != 0
    };
    for block in 2..1760 {
        assert_eq!(
            free(&theirs, block),
            free(&ours, block),
            "bitmap bit for block {block}"
        );
    }
    // The rootblock, but for its checksum and its three datestamps.
    let mut a = theirs[880 * 512..881 * 512].to_vec();
    let mut b = ours[880 * 512..881 * 512].to_vec();
    for raw in [&mut a, &mut b] {
        raw[20..24].fill(0);
        for back in [92usize, 40, 28] {
            raw[512 - back..512 - back + 12].fill(0);
        }
    }
    assert_eq!(a, b, "the rootblock, datestamps aside");
}

#[test]
fn what_amigados_deletes_ade_puts_back_and_amigados_reads_again() {
    // F-012 against the filesystem that did the deleting. AmigaDOS writes
    // files onto OFS, FFS and dircache disks and deletes them — a file, one
    // big enough for an extension block, a file in a directory, and then the
    // directory — and ADE puts each back, directory first. AmigaDOS must then
    // find every file by name with its bytes, and write into the restored
    // directory.
    let Some(setup) = setup() else { return };
    let dir = scratch("undelete");
    let small = pattern(700, 1);
    let big = pattern(40_000, 2);
    let inner = pattern(1_500, 3);
    let mut disks = Vec::new();
    for (flags, label) in [(0u8, "OFS"), (1, "FFS"), (5, "DC")] {
        let blank = format::blank(Geometry::DD_FLOPPY, dostype(flags), label, WHEN).unwrap();
        disks.push(floppy(&dir, &format!("{label}.adf"), &blank));
    }
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    let inputs: [(&str, &[u8]); 3] = [("small", &small), ("big", &big), ("inner", &inner)];
    // Written and deleted in separate runs, as a person does. Deleted in the
    // same run, a dircache file's header reached the disk still saying 0
    // bytes: AmigaDOS writes a header when the file is opened and its size
    // when it is closed, into a buffer that a `Delete` straight after throws
    // away (SPEC §Deletion).
    let mut written = String::new();
    let mut deleted = String::new();
    for v in ["OFS", "FFS", "DC"] {
        let _ = write!(
            written,
            "C:Copy SYS:in/small {v}:small.bin\nC:Copy SYS:in/big {v}:big.bin\n\
             C:MakeDir {v}:Drawer\nC:Copy SYS:in/inner {v}:Drawer/inner.bin\n\
             C:Copy SYS:in/small {v}:keep.bin\n"
        );
        let _ = write!(
            deleted,
            "C:Delete {v}:small.bin\nC:Delete {v}:big.bin\n\
             C:Delete {v}:Drawer/inner.bin\nC:Delete {v}:Drawer\n"
        );
    }
    for (step, commands) in [("undelete-1", &written), ("undelete-2", &deleted)] {
        run(
            &setup,
            step,
            &paths,
            &inputs,
            commands,
            Duration::from_secs(90),
        )
        .unwrap();
    }

    for disk in &disks {
        let mut s = Session::from_bytes(fs::read(disk).unwrap()).unwrap();
        s.set_clock(Some(WHEN));
        let found = ade_core::carve::carve(&Image::from_bytes(s.bytes().to_vec()).unwrap());
        let block = |name: &str| found.iter().find(|c| c.name == name).unwrap().block;
        let drawer = block("Drawer");
        let inner_block = block("inner.bin");
        // The file cannot come back before the directory it was in.
        assert!(s.undelete(inner_block).is_err(), "{}", disk.display());
        for b in [drawer, inner_block, block("big.bin"), block("small.bin")] {
            s.undelete(b)
                .unwrap_or_else(|e| panic!("{}: block {b}: {e}", disk.display()));
        }
        fs::write(disk, s.bytes()).unwrap();
    }

    let mut commands = String::new();
    for v in ["OFS", "FFS", "DC"] {
        let _ = write!(
            commands,
            "C:Copy {v}:small.bin SYS:out/{v}-small\nC:Copy {v}:big.bin SYS:out/{v}-big\n\
             C:Copy {v}:Drawer/inner.bin SYS:out/{v}-inner\n\
             C:Copy SYS:in/small {v}:Drawer/again.bin\n"
        );
    }
    let outcome = run(
        &setup,
        "undelete-3",
        &paths,
        &inputs,
        &commands,
        Duration::from_secs(90),
    )
    .unwrap();
    for v in ["OFS", "FFS", "DC"] {
        for (name, data) in [("small", &small), ("big", &big), ("inner", &inner)] {
            let got = fs::read(outcome.out.join(format!("{v}-{name}")))
                .unwrap_or_else(|_| panic!("{v}: AmigaDOS did not find {name} after undelete"));
            assert!(&got == data, "{v}: {name} came back with different bytes");
        }
    }
    for (disk, v) in disks.iter().zip(["OFS", "FFS", "DC"]) {
        let bytes = fs::read(disk).unwrap();
        let health = ade_core::health::examine(bytes.clone());
        let faults: Vec<String> = health
            .findings
            .iter()
            .filter(|f| f.severity >= ade_core::Severity::Warning)
            .map(|f| format!("{}: {}", f.code, f.message))
            .collect();
        assert!(faults.is_empty(), "{v}: {faults:?}");
        let image = Image::from_bytes(bytes).unwrap();
        let volume = image.volume().unwrap();
        let again = volume.lookup("Drawer/again.bin").unwrap();
        assert!(volume.read_file(&again).unwrap().bytes == small, "{v}");
    }
}

/// Faults the health report finds on `disk`, warnings and worse.
fn faults_on(disk: &Path) -> Vec<String> {
    ade_core::health::examine(fs::read(disk).unwrap())
        .findings
        .iter()
        .filter(|f| f.severity >= ade_core::Severity::Warning)
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect()
}

#[test]
fn hard_links_ade_makes_are_ones_amigados_reads_and_deletes() {
    // ADE links to a file from the root, from a directory and under an
    // accented name, and to a directory. AmigaDOS reads through every one,
    // then deletes the file — promoting the newest link, in a chain ADE
    // wrote — a link, and the directory link. ADE then finds nothing wrong
    // and reads what is left.
    let Some(setup) = setup() else { return };
    let dir = scratch("links-ade");
    let target = pattern(3_000, 9);
    let other = pattern(400, 10);
    let mut disks = Vec::new();
    for (flags, label) in [(0u8, "OFS"), (1, "FFS"), (5, "DC")] {
        let blank = format::blank(Geometry::DD_FLOPPY, dostype(flags), label, WHEN).unwrap();
        let mut s = Session::from_bytes(blank).unwrap();
        s.set_clock(Some(WHEN));
        let root = s.root();
        let file = s.add_file(root, b"target.bin", &target).unwrap();
        let sub = s.make_dir(root, b"Sub").unwrap();
        s.add_file(sub, b"other.bin", &other).unwrap();
        s.make_link(file, root, b"link.bin").unwrap();
        s.make_link(file, sub, b"deep.bin").unwrap();
        s.make_link(sub, root, b"dirlink").unwrap();
        // Made last, so the newest: the one AmigaDOS promotes.
        s.make_link(file, root, b"l\xefnk.bin").unwrap();
        disks.push(floppy(&dir, &format!("{label}.adf"), s.bytes()));
    }
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    let mut commands = String::new();
    for v in ["OFS", "FFS", "DC"] {
        let _ = write!(
            commands,
            "C:Copy {v}:link.bin SYS:out/{v}-link\nC:Copy {v}:Sub/deep.bin SYS:out/{v}-deep\n\
             C:Copy \"{v}:lïnk.bin\" SYS:out/{v}-accent\nC:Copy {v}:dirlink/other.bin SYS:out/{v}-other\n\
             C:Delete {v}:target.bin\nC:Delete {v}:link.bin\nC:Delete {v}:dirlink\n"
        );
    }
    let outcome = run(
        &setup,
        "links-ade",
        &paths,
        &[],
        &commands,
        Duration::from_secs(90),
    )
    .unwrap();
    for v in ["OFS", "FFS", "DC"] {
        for (name, want) in [
            ("link", &target),
            ("deep", &target),
            ("accent", &target),
            ("other", &other),
        ] {
            let got = fs::read(outcome.out.join(format!("{v}-{name}")))
                .unwrap_or_else(|_| panic!("{v}: AmigaDOS could not read through {name}"));
            assert!(&got == want, "{v}: {name} read different bytes");
        }
    }
    for (disk, v) in disks.iter().zip(["OFS", "FFS", "DC"]) {
        assert!(faults_on(disk).is_empty(), "{v}: {:?}", faults_on(disk));
        let image = Image::from_bytes(fs::read(disk).unwrap()).unwrap();
        let volume = image.volume().unwrap();
        // The newest link took the file's place; the other still reaches it.
        for path in ["lïnk.bin", "Sub/deep.bin"] {
            let entry = volume
                .lookup(path)
                .unwrap_or_else(|e| panic!("{v}: {path}: {e}"));
            assert!(
                volume.read_file(&entry).unwrap().bytes == target,
                "{v}: {path}"
            );
        }
        assert!(volume.lookup("target.bin").is_err() && volume.lookup("dirlink").is_err());
    }
}

#[test]
fn hard_links_amigados_makes_are_ones_ade_deletes_as_amigados_would() {
    // AmigaDOS links; ADE deletes the file, promoting the newest link, and an
    // older link; AmigaDOS must then read the promoted file by name and write
    // beside it, and ADE find nothing wrong.
    let Some(setup) = setup() else { return };
    let dir = scratch("links-amiga");
    let target = pattern(3_000, 11);
    let mut disks = Vec::new();
    for (flags, label) in [(0u8, "OFS"), (1, "FFS"), (5, "DC")] {
        let blank = format::blank(Geometry::DD_FLOPPY, dostype(flags), label, WHEN).unwrap();
        disks.push(floppy(&dir, &format!("{label}.adf"), &blank));
    }
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    let mut made = String::new();
    for v in ["OFS", "FFS", "DC"] {
        let _ = write!(
            made,
            "C:Copy SYS:in/target {v}:target.bin\nC:MakeDir {v}:Sub\n\
             C:MakeLink {v}:link.bin {v}:target.bin HARD\n\
             C:MakeLink {v}:Sub/deep.bin {v}:target.bin HARD\n"
        );
    }
    let inputs: [(&str, &[u8]); 1] = [("target", &target)];
    run(
        &setup,
        "links-amiga-1",
        &paths,
        &inputs,
        &made,
        Duration::from_secs(90),
    )
    .unwrap();

    for disk in &disks {
        let mut s = Session::from_bytes(fs::read(disk).unwrap()).unwrap();
        s.set_clock(Some(WHEN));
        // `deep.bin` is the newest, so it takes the file's place.
        let file = s.entry("target.bin").unwrap();
        s.delete(file)
            .unwrap_or_else(|e| panic!("{}: {e}", disk.display()));
        let link = s.entry("link.bin").unwrap();
        s.delete(link)
            .unwrap_or_else(|e| panic!("{}: {e}", disk.display()));
        fs::write(disk, s.bytes()).unwrap();
    }

    let mut read = String::new();
    for v in ["OFS", "FFS", "DC"] {
        let _ = write!(
            read,
            "C:Copy {v}:Sub/deep.bin SYS:out/{v}-deep\nC:Copy SYS:in/target {v}:Sub/beside.bin\n"
        );
    }
    let outcome = run(
        &setup,
        "links-amiga-2",
        &paths,
        &inputs,
        &read,
        Duration::from_secs(90),
    )
    .unwrap();
    for (disk, v) in disks.iter().zip(["OFS", "FFS", "DC"]) {
        let got = fs::read(outcome.out.join(format!("{v}-deep")))
            .unwrap_or_else(|_| panic!("{v}: AmigaDOS did not find the promoted file"));
        assert!(
            got == target,
            "{v}: the promoted file reads different bytes"
        );
        assert!(faults_on(disk).is_empty(), "{v}: {:?}", faults_on(disk));
    }
}

#[test]
fn a_run_that_times_out_keeps_a_screenshot_of_the_amiga() {
    // IMP-022: a script that never leaves its marker, bounded tightly. The
    // failure must name a screenshot of the emulator's window when the tools
    // to take one are there, and say why not when they are not.
    let Some(setup) = setup() else { return };
    let failed = run(
        &setup,
        "timeout",
        &[],
        &[],
        "C:Wait 600\n",
        Duration::from_secs(25),
    )
    .err()
    .expect("a script that waits ten minutes cannot finish in 25 seconds");
    assert!(failed.contains("no marker"), "{failed}");
    let tools = ["xwininfo", "import"].iter().all(|t| {
        Command::new(t)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    });
    if tools && !setup.xvfb {
        let path = failed
            .split("the screen is in ")
            .nth(1)
            .and_then(|rest| rest.split(';').next())
            .unwrap_or_else(|| panic!("no screenshot named: {failed}"));
        let png = fs::read(path).unwrap();
        assert!(png.starts_with(b"\x89PNG"), "{path} is not a PNG");
    } else {
        assert!(failed.contains("no screenshot"), "{failed}");
    }
}

#[test]
fn large_hardfiles_need_a_bitmap_chain_and_amigados_reads_and_writes_both_ways() {
    // Past 25 bitmap blocks — a hard disk above about 50 MB — the rest are
    // listed in a `bm_ext` chain, which ADE now writes. A 330 MB hardfile has
    // 167 bitmap blocks and two extension blocks; the blocks around the
    // rootblock, where both AmigaDOS and ADE allocate first, are mapped by
    // pages only the chain lists. ADE makes it and writes a file; AmigaDOS
    // reads it and writes its own. AmigaDOS formats a 100 MB one; ADE writes
    // a file onto it; AmigaDOS reads that.
    let Some(setup) = setup() else { return };
    let dir = scratch("bighd");
    fs::create_dir_all(&dir).unwrap();
    let payload = pattern(300_000, 12);
    let theirs = pattern(200_000, 13);

    let shape = ade_core::create::Shape::Hard(330);
    let geometry = shape.geometry().unwrap();
    let blank = format::blank(geometry, dostype(1), "BigF", WHEN).unwrap();
    let mut s = Session::from_bytes(blank).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    let sub = s.make_dir(root, b"Dir").unwrap();
    s.add_file(sub, b"payload.bin", &payload).unwrap();
    let ade_made = dir.join("ade330.hdf");
    fs::write(&ade_made, s.bytes()).unwrap();
    drop(s);
    let zeros = dir.join("zeros100.hdf");
    fs::File::create(&zeros)
        .unwrap()
        .set_len(100 * 1024 * 1024)
        .unwrap();

    let commands = "SYS:System/Format DRIVE DH2: NAME Fmt FFS NOICONS QUICK <S:cr >NIL:\n\
                    C:Copy BigF:Dir/payload.bin SYS:out/payload\n\
                    C:Copy SYS:in/theirs BigF:theirs.bin\n\
                    C:MakeDir BigF:Theirs\n";
    let outcome = run_with(
        &setup,
        "bighd-1",
        (&[], &[&ade_made, &zeros]),
        &[("theirs", &theirs)],
        commands,
        Duration::from_secs(120),
    )
    .unwrap();
    let got = fs::read(outcome.out.join("payload")).unwrap();
    assert!(
        got == payload,
        "AmigaDOS read different bytes from ADE's 330 MB hardfile"
    );
    assert!(
        faults_on(&ade_made).is_empty(),
        "{:?}",
        faults_on(&ade_made)
    );
    let image = Image::from_bytes(fs::read(&ade_made).unwrap()).unwrap();
    let volume = image.volume().unwrap();
    let entry = volume.lookup("theirs.bin").unwrap();
    assert!(volume.read_file(&entry).unwrap().bytes == theirs);
    assert!(volume.lookup("Theirs").is_ok());
    drop(image);

    // AmigaDOS's own 100 MB volume: ADE finds it sound, writes a file onto
    // it, and AmigaDOS reads that back.
    assert!(faults_on(&zeros).is_empty(), "{:?}", faults_on(&zeros));
    let mut s = Session::from_bytes(fs::read(&zeros).unwrap()).unwrap();
    s.set_clock(Some(WHEN));
    let root = s.root();
    s.add_file(root, b"from-ade.bin", &payload).unwrap();
    fs::write(&zeros, s.bytes()).unwrap();
    drop(s);
    let outcome = run_with(
        &setup,
        "bighd-2",
        (&[], &[&ade_made, &zeros]),
        &[],
        "C:Copy Fmt:from-ade.bin SYS:out/from-ade\n",
        Duration::from_secs(90),
    )
    .unwrap();
    let got = fs::read(outcome.out.join("from-ade")).unwrap();
    assert!(
        got == payload,
        "AmigaDOS read different bytes from its own volume after ADE wrote"
    );
    assert!(faults_on(&zeros).is_empty(), "{:?}", faults_on(&zeros));
}
