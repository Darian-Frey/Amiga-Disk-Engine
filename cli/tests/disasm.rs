//! `ade disasm` and the views beside its listing (F-032, D-021): the
//! engine's tests check the walks and the export; these check that the
//! command reaches each view by its flag, and refuses what it cannot do.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    reason = "integration tests over files they build by hand"
)]

use std::{fs, path::PathBuf, process::Command};

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A program that finds exec at address 4, opens dos.library by name, and
/// calls `Output` through the base it was given.
fn program() -> Vec<u8> {
    let mut code: Vec<u8> = [
        0x2C78u16, 0x0004, // movea.l $4.w,a6
        0x43FA, 0x0012, // lea $18(pc),a1
        0x7000, // moveq #0,d0
        0x4EAE, 0xFDD8, // jsr -552(a6): OpenLibrary
        0x2C40, // movea.l d0,a6
        0x4EAE, 0xFFC4, // jsr -60(a6): Output
        0x4E75, 0x4E71, // rts; nop
    ]
    .iter()
    .flat_map(|w| w.to_be_bytes())
    .collect();
    code.extend_from_slice(b"dos.library\0");
    let longs = u32::try_from(code.len() / 4).unwrap();
    let mut out = Vec::new();
    for l in [0x3F3u32, 0, 1, 0, 0, longs, 0x3E9, longs] {
        out.extend(l.to_be_bytes());
    }
    out.extend(code);
    out.extend(0x3F2u32.to_be_bytes());
    out
}

#[test]
fn each_view_comes_by_its_flag() {
    let dir = std::env::temp_dir().join(format!("ade-disasm-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let blank = dir.join("blank.adf");
    let disk = dir.join("disk.adf");
    fs::write(dir.join("prog"), program()).unwrap();
    let s = |p: &PathBuf| p.to_str().unwrap().to_owned();
    assert_eq!(run(&["create", &s(&blank)]).2, Some(0));
    let (_, err, code) = run(&[
        "put",
        &s(&blank),
        &s(&dir.join("prog")),
        &format!("--output={}", s(&disk)),
    ]);
    assert_eq!(code, Some(0), "{err}");
    let d = s(&disk);

    // The listing names what it proves.
    let (text, err, code) = run(&["disasm", &d, "prog"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        text.contains("; ---- function: the program starts here"),
        "{text}"
    );
    assert!(text.contains("jsr     _LVOOpenLibrary(a6)"), "{text}");
    assert!(
        text.contains("; call exec OpenLibrary \"dos.library\""),
        "{text}"
    );
    assert!(text.contains("jsr     _LVOOutput(a6)"), "{text}");

    let (text, err, code) = run(&["disasm", &d, "prog", "--functions"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("h0_0000 (entry)"), "{text}");
    assert!(
        text.contains("h0_000a     exec OpenLibrary \"dos.library\""),
        "{text}"
    );
    assert!(text.contains("h0_0010     dos Output"), "{text}");

    let (text, err, code) = run(&["disasm", &d, "prog", "--xrefs"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("address from h0_0004"), "{text}");

    let (text, err, code) = run(&["disasm", &d, "prog", "--call-graph"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("  - dos: Output"), "{text}");
    let (text, _, code) = run(&["disasm", &d, "prog", "--call-graph=dot"]);
    assert_eq!(code, Some(0));
    assert!(text.starts_with("digraph calls {"), "{text}");

    let (text, err, code) = run(&["disasm", &d, "prog", "--export"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.contains("_LVOOpenLibrary\tequ\t-552"), "{text}");
    assert!(text.contains("\tSECTION\thunk0,CODE"), "{text}");
    assert!(text.contains("\tjsr     _LVOOutput(a6)"), "{text}");

    // Refused, not ignored (BUG-007): a guess has no start to follow, one
    // view at a time, and boot code has no source.
    let (_, err, code) = run(&["disasm", &d, "prog", "--raw", "--functions"]);
    assert_ne!(code, Some(0));
    assert!(err.contains("--raw"), "{err}");
    let (_, err, code) = run(&["disasm", &d, "prog", "--functions", "--xrefs"]);
    assert_ne!(code, Some(0));
    assert!(err.contains("one view"), "{err}");
    let (_, err, code) = run(&["disasm", &d, "--export"]);
    assert_ne!(code, Some(0));
    assert!(err.contains("not a program file"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}
