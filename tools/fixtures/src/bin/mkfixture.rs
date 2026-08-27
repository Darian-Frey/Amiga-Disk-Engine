//! Write a standard test image to a path.
//!
//! Exists so that things which are not Rust — the C ABI smoke test, and CI
//! running it — can get a real image to work on. D-010 commits no binaries, so
//! anything needing one has to generate it, and this is the one-line way to do
//! that from a shell.
//!
//! Usage: `mkfixture <path.adf>`

#![allow(
    clippy::print_stderr,
    clippy::exit,
    reason = "a command-line tool reports to a person"
)]

use ade_fixtures::Volume;

fn main() -> std::process::ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: mkfixture <path.adf>");
        return std::process::ExitCode::from(2);
    };

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
