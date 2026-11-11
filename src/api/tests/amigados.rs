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
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .status();
    // And the child itself, should the group kill not have reached it: a
    // harness that waits on a live emulator never returns.
    let _ = child.kill();
    let _ = child.wait();

    let log = fs::read_to_string(dir.join("base/Cache/Logs/fs-uae.log.txt")).unwrap_or_default();
    let resets = log.matches("Reset at").count();
    let illegal = log.matches("Illegal instruction").count();
    if !done.exists() {
        let tail: Vec<&str> = log.lines().rev().take(25).collect();
        return Err(format!(
            "no marker after {:?}: {resets} resets, {illegal} illegal instructions; \
             the log ends\n{}",
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
#[ignore = "BUG-024: a full format's fill reads as data"]
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
