//! ProTracker modules in the Decoded view (F-033), across the corpus, against
//! libopenmpt.
//!
//! The layout is the format's own; the unit tests in `ade_object::tracker`
//! build modules to it. libopenmpt, through `ffprobe`, is an independent
//! reading of the same files: their title, their sample names, and how long
//! they play — which ADE works out by walking the patterns' speed, tempo,
//! jump, break, loop and delay commands, and which a misread pattern or
//! effect would change. Measured 2026-10-29: 359 corpus modules, every title
//! and every playing time within 0.05 seconds. Local only; skips without the
//! corpus or without `ffprobe`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test scaffolding"
)]

use std::path::PathBuf;
use std::process::Command;

use ade_core::layers::object::tracker;

/// What libopenmpt says of a module: its title, its sample names and how
/// long it plays. `None` when `ffprobe` cannot be run.
fn libopenmpt(bytes: &[u8], scratch: &std::path::Path) -> Option<(String, Vec<String>, f64)> {
    let file = scratch.join("m.mod");
    std::fs::write(&file, bytes).ok()?;
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration:format_tags=title,comment",
        ])
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(&file)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut title = String::new();
    let mut seconds = f64::NAN;
    let mut names = Vec::new();
    let mut in_comment = false;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("TAG:title=") {
            v.clone_into(&mut title);
            in_comment = false;
        } else if let Some(v) = line.strip_prefix("TAG:comment=") {
            names.push(v.to_owned());
            in_comment = true;
        } else if let Some(v) = line.strip_prefix("duration=") {
            seconds = v.parse().unwrap_or(f64::NAN);
            in_comment = false;
        } else if in_comment {
            names.push(line.to_owned());
        }
    }
    Some((title, names, seconds))
}

#[test]
fn corpus_modules_agree_with_libopenmpt() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    if Command::new("ffprobe").arg("-version").output().is_err() {
        eprintln!("skipping: no ffprobe");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("ade-modules-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut modules, mut title_agree, mut time_agree, mut name_agree) =
        (0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !tracker::is_module(&bytes) {
                continue;
            }
            let m = tracker::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let _ = m.fields();
            modules += 1;
            let Some((title, theirs, seconds)) = libopenmpt(&bytes, &scratch) else {
                continue;
            };
            assert_eq!(m.title, title.trim_end(), "{}: {name}", path.display());
            title_agree += 1;
            assert!(
                (m.seconds - seconds).abs() <= 0.05,
                "{}: {name} plays {:.2}s, libopenmpt says {seconds:.2}s",
                path.display(),
                m.seconds
            );
            time_agree += 1;
            // libopenmpt shows the bytes after a name's NUL as spaces, and a
            // name holding a line break splits its list; ADE stops at the
            // NUL, as an Amiga string does.
            // The list ends with a line break of its own.
            let mut names = theirs;
            while names.len() > m.samples.len() && names.last().is_some_and(String::is_empty) {
                names.pop();
            }
            if names.is_empty() {
                // libopenmpt lists nothing when every name is empty.
                assert!(
                    m.samples.iter().all(|s| s.name.is_empty()),
                    "{}: {name}",
                    path.display()
                );
                name_agree += 1;
            } else if names.len() == m.samples.len() {
                for (theirs, ours) in names.iter().zip(&m.samples) {
                    assert!(
                        theirs.trim_end().starts_with(ours.name.trim_end()),
                        "{}: {name}: {theirs:?} against {:?}",
                        path.display(),
                        ours.name
                    );
                }
                name_agree += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!(
        "{modules} modules: {title_agree} titles, {time_agree} playing times, {name_agree} name lists agree"
    );
    assert!(modules >= 359, "{modules}");
    assert_eq!((title_agree, time_agree), (modules, modules));
    assert_eq!(name_agree, modules);
}
