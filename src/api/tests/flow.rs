//! Functions, references and library calls on real programs (D-021).
//!
//! The library naming is checked by what it predicts. A walk that names a
//! call exec's `OpenLibrary` has proved two things — that `a6` held exec's
//! base and that the offset is `OpenLibrary`'s — and when it also knows
//! where `a1` points, the bytes there must be a library's name. A wrong base,
//! a slot off by one, or a wrong address for `a1` would each show as a name
//! that is not one.
//!
//! Measured 2026-10-29 over all 39,363 corpus programs: 675,689 functions
//! and 118,397 library calls, 118,396 of them at a slot the table names;
//! 11,640 `OpenLibrary` names read, 11,634 of them a library's. The other six
//! are an empty string and a scrambled one — buffers a program fills before
//! the call — and since they name no library, no base comes of them. The
//! one unnamed call is intuition's −30, a slot AmigaOS keeps private
//! (`OpenIntuition`). Before calls above a base were set aside, 28 more
//! were "named" there — `jsr $3c(a6)` reaches exec's data, not a function:
//! walks that had followed a branch into data. Local only; skips without the
//! corpus. `ADE_DISASM_CORPUS=all` takes every program; by default every
//! 10th.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::Image;
use ade_core::layers::object::flow;
use ade_core::layers::object::hunk;

#[test]
fn library_calls_on_corpus_programs_name_what_they_open() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let step = if std::env::var("ADE_DISASM_CORPUS").as_deref() == Ok("all") {
        1
    } else {
        10
    };
    let (mut programs, mut seen, mut functions, mut calls, mut named) = (0usize, 0, 0, 0, 0);
    let (mut opens, mut library_names) = (0usize, 0usize);
    let mut by_library: BTreeMap<&str, usize> = BTreeMap::new();
    let mut odd: BTreeMap<String, usize> = BTreeMap::new();
    let mut unnamed: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(image) = Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (_, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !hunk::is_executable(&bytes) {
                continue;
            }
            seen += 1;
            if seen % step != 0 {
                continue;
            }
            let Some(a) = flow::executable(&bytes) else {
                continue;
            };
            programs += 1;
            functions += a.functions.len();
            for c in &a.library_calls {
                calls += 1;
                *by_library.entry(c.library.short()).or_default() += 1;
                if c.function.is_some() {
                    named += 1;
                } else {
                    *unnamed.entry(c.describe()).or_default() += 1;
                }
                if let Some(name) = &c.opens {
                    opens += 1;
                    if name.ends_with(".library") {
                        library_names += 1;
                    } else {
                        *odd.entry(name.clone()).or_default() += 1;
                    }
                }
            }
        }
    }
    eprintln!(
        "{programs} programs, {functions} functions, {calls} library calls, {named} named; \
         {opens} OpenLibrary names known, {library_names} of them a library's"
    );
    eprintln!("by library: {by_library:?}");
    eprintln!("names that are not a library's: {odd:?}");
    let mut un: Vec<_> = unnamed.into_iter().collect();
    un.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!(
        "unnamed offsets: {:?}",
        un.iter().take(20).collect::<Vec<_>>()
    );
    assert!(programs > 0 && opens > 0);
    // A wrong base, a slot off by one or a wrong `a1` would each show here.
    assert!(
        library_names * 1000 >= opens * 999,
        "{library_names} of {opens} names a library's: {odd:?}"
    );
    assert!(named * 1000 >= calls * 999, "{named} of {calls} named");
}
