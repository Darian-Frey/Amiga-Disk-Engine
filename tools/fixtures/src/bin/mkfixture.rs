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

    match std::fs::write(&path, volume.build()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mkfixture: {path}: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
