//! Write a standard test image to a path.
//!
//! Exists so that things which are not Rust — the C ABI smoke test, and CI
//! running it — can get a real image to work on. D-010 commits no binaries, so
//! anything needing one has to generate it, and this is the one-line way to do
//! that from a shell.
//!
//! Usage: `mkfixture <path.adf> [--device]`
//!
//! `--device` writes a two-partition hard disk instead, for anything that
//! needs a Rigid Disk Block — a device holds no volume of its own, so it is a
//! different shape rather than a bigger floppy.
//!
//! `--datfile <path>` also writes a one-entry TOSEC-style datfile naming the
//! image it just generated. Anything testing identification needs a dataset
//! that matches its fixture, and computing the CRC32 by hand in a test would
//! be reimplementing the thing under test.

#![allow(
    clippy::print_stderr,
    clippy::exit,
    reason = "a command-line tool reports to a person"
)]

use ade_fixtures::{Volume, device::Device};

fn main() -> std::process::ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: mkfixture <path.adf> [--device]");
        return std::process::ExitCode::from(2);
    };
    let device = std::env::args().any(|a| a == "--device");
    let datfile = std::env::args()
        .skip_while(|a| a != "--datfile")
        .nth(1)
        .map(std::path::PathBuf::from);

    if device {
        // Two partitions, the first bootable, so a front end has something
        // with more than one volume to show.
        let mut disk = Device::new(64, 4, 32);
        // `readme` is deliberately in both: one name in two volumes of one
        // disk is the case a partition-blind reader gets wrong, and the case a
        // search result must distinguish.
        disk.add_partition("DH0", 2, 30, 1, true, |v| {
            v.add_file("startup-sequence", b"hello from DH0");
            v.add_file("readme", b"this is DH0");
            v.add_dir("Tools");
        });
        disk.add_partition("DH1", 31, 63, 0, false, |v| {
            v.add_file("data.bin", &[0xA5u8; 4096]);
            v.add_file("readme", b"this is DH1");
        });
        return match std::fs::write(&path, disk.build()) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("mkfixture: {path}: {e}");
                std::process::ExitCode::from(1)
            }
        };
    }

    // The same shape the C test expects: a mountable volume with a directory
    // and a file whose contents it can check.
    let mut volume = Volume::dd(1).named("Fixture");
    volume.add_file("startup", b"hello from a generated fixture");
    volume.add_file("data.bin", &[0xA5u8; 4096]);
    volume.add_dir("Tools");

    let bytes = volume.build();
    if let Some(dat) = &datfile
        && let Err(e) = write_datfile(dat, &bytes)
    {
        eprintln!("mkfixture: {}: {e}", dat.display());
        return std::process::ExitCode::from(1);
    }

    match std::fs::write(&path, bytes) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mkfixture: {path}: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

/// A one-entry datfile naming the image, matched the way ADE matches: CRC32
/// and size.
fn write_datfile(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let dat = format!(
        "<datafile><game name=\"Fixture\"><rom name=\"A Named Disk.adf\" size=\"{}\" \
         crc=\"{:08x}\"/></game></datafile>",
        bytes.len(),
        crc32(bytes)
    );
    std::fs::write(path, dat)
}

/// CRC32, implemented here rather than borrowed from `ade-block`.
///
/// D-010 keeps the fixture generator dependent on nothing: it is an
/// independent statement of what a correct disk looks like, so a misreading in
/// a layer crate cannot cancel out against it. That applies to the checksum a
/// datfile is matched on as much as to the disk — if this and `ade-block`
/// disagree, the identification test fails, which is precisely the signal the
/// independence exists to give. Fifteen lines is a cheap price for it.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
