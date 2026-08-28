//! Every command, piped into something that stops reading (BUG-008).
//!
//! # What this catches, and what it does not
//!
//! It catches a command that writes with `println!` instead of going through
//! `emit`: the former panics on a closed pipe, exits 101, and this notices.
//! That is the regression net, because the defect it guards against has now
//! occurred twice — IMP-001 fixed `info` and `ls` in August, and six commands
//! written afterwards each reintroduced it.
//!
//! It is a net rather than a proof. Output that fits the pipe buffer can be
//! written in full before the reader closes, so a small command may pass here
//! whether or not it is correct. The deterministic check on the mechanism is
//! in `main.rs`'s own tests, against a writer that fails on demand; this one
//! exists to notice a *new* command that forgot.
//!
//! `--help` and `--version` are included even though they are short, because
//! the point is that no command is exempt from the rule.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::process::Command;

/// Rust's panic exit code. A command that dies this way on a closed pipe is
/// the bug.
const PANIC: i32 = 101;

/// Run `ade <args> | head -1` and return the exit code of `ade` itself.
///
/// `pipefail` is what makes the left-hand side's status visible; without it
/// the pipeline reports `head`'s, which is always zero and would hide exactly
/// what this test is looking for. `bash` rather than `sh` because `dash` has
/// no `pipefail`.
fn through_a_closing_pipe(args: &[&str]) -> i32 {
    let ade = env!("CARGO_BIN_EXE_ade");
    let quoted: Vec<String> = args.iter().map(|a| format!("'{a}'")).collect();
    let script = format!("set -o pipefail; '{ade}' {} | head -1", quoted.join(" "));
    let out = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("bash should run");
    out.status.code().unwrap_or(PANIC)
}

#[test]
fn no_command_panics_when_its_reader_stops_reading() {
    // Commands needing no image, which is every one whose output is a fixed
    // report. The ones that take images are covered by the corpus tests, which
    // pipe their output too.
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["--version"],
        vec!["-V"],
        vec!["formats"],
        // An unrecognised command prints usage and exits 2; it writes to the
        // same stdout as everything else.
        vec!["not-a-command"],
    ] {
        let code = through_a_closing_pipe(&args);
        assert_ne!(
            code,
            PANIC,
            "`ade {}` panicked when its output was not read",
            args.join(" ")
        );
    }
}

#[test]
fn a_closed_pipe_does_not_change_what_a_command_reports() {
    // The exit code is a contract (F-015), and "the reader went away" is not
    // one of the things it reports. `formats` is clean, an unknown command is
    // a usage error, and both must say so regardless of who is listening.
    assert_eq!(through_a_closing_pipe(&["formats"]), 0);
    assert_eq!(through_a_closing_pipe(&["--help"]), 0);
    assert_eq!(through_a_closing_pipe(&["not-a-command"]), 2);
}
