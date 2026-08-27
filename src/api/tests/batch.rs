//! Corpus-scale batch operations (Phase 5, F-014).
//!
//! The workflow this project exists for is not one disk but thousands. Every
//! corpus measurement in SPEC was taken by hand-rolling a script; this is that
//! capability made part of the tool, and these tests pin the properties that
//! make a four-thousand-image run trustworthy.
//!
//! The one that matters most: **nothing aborts the run.** A run that stops at
//! the first unreadable file has told you about one file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "tests over data they construct"
)]

use std::path::PathBuf;

use ade_core::batch::{examine_one, run};
use ade_fixtures::{Volume as Fixture, corrupt};

/// A directory of images built for one test.
struct Corpus {
    dir: PathBuf,
}

impl Corpus {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("ade-batch-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create dir");
        Self { dir }
    }

    fn add(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.dir.join(name), bytes).expect("write image");
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A sound, mountable image.
fn good(name: &str) -> Vec<u8> {
    let mut v = Fixture::dd(1).named(name);
    v.add_file("startup", b"hello");
    v.add_dir("Tools");
    v.build()
}

#[test]
fn a_directory_of_images_is_summarised() {
    let corpus = Corpus::new("basic");
    corpus.add("a.adf", &good("Alpha"));
    corpus.add("b.adf", &good("Beta"));
    corpus.add("c.adf", &good("Gamma"));

    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    assert_eq!(summary.examined, 3);
    assert_eq!(summary.mounted, 3);
    assert_eq!(summary.sound, 3);
    assert_eq!(summary.unreadable, 0);
    assert!(summary.bytes_recovered > 0);
    assert_eq!(summary.containers.values().sum::<usize>(), 3);
}

#[test]
fn an_unreadable_file_is_a_record_not_an_abort() {
    // The property the whole feature rests on. A run over four thousand disks
    // that stops at the first bad one has told you about one disk.
    let corpus = Corpus::new("unreadable");
    corpus.add("good.adf", &good("Fine"));
    corpus.add("rubbish.adf", b"not a disk image at all");
    corpus.add("also-good.adf", &good("AlsoFine"));

    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    assert_eq!(summary.examined, 3, "every file must be reached");
    assert_eq!(summary.mounted, 2);
    // The rubbish file is short but readable, so it is examined and simply
    // yields no volume — which is itself the honest answer.
    assert!(summary.records.iter().any(|r| r.volume.is_none()));
}

#[test]
fn a_missing_file_is_recorded_with_its_reason() {
    let record = examine_one(std::path::Path::new("/nonexistent/definitely/not/here.adf"));

    assert!(record.unreadable.is_some(), "the reason must be kept");
    assert!(!record.is_sound());
    assert_eq!(record.files, 0);
}

#[test]
fn findings_are_counted_once_per_image_not_once_each() {
    // The bug this feature found in its own first run. A damaged disk can
    // report the same code dozens of times; summing those made 186 affected
    // images read as 1050, which is the most misleading thing a corpus report
    // could say.
    let corpus = Corpus::new("histogram");
    let mut damaged = good("Damaged");
    corrupt::bitmap_flag_invalid(&mut damaged, 880);
    corpus.add("one.adf", &damaged);

    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    for (code, count) in &summary.findings {
        assert!(
            *count <= summary.examined,
            "{code} counted {count} times across {} images",
            summary.examined
        );
    }
}

#[test]
fn a_damaged_image_is_not_counted_as_sound() {
    let corpus = Corpus::new("damaged");
    corpus.add("good.adf", &good("Good"));
    let mut damaged = good("Damaged");
    corrupt::bitmap_flag_invalid(&mut damaged, 880);
    corpus.add("bad.adf", &damaged);

    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    assert_eq!(summary.mounted, 2, "both still mount");
    assert_eq!(summary.sound, 1, "only one is sound");
    assert!(summary.findings.contains_key("bitmap-flag-clear"));
}

#[test]
fn results_are_in_a_deterministic_order() {
    // Two runs over one corpus must be comparable, and a failure reproducible.
    let corpus = Corpus::new("order");
    for name in ["z.adf", "a.adf", "m.adf"] {
        corpus.add(name, &good("Ordered"));
    }

    let first = run(std::slice::from_ref(&corpus.dir), |_, _| {});
    let second = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    let paths: Vec<&PathBuf> = first.records.iter().map(|r| &r.path).collect();
    let again: Vec<&PathBuf> = second.records.iter().map(|r| &r.path).collect();
    assert_eq!(paths, again);
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "records should be in sorted path order");
}

#[test]
fn progress_is_reported_for_every_image() {
    let corpus = Corpus::new("progress");
    for name in ["a.adf", "b.adf", "c.adf", "d.adf"] {
        corpus.add(name, &good("P"));
    }

    let mut seen = Vec::new();
    let summary = run(std::slice::from_ref(&corpus.dir), |done, total| {
        seen.push((done, total));
    });

    assert_eq!(summary.examined, 4);
    assert_eq!(seen, vec![(1, 4), (2, 4), (3, 4), (4, 4)]);
}

#[test]
fn individual_files_can_be_given_instead_of_a_directory() {
    let corpus = Corpus::new("files");
    corpus.add("a.adf", &good("One"));
    corpus.add("b.adf", &good("Two"));

    let summary = run(
        &[corpus.dir.join("a.adf"), corpus.dir.join("b.adf")],
        |_, _| {},
    );

    assert_eq!(summary.examined, 2);
}

#[test]
fn an_empty_run_summarises_nothing_rather_than_failing() {
    let corpus = Corpus::new("empty");
    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});

    assert_eq!(summary.examined, 0);
    assert!(summary.records.is_empty());
    assert!(summary.at_risk().is_empty());
}

#[test]
fn the_json_carries_counts_and_records() {
    let corpus = Corpus::new("json");
    corpus.add("a.adf", &good("Jason"));

    let summary = run(std::slice::from_ref(&corpus.dir), |_, _| {});
    let json = summary.to_json().to_json();

    for field in [
        "\"examined\":1",
        "\"mounted\":1",
        "\"containers\"",
        "\"findings\"",
    ] {
        assert!(json.contains(field), "missing {field}\n{json}");
    }
    let record = summary.records[0].to_json().to_json();
    assert!(record.contains("\"volume\":\"Jason\""), "{record}");
    assert!(record.contains("\"path\""), "{record}");
}

#[test]
fn the_whole_corpus_runs_in_one_pass() {
    // The acceptance criterion: thousands of images in one run with a
    // machine-readable summary. Skips cleanly without a corpus.
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    if !corpus.is_dir() {
        eprintln!("no corpus — skipping");
        return;
    }

    let summary = run(&[corpus], |_, _| {});

    eprintln!(
        "batch: {} examined, {} mounted, {} sound, {} unreadable, {} bytes recovered",
        summary.examined,
        summary.mounted,
        summary.sound,
        summary.unreadable,
        summary.bytes_recovered
    );

    assert!(summary.examined > 4000, "expected the corpus");
    assert_eq!(summary.records.len(), summary.examined);
    assert_eq!(
        summary.unreadable, 0,
        "every corpus image should be readable"
    );
    // Every histogram entry must be a real image count.
    for (code, count) in &summary.findings {
        assert!(*count <= summary.examined, "{code} over-counted");
    }
    assert!(summary.mounted > 3000);
    assert!(summary.sound > 2000);
}
